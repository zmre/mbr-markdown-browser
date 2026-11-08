//! Search date expressions: the values of the `date:`, `created:`,
//! `modified:` and `when:` search facets (design/calendar.md §6.1).
//!
//! ```text
//! expr   := range | cmp | rel | period
//! period := YYYY | YYYY-MM | YYYY-MM-DD | YYYY-Www | YYYY-Qn   the whole period
//! range  := [period] ".." [period]                             inclusive; one end may be open
//! cmp    := (">=" | ">" | "<=" | "<") period                    against the period's start/end
//! rel    := "today" | "yesterday" | "this-week" | "last-week" | "this-month"
//!         | "last-month" | "this-year" | "-" N ("d" | "w" | "m" | "y")
//! ```
//!
//! [`evaluate`] resolves an expression to a [`DateRange`] of calendar dates; the relative
//! forms resolve against a `today` the caller supplies (the search request's,
//! which is the reader's local date, else the server's).
//!
//! Choices this module makes, each pinned by `tests/fixtures/date-expr-cases.json`
//! (which the TypeScript mirror, `components/src/calendar/date-expr.ts`, is
//! held to as well):
//!
//! - **Weeks are ISO 8601** (Monday to Sunday), for `this-week` and
//!   `last-week` as for `2026-W44`. The `[calendar] week_start` setting only
//!   lays out grids: it is resolved in the browser from the reader's locale,
//!   which the server cannot see, and a query must mean the same thing on the
//!   server and in a static build.
//! - **`-N` units count back from today, today included.** `-7d` is today and
//!   the six days before it; `-1w` is the same; `-3m` starts the day after the
//!   same date three months ago (clamped to the month's end, so `-1m` on
//!   March 31 starts March 1); `-1y` is `-12m`. A start before year 1000 is
//!   open-ended. `N` is 1 to 6 digits.
//! - **A reversed range is swapped**: `2026-05..2026-02` is February to May.
//!   A bare `..` constrains nothing and is not an expression.
//! - The keywords and the `W`/`Q`/unit letters are ASCII case-insensitive;
//!   digits are ASCII only.
//! - **Nothing lies past 9999-12-31** (the last date a period, a note date or
//!   a date key can name). An end past it is clamped to it, and a range that
//!   *starts* past it (`>9999`, `>9999-12-31`, `>9999-W52`) is
//!   [`DateRange::EMPTY`]: it matches nothing. The TypeScript side compares
//!   date keys as strings, where `10000-01-01` would sort before `2026`, so
//!   both sides normalize here rather than let a five-digit year exist.
//!
//! Pure, no I/O.

use chrono::{Datelike, Days, LocalResult, Months, NaiveDate, NaiveTime, TimeDelta, TimeZone};

use crate::calendar::{Period, PeriodKind};

/// Longest value worth trying: `YYYY-MM-DD..YYYY-MM-DD` is 22 bytes.
const MAX_EXPR_LEN: usize = 32;

/// Most digits accepted in a `-N` count.
const MAX_COUNT_DIGITS: usize = 6;

/// A relative start earlier than this year is treated as open-ended.
const MIN_RELATIVE_YEAR: i32 = 1000;

/// The last date an expression can name; see the module docs.
const LAST_DATE: NaiveDate = NaiveDate::from_ymd_opt(9999, 12, 31).expect("valid date");

/// The first date a period can name; [`DateRange::EMPTY`]'s `to`.
const FIRST_DATE: NaiveDate = NaiveDate::from_ymd_opt(1000, 1, 1).expect("valid date");

/// An inclusive range of calendar dates; `None` is an open end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DateRange {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
}

impl DateRange {
    /// The range that matches nothing: an expression starting past
    /// 9999-12-31. `from` after `to`, so [`contains`](Self::contains) and
    /// [`instants`](Self::instants) need no special case; spelled with dates
    /// both languages can write (`components/src/calendar/date-expr.ts` has
    /// the same constant).
    pub const EMPTY: DateRange = DateRange {
        from: Some(LAST_DATE),
        to: Some(FIRST_DATE),
    };

    /// Whether no date lies in the range.
    pub fn is_empty(&self) -> bool {
        matches!((self.from, self.to), (Some(from), Some(to)) if from > to)
    }

