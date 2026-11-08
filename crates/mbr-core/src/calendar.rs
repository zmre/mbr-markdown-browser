//! Calendar periods, note dates and periodic notes.
//!
//! One notion of *a note's calendar date* and one notion of *periodic notes*
//! (daily / weekly / monthly / quarterly / yearly), both computed here and
//! published in `site.json`. Pure: no I/O, no clock (except [`note_date`],
//! which takes the time zone it is told).
//!
//! - [`Period`] is one calendar period; weeks are ISO 8601 (Monday start, ISO
//!   week-year). Its canonical key (`2026-10-31`, `2026-W44`, `2026-10`,
//!   `2026-Q4`, `2026`) is the wire form everywhere.
//! - [`period_from_stem`] says whether a file stem *is* a period (the whole
//!   stem, ASCII digits only), [`date_prefix_of_stem`] whether it *starts*
//!   with a date (`2026-10-31 Acme sync`), and [`date_from_frontmatter_value`]
//!   reads a frontmatter date.
//! - [`CalendarRules`] applies the `[calendar]` configuration to one note at
//!   scan time ([`CalendarRules::resolve`]): a few string checks, no
//!   allocation for an undated note.
//! - [`PeriodicIndex`] holds every periodic note by kind, for the prev / next /
//!   up bar ([`CalendarNav`]). Built from the index in one pass, at most once
//!   per change batch (the server memoizes it beside its `site.json` body;
//!   the static build builds it once).
//!
//! The browser mirrors the date rule in `components/src/calendar/note-date.ts`
//! and the periodic lookups in `components/src/calendar/periodic.ts`.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::Path;

use chrono::{Datelike, Days, NaiveDate, TimeZone, Weekday};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::markdown::SimpleMetadata;

/// The five kinds of periodic note, smallest first. The order is used by
/// [`CalendarNav::up`] (week before month before quarter before year).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeriodKind {
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl PeriodKind {
    /// Every kind, smallest first.
    pub const ALL: [PeriodKind; 5] = [
        PeriodKind::Day,
        PeriodKind::Week,
        PeriodKind::Month,
        PeriodKind::Quarter,
        PeriodKind::Year,
    ];

    /// The kind's position in [`Self::ALL`], for per-kind tables.
    const fn index(self) -> usize {
        match self {
            PeriodKind::Day => 0,
            PeriodKind::Week => 1,
            PeriodKind::Month => 2,
            PeriodKind::Quarter => 3,
            PeriodKind::Year => 4,
        }
    }

    /// The lowercase name (`day`, `week`, …), as serialized.
    pub const fn as_str(self) -> &'static str {
        match self {
            PeriodKind::Day => "day",
            PeriodKind::Week => "week",
            PeriodKind::Month => "month",
            PeriodKind::Quarter => "quarter",
            PeriodKind::Year => "year",
        }
    }
}

/// Years a stem or a frontmatter date may name. Four ASCII digits, and a
/// leading zero is far more likely an id than a date.
const MIN_YEAR: i32 = 1000;
const MAX_YEAR: i32 = 9999;

/// One calendar period. Weeks are ISO 8601 (Monday start, ISO week-year).
///
/// Build one with [`Period::parse_key`], [`period_from_stem`], [`Period::of_date`]
/// or the checked constructors ([`Period::week`], [`Period::month`],
/// [`Period::quarter`], [`Period::year`]); every value they produce is a real
/// period with a year in 1000–9999. The variants are public for matching. A
/// hand-built out-of-range value never panics: its dates clamp.
///
/// `Ord` is chronological within one kind, which is how [`PeriodicIndex`]
/// keys its maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Period {
    Day(NaiveDate),
    Week { iso_year: i32, week: u32 },
    Month { year: i32, month: u32 },
    Quarter { year: i32, quarter: u32 },
    Year(i32),
}

/// Whether `year` is in the accepted range.
fn year_in_range(year: i32) -> bool {
    (MIN_YEAR..=MAX_YEAR).contains(&year)
}

impl Period {
    /// The ISO week `week` of ISO week-year `iso_year`, if that week exists
    /// (`2025-W53` does not: 2025 has 52 ISO weeks).
    pub fn week(iso_year: i32, week: u32) -> Option<Period> {
        (year_in_range(iso_year)
            && NaiveDate::from_isoywd_opt(iso_year, week, Weekday::Mon).is_some())
        .then_some(Period::Week { iso_year, week })
    }

    /// Month `month` (1–12) of `year`.
    pub fn month(year: i32, month: u32) -> Option<Period> {
        (year_in_range(year) && (1..=12).contains(&month)).then_some(Period::Month { year, month })
    }

    /// Quarter `quarter` (1–4) of `year`.
    pub fn quarter(year: i32, quarter: u32) -> Option<Period> {
        (year_in_range(year) && (1..=4).contains(&quarter))
            .then_some(Period::Quarter { year, quarter })
    }

    /// The year `year`.
    pub fn year(year: i32) -> Option<Period> {
        year_in_range(year).then_some(Period::Year(year))
    }

    /// The day `date`, if its year is in range.
    pub fn day(date: NaiveDate) -> Option<Period> {
        year_in_range(date.year()).then_some(Period::Day(date))
    }

    /// The period of `kind` that contains `date`.
    pub fn of_date(date: NaiveDate, kind: PeriodKind) -> Period {
        match kind {
            PeriodKind::Day => Period::Day(date),
            PeriodKind::Week => {
                let iso = date.iso_week();
                Period::Week {
                    iso_year: iso.year(),
                    week: iso.week(),
                }
            }
            PeriodKind::Month => Period::Month {
                year: date.year(),
                month: date.month(),
            },
            PeriodKind::Quarter => Period::Quarter {
                year: date.year(),
                quarter: date.month0() / 3 + 1,
            },
            PeriodKind::Year => Period::Year(date.year()),
        }
    }

    pub fn kind(&self) -> PeriodKind {
        match self {
            Period::Day(_) => PeriodKind::Day,
            Period::Week { .. } => PeriodKind::Week,
            Period::Month { .. } => PeriodKind::Month,
            Period::Quarter { .. } => PeriodKind::Quarter,
            Period::Year(_) => PeriodKind::Year,
        }
    }

    /// The first day of the period.
    pub fn start(&self) -> NaiveDate {
        match *self {
            Period::Day(date) => date,
            Period::Week { iso_year, week } => {
                NaiveDate::from_isoywd_opt(iso_year, week, Weekday::Mon)
                    .unwrap_or_else(|| first_of_month(iso_year, 1))
            }
            Period::Month { year, month } => first_of_month(year, month),
            Period::Quarter { year, quarter } => first_of_month(year, quarter.clamp(1, 4) * 3 - 2),
            Period::Year(year) => first_of_month(year, 1),
        }
    }

    /// The last day of the period, inclusive.
    pub fn end(&self) -> NaiveDate {
        let start = self.start();
        match *self {
            Period::Day(date) => date,
            Period::Week { .. } => start.checked_add_days(Days::new(6)).unwrap_or(start),
            Period::Month { .. } => last_day_after_months(start, 1),
            Period::Quarter { .. } => last_day_after_months(start, 3),
            Period::Year(_) => last_day_after_months(start, 12),
        }
    }