    /// The canonical form: a start past [`LAST_DATE`] is [`Self::EMPTY`], an
    /// end past it is clamped to it.
    fn bounded(self) -> DateRange {
        if self.from.is_some_and(|from| from > LAST_DATE) {
            return DateRange::EMPTY;
        }
        DateRange {
            from: self.from,
            to: self.to.map(|to| to.min(LAST_DATE)),
        }
    }

    /// Whether `date` lies in the range.
    pub fn contains(&self, date: NaiveDate) -> bool {
        self.from.is_none_or(|from| date >= from) && self.to.is_none_or(|to| date <= to)
    }

    /// The same range as unix-second instants in `tz`: from the first instant
    /// of `from` up to (excluding) the first instant of the day after `to`.
    /// Computed once per query, so that a timestamp is checked with two
    /// integer compares instead of a time-zone conversion per file.
    pub fn instants<Tz: TimeZone>(&self, tz: &Tz) -> InstantRange {
        InstantRange {
            from: self.from.map(|d| start_of_day(d, tz)),
            until: self
                .to
                .and_then(|d| d.succ_opt())
                .map(|d| start_of_day(d, tz)),
        }
    }
}

/// [`DateRange::instants`]: `[from, until)` in unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InstantRange {
    from: Option<i64>,
    until: Option<i64>,
}

impl InstantRange {
    /// Whether the instant `secs` (unix seconds) falls on a date in the range.
    pub fn contains(&self, secs: u64) -> bool {
        let Ok(secs) = i64::try_from(secs) else {
            return false;
        };
        self.from.is_none_or(|from| secs >= from) && self.until.is_none_or(|until| secs < until)
    }
}

/// The first instant of `date` in `tz`, in unix seconds.
///
/// Local midnight does not exist on a day whose clocks jump forward at
/// midnight, and a whole day can be skipped (Samoa, 2011-12-30), so this
/// probes forward in 15-minute steps (every real transition is on a quarter
/// hour) for the first local time that exists. On a skipped day that is the
/// next day's first instant, which is still right for "on or after `date`".
fn start_of_day<Tz: TimeZone>(date: NaiveDate, tz: &Tz) -> i64 {
    let midnight = date.and_time(NaiveTime::MIN);
    (0..=(48 * 4))
        .find_map(|step| {
            let local = midnight + TimeDelta::minutes(15 * step);
            match tz.from_local_datetime(&local) {
                LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t.timestamp()),
                LocalResult::None => None,
            }
        })
        .unwrap_or_else(|| midnight.and_utc().timestamp())
}

/// `>=`, `>`, `<=` or `<`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comparison {
    AtOrAfter,
    After,
    AtOrBefore,
    Before,
}

/// The unit of a `-N` count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CountUnit {
    Days,
    Weeks,
    Months,
    Years,
}

/// The forms resolved against today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Relative {
    Today,
    Yesterday,
    ThisWeek,
    LastWeek,
    ThisMonth,
    LastMonth,
    ThisYear,
    /// `-N` + unit: the last `count` units, today included.
    Last {
        count: u32,
        unit: CountUnit,
    },
}

/// A parsed date expression. See the module docs for the grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateExpr {
    /// A whole period: `2026`, `2026-10`, `2026-10-31`, `2026-W44`, `2026-Q4`.
    Period(Period),
    /// `A..B`, `A..` or `..B`; at least one end is set, and `from` never
    /// starts after `to` ends (a reversed range is swapped when parsed).
    Range {
        from: Option<Period>,
        to: Option<Period>,
    },
    /// `>=P`, `>P`, `<=P`, `<P`.
    Compare(Comparison, Period),
    Relative(Relative),
}

impl DateExpr {
    /// Parses a facet value, or `None` when it is not a date expression.
    fn parse(value: &str) -> Option<DateExpr> {
        if value.is_empty() || value.len() > MAX_EXPR_LEN || !value.is_ascii() {
            return None;
        }
        if let Some((from, to)) = value.split_once("..") {
            return parse_range(from, to);
        }
        if let Some(rest) = value.strip_prefix(">=") {
            return parse_period(rest).map(|p| DateExpr::Compare(Comparison::AtOrAfter, p));
        }
        if let Some(rest) = value.strip_prefix('>') {
            return parse_period(rest).map(|p| DateExpr::Compare(Comparison::After, p));
        }
        if let Some(rest) = value.strip_prefix("<=") {
            return parse_period(rest).map(|p| DateExpr::Compare(Comparison::AtOrBefore, p));
        }
        if let Some(rest) = value.strip_prefix('<') {
            return parse_period(rest).map(|p| DateExpr::Compare(Comparison::Before, p));
        }
        parse_relative(value)
            .map(DateExpr::Relative)
            .or_else(|| parse_period(value).map(DateExpr::Period))
    }

    /// The dates the expression names, relative forms resolved against `today`.
    fn resolve(&self, today: NaiveDate) -> DateRange {
        self.resolve_unbounded(today).bounded()
    }

    fn resolve_unbounded(&self, today: NaiveDate) -> DateRange {
        match *self {
            DateExpr::Period(p) => period_range(p),
            DateExpr::Range { from, to } => DateRange {
                from: from.map(|p| p.start()),
                to: to.map(|p| p.end()),
            },
            DateExpr::Compare(op, p) => match op {
                Comparison::AtOrAfter => DateRange {
                    from: Some(p.start()),
                    to: None,
                },
                // `succ_opt` of 9999-12-31 is 10000-01-01, which `bounded`
                // turns into the empty range.
                Comparison::After => match p.end().succ_opt() {
                    Some(from) => DateRange {
                        from: Some(from),
                        to: None,
                    },
                    None => DateRange::EMPTY,
                },
                Comparison::AtOrBefore => DateRange {
                    from: None,
                    to: Some(p.end()),
                },
                Comparison::Before => DateRange {
                    from: None,
                    to: p.start().pred_opt(),
                },
            },
            DateExpr::Relative(rel) => resolve_relative(rel, today),
        }
    }
}

/// The dates a facet value names, relative forms resolved against `today`;
/// `None` when it is not a date expression (the facet is then an ordinary
/// substring facet). The module's one entry point: the parsed form never
/// leaves it.
pub fn evaluate(value: &str, today: NaiveDate) -> Option<DateRange> {
    DateExpr::parse(value).map(|expr| expr.resolve(today))
}

fn period_range(p: Period) -> DateRange {
    DateRange {
        from: Some(p.start()),
        to: Some(p.end()),
    }
}

/// A period key, with `W` and `Q` in either case.
fn parse_period(s: &str) -> Option<Period> {
    Period::parse_key(&s.to_ascii_uppercase())
}

fn parse_range(from: &str, to: &str) -> Option<DateExpr> {
    let side = |s: &str| {
        if s.is_empty() {
            Some(None)
        } else {
            parse_period(s).map(Some)
        }
    };
    let (from, to) = (side(from)?, side(to)?);
    match (from, to) {
        (None, None) => None,
        (Some(a), Some(b)) if a.start() > b.end() => Some(DateExpr::Range {
            from: Some(b),
            to: Some(a),
        }),
        _ => Some(DateExpr::Range { from, to }),
    }
}

fn parse_relative(s: &str) -> Option<Relative> {
    let lower = s.to_ascii_lowercase();
    let keyword = match lower.as_str() {
        "today" => Some(Relative::Today),
        "yesterday" => Some(Relative::Yesterday),
        "this-week" => Some(Relative::ThisWeek),
        "last-week" => Some(Relative::LastWeek),
        "this-month" => Some(Relative::ThisMonth),
        "last-month" => Some(Relative::LastMonth),
        "this-year" => Some(Relative::ThisYear),
        _ => None,
    };
    keyword.or_else(|| {
        let body = lower.strip_prefix('-')?;
        let (digits, unit) = body.split_at(body.len().checked_sub(1)?);
        let unit = match unit {
            "d" => CountUnit::Days,
            "w" => CountUnit::Weeks,
            "m" => CountUnit::Months,
            "y" => CountUnit::Years,
            _ => return None,
        };
        if digits.is_empty()
            || digits.len() > MAX_COUNT_DIGITS
            || !digits.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let count: u32 = digits.parse().ok()?;
        (count > 0).then_some(Relative::Last { count, unit })
    })
}