    /// The canonical key: `2026-10-31`, `2026-W44`, `2026-10`, `2026-Q4`, `2026`.
    pub fn key(&self) -> String {
        self.to_string()
    }

    /// Parses a canonical key ([`Self::key`]'s forms only: hyphens, capital
    /// `W` and `Q`, zero-padded). The looser filename forms are
    /// [`period_from_stem`]'s.
    pub fn parse_key(s: &str) -> Option<Period> {
        let b = s.as_bytes();
        match b.len() {
            10 if b[4] == b'-' && b[7] == b'-' => parse_day(b, b'-'),
            8 if b[4] == b'-' && b[5] == b'W' => parse_week(b),
            7 if b[4] == b'-' && b[5] == b'Q' => parse_quarter(b),
            7 if b[4] == b'-' => parse_month(b),
            4 => parse_year(b),
            _ => None,
        }
    }

    /// The period of `kind` containing this one: a day's week, month, quarter
    /// or year.
    ///
    /// A week belongs to the month, quarter and year of its **Thursday** (the
    /// ISO 8601 rule that also decides its week-year), so `2026-W01`, which
    /// starts on 2025-12-29, is in January 2026. For a `kind` no larger than
    /// this period's own, the result is the period of `kind` containing this
    /// period's first day.
    pub fn containing(&self, kind: PeriodKind) -> Period {
        let anchor = match self {
            Period::Week { .. } if kind > PeriodKind::Week => self
                .start()
                .checked_add_days(Days::new(3))
                .unwrap_or_else(|| self.start()),
            _ => self.start(),
        };
        Period::of_date(anchor, kind)
    }

    /// The `datetime` attribute value for `<time>`: the key itself for every
    /// kind HTML has a form for (date, week, month, year). A quarter has none,
    /// so it gets `None` and the label stands alone.
    pub fn html_datetime(&self) -> Option<String> {
        (self.kind() != PeriodKind::Quarter).then(|| self.key())
    }

    /// The full English label of the period a page is about:
    /// `Friday, October 31, 2026`, `Week 44, 2026`, `October 2026`,
    /// `Q4 2026`, `2026`.
    fn label_long(&self) -> String {
        match *self {
            Period::Day(date) => date.format("%A, %B %-d, %Y").to_string(),
            Period::Week { iso_year, week } => format!("Week {week}, {iso_year}"),
            _ => self.label_short(None),
        }
    }

    /// A compact English label for a link from the page about `reference`:
    /// `Thu, Oct 30`, `Week 43`, `September 2026`, `Q3 2026`, `2025`. The
    /// year is added to a day or week when it differs from `reference`'s, so
    /// stepping across New Year never reads as going back eleven months.
    fn label_short(&self, reference: Option<&Period>) -> String {
        let reference_year = reference.map(|p| p.label_year());
        let other_year = reference_year.is_some_and(|y| y != self.label_year());
        match *self {
            Period::Day(date) if other_year => date.format("%a, %b %-d, %Y").to_string(),
            Period::Day(date) => date.format("%a, %b %-d").to_string(),
            Period::Week { iso_year, week } if other_year => format!("Week {week}, {iso_year}"),
            Period::Week { week, .. } => format!("Week {week}"),
            Period::Month { .. } => self.start().format("%B %Y").to_string(),
            Period::Quarter { year, quarter } => format!("Q{quarter} {year}"),
            Period::Year(year) => year.to_string(),
        }
    }

    /// The year a label names: the ISO week-year for a week.
    fn label_year(&self) -> i32 {
        match *self {
            Period::Week { iso_year, .. } => iso_year,
            _ => self.start().year(),
        }
    }
}

impl fmt::Display for Period {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Period::Day(date) => write!(
                f,
                "{:04}-{:02}-{:02}",
                date.year(),
                date.month(),
                date.day()
            ),
            Period::Week { iso_year, week } => write!(f, "{iso_year:04}-W{week:02}"),
            Period::Month { year, month } => write!(f, "{year:04}-{month:02}"),
            Period::Quarter { year, quarter } => write!(f, "{year:04}-Q{quarter}"),
            Period::Year(year) => write!(f, "{year:04}"),
        }
    }
}

/// Serialized as its canonical key.
impl Serialize for Period {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Period {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = String::deserialize(deserializer)?;
        Period::parse_key(&key)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid period key {key:?}")))
    }
}

/// The 1st of `month` in `year`, clamped into chrono's range for a hand-built
/// out-of-range value (never reached by a checked constructor).
fn first_of_month(year: i32, month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month.clamp(1, 12), 1).unwrap_or(NaiveDate::MIN)
}

/// The day before `start` + `months` months.
fn last_day_after_months(start: NaiveDate, months: u32) -> NaiveDate {
    start
        .checked_add_months(chrono::Months::new(months))
        .and_then(|next| next.pred_opt())
        .unwrap_or(start)
}

// ==================== Byte-level parsing ====================

/// The value of `digits` (ASCII digits only), or `None`. At most 4 digits, so
/// it cannot overflow.
fn digits(b: &[u8]) -> Option<u32> {
    (!b.is_empty() && b.len() <= 4 && b.iter().all(u8::is_ascii_digit))
        .then(|| b.iter().fold(0, |n, d| n * 10 + u32::from(d - b'0')))
}

fn year_of(b: &[u8]) -> Option<i32> {
    let year = i32::try_from(digits(b.get(..4)?)?).ok()?;
    year_in_range(year).then_some(year)
}

/// `YYYY?MM?DD` with separator `sep` at 4 and 7 (callers check the length).
fn parse_day(b: &[u8], sep: u8) -> Option<Period> {
    if b.len() != 10 || b[4] != sep || b[7] != sep {
        return None;
    }
    date_from(year_of(b)?, digits(&b[5..7])?, digits(&b[8..10])?).map(Period::Day)
}

/// `YYYYMMDD`.
fn parse_compact_day(b: &[u8]) -> Option<Period> {
    if b.len() != 8 {
        return None;
    }
    date_from(year_of(b)?, digits(&b[4..6])?, digits(&b[6..8])?).map(Period::Day)
}

fn date_from(year: i32, month: u32, day: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, day)
}

/// `YYYY-Www` (the caller has checked `-` and the `W`'s case).
fn parse_week(b: &[u8]) -> Option<Period> {
    if b.len() != 8 {
        return None;
    }
    Period::week(year_of(b)?, digits(&b[6..8])?)
}

/// `YYYY-MM`.
fn parse_month(b: &[u8]) -> Option<Period> {
    if b.len() != 7 || b[4] != b'-' {
        return None;
    }
    Period::month(year_of(b)?, digits(&b[5..7])?)
}

/// `YYYY-Qn` (the caller has checked the `Q`'s case).
fn parse_quarter(b: &[u8]) -> Option<Period> {
    if b.len() != 7 {
        return None;
    }
    Period::quarter(year_of(b)?, digits(&b[6..7])?)
}

fn parse_year(b: &[u8]) -> Option<Period> {
    if b.len() != 4 {
        return None;
    }
    Period::year(year_of(b)?)
}

/// The period a note's file stem names, if the **whole** stem is a period.
///
/// | Form         | Example      | Kind    |
/// | ------------ | ------------ | ------- |
/// | `YYYY-MM-DD` | `2026-10-31` | Day     |
/// | `YYYYMMDD`   | `20261031`   | Day     |
/// | `YYYY_MM_DD` | `2026_10_31` | Day     |
/// | `YYYY-Www`   | `2026-W44`   | Week (`w` too; the week must exist that year) |
/// | `YYYY-MM`    | `2026-10`    | Month   |
/// | `YYYY-Qn`    | `2026-Q4`    | Quarter (`q` too) |
/// | `YYYY`       | `2026`       | Year    |
///
/// ASCII digits only; an invalid date (`2026-02-30`) is not a period; years
/// are 1000–9999. Dispatch is on length and separator bytes, so an ordinary
/// note name costs one or two comparisons.
pub fn period_from_stem(stem: &str) -> Option<Period> {
    let b = stem.as_bytes();
    match b.len() {
        10 => match b[4] {
            b'-' | b'_' => parse_day(b, b[4]),
            _ => None,
        },
        8 if b[4] == b'-' && (b[5] == b'W' || b[5] == b'w') => parse_week(b),
        8 => parse_compact_day(b),
        7 if b[4] == b'-' && (b[5] == b'Q' || b[5] == b'q') => parse_quarter(b),
        7 => parse_month(b),
        4 => parse_year(b),
        _ => None,
    }
}

/// Whether `b` may follow a date at the start of a stem.
fn is_prefix_separator(b: u8) -> bool {
    matches!(b, b'-' | b'_' | b' ' | b'.' | b'T')
}

/// A date the stem **starts with**, without being a periodic note: Jekyll's
/// `2026-10-31-my-post`, `2026-10-31 Standup notes`, `20261031_call`.
///
/// The date (`YYYY-MM-DD`, `YYYYMMDD` or `YYYY_MM_DD`) must be followed by one
/// of `-`, `_`, space, `.` or `T` and then at least one more character. A
/// Zettelkasten timestamp (`202610311230 Title`) is not a date: its 9th byte
/// is a digit.
fn date_prefix_of_stem(stem: &str) -> Option<NaiveDate> {
    let b = stem.as_bytes();
    let dashed = |sep: u8| {
        (b.len() > 11 && is_prefix_separator(b[10]))
            .then(|| parse_day(&b[..10], sep))
            .flatten()
    };
    let period = match b.get(4)? {
        b'-' => dashed(b'-'),
        b'_' => dashed(b'_'),
        _ => (b.len() > 9 && is_prefix_separator(b[8]))
            .then(|| parse_compact_day(&b[..8]))
            .flatten(),
    };
    match period? {
        Period::Day(date) => Some(date),
        _ => None,
    }
}

/// The calendar date a frontmatter value names, date part only.
///
/// Accepts a string holding:
///
/// - `YYYY-MM-DD`;
/// - `YYYY-MM-DD[T ]HH:MM[:SS[.fff]]` with an optional zone (`Z`, `±HH:MM`,
///   `±HHMM`, optionally after a space, as Jekyll writes `-0700`) — the
///   Jekyll / Hugo timestamps. **The date is taken as written and the offset
///   ignored**: the author's wall-clock date is the note's date, and
///   converting would move late-evening posts to the next day;
/// - `YYYY/MM/DD`.
///
/// Partial dates (`2026-10`), bare-year numbers and anything else are `None`,
/// so resolution falls through to the next source.
fn date_from_frontmatter_value(v: &serde_json::Value) -> Option<NaiveDate> {
    let s = v.as_str()?.trim();
    let b = s.as_bytes();
    if b.len() < 10 {
        return None;
    }
    let (date, rest) = b.split_at(10);
    match date[4] {
        b'-' => {
            let Period::Day(day) = parse_day(date, b'-')? else {
                return None;
            };
            (rest.is_empty() || is_timestamp_tail(rest)).then_some(day)
        }
        b'/' if rest.is_empty() => match parse_day(date, b'/')? {
            Period::Day(day) => Some(day),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `b` is `[T ]HH:MM[:SS[.fff]][ ][Z|±HH:MM|±HHMM]`, the tail of a
/// timestamp after its date. Hours 0–23, minutes 0–59, seconds 0–60.
fn is_timestamp_tail(b: &[u8]) -> bool {
    let Some((&first, b)) = b.split_first() else {
        return true;
    };
    if !matches!(first, b'T' | b't' | b' ') || b.len() < 5 || b[2] != b':' {
        return false;
    }
    let in_range = |digits_at: &[u8], max: u32| digits(digits_at).is_some_and(|n| n <= max);
    if !(in_range(&b[0..2], 23) && in_range(&b[3..5], 59)) {
        return false;
    }
    let mut rest = &b[5..];
    if let Some(after) = rest.strip_prefix(b":") {
        if after.len() < 2 || !in_range(&after[..2], 60) {
            return false;
        }
        rest = &after[2..];
        if let Some(fraction) = rest.strip_prefix(b".") {
            let n = fraction.iter().take_while(|c| c.is_ascii_digit()).count();
            if n == 0 {
                return false;
            }
            rest = &fraction[n..];
        }
    }
    let rest = rest.strip_prefix(b" ").unwrap_or(rest);
    is_zone(rest)
}

/// Empty, `Z`, `±HH:MM` or `±HHMM`.
fn is_zone(b: &[u8]) -> bool {
    match b {
        [] | [b'Z' | b'z'] => true,
        [b'+' | b'-', h1, h2, b':', m1, m2] | [b'+' | b'-', h1, h2, m1, m2] => {
            digits(&[*h1, *h2]).is_some_and(|h| h <= 23)
                && digits(&[*m1, *m2]).is_some_and(|m| m <= 59)
        }
        _ => false,
    }
}

// ==================== A note's calendar data ====================

/// Where a note's calendar date came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DateSource {
    Filename,
    Frontmatter,
}

/// A note's calendar data, published as `calendar` on its `site.json` entry
/// and omitted when the note has no filename or frontmatter date:
///
/// ```json
/// "calendar": { "date": "2026-10-31", "source": "filename", "period": "2026-10-31" }
/// ```
///
/// `period` is present only for a periodic note: `2026-W44` means the note
/// *is* the weekly note for that week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteCalendar {
    /// Serialized as `YYYY-MM-DD` (chrono's `NaiveDate` serde form).
    pub date: NaiveDate,
    pub source: DateSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}

/// A note's date for search and grouping (D1): the resolved calendar date,
/// else the local date of `created` (birthtime, else mtime; unix seconds) in
/// `tz`. The browser computes the same in `note-date.ts`, in the reader's own
/// time zone. `None` only for a timestamp chrono cannot represent.
pub fn note_date<Tz: TimeZone>(
    calendar: Option<&NoteCalendar>,
    created_secs: u64,
    tz: &Tz,
) -> Option<NaiveDate> {
    calendar.map(|c| c.date).or_else(|| {
        let secs = i64::try_from(created_secs).ok()?;
        let utc = chrono::DateTime::from_timestamp(secs, 0)?;
        Some(utc.with_timezone(tz).date_naive())
    })
}

// ==================== Configuration ====================

/// `week_start`: the first column of every calendar grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeekStart {
    /// The reader's locale (`Intl.Locale(…).getWeekInfo()`), falling back to
    /// Sunday for `en-US` and Monday otherwise. Resolved in the browser.
    #[default]
    Locale,
    Monday,
    Sunday,
    Saturday,
}