fn resolve_relative(rel: Relative, today: NaiveDate) -> DateRange {
    let day = |d: NaiveDate| DateRange {
        from: Some(d),
        to: Some(d),
    };
    let of = |d: NaiveDate, kind| period_range(Period::of_date(d, kind));
    match rel {
        Relative::Today => day(today),
        Relative::Yesterday => day(today.pred_opt().unwrap_or(today)),
        Relative::ThisWeek => of(today, PeriodKind::Week),
        Relative::LastWeek => of(
            today.checked_sub_days(Days::new(7)).unwrap_or(today),
            PeriodKind::Week,
        ),
        Relative::ThisMonth => of(today, PeriodKind::Month),
        Relative::LastMonth => of(
            today
                .with_day(1)
                .and_then(|first| first.checked_sub_months(Months::new(1)))
                .unwrap_or(today),
            PeriodKind::Month,
        ),
        Relative::ThisYear => of(today, PeriodKind::Year),
        Relative::Last { count, unit } => {
            let start = match unit {
                CountUnit::Days => today.checked_sub_days(Days::new(u64::from(count) - 1)),
                CountUnit::Weeks => today.checked_sub_days(Days::new(u64::from(count) * 7 - 1)),
                CountUnit::Months => months_back_inclusive(today, count),
                CountUnit::Years => count
                    .checked_mul(12)
                    .and_then(|months| months_back_inclusive(today, months)),
            };
            DateRange {
                from: start.filter(|d| d.year() >= MIN_RELATIVE_YEAR),
                to: Some(today),
            }
        }
    }
}