/// `companion`: when the daily-note companion shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompanionMode {
    /// Docks as a right column on day notes when the window is wide enough.
    #[default]
    Auto,
    /// Only when opened (key or button).
    OnDemand,
    /// Never.
    Off,
}

/// The `[calendar]` table of `.mbr/config.toml` (and `MBR_CALENDAR__*`).
///
/// Typed with `deny_unknown_fields`, so a typo fails at startup like every
/// other option. Its browser half is published as `__MBR_CONFIG__.calendar`
/// (`page_context::CalendarChrome`, serialized once); the scan-time half is
/// compiled into [`CalendarRules`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CalendarConfig {
    /// Master switch for periodic navigation, the `d` / `[` / `]` keys and
    /// (later phases) the companion and calendar views. Off, notes still get
    /// their calendar **date** (search date facets use it), but no note is
    /// periodic.
    pub enabled: bool,
    /// Which stem forms make a note periodic. Default: all five.
    pub periods: Vec<PeriodKind>,
    /// Restricts periodic detection to these repository-relative folders
    /// (recursive). Empty: anywhere.
    pub periodic_folders: Vec<String>,
    /// Frontmatter fields consulted, in order, after the filename.
    pub date_fields: Vec<String>,
    /// First column of every grid.
    pub week_start: WeekStart,
    /// Whether grids show the ISO week-number column. Unset: on when the
    /// repository has any week notes (decided in the browser).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub week_numbers: Option<bool>,
    /// When the daily-note companion shows.
    pub companion: CompanionMode,
    /// Whether time-prefixed list items get an inline time chip.
    pub agenda_chips: bool,
}

impl Default for CalendarConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            periods: PeriodKind::ALL.to_vec(),
            periodic_folders: Vec::new(),
            date_fields: vec!["date".to_string(), "created".to_string()],
            week_start: WeekStart::Locale,
            week_numbers: None,
            companion: CompanionMode::Auto,
            agenda_chips: true,
        }
    }
}

/// The scan-time half of [`CalendarConfig`], prepared once: which kinds are
/// periodic, the normalized folder filter and the date fields. Serializable
/// because it is part of the repository snapshot's fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CalendarRules {
    enabled: bool,
    periods: [bool; 5],
    /// `/`-separated, no leading `./` or `/`, no trailing `/`; `""` is the root.
    periodic_folders: Vec<String>,
    date_fields: Vec<String>,
}

impl Default for CalendarRules {
    fn default() -> Self {
        Self::new(&CalendarConfig::default())
    }
}

impl CalendarRules {
    pub fn new(config: &CalendarConfig) -> Self {
        let mut periods = [false; 5];
        for kind in &config.periods {
            periods[kind.index()] = true;
        }
        Self {
            enabled: config.enabled,
            periods,
            periodic_folders: config
                .periodic_folders
                .iter()
                .map(|f| crate::url_path::normalize_folder(f))
                .collect(),
            date_fields: config.date_fields.clone(),
        }
    }

    /// Whether a note at `raw_path` naming `period` is a periodic note under
    /// these rules.
    fn is_periodic(&self, period: &Period, raw_path: &Path) -> bool {
        self.enabled
            && self.periods[period.kind().index()]
            && (self.periodic_folders.is_empty() || {
                let folder = raw_path
                    .parent()
                    .map(crate::url_path::path_to_url)
                    .unwrap_or_default();
                self.periodic_folders
                    .iter()
                    .any(|f| crate::url_path::folder_contains(f, &folder))
            })
    }

    /// The calendar data of the note at `raw_path` (repository-relative) with
    /// `frontmatter` (D1, steps 1–2):
    ///
    /// 1. **Filename.** A stem that is a period dates the note to its start
    ///    (a week note to its Monday) and, when the rules accept it, makes the
    ///    note periodic. One they do not accept (another folder, a disabled
    ///    kind) still dates the note only if it is a **day** stem: a date is a
    ///    date, but `1984` outside the configured places is just a title.
    ///    Else a date the stem starts with ([`date_prefix_of_stem`]).
    /// 2. **Frontmatter**, in `date_fields` order ([`date_from_frontmatter_value`]).
    ///
    /// `None` when neither applies; the browser and search then fall back to
    /// the file's birthtime ([`note_date`]).
    pub fn resolve(
        &self,
        raw_path: &Path,
        frontmatter: Option<&SimpleMetadata>,
    ) -> Option<NoteCalendar> {
        let stem = raw_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if let Some(period) = period_from_stem(stem) {
            if self.is_periodic(&period, raw_path) {
                return Some(NoteCalendar {
                    date: period.start(),
                    source: DateSource::Filename,
                    period: Some(period),
                });
            }
            if let Period::Day(date) = period {
                return Some(filename_date(date));
            }
        } else if let Some(date) = date_prefix_of_stem(stem) {
            return Some(filename_date(date));
        }
        let frontmatter = frontmatter?;
        self.date_fields
            .iter()
            .find_map(|field| date_from_frontmatter_value(frontmatter.get(field)?))
            .map(|date| NoteCalendar {
                date,
                source: DateSource::Frontmatter,
                period: None,
            })
    }
}

fn filename_date(date: NaiveDate) -> NoteCalendar {
    NoteCalendar {
        date,
        source: DateSource::Filename,
        period: None,
    }
}

// ==================== The periodic index ====================

/// A periodic note in the [`PeriodicIndex`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodicNote {
    pub url_path: String,
    pub title: String,
}

/// One note claiming a period, as [`PeriodicIndex::build`] takes it.
#[derive(Debug, Clone)]
pub struct PeriodicEntry {
    pub period: Period,
    pub url_path: String,
    /// Repository-relative source path, `/`-separated: the tie-break between
    /// two notes at one `url_path`, as `site.json` orders them.
    pub raw_path: String,
    pub title: String,
}

/// A periodic note that lost its period to another note
/// ([`crate::page_errors::PageError::PeriodicDuplicate`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodicDuplicate {
    pub period: Period,
    /// The `url_path` of the note that keeps the period.
    pub kept: String,
}

/// Every periodic note, by kind and period, plus the notes that lost a
/// period to another (`daily/2026-10-31.md` and `archive/20261031.md`):
/// **the first by `url_path` order wins**, as in `site.json`.
#[derive(Debug, Clone, Default)]
pub struct PeriodicIndex {
    by_kind: [BTreeMap<Period, PeriodicNote>; 5],
    /// Losing notes by `url_path`.
    duplicates: HashMap<String, PeriodicDuplicate>,
}

impl PeriodicIndex {
    /// Builds the index in one sort and one pass over the periodic notes only.
    pub fn build(entries: impl IntoIterator<Item = PeriodicEntry>) -> Self {
        let mut entries: Vec<PeriodicEntry> = entries.into_iter().collect();
        entries.sort_unstable_by(|a, b| {
            a.url_path
                .cmp(&b.url_path)
                .then_with(|| a.raw_path.cmp(&b.raw_path))
        });
        let mut index = Self::default();
        for entry in entries {
            let map = &mut index.by_kind[entry.period.kind().index()];
            match map.get(&entry.period) {
                Some(kept) => {
                    let duplicate = PeriodicDuplicate {
                        period: entry.period,
                        kept: kept.url_path.clone(),
                    };
                    index.duplicates.insert(entry.url_path, duplicate);
                }
                None => {
                    map.insert(
                        entry.period,
                        PeriodicNote {
                            url_path: entry.url_path,
                            title: entry.title,
                        },
                    );
                }
            }
        }
        index
    }

    /// Number of periods with a note.
    pub fn len(&self) -> usize {
        self.by_kind.iter().map(BTreeMap::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The note for `period`.
    pub fn get(&self, period: &Period) -> Option<&PeriodicNote> {
        self.by_kind[period.kind().index()].get(period)
    }

    /// The nearest earlier note of the same kind, skipping gaps.
    pub fn prev(&self, period: &Period) -> Option<(&Period, &PeriodicNote)> {
        self.by_kind[period.kind().index()]
            .range(..period)
            .next_back()
    }

    /// The nearest later note of the same kind, skipping gaps.
    pub fn next(&self, period: &Period) -> Option<(&Period, &PeriodicNote)> {
        use std::ops::Bound::{Excluded, Unbounded};
        self.by_kind[period.kind().index()]
            .range((Excluded(period), Unbounded))
            .next()
    }

    /// Why the note at `url_path` is not the note for its period, if it lost
    /// it to another.
    pub fn duplicate_of(&self, url_path: &str) -> Option<&PeriodicDuplicate> {
        self.duplicates.get(url_path)
    }

    /// Every note that lost its period, by `url_path` (unordered).
    pub fn duplicates(&self) -> impl Iterator<Item = (&str, &PeriodicDuplicate)> {
        self.duplicates.iter().map(|(url, dup)| (url.as_str(), dup))
    }

    /// The navigation bar for the page about `period`: the nearest notes of
    /// the same kind either side and the containing week / month / quarter /
    /// year notes that exist. URLs are site-absolute; a static build
    /// relativizes them (`page_context`).
    pub fn nav(&self, period: &Period) -> CalendarNav {
        let link = |p: &Period, note: &PeriodicNote| CalendarNavLink {
            kind: p.kind(),
            period: p.key(),
            url: note.url_path.clone(),
            label: p.label_short(Some(period)),
            title: note.title.clone(),
            datetime: p.html_datetime(),
        };
        let up = PeriodKind::ALL
            .into_iter()
            .filter(|kind| *kind > period.kind())
            .filter_map(|kind| {
                let container = period.containing(kind);
                self.get(&container).map(|note| link(&container, note))
            })
            .collect();
        CalendarNav {
            period: period.key(),
            kind: period.kind(),
            label: period.label_long(),
            datetime: period.html_datetime(),
            prev: self.prev(period).map(|(p, note)| link(p, note)),
            next: self.next(period).map(|(p, note)| link(p, note)),
            up,
        }
    }
}

/// One link of the [`CalendarNav`] bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CalendarNavLink {
    pub kind: PeriodKind,
    /// Canonical period key.
    pub period: String,
    /// Site-absolute in server mode, page-relative in a static build.
    pub url: String,
    /// English, compact (`Thu, Oct 30`, `Week 44`, `October 2026`).
    pub label: String,
    /// The note's title, for the link's tooltip.
    pub title: String,
    /// `<time datetime>`, absent for a quarter (HTML has no quarter form).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datetime: Option<String>,
}

/// `calendar_nav` in a periodic note's template context, drawn by
/// `_calendar_nav.html`:
///
/// ```json
/// { "period": "2026-10-31", "kind": "day", "label": "Friday, October 31, 2026",
///   "datetime": "2026-10-31",
///   "prev": { "kind": "day", "period": "2026-10-30", "url": "/daily/2026-10-30/",
///             "label": "Thu, Oct 30", "title": "2026-10-30", "datetime": "2026-10-30" },
///   "next": { … },
///   "up": [ { "kind": "week", "period": "2026-W44", "url": "/weekly/2026-W44/",
///             "label": "Week 44", … } ] }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CalendarNav {
    pub period: String,
    pub kind: PeriodKind,
    /// English, long (`Friday, October 31, 2026`).
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datetime: Option<String>,
    pub prev: Option<CalendarNavLink>,
    pub next: Option<CalendarNavLink>,
    /// Containing periods with a note, smallest first.
    pub up: Vec<CalendarNavLink>,
}