/// The day after the same date `months` months before `today` (clamped to a
/// shorter month's last day).
fn months_back_inclusive(today: NaiveDate, months: u32) -> Option<NaiveDate> {
    today
        .checked_sub_months(Months::new(months))
        .and_then(|d| d.succ_opt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};
    use proptest::prelude::*;
    use serde::Deserialize;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("test date")
    }

    #[derive(Deserialize)]
    struct CaseTable {
        today: String,
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        expr: String,
        today: Option<String>,
        #[serde(default)]
        invalid: bool,
        #[serde(default)]
        empty: bool,
        from: Option<String>,
        to: Option<String>,
    }

    /// The shared table `components/src/calendar/date-expr.test.ts` also reads.
    #[test]
    fn shared_case_table() {
        let raw = include_str!("../../../tests/fixtures/date-expr-cases.json");
        let table: CaseTable = serde_json::from_str(raw).expect("case table parses");
        assert!(
            table.cases.len() > 40,
            "the table is the spec; keep it full"
        );
        for case in &table.cases {
            let today = date(case.today.as_deref().unwrap_or(&table.today));
            let got = evaluate(&case.expr, today);
            if case.invalid {
                assert_eq!(got, None, "{:?} must not parse", case.expr);
                continue;
            }
            if case.empty {
                let got = got.unwrap_or_else(|| panic!("{:?} must parse", case.expr));
                assert!(got.is_empty(), "{:?} must be empty, got {got:?}", case.expr);
                assert_eq!(got, DateRange::EMPTY, "{:?}", case.expr);
                for d in ["1000-01-01", "2026-11-06", "9999-12-31"] {
                    assert!(!got.contains(date(d)), "{:?} contains {d}", case.expr);
                }
                continue;
            }
            let want = DateRange {
                from: case.from.as_deref().map(date),
                to: case.to.as_deref().map(date),
            };
            assert_eq!(got, Some(want), "{:?} today {today}", case.expr);
        }
    }

    #[test]
    fn range_contains_is_inclusive_with_open_ends() {
        let r = DateRange {
            from: Some(date("2026-10-01")),
            to: Some(date("2026-10-31")),
        };
        assert!(r.contains(date("2026-10-01")));
        assert!(r.contains(date("2026-10-31")));
        assert!(!r.contains(date("2026-09-30")));
        assert!(!r.contains(date("2026-11-01")));
        let open = DateRange {
            from: None,
            to: Some(date("2026-10-31")),
        };
        assert!(open.contains(date("1970-01-01")));
        assert!(DateRange::default().contains(date("2026-10-31")));
    }

    #[test]
    fn instants_cover_whole_local_days() {
        let tz = FixedOffset::west_opt(5 * 3600).unwrap();
        let r = DateRange {
            from: Some(date("2026-10-31")),
            to: Some(date("2026-10-31")),
        };
        let i = r.instants(&tz);
        let at = |s: &str| {
            tz.from_local_datetime(
                &chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap(),
            )
            .unwrap()
            .timestamp() as u64
        };
        assert!(i.contains(at("2026-10-31 00:00:00")));
        assert!(i.contains(at("2026-10-31 23:59:59")));
        assert!(!i.contains(at("2026-10-30 23:59:59")));
        assert!(!i.contains(at("2026-11-01 00:00:00")));
    }

    proptest! {
        #[test]
        fn parse_never_panics(s in "\\PC{0,40}") {
            let _ = evaluate(&s, date("2026-11-06"));
        }

        #[test]
        fn grammar_shaped_noise_never_panics(s in "[-.<>=0-9WwQqdmy]{0,24}") {
            let _ = evaluate(&s, date("2026-11-06"));
        }

        /// A period expression is exactly that period (its end clamped to
        /// 9999-12-31: the last ISO week of 9999 ends in 10000).
        #[test]
        fn period_expression_is_the_period(day in arb_date(), kind in arb_kind()) {
            let period = Period::of_date(day, kind);
            let range = evaluate(&period.key(), date("2026-11-06")).expect("period key parses");
            prop_assert_eq!(
                range,
                DateRange { from: Some(period.start()), to: Some(period.end().min(LAST_DATE)) }
            );
        }

        /// Every expression the grammar builds resolves with `from <= to`,
        /// unless it is the one empty range, and never names a date past
        /// 9999-12-31.
        #[test]
        fn from_never_after_to(expr in arb_expr(), today in arb_date()) {
            let range = evaluate(&expr, today).expect("generated expressions parse");
            if range.is_empty() {
                prop_assert_eq!(range, DateRange::EMPTY, "{}", expr);
            } else if let (Some(from), Some(to)) = (range.from, range.to) {
                prop_assert!(from <= to, "{expr}: {from} > {to}");
            }
            prop_assert!(range.from.is_none_or(|d| d <= LAST_DATE), "{}", expr);
            prop_assert!(range.to.is_none_or(|d| d <= LAST_DATE), "{}", expr);
        }

        /// The instant form agrees with the date form, for any offset.
        #[test]
        fn instants_agree_with_dates(
            a in arb_date(),
            b in arb_date(),
            secs in 0u64..4_102_444_800,
            offset_minutes in -14 * 60i32..=14 * 60,
        ) {
            let tz = FixedOffset::east_opt(offset_minutes * 60).unwrap();
            let (from, to) = if a <= b { (a, b) } else { (b, a) };
            let range = DateRange { from: Some(from), to: Some(to) };
            let local = chrono::DateTime::from_timestamp(secs as i64, 0)
                .unwrap()
                .with_timezone(&tz)
                .date_naive();
            prop_assert_eq!(range.instants(&tz).contains(secs), range.contains(local));
        }
    }

    #[test]
    fn empty_range_matches_no_instant() {
        let i = DateRange::EMPTY.instants(&Utc);
        for secs in [0, 1_700_000_000, 253_402_300_799] {
            assert!(!i.contains(secs), "{secs}");
        }
    }

    #[test]
    fn instants_in_utc_match_date_boundaries() {
        let r = DateRange {
            from: Some(date("1970-01-02")),
            to: None,
        };
        let i = r.instants(&Utc);
        assert!(!i.contains(86_399));
        assert!(i.contains(86_400));
    }

    fn arb_date() -> impl Strategy<Value = NaiveDate> {
        (1000i32..=9999, 1u32..=366).prop_map(|(y, ord)| {
            NaiveDate::from_yo_opt(y, ord)
                .unwrap_or_else(|| NaiveDate::from_yo_opt(y, 365).unwrap())
        })
    }

    fn arb_kind() -> impl Strategy<Value = PeriodKind> {
        prop::sample::select(PeriodKind::ALL.to_vec())
    }

    fn arb_period_key() -> impl Strategy<Value = String> {
        (arb_date(), arb_kind()).prop_map(|(d, k)| Period::of_date(d, k).key())
    }

    fn arb_expr() -> impl Strategy<Value = String> {
        prop_oneof![
            arb_period_key(),
            (arb_period_key(), arb_period_key()).prop_map(|(a, b)| format!("{a}..{b}")),
            arb_period_key().prop_map(|a| format!("{a}..")),
            arb_period_key().prop_map(|b| format!("..{b}")),
            (
                prop::sample::select(vec![">=", ">", "<=", "<"]),
                arb_period_key()
            )
                .prop_map(|(op, p)| format!("{op}{p}")),
            prop::sample::select(vec![
                "today".to_string(),
                "yesterday".to_string(),
                "this-week".to_string(),
                "last-week".to_string(),
                "this-month".to_string(),
                "last-month".to_string(),
                "this-year".to_string(),
            ]),
            (
                1u32..=999_999,
                prop::sample::select(vec!['d', 'w', 'm', 'y'])
            )
                .prop_map(|(n, u)| format!("-{n}{u}")),
        ]
    }
}