impl CalendarNav {
    /// Every URL rewritten by `rewrite` (a static build's page-relative form).
    pub fn map_urls(mut self, rewrite: impl Fn(&str) -> String) -> Self {
        let fix = |link: &mut CalendarNavLink| link.url = rewrite(&link.url);
        self.prev.iter_mut().for_each(fix);
        self.next.iter_mut().for_each(fix);
        self.up.iter_mut().for_each(fix);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    // ---- Stem grammar ----

    #[test]
    fn stems_that_are_periods() {
        let cases = [
            ("2026-10-31", Period::Day(ymd(2026, 10, 31))),
            ("20261031", Period::Day(ymd(2026, 10, 31))),
            ("2026_10_31", Period::Day(ymd(2026, 10, 31))),
            (
                "2026-W44",
                Period::Week {
                    iso_year: 2026,
                    week: 44,
                },
            ),
            (
                "2026-w01",
                Period::Week {
                    iso_year: 2026,
                    week: 1,
                },
            ),
            (
                "2026-W53",
                Period::Week {
                    iso_year: 2026,
                    week: 53,
                },
            ),
            (
                "2020-W53",
                Period::Week {
                    iso_year: 2020,
                    week: 53,
                },
            ),
            (
                "2026-10",
                Period::Month {
                    year: 2026,
                    month: 10,
                },
            ),
            (
                "2026-Q4",
                Period::Quarter {
                    year: 2026,
                    quarter: 4,
                },
            ),
            (
                "2026-q1",
                Period::Quarter {
                    year: 2026,
                    quarter: 1,
                },
            ),
            ("2026", Period::Year(2026)),
            ("1984", Period::Year(1984)),
        ];
        for (stem, expected) in cases {
            assert_eq!(period_from_stem(stem), Some(expected), "{stem}");
        }
    }

    #[test]
    fn stems_that_are_not_periods() {
        for stem in [
            "",
            "2026-02-30",
            "2025-02-29",
            "2025-W53", // 2025 has 52 ISO weeks
            "2026-W54", // 2026 has 53
            "2026-W00",
            "2026-Q5",
            "2026-Q0",
            "2026-13",
            "2026-00",
            "12345",
            "0999",
            "999",
            "2026-10-31x",
            "x2026-10-31",
            "2026-10-3",
            "2026/10/31",
            "2026.10.31",
            "20261331",
            "2026-1-31",
            "２０２６",         // fullwidth digits
            "٢٠٢٦-١٠-٣١",       // Arabic-Indic digits
            "2026-10-31 notes", // a prefix, not a period
            "README",
            "index",
            "2026-W4",
            "2026-Wxx",
            "+026",
            "-202",
        ] {
            assert_eq!(period_from_stem(stem), None, "{stem:?}");
        }
    }

    #[test]
    fn leap_days() {
        assert_eq!(
            period_from_stem("2024-02-29"),
            Some(Period::Day(ymd(2024, 2, 29)))
        );
        assert_eq!(period_from_stem("2100-02-29"), None);
    }

    // ---- Prefix dates ----

    #[test]
    fn date_prefixes() {
        for (stem, expected) in [
            ("2026-10-31-my-post", ymd(2026, 10, 31)),
            ("2026-10-31 Acme sync", ymd(2026, 10, 31)),
            ("2026-10-31_standup", ymd(2026, 10, 31)),
            ("2026-10-31.draft", ymd(2026, 10, 31)),
            ("2026-10-31T0930", ymd(2026, 10, 31)),
            ("20261031 call", ymd(2026, 10, 31)),
            ("20261031-call", ymd(2026, 10, 31)),
            ("2026_10_31 journal", ymd(2026, 10, 31)),
        ] {
            assert_eq!(date_prefix_of_stem(stem), Some(expected), "{stem}");
        }
    }

    #[test]
    fn not_date_prefixes() {
        for stem in [
            "2026-10-31",         // the whole stem: a period, not a prefix
            "2026-10-31-",        // nothing after the separator
            "2026-10-31x",        // not a separator
            "2026-02-30 x",       // not a date
            "202610311230 Title", // Zettelkasten timestamp (O2)
            "20261031",
            "2026-10 notes",
            "notes 2026-10-31",
            "2026_10-31 x",
            "",
            "abc",
        ] {
            assert_eq!(date_prefix_of_stem(stem), None, "{stem:?}");
        }
    }

    // ---- Frontmatter values ----

    #[test]
    fn frontmatter_values_that_are_dates() {
        let d = ymd(2026, 10, 31);
        for v in [
            "2026-10-31",
            " 2026-10-31 ",
            "2026-10-31T09:30",
            "2026-10-31T09:30:00",
            "2026-10-31T09:30:00.123",
            "2026-10-31T09:30:00Z",
            "2026-10-31T23:30:00-07:00", // late evening: the written date, not UTC's
            "2026-10-31T00:30:00+1400",
            "2026-10-31 09:30",
            "2026-10-31 09:30:00 -0700", // Jekyll
            "2026-10-31 09:30:00.5+02:00",
            "2026/10/31",
        ] {
            assert_eq!(date_from_frontmatter_value(&json!(v)), Some(d), "{v}");
        }
    }

    #[test]
    fn frontmatter_values_that_are_not_dates() {
        for v in [
            json!("2026-10"),
            json!("2026"),
            json!(2026),
            json!(20261031),
            json!(true),
            json!(null),
            json!(["2026-10-31"]),
            json!({"date": "2026-10-31"}),
            json!("2026-02-30"),
            json!("2026-10-31T25:00"),
            json!("2026-10-31T09:60"),
            json!("2026-10-31T09"),
            json!("2026-10-31 garbage"),
            json!("2026-10-31T09:30:00+25:00"),
            json!("2026-10-31T09:30:00."),
            json!("2026/10/31 09:30"),
            json!("October 31, 2026"),
            json!("31/10/2026"),
            json!("0999-01-01"),
        ] {
            assert_eq!(date_from_frontmatter_value(&v), None, "{v}");
        }
    }

    // ---- Period arithmetic ----

    #[test]
    fn keys_and_bounds() {
        let w = Period::week(2026, 1).unwrap();
        assert_eq!(w.key(), "2026-W01");
        assert_eq!(w.start(), ymd(2025, 12, 29), "2026-W01 starts in 2025");
        assert_eq!(w.end(), ymd(2026, 1, 4));
        let w53 = Period::week(2020, 53).unwrap();
        assert_eq!(w53.start(), ymd(2020, 12, 28));
        assert_eq!(w53.end(), ymd(2021, 1, 3));
        let q = Period::quarter(2026, 4).unwrap();
        assert_eq!((q.start(), q.end()), (ymd(2026, 10, 1), ymd(2026, 12, 31)));
        let m = Period::month(2024, 2).unwrap();
        assert_eq!((m.start(), m.end()), (ymd(2024, 2, 1), ymd(2024, 2, 29)));
        let y = Period::year(2026).unwrap();
        assert_eq!((y.start(), y.end()), (ymd(2026, 1, 1), ymd(2026, 12, 31)));
        assert_eq!(q.key(), "2026-Q4");
        assert_eq!(m.key(), "2024-02");
        assert_eq!(y.key(), "2026");
    }

    #[test]
    fn containing_uses_the_iso_rules_for_weeks() {
        let day = Period::Day(ymd(2025, 12, 30));
        assert_eq!(day.containing(PeriodKind::Week).key(), "2026-W01");
        assert_eq!(day.containing(PeriodKind::Month).key(), "2025-12");
        assert_eq!(day.containing(PeriodKind::Year).key(), "2025");
        let week = Period::week(2026, 1).unwrap();
        assert_eq!(week.containing(PeriodKind::Month).key(), "2026-01");
        assert_eq!(week.containing(PeriodKind::Quarter).key(), "2026-Q1");
        assert_eq!(week.containing(PeriodKind::Year).key(), "2026");
        assert_eq!(week.containing(PeriodKind::Day).key(), "2025-12-29");
    }

    #[test]
    fn parse_key_is_strict() {
        assert_eq!(Period::parse_key("2026-w44"), None);
        assert_eq!(Period::parse_key("2026-q4"), None);
        assert_eq!(Period::parse_key("20261031"), None);
        assert_eq!(Period::parse_key("2026_10_31"), None);
        assert_eq!(Period::parse_key("2026-W44"), Period::week(2026, 44));
    }

    #[test]
    fn labels() {
        let day = Period::Day(ymd(2026, 10, 31));
        assert_eq!(day.label_long(), "Saturday, October 31, 2026");
        assert_eq!(
            Period::Day(ymd(2026, 10, 30)).label_short(Some(&day)),
            "Fri, Oct 30"
        );
        assert_eq!(
            Period::Day(ymd(2025, 12, 31)).label_short(Some(&Period::Day(ymd(2026, 1, 2)))),
            "Wed, Dec 31, 2025"
        );
        let week = Period::week(2026, 44).unwrap();
        assert_eq!(week.label_long(), "Week 44, 2026");
        assert_eq!(week.label_short(Some(&day)), "Week 44");
        assert_eq!(
            Period::month(2026, 10).unwrap().label_short(Some(&day)),
            "October 2026"
        );
        assert_eq!(Period::quarter(2026, 4).unwrap().label_long(), "Q4 2026");
        assert_eq!(Period::year(2026).unwrap().label_long(), "2026");
        assert_eq!(Period::quarter(2026, 4).unwrap().html_datetime(), None);
        assert_eq!(week.html_datetime().as_deref(), Some("2026-W44"));
    }

    #[test]
    fn serde_round_trip() {
        let cal = NoteCalendar {
            date: ymd(2026, 10, 26),
            source: DateSource::Filename,
            period: Period::week(2026, 44),
        };
        let v = serde_json::to_value(cal).unwrap();
        assert_eq!(
            v,
            json!({"date": "2026-10-26", "source": "filename", "period": "2026-W44"})
        );
        assert_eq!(serde_json::from_value::<NoteCalendar>(v).unwrap(), cal);
        let undated = NoteCalendar {
            date: ymd(2026, 10, 26),
            source: DateSource::Frontmatter,
            period: None,
        };
        assert_eq!(
            serde_json::to_value(undated).unwrap(),
            json!({"date": "2026-10-26", "source": "frontmatter"})
        );
    }

    #[test]
    fn note_date_falls_back_to_the_local_date_of_created() {
        // 2026-11-01T02:30:00Z: still Oct 31 in New York (UTC-4 that night).
        let created = 1_793_500_200;
        let ny = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
        assert_eq!(note_date(None, created, &ny), Some(ymd(2026, 10, 31)));
        assert_eq!(
            note_date(None, created, &chrono::Utc),
            Some(ymd(2026, 11, 1))
        );
        let cal = NoteCalendar {
            date: ymd(2020, 1, 1),
            source: DateSource::Frontmatter,
            period: None,
        };
        assert_eq!(note_date(Some(&cal), created, &ny), Some(ymd(2020, 1, 1)));
    }

    // ---- Rules ----

    fn fm(pairs: &[(&str, serde_json::Value)]) -> SimpleMetadata {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    fn resolve(
        rules: &CalendarRules,
        path: &str,
        frontmatter: &SimpleMetadata,
    ) -> Option<NoteCalendar> {
        rules.resolve(&PathBuf::from(path), Some(frontmatter))
    }

    #[test]
    fn resolution_prefers_the_filename() {
        let rules = CalendarRules::default();
        let front = fm(&[("date", json!("2020-01-01"))]);
        let periodic = resolve(&rules, "daily/2026-10-31.md", &front).unwrap();
        assert_eq!(periodic.date, ymd(2026, 10, 31));
        assert_eq!(periodic.source, DateSource::Filename);
        assert_eq!(periodic.period, Some(Period::Day(ymd(2026, 10, 31))));

        let week = resolve(&rules, "weekly/2026-W44.md", &front).unwrap();
        assert_eq!(
            week.date,
            ymd(2026, 10, 26),
            "a week note dates to its Monday"
        );

        let meeting = resolve(&rules, "meetings/2026-10-31 Acme sync.md", &front).unwrap();
        assert_eq!(meeting.date, ymd(2026, 10, 31));
        assert_eq!(meeting.period, None, "a prefix date is not periodic");

        let post = resolve(&rules, "posts/hello.md", &front).unwrap();
        assert_eq!(post.date, ymd(2020, 1, 1));
        assert_eq!(post.source, DateSource::Frontmatter);
    }

    #[test]
    fn resolution_walks_date_fields_in_order() {
        let rules = CalendarRules::default();
        let front = fm(&[("date", json!("2026-10")), ("created", json!("2026-09-01"))]);
        let cal = resolve(&rules, "a.md", &front).unwrap();
        assert_eq!(cal.date, ymd(2026, 9, 1), "a partial date falls through");
        assert_eq!(resolve(&rules, "a.md", &fm(&[("date", json!(2026))])), None);
        assert_eq!(rules.resolve(Path::new("a.md"), None), None);

        let custom = CalendarRules::new(&CalendarConfig {
            date_fields: vec!["taxonomy.published".into()],
            ..CalendarConfig::default()
        });
        let front = fm(&[
            ("date", json!("2026-01-01")),
            ("taxonomy.published", json!("2026-02-02")),
        ]);
        assert_eq!(
            resolve(&custom, "a.md", &front).unwrap().date,
            ymd(2026, 2, 2)
        );
    }

    #[test]
    fn filters_keep_a_day_stems_date_only() {
        let rules = CalendarRules::new(&CalendarConfig {
            periods: vec![PeriodKind::Day, PeriodKind::Week],
            periodic_folders: vec!["./journal/".into(), "work\\weekly".into()],
            ..CalendarConfig::default()
        });
        let none = SimpleMetadata::new();
        let inside = resolve(&rules, "journal/2026/2026-10-31.md", &none).unwrap();
        assert!(inside.period.is_some(), "recursive folder match");
        let outside = resolve(&rules, "archive/2026-10-31.md", &none).unwrap();
        assert_eq!(outside.period, None);
        assert_eq!(
            outside.date,
            ymd(2026, 10, 31),
            "a day stem still dates the note"
        );
        assert_eq!(
            resolve(&rules, "journalism/2026-10-31.md", &none)
                .unwrap()
                .period,
            None,
            "a folder name prefix is not the folder"
        );
        assert!(
            resolve(&rules, "work/weekly/2026-W44.md", &none)
                .unwrap()
                .period
                .is_some()
        );
        assert_eq!(
            resolve(&rules, "journal/1984.md", &none),
            None,
            "a disabled kind's stem neither dates nor makes periodic"
        );

        let off = CalendarRules::new(&CalendarConfig {
            enabled: false,
            ..CalendarConfig::default()
        });
        let day = resolve(&off, "2026-10-31.md", &none).unwrap();
        assert_eq!((day.date, day.period), (ymd(2026, 10, 31), None));
    }

    /// The owner's switches: by default a period-named note is periodic
    /// anywhere (`books/1984.md` is a year note); `periodic_folders` narrows
    /// that; `periods = []` turns periodic detection off while a day stem
    /// still dates its note.
    #[test]
    fn periodic_detection_default_narrowed_and_off() {
        let none = SimpleMetadata::new();
        let year_1984 = Some(Period::Year(1984));

        let defaults = CalendarRules::default();
        assert_eq!(
            resolve(&defaults, "books/1984.md", &none).unwrap().period,
            year_1984,
            "default: periodic anywhere"
        );

        let narrowed = CalendarRules::new(&CalendarConfig {
            periodic_folders: vec!["journal".into()],
            ..CalendarConfig::default()
        });
        assert_eq!(
            resolve(&narrowed, "books/1984.md", &none),
            None,
            "outside journal: not periodic, and a year stem is not a date"
        );
        assert_eq!(
            resolve(&narrowed, "journal/1984.md", &none).unwrap().period,
            year_1984
        );

        let off = CalendarRules::new(&CalendarConfig {
            periods: Vec::new(),
            ..CalendarConfig::default()
        });
        assert_eq!(resolve(&off, "1984.md", &none), None);
        assert_eq!(resolve(&off, "books/1984.md", &none), None);
        for path in ["2026-W44.md", "2026-10.md", "2026-Q4.md"] {
            assert_eq!(resolve(&off, path, &none), None, "{path}");
        }
        let day = resolve(&off, "daily/2026-10-31.md", &none).unwrap();
        assert_eq!(
            (day.date, day.source, day.period),
            (ymd(2026, 10, 31), DateSource::Filename, None),
            "a day file name still dates the note"
        );
        let front = fm(&[("date", json!("2026-02-02"))]);
        assert_eq!(
            resolve(&off, "post.md", &front).unwrap().date,
            ymd(2026, 2, 2),
            "frontmatter dates still apply"
        );
    }

    // ---- PeriodicIndex ----

    fn entry(key: &str, url: &str) -> PeriodicEntry {
        PeriodicEntry {
            period: Period::parse_key(key).unwrap(),
            url_path: url.to_string(),
            raw_path: url.trim_matches('/').to_string() + ".md",
            title: key.to_string(),
        }
    }

    #[test]
    fn index_skips_gaps_and_lists_containers() {
        let index = PeriodicIndex::build([
            entry("2026-10-30", "/daily/2026-10-30/"),
            entry("2026-11-02", "/daily/2026-11-02/"),
            entry("2026-10-31", "/daily/2026-10-31/"),
            entry("2026-W44", "/weekly/2026-W44/"),
            entry("2026-10", "/2026-10/"),
            entry("2026", "/2026/"),
        ]);
        let day = Period::parse_key("2026-10-31").unwrap();
        let nav = index.nav(&day);
        assert_eq!(nav.label, "Saturday, October 31, 2026");
        assert_eq!(nav.prev.as_ref().unwrap().url, "/daily/2026-10-30/");
        assert_eq!(nav.next.as_ref().unwrap().url, "/daily/2026-11-02/");
        assert_eq!(nav.next.as_ref().unwrap().label, "Mon, Nov 2");
        let up: Vec<_> = nav.up.iter().map(|l| (l.kind, l.url.as_str())).collect();
        assert_eq!(
            up,
            [
                (PeriodKind::Week, "/weekly/2026-W44/"),
                (PeriodKind::Month, "/2026-10/"),
                (PeriodKind::Year, "/2026/"),
            ],
            "only the containers that exist, smallest first"
        );

        let first = index.nav(&Period::parse_key("2026-10-30").unwrap());
        assert!(first.prev.is_none());
        let week = index.nav(&Period::parse_key("2026-W44").unwrap());
        assert!(
            week.prev.is_none() && week.next.is_none(),
            "kinds do not mix"
        );
        assert_eq!(index.len(), 6);
    }

    #[test]
    fn index_duplicates_first_by_url_path_wins() {
        let index = PeriodicIndex::build([
            entry("2026-10-31", "/daily/2026-10-31/"),
            entry("2026-10-31", "/archive/20261031/"),
        ]);
        let day = Period::parse_key("2026-10-31").unwrap();
        assert_eq!(index.get(&day).unwrap().url_path, "/archive/20261031/");
        assert_eq!(
            index.duplicate_of("/daily/2026-10-31/"),
            Some(&PeriodicDuplicate {
                period: day,
                kept: "/archive/20261031/".into()
            })
        );
        assert_eq!(index.duplicate_of("/archive/20261031/"), None);
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn nav_urls_can_be_rewritten() {
        let index = PeriodicIndex::build([
            entry("2026-10-30", "/daily/2026-10-30/"),
            entry("2026-10-31", "/daily/2026-10-31/"),
            entry("2026-10", "/2026-10/"),
        ]);
        let nav = index
            .nav(&Period::parse_key("2026-10-31").unwrap())
            .map_urls(|u| format!("..{u}"));
        assert_eq!(nav.prev.unwrap().url, "../daily/2026-10-30/");
        assert_eq!(nav.up[0].url, "../2026-10/");
    }

    fn calendar_from_toml(toml: &str) -> Result<CalendarConfig, String> {
        use figment::providers::Format;
        figment::Figment::from(figment::providers::Toml::string(toml))
            .extract()
            .map_err(|e| e.to_string())
    }

    #[test]
    fn calendar_config_rejects_unknown_keys() {
        let err = calendar_from_toml("perods = [\"day\"]").unwrap_err();
        assert!(err.to_string().contains("perods"), "{err}");
        assert!(calendar_from_toml("periods = [\"fortnight\"]").is_err());
        let ok = calendar_from_toml(
            "periods = [\"day\"]\ncompanion = \"on-demand\"\nweek_start = \"sunday\"",
        )
        .unwrap();
        assert_eq!(ok.periods, [PeriodKind::Day]);
        assert_eq!(ok.companion, CompanionMode::OnDemand);
        assert_eq!(ok.week_start, WeekStart::Sunday);
        assert!(
            ok.enabled && ok.agenda_chips,
            "unset keys keep their defaults"
        );
    }

    // ---- Properties ----

    fn any_period() -> impl Strategy<Value = Period> {
        prop_oneof![
            (1000i32..=9999, 0u32..366).prop_map(|(y, d)| {
                Period::Day(
                    NaiveDate::from_yo_opt(y, d + 1)
                        .unwrap_or_else(|| NaiveDate::from_yo_opt(y, 1).unwrap()),
                )
            }),
            (1000i32..=9999, 1u32..=53).prop_map(|(y, w)| Period::week(y, w).unwrap_or(
                Period::Week {
                    iso_year: y,
                    week: 1
                }
            )),
            (1000i32..=9999, 1u32..=12).prop_map(|(year, month)| Period::Month { year, month }),
            (1000i32..=9999, 1u32..=4)
                .prop_map(|(year, quarter)| Period::Quarter { year, quarter }),
            (1000i32..=9999).prop_map(Period::Year),
        ]
    }

    fn any_kind() -> impl Strategy<Value = PeriodKind> {
        prop::sample::select(PeriodKind::ALL.to_vec())
    }

    proptest! {
        #[test]
        fn parse_key_round_trips(p in any_period()) {
            prop_assert_eq!(Period::parse_key(&p.key()), Some(p));
            prop_assert_eq!(period_from_stem(&p.key()), Some(p));
        }

        #[test]
        fn start_is_not_after_end(p in any_period()) {
            prop_assert!(p.start() <= p.end());
            prop_assert_eq!(Period::of_date(p.start(), p.kind()), p);
            prop_assert_eq!(Period::of_date(p.end(), p.kind()), p);
        }

        #[test]
        fn containing_contains_the_day(y in 1000i32..=9999, d in 1u32..=366, kind in any_kind()) {
            let day = NaiveDate::from_yo_opt(y, d).unwrap_or_else(|| NaiveDate::from_yo_opt(y, 1).unwrap());
            let container = Period::Day(day).containing(kind);
            prop_assert!(container.start() <= day && day <= container.end(), "{} in {}", day, container.key());
        }

        #[test]
        fn stem_parsing_never_panics(s in "\\PC{0,14}") {
            let _ = period_from_stem(&s);
            let _ = date_prefix_of_stem(&s);
            let _ = date_from_frontmatter_value(&serde_json::Value::String(s.clone()));
            let _ = Period::parse_key(&s);
        }

        #[test]
        fn digit_shaped_stems_never_panic(s in "[0-9WwQq_ .T:+Z-]{0,26}") {
            if let Some(p) = period_from_stem(&s) {
                prop_assert!(p.start() <= p.end());
            }
            let _ = date_prefix_of_stem(&s);
            let _ = date_from_frontmatter_value(&serde_json::Value::String(s));
        }
    }
}
