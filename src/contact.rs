//! Contact model for `type: person` and `type: organization` notes.
//!
//! Pure: no I/O, no clock. [`Contact::from_yaml`] reads the **raw** frontmatter
//! rather than the simplified map every other page feature uses, because the
//! simplifier is lossy in exactly the places contacts live — it drops arrays of
//! maps (`phones: [{mobile: …}]`, labeled `aliases`) and flattens nested maps
//! into dot keys. The relationship parser takes the same raw path for the same
//! reason.
//!
//! # Shapes accepted
//!
//! The frontmatter schema is meant to round-trip through vCard 3/4, Apple
//! `CNContact` and the Google People API, all of which model *labeled
//! multi-values*. A YAML author can spell one three ways, and all three read
//! identically for unique labels (pinned by a proptest):
//!
//! ```yaml
//! emails: {work: jane@abc.com, home: jane@gmail.com}   # map: label → value
//! emails: [{work: jane@abc.com}, jane@gmail.com]        # list: order = preference, bare = no label
//! emails: jane@abc.com                                  # single bare value
//! ```
//!
//! `{label: work, value: jane@abc.com}` list items are tolerated too
//! (undocumented). Anything else — a multi-key map, a nested structure where a
//! scalar belongs — is skipped and recorded in [`Contact::problems`], which the
//! page-problems panel shows; a contact card never fails to render over bad data.
//!
//! # Shared helpers
//!
//! [`alias_names`] is the **one** definition of "the names in an `aliases`
//! field". The frontmatter simplifier flattens `aliases` through it, so every
//! consumer of the simplified map (name index, wikilink index, link grep,
//! rename rewriting, search) sees the labeled names too without each re-parsing
//! the mixed list. [`normalize_simplified`] is that simplifier hook.

use serde::Serialize;
use serde_json::{Map, Value};
use yaml_rust2::Yaml;

use crate::markdown::SimpleMetadata;

/// The frontmatter key holding a note's alternate names.
pub const ALIASES_KEY: &str = "aliases";

/// Contact-detail keys that only ever appear on the page itself.
///
/// They are kept out of `site.json` (see [`is_public_frontmatter_key`]): that file is
/// fetched by every page view, often published with a static site, and a phone
/// number has no business in a sidebar index.
pub const PRIVATE_DETAIL_KEYS: &[&str] = &["emails", "phones", "urls", "social", "im", "addresses"];

/// Well-known date labels, in display order. Others follow in authored order.
const WELL_KNOWN_DATES: &[&str] = &["birthday", "death", "anniversary"];

/// Field names that mark a list item as an *address object* rather than a
/// one-key `label: value` pair (`- street: "1 Main St"` is an address with no
/// label, not an address labeled "street").
const ADDRESS_FIELDS: &[&str] = &[
    "street",
    "city",
    "region",
    "postcode",
    "country",
    "country_code",
];

/// Which kind of contact a note is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContactKind {
    Person,
    Organization,
}

impl ContactKind {
    /// Reads a frontmatter `type`. Exact (trimmed) match, the same test the
    /// templates and the frontend apply, so the three can never disagree about
    /// which notes are contacts.
    pub fn from_type(value: &str) -> Option<Self> {
        match value.trim() {
            "person" => Some(Self::Person),
            "organization" => Some(Self::Organization),
            _ => None,
        }
    }
}

/// A date that may be missing its year, month or day.
///
/// Genealogy and address-book data are full of partial dates — a birthday
/// without a year is the *normal* case in a phone's contacts — so the year is
/// as optional as the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialDate {
    Full { year: u16, month: u8, day: u8 },
    YearMonth { year: u16, month: u8 },
    Year { year: u16 },
    MonthDay { month: u8, day: u8 },
}

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

impl PartialDate {
    /// Parses `YYYY-MM-DD`, `YYYY-MM`, `YYYY`, `MM-DD` and `--MM-DD` (the vCard
    /// spelling of a yearless date). Components must be zero-padded and in
    /// range; February allows the 29th since the year may be unknown. Anything
    /// else — prose like "circa 1855", a two-digit year — is `None`, and the
    /// caller shows the authored text unchanged.
    pub fn parse(input: &str) -> Option<Self> {
        let s = input.trim();
        if let Some(md) = s.strip_prefix("--") {
            return Self::parse_month_day(md);
        }
        let parts: Vec<&str> = s.split('-').collect();
        match parts.as_slice() {
            [y, m, d] => {
                let (year, month) = (year4(y)?, month2(m)?);
                let day = day2(d, month)?;
                Some(Self::Full { year, month, day })
            }
            [y, m] if y.len() == 4 => Some(Self::YearMonth {
                year: year4(y)?,
                month: month2(m)?,
            }),
            [_, _] => Self::parse_month_day(s),
            [y] => Some(Self::Year { year: year4(y)? }),
            _ => None,
        }
    }

    fn parse_month_day(s: &str) -> Option<Self> {
        let (m, d) = s.split_once('-')?;
        let month = month2(m)?;
        Some(Self::MonthDay {
            month,
            day: day2(d, month)?,
        })
    }

    /// Reader-facing form: "March 19, 1927", "March 1927", "1927", "March 19".
    pub fn display(&self) -> String {
        let month_name = |m: u8| MONTH_NAMES[usize::from(m) - 1];
        match *self {
            Self::Full { year, month, day } => format!("{} {day}, {year}", month_name(month)),
            Self::YearMonth { year, month } => format!("{} {year}", month_name(month)),
            Self::Year { year } => year.to_string(),
            Self::MonthDay { month, day } => format!("{} {day}", month_name(month)),
        }
    }

    /// The HTML `<time datetime>` spelling: like [`Self::iso`], except that a
    /// yearless date is `MM-DD` (HTML's "yearless date string"), not vCard's
    /// `--MM-DD`.
    pub fn html_datetime(&self) -> String {
        match *self {
            Self::MonthDay { month, day } => format!("{month:02}-{day:02}"),
            other => other.iso(),
        }
    }

    /// Canonical machine form: ISO 8601 for dated forms, `--MM-DD` (vCard) for
    /// a yearless one. This is what `site.json` carries.
    pub fn iso(&self) -> String {
        match *self {
            Self::Full { year, month, day } => format!("{year:04}-{month:02}-{day:02}"),
            Self::YearMonth { year, month } => format!("{year:04}-{month:02}"),
            Self::Year { year } => format!("{year:04}"),
            Self::MonthDay { month, day } => format!("--{month:02}-{day:02}"),
        }
    }
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn year4(s: &str) -> Option<u16> {
    (s.len() == 4 && all_digits(s))
        .then(|| s.parse().ok())
        .flatten()
}

fn month2(s: &str) -> Option<u8> {
    (s.len() == 2 && all_digits(s))
        .then(|| s.parse().ok())
        .flatten()
        .filter(|m| (1..=12).contains(m))
}

fn day2(s: &str, month: u8) -> Option<u8> {
    const DAYS: [u8; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    (s.len() == 2 && all_digits(s))
        .then(|| s.parse().ok())
        .flatten()
        .filter(|d| (1..=DAYS[usize::from(month) - 1]).contains(d))
}

/// Humanizes a date string for display, passing anything unparseable through
/// unchanged. Backs the `humandate` Tera filter.
pub fn humanize_date(input: &str) -> String {
    PartialDate::parse(input)
        .map(|d| d.display())
        .unwrap_or_else(|| input.to_string())
}

/// The parts of a person's name. All optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NameParts {
    pub prefix: Option<String>,
    pub first: Option<String>,
    pub middle: Option<String>,
    pub last: Option<String>,
    pub suffix: Option<String>,
}

impl NameParts {
    /// "Dr. Jane Q. Doe Jr." — the parts present, space-joined.
    fn full(&self) -> Option<String> {
        let joined = [
            &self.prefix,
            &self.first,
            &self.middle,
            &self.last,
            &self.suffix,
        ]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
        (!joined.is_empty()).then_some(joined)
    }
}

/// A `company` value: plain text, or a wikilink to an organization note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Company {
    /// Display text: the wikilink's display/target, or the plain text.
    pub name: String,
    /// The value exactly as authored (e.g. `[[Acme Corp]]`). This is the
    /// `neighbor_raw` of the implied `employer` relationship, which is how
    /// [`Contact::resolve_company`] finds it.
    pub raw: String,
    /// Whether the value was a wikilink (and so may resolve to a note).
    pub is_link: bool,
    /// The resolved note URL, filled in by [`Contact::resolve_company`].
    pub url: Option<String>,
}

/// One alternate name, optionally labeled (`maiden_name: Mary Smith`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Alias {
    pub label: Option<String>,
    pub name: String,
}

/// One labeled value of a multi-value field (an email, a phone, a URL…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Labeled {
    /// The label as authored (`home_fax`), if any.
    pub label: Option<String>,
    /// The label for display (`Home fax`).
    pub label_display: Option<String>,
    pub value: String,
    /// A link target, only ever `mailto:`, `tel:`, `http:` or `https:`. `None`
    /// when the value does not make a safe link of its field's kind — the card
    /// then shows it as text. Never derived from anything but the value, so a
    /// `javascript:` URL in frontmatter cannot become a link.
    pub href: Option<String>,
}

/// One postal address.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Address {
    pub label: Option<String>,
    pub label_display: Option<String>,
    pub street: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub postcode: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
    /// Display lines, street first. A plain-string address is one or more
    /// lines of preformatted text; a structured one is assembled as
    /// street / "city, region postcode" / country.
    pub lines: Vec<String>,
}

/// One labeled date (`birthday`, `first_met`, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabeledDate {
    pub label: String,
    pub label_display: String,
    /// The value as authored (a YAML integer year is stringified).
    pub raw: String,
    /// `PartialDate::iso` when parseable, else `raw`.
    pub value: String,
    /// `PartialDate::display` when parseable, else `raw`.
    pub display: String,
    /// A valid HTML `<time datetime>` value (`PartialDate::html_datetime`), or
    /// `None` for unparseable text.
    pub datetime: Option<String>,
    /// `born_place` on the birthday, `died_place` on the death date.
    pub place: Option<String>,
    #[serde(skip)]
    pub date: Option<PartialDate>,
}

/// A malformed contact field, reported in the page-problems panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContactProblem {
    /// The frontmatter key at fault (`phones`, `aliases`, …).
    pub field: String,
    pub message: String,
}

/// Everything the contact card shows, parsed from one note's frontmatter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Contact {
    pub kind: ContactKind,
    /// The headline: `title` when set, else the name parts (person) or the
    /// company (organization). `None` when there is nothing to show.
    pub display_name: Option<String>,
    pub name: NameParts,
    pub company: Option<Company>,
    pub department: Option<String>,
    pub job_title: Option<String>,
    /// `image` as authored; the renderer rewrites relative paths for the
    /// page's own URL before the card sees it.
    pub image: Option<String>,
    pub gender: Option<String>,
    pub aliases: Vec<Alias>,
    /// Ready-to-show alias phrases ("aka Mare, Bob", "née Mary Smith", …).
    pub alias_phrases: Vec<String>,
    pub emails: Vec<Labeled>,
    pub phones: Vec<Labeled>,
    pub urls: Vec<Labeled>,
    pub social: Vec<Labeled>,
    pub im: Vec<Labeled>,
    pub addresses: Vec<Address>,
    /// Well-known labels first (birthday, death, anniversary), then authored order.
    pub dates: Vec<LabeledDate>,
    pub born_place: Option<String>,
    /// Places whose date is unknown (`born_place` with no birthday), shown as
    /// their own lines in the Dates section.
    pub places: Vec<Labeled>,
    pub died_place: Option<String>,
    pub problems: Vec<ContactProblem>,
}

/// How a field's values become links.
#[derive(Clone, Copy)]
enum HrefKind {
    Email,
    Phone,
    Web,
}

impl Contact {
    /// Parses a contact from raw YAML frontmatter, or `None` when the note is
    /// not `type: person` / `type: organization`.
    ///
    /// Checks the type on the YAML before converting anything, so the cost for
    /// every other note is one hash lookup.
    pub fn from_yaml(yaml: &Yaml) -> Option<Self> {
        let hash = yaml.as_hash()?;
        let kind = hash
            .get(&Yaml::String("type".to_string()))
            .and_then(Yaml::as_str)
            .and_then(ContactKind::from_type)?;
        match crate::relationships::yaml_to_json(yaml) {
            Value::Object(fm) => Some(Self::from_map(kind, &fm)),
            _ => None,
        }
    }

    /// Parses a contact from frontmatter already converted to JSON.
    pub fn from_frontmatter(fm: &Map<String, Value>) -> Option<Self> {
        let kind = fm
            .get("type")
            .and_then(Value::as_str)
            .and_then(ContactKind::from_type)?;
        Some(Self::from_map(kind, fm))
    }

    fn from_map(kind: ContactKind, fm: &Map<String, Value>) -> Self {
        let mut problems = Vec::new();
        let text = |key: &str| fm.get(key).and_then(scalar_text);

        let name = NameParts {
            prefix: text("prefix"),
            first: text("first_name"),
            middle: text("middle_name"),
            last: text("last_name"),
            suffix: text("suffix"),
        };
        let company = text("company").map(|raw| parse_company(&raw));
        let display_name = text("title").or_else(|| match kind {
            ContactKind::Person => name.full(),
            ContactKind::Organization => company.as_ref().map(|c| c.name.clone()),
        });

        let aliases = parse_aliases(fm.get(ALIASES_KEY), Some(&mut problems));
        let alias_phrases = alias_phrases(&aliases);

        let mut labeled = |key: &str, kind: HrefKind| {
            labeled_values(key, fm.get(key), &mut problems)
                .into_iter()
                .filter_map(|(label, value)| {
                    let value = scalar_text(&value)?;
                    Some(Labeled {
                        label_display: label.as_deref().map(humanize_label),
                        href: href_for(kind, &value),
                        label,
                        value,
                    })
                })
                .collect::<Vec<_>>()
        };
        let emails = labeled("emails", HrefKind::Email);
        let phones = labeled("phones", HrefKind::Phone);
        let urls = labeled("urls", HrefKind::Web);
        let social = labeled("social", HrefKind::Web);
        let im = labeled("im", HrefKind::Web);

        let addresses = labeled_values("addresses", fm.get("addresses"), &mut problems)
            .into_iter()
            .filter_map(|(label, value)| parse_address(label, &value, &mut problems))
            .collect();
        let mut dates = parse_dates(fm, Some(&mut problems));
        let (born_place, died_place) = (text("born_place"), text("died_place"));
        let places = attach_places(&mut dates, born_place.as_ref(), died_place.as_ref());

        Self {
            kind,
            display_name,
            name,
            company,
            department: text("department"),
            job_title: text("job_title"),
            image: text("image"),
            gender: text("gender"),
            aliases,
            alias_phrases,
            emails,
            phones,
            urls,
            social,
            im,
            addresses,
            dates,
            born_place,
            died_place,
            places,
            problems,
        }
    }

    /// Fills [`Company::url`] from the note's resolved relationships: the
    /// implied `employer` edge whose raw endpoint is exactly the authored
    /// `company`. Reading it back from the index (rather than resolving the
    /// wikilink again here) guarantees the card links to the same note the
    /// relationship graph does.
    pub fn resolve_company(
        &mut self,
        relationships: &[crate::relationships::ResolvedRelationship],
    ) {
        let Some(company) = self.company.as_mut().filter(|c| c.is_link) else {
            return;
        };
        company.url = relationships
            .iter()
            .find(|r| {
                r.resolved
                    && r.direction == crate::relationships::Direction::Outgoing
                    && r.rel_type
                        .eq_ignore_ascii_case(crate::relationships::COMPANY_RELATION_TYPE)
                    && r.neighbor_raw.trim() == company.raw
            })
            .map(|r| r.neighbor.clone());
    }
}

/// The text of a scalar YAML/JSON value, trimmed; `None` for empty strings,
/// nulls and structures. Numbers are kept (a postcode or a year often arrives
/// as one).
fn scalar_text(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    (!s.is_empty()).then_some(s)
}

fn parse_company(raw: &str) -> Company {
    let link = raw
        .strip_prefix("[[")
        .and_then(|r| r.strip_suffix("]]"))
        .map(str::trim)
        .filter(|inner| !inner.is_empty());
    match link {
        Some(inner) => {
            let (target, shown) = inner.split_once('|').unwrap_or((inner, inner));
            let name = Some(shown.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or(target.trim());
            Company {
                name: name.to_string(),
                raw: raw.to_string(),
                is_link: true,
                url: None,
            }
        }
        None => Company {
            name: raw.to_string(),
            raw: raw.to_string(),
            is_link: false,
            url: None,
        },
    }
}

/// Splits a labeled multi-value field into `(label, value)` pairs, in authored
/// order. See the module docs for the accepted shapes.
///
/// `value` is returned unconverted so addresses can be structured; callers
/// that want a scalar reject anything else themselves.
fn labeled_values(
    field: &str,
    v: Option<&Value>,
    problems: &mut Vec<ContactProblem>,
) -> Vec<(Option<String>, Value)> {
    let mut report = |message: String| {
        problems.push(ContactProblem {
            field: field.to_string(),
            message,
        })
    };
    match v {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| match labeled_item(field, item) {
                Some(pair) => Some(pair),
                None => {
                    report(format!(
                        "entry {} is not a value, a one-key `label: value` map or \
                         `{{label, value}}`; ignored",
                        i + 1
                    ));
                    None
                }
            })
            .collect(),
        Some(Value::Object(map)) => {
            if let Some(pair) = label_value_object(map) {
                return vec![pair];
            }
            if field == "addresses" && is_address_object(map) {
                return vec![(None, Value::Object(map.clone()))];
            }
            map.iter().map(|(k, v)| (non_empty(k), v.clone())).collect()
        }
        Some(scalar) => vec![(None, scalar.clone())],
    }
}

/// One list item of a labeled field, or `None` when it has an unusable shape.
fn labeled_item(field: &str, item: &Value) -> Option<(Option<String>, Value)> {
    match item {
        Value::Object(map) => label_value_object(map)
            .or_else(|| {
                (field == "addresses" && is_address_object(map)).then(|| {
                    let label = map.get("label").and_then(scalar_text);
                    let mut rest = map.clone();
                    rest.remove("label");
                    (label, Value::Object(rest))
                })
            })
            .or_else(|| match map.len() {
                1 => map.iter().next().map(|(k, v)| (non_empty(k), v.clone())),
                _ => None,
            }),
        Value::Array(_) | Value::Null => None,
        scalar => Some((None, scalar.clone())),
    }
}

/// The tolerated `{label: work, value: x}` (or `{label, name}`) object form.
fn label_value_object(map: &Map<String, Value>) -> Option<(Option<String>, Value)> {
    if map.len() != 2 {
        return None;
    }
    let label = map.get("label")?;
    let value = map.get("value").or_else(|| map.get("name"))?;
    Some((scalar_text(label), value.clone()))
}

fn is_address_object(map: &Map<String, Value>) -> bool {
    map.keys().any(|k| ADDRESS_FIELDS.contains(&k.as_str()))
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn parse_address(
    label: Option<String>,
    value: &Value,
    problems: &mut Vec<ContactProblem>,
) -> Option<Address> {
    let label_display = label.as_deref().map(humanize_label);
    match value {
        Value::Object(map) => {
            let field = |k: &str| map.get(k).and_then(scalar_text);
            let street = field("street");
            let (city, region, postcode) = (field("city"), field("region"), field("postcode"));
            let country = field("country");
            // "City, Region Postcode" — the common Western single-line form;
            // anything locale-specific belongs in a preformatted string address.
            let tail = [&region, &postcode]
                .into_iter()
                .flatten()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            let locality = match (&city, tail.is_empty()) {
                (Some(c), true) => Some(c.clone()),
                (Some(c), false) => Some(format!("{c}, {tail}")),
                (None, false) => Some(tail),
                (None, true) => None,
            };
            let lines: Vec<String> = street
                .iter()
                .flat_map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()))
                .map(str::to_string)
                .chain(locality)
                .chain(country.clone())
                .collect();
            if lines.is_empty() {
                problems.push(ContactProblem {
                    field: "addresses".to_string(),
                    message: format!(
                        "address{} has none of street, city, region, postcode or country; ignored",
                        label
                            .as_deref()
                            .map(|l| format!(" `{l}`"))
                            .unwrap_or_default()
                    ),
                });
                return None;
            }
            Some(Address {
                label,
                label_display,
                street,
                city,
                region,
                postcode,
                country,
                country_code: field("country_code"),
                lines,
            })
        }
        other => {
            let text = scalar_text(other)?;
            Some(Address {
                label,
                label_display,
                lines: text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect(),
                ..Address::default()
            })
        }
    }
}

/// Parses an `aliases` value: a bare string, a list of strings and one-key
/// `label: name` maps (`{label, name}` objects tolerated), or a map of
/// label → name. Malformed entries are skipped and, when `problems` is given,
/// reported.
fn parse_aliases(v: Option<&Value>, mut problems: Option<&mut Vec<ContactProblem>>) -> Vec<Alias> {
    let mut sink = Vec::new();
    let items = labeled_values(ALIASES_KEY, v, &mut sink);
    if let Some(p) = problems.as_deref_mut() {
        p.extend(sink);
    }
    items
        .into_iter()
        .filter_map(|(label, value)| match scalar_text(&value) {
            Some(name) => Some(Alias { label, name }),
            None => {
                if let Some(p) = problems.as_deref_mut() {
                    p.push(ContactProblem {
                        field: ALIASES_KEY.to_string(),
                        message: format!(
                            "alias{} is not a plain name; ignored",
                            label.map(|l| format!(" `{l}`")).unwrap_or_default()
                        ),
                    });
                }
                None
            }
        })
        .collect()
}

/// Every name in an `aliases` value, labels dropped, authored order, deduped.
///
/// The single definition shared by the frontmatter simplifier and therefore by
/// every name-resolution consumer. Accepts `aliases: Bob` as `["Bob"]`.
pub fn alias_names(v: Option<&Value>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    parse_aliases(v, None)
        .into_iter()
        .map(|a| a.name)
        .filter(|n| seen.insert(n.clone()))
        .collect()
}

/// Alias names read from a (simplified) frontmatter map.
pub fn alias_names_in(frontmatter: &SimpleMetadata) -> Vec<String> {
    alias_names(frontmatter.get(ALIASES_KEY))
}

/// Short, readable phrases for the card: all unlabeled names as one "aka"
/// phrase, then one phrase per labeled alias.
fn alias_phrases(aliases: &[Alias]) -> Vec<String> {
    let unlabeled: Vec<&str> = aliases
        .iter()
        .filter(|a| a.label.is_none())
        .map(|a| a.name.as_str())
        .collect();
    let aka = (!unlabeled.is_empty()).then(|| format!("aka {}", unlabeled.join(", ")));
    let labeled = aliases.iter().filter_map(|a| {
        let label = a.label.as_deref()?;
        Some(match label.to_lowercase().replace('-', "_").as_str() {
            "maiden_name" | "birth_name" | "nee" | "née" => format!("née {}", a.name),
            "nickname" => format!("Nickname: {}", a.name),
            "formerly" | "former_name" | "previous_name" => format!("formerly {}", a.name),
            _ => format!("{}: {}", humanize_label(label), a.name),
        })
    });
    aka.into_iter().chain(labeled).collect()
}

/// Parses `dates` plus the legacy `born`/`died`, in display order.
///
/// `dates.birthday` wins over `born` (and `death` over `died`) when both are
/// present; the legacy keys are otherwise read as those labels.
fn parse_dates(
    fm: &Map<String, Value>,
    problems: Option<&mut Vec<ContactProblem>>,
) -> Vec<LabeledDate> {
    let mut sink = Vec::new();
    let authored: Vec<LabeledDate> = labeled_values("dates", fm.get("dates"), &mut sink)
        .into_iter()
        .filter_map(|(label, value)| {
            let raw = scalar_text(&value);
            match (label, raw) {
                (Some(label), Some(raw)) => Some(labeled_date(label, raw)),
                (label, _) => {
                    sink.push(ContactProblem {
                        field: "dates".to_string(),
                        message: match label {
                            Some(l) => format!("date `{l}` is not a single value; ignored"),
                            None => {
                                "a date needs a label (`birthday: 1927-03-19`); ignored".to_string()
                            }
                        },
                    });
                    None
                }
            }
        })
        .collect();
    if let Some(p) = problems {
        p.extend(sink);
    }

    let has = |label: &str| authored.iter().any(|d| d.label.eq_ignore_ascii_case(label));
    let legacy = [("born", "birthday"), ("died", "death")]
        .into_iter()
        .filter(|(_, label)| !has(label))
        .filter_map(|(key, label)| {
            let raw = fm.get(key).and_then(scalar_text)?;
            Some(labeled_date(label.to_string(), raw))
        })
        .collect::<Vec<_>>();

    let rank = |d: &LabeledDate| {
        WELL_KNOWN_DATES
            .iter()
            .position(|w| d.label.eq_ignore_ascii_case(w))
            .unwrap_or(WELL_KNOWN_DATES.len())
    };
    let mut all: Vec<LabeledDate> = authored.into_iter().chain(legacy).collect();
    // Stable: authored order survives within the "other labels" bucket.
    all.sort_by_key(rank);
    all
}

fn labeled_date(label: String, raw: String) -> LabeledDate {
    let date = PartialDate::parse(&raw);
    LabeledDate {
        label_display: humanize_label(&label),
        value: date.map(|d| d.iso()).unwrap_or_else(|| raw.clone()),
        display: date.map(|d| d.display()).unwrap_or_else(|| raw.clone()),
        datetime: date.map(|d| d.html_datetime()),
        place: None,
        label,
        raw,
        date,
    }
}

/// Hangs `born_place`/`died_place` on the birthday/death dates. A place with
/// no matching date is returned as a standalone entry ("Born: Boulder, CO").
fn attach_places(
    dates: &mut [LabeledDate],
    born_place: Option<&String>,
    died_place: Option<&String>,
) -> Vec<Labeled> {
    [
        ("birthday", born_place, "Born"),
        ("death", died_place, "Died"),
    ]
    .into_iter()
    .filter_map(|(label, place, shown)| {
        let place = place?;
        match dates
            .iter_mut()
            .find(|d| d.label.eq_ignore_ascii_case(label))
        {
            Some(date) => {
                date.place = Some(place.clone());
                None
            }
            None => Some(Labeled {
                label: Some(shown.to_lowercase()),
                label_display: Some(shown.to_string()),
                value: place.clone(),
                href: None,
            }),
        }
    })
    .collect()
}

/// Display form of a free-text label: `home_fax` → "Home fax", well-known
/// service names in their usual spelling (`linkedin` → "LinkedIn"). A label
/// that already has capitals is the author's chosen spelling and only has its
/// separators replaced.
pub fn humanize_label(label: &str) -> String {
    const SERVICES: &[(&str, &str)] = &[
        ("linkedin", "LinkedIn"),
        ("github", "GitHub"),
        ("gitlab", "GitLab"),
        ("youtube", "YouTube"),
        ("tiktok", "TikTok"),
        ("whatsapp", "WhatsApp"),
        ("bluesky", "Bluesky"),
        ("x", "X"),
        ("imessage", "iMessage"),
        ("facetime", "FaceTime"),
    ];
    let spaced = label.trim().replace(['_', '-'], " ");
    let lower = spaced.to_lowercase();
    if let Some((_, proper)) = SERVICES.iter().find(|(k, _)| *k == lower) {
        return (*proper).to_string();
    }
    if spaced.chars().any(char::is_uppercase) {
        return spaced;
    }
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A safe link for a value of the given kind, or `None`.
///
/// The scheme is always chosen *here*, never taken from the value — except for
/// web values, which are accepted only when they already start with
/// `http://`/`https://` (or `www.`, which gets `https://`). That is the whole
/// defence against `javascript:` and friends, so keep it an allowlist.
fn href_for(kind: HrefKind, value: &str) -> Option<String> {
    let v = value.trim();
    match kind {
        HrefKind::Email => (v.contains('@')
            && !v.contains(char::is_whitespace)
            && !v.contains([':', '/', '?', '#', '<', '>', '"']))
        .then(|| format!("mailto:{v}")),
        HrefKind::Phone => tel_href(v),
        HrefKind::Web => {
            let lower = v.to_ascii_lowercase();
            if v.contains(char::is_whitespace) {
                None
            } else if lower.starts_with("https://") || lower.starts_with("http://") {
                Some(v.to_string())
            } else if lower.starts_with("www.") {
                Some(format!("https://{v}"))
            } else {
                None
            }
        }
    }
}

/// A `tel:` URI for a phone number as people write one, or `None` when the
/// value is not a number.
///
/// Spaces, dots, parentheses and dashes are layout and are dropped; a leading
/// `+`, the digits, and the dial-string pause/wait characters `,` `;` `p` `w`
/// are kept. A trailing extension written `ext. 12`, `extension 12` or `x12`
/// becomes the RFC 3966 `;ext=12` parameter rather than being glued onto the
/// number. Any other letter means this is not a dialable number ("ask for
/// Pam"), so no link is made — the card still shows the text as authored.
fn tel_href(value: &str) -> Option<String> {
    let lower = value.trim().to_ascii_lowercase();
    let (number, extension) = split_extension(&lower);
    let mut out = String::with_capacity(number.len());
    let mut digits = 0usize;
    for (i, c) in number.chars().enumerate() {
        match c {
            '+' if i == 0 => out.push('+'),
            '0'..='9' => {
                digits += 1;
                out.push(c);
            }
            ',' | ';' | 'p' | 'w' if digits > 0 => out.push(c),
            ' ' | '.' | '(' | ')' | '-' | '\u{a0}' => {}
            _ => return None,
        }
    }
    let ext = match extension {
        Some(ext) if !ext.is_empty() && ext.bytes().all(|b| b.is_ascii_digit()) => {
            format!(";ext={ext}")
        }
        Some(_) => return None,
        None => String::new(),
    };
    (digits >= 3).then(|| format!("tel:{out}{ext}"))
}

/// Splits `"303 555 0100 ext. 12"` into the number and `"12"`. Recognises
/// `extension`, `ext`, `ext.` and `x`, each optionally followed by spaces.
fn split_extension(lower: &str) -> (&str, Option<String>) {
    ["extension", "ext.", "ext", "x"]
        .iter()
        .find_map(|marker| {
            let at = lower.rfind(marker)?;
            let rest: String = lower[at + marker.len()..]
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            Some((lower[..at].trim_end(), Some(rest)))
        })
        .unwrap_or((lower, None))
}

/// The frontmatter simplifier's contact hook. Called once per note on the
/// simplified map, with the raw YAML it came from:
///
/// - `aliases` becomes a flat array of names (labels dropped) for **every**
///   note, so labeled aliases resolve wikilinks and relationship endpoints.
/// - On person/organization notes, every date — `dates` in any accepted shape,
///   plus legacy `born`/`died` — is written as a `dates.<label>` string
///   ([`PartialDate::iso`] when parseable), the label **lowercased**. This is
///   the `site.json` contract the charts read; `born`/`died` stay as authored
///   for existing templates.
///
/// Costs nothing on a note with neither key.
pub fn normalize_simplified(hm: &mut SimpleMetadata, hash: &yaml_rust2::yaml::Hash) {
    let key = |k: &str| Yaml::String(k.to_string());
    if let Some(aliases) = hash.get(&key(ALIASES_KEY)) {
        let json = crate::relationships::yaml_to_json(aliases);
        let names = alias_names(Some(&json))
            .into_iter()
            .map(Value::String)
            .collect();
        hm.insert(ALIASES_KEY.to_string(), Value::Array(names));
    }

    let is_contact = hash
        .get(&key("type"))
        .and_then(Yaml::as_str)
        .and_then(ContactKind::from_type)
        .is_some();
    if !is_contact {
        return;
    }
    let date_keys = ["dates", "born", "died"];
    if !date_keys.iter().any(|k| hash.contains_key(&key(k))) {
        return;
    }
    let fm: Map<String, Value> = date_keys
        .iter()
        .filter_map(|k| {
            hash.get(&key(k))
                .map(|v| (k.to_string(), crate::relationships::yaml_to_json(v)))
        })
        .collect();
    for date in parse_dates(&fm, None) {
        // Lowercase, because every reader of a label matches it
        // case-insensitively (`has` and `rank` above) except a key lookup: the
        // charts read exactly `dates.birthday`. The generic simplifier has
        // already flattened a `dates:` map under the authored case, so that
        // copy goes, leaving one canonical key per date.
        let canonical = date.label.to_lowercase();
        if canonical != date.label {
            hm.remove(&format!("dates.{}", date.label));
        }
        hm.insert(format!("dates.{canonical}"), Value::String(date.value));
    }
}

/// The frontmatter as published in `site.json`: everything except the
/// [`PRIVATE_DETAIL_KEYS`], including their flattened `emails.work`-style
/// variants.
///
/// Applied at serialization time rather than in the simplifier on purpose: the
/// in-memory map still holds them, so server-side search can match a phone
/// number, while the file every page view downloads does not.
///
/// The root is compared **case-insensitively**: YAML keys are case-sensitive,
/// so `Emails:` or `PHONES.work` is a different key to the card reader, but
/// it is the same private data to a reader of `site.json`. Erring towards
/// withholding is the only safe direction for a privacy filter.
pub fn is_public_frontmatter_key(key: &str) -> bool {
    let root = key.split_once('.').map_or(key, |(root, _)| root);
    !PRIVATE_DETAIL_KEYS
        .iter()
        .any(|private| private.eq_ignore_ascii_case(root))
}

/// `serialize_with` adapter for `MarkdownInfo::frontmatter`: see
/// [`is_public_frontmatter_key`].
pub fn serialize_public_frontmatter<S: serde::Serializer>(
    frontmatter: &Option<SimpleMetadata>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    match frontmatter {
        None => serializer.serialize_none(),
        Some(fm) => {
            let public = fm.iter().filter(|(k, _)| is_public_frontmatter_key(k));
            let mut map = serializer.serialize_map(None)?;
            for (k, v) in public {
                map.serialize_entry(k, v)?;
            }
            map.end()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;
    use yaml_rust2::YamlLoader;

    fn yaml(s: &str) -> Yaml {
        YamlLoader::load_from_str(s)
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
    }

    fn contact(s: &str) -> Contact {
        Contact::from_yaml(&yaml(s)).expect("a contact")
    }

    // ----- PartialDate -----

    #[test]
    fn partial_date_forms() {
        use PartialDate::*;
        assert_eq!(
            PartialDate::parse("1927-03-19"),
            Some(Full {
                year: 1927,
                month: 3,
                day: 19
            })
        );
        assert_eq!(
            PartialDate::parse("1927-03"),
            Some(YearMonth {
                year: 1927,
                month: 3
            })
        );
        assert_eq!(PartialDate::parse("1898"), Some(Year { year: 1898 }));
        assert_eq!(
            PartialDate::parse("03-19"),
            Some(MonthDay { month: 3, day: 19 })
        );
        assert_eq!(
            PartialDate::parse("--03-19"),
            Some(MonthDay { month: 3, day: 19 })
        );
        assert_eq!(
            PartialDate::parse("--02-29"),
            Some(MonthDay { month: 2, day: 29 })
        );
    }

    #[test]
    fn partial_date_rejects_garbage() {
        for bad in [
            "",
            "circa 1855",
            "2020-13-40",
            "2020-00-10",
            "2020-02-32",
            "2020-02-30",
            "04-31",
            "1855-1-1",
            "55-10-30",
            "--3-19",
            "13-01",
            "1927-03-19-01",
        ] {
            assert_eq!(PartialDate::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn partial_date_display_and_iso() {
        let show = |s: &str| {
            let d = PartialDate::parse(s).unwrap();
            (d.display(), d.iso())
        };
        assert_eq!(
            show("1927-03-09"),
            ("March 9, 1927".into(), "1927-03-09".into())
        );
        assert_eq!(show("1927-03"), ("March 1927".into(), "1927-03".into()));
        assert_eq!(show("1898"), ("1898".into(), "1898".into()));
        assert_eq!(show("03-19"), ("March 19".into(), "--03-19".into()));
        assert_eq!(humanize_date("circa 1855"), "circa 1855");
        assert_eq!(humanize_date("--12-25"), "December 25");
    }

    // ----- headline -----

    #[test]
    fn display_name_prefers_title_then_parts_then_company() {
        assert_eq!(
            contact("type: person\ntitle: Jane\nfirst_name: J\n").display_name,
            Some("Jane".into())
        );
        let parts = contact(
            "type: person\nprefix: Dr.\nfirst_name: Jane\nmiddle_name: Q.\nlast_name: Doe\nsuffix: Jr.\n",
        );
        assert_eq!(parts.display_name.as_deref(), Some("Dr. Jane Q. Doe Jr."));
        assert_eq!(
            contact("type: organization\ncompany: Acme Corp\n").display_name,
            Some("Acme Corp".into())
        );
        assert_eq!(contact("type: person\n").display_name, None);
    }

    #[test]
    fn non_contacts_are_none() {
        assert!(Contact::from_yaml(&yaml("type: character\ntitle: Gandalf\n")).is_none());
        assert!(Contact::from_yaml(&yaml("title: x\n")).is_none());
        assert!(Contact::from_yaml(&yaml("- a\n")).is_none());
        assert!(Contact::from_yaml(&yaml("type: Person\n")).is_none());
    }

    // ----- company -----

    #[test]
    fn company_plain_and_wikilink() {
        let plain = contact("type: person\ncompany: Acme\n").company.unwrap();
        assert!(!plain.is_link);
        assert_eq!(plain.name, "Acme");

        let link = contact("type: person\ncompany: \"[[Acme Corp|Acme]]\"\n")
            .company
            .unwrap();
        assert!(link.is_link);
        assert_eq!(link.name, "Acme");
        assert_eq!(link.raw, "[[Acme Corp|Acme]]");
    }

    #[test]
    fn resolve_company_reads_the_implied_employer_edge() {
        use crate::relationships::{Direction, ResolvedRelationship};
        let mut c = contact("type: person\ncompany: \"[[Acme Corp]]\"\n");
        let edge = |rel_type: &str, raw: &str, resolved: bool| ResolvedRelationship {
            rel_type: rel_type.into(),
            predicate: rel_type.into(),
            neighbor: if resolved {
                "/orgs/acme/".into()
            } else {
                String::new()
            },
            neighbor_title: "Acme Corp".into(),
            neighbor_raw: raw.into(),
            resolved,
            direction: Direction::Outgoing,
            label: None,
            attributes: Default::default(),
            derived: true,
        };
        c.resolve_company(&[edge("spouse", "[[Acme Corp]]", true)]);
        assert_eq!(c.company.as_ref().unwrap().url, None);
        c.resolve_company(&[edge("employer", "[[Acme Corp]]", false)]);
        assert_eq!(c.company.as_ref().unwrap().url, None);
        c.resolve_company(&[edge("employer", "[[Acme Corp]]", true)]);
        assert_eq!(
            c.company.as_ref().unwrap().url.as_deref(),
            Some("/orgs/acme/")
        );
    }

    // ----- aliases -----

    #[test]
    fn mixed_aliases_list() {
        let c = contact(
            "type: person\naliases:\n  - Mare\n  - maiden_name: Mary Smith\n  - nickname: Mimi\n  - {label: married_name, name: Mary Doe}\n  - {a: 1, b: 2}\n  - [nested]\n",
        );
        let names: Vec<_> = c.aliases.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Mare", "Mary Smith", "Mimi", "Mary Doe"]);
        assert_eq!(c.aliases[1].label.as_deref(), Some("maiden_name"));
        assert_eq!(
            c.alias_phrases,
            [
                "aka Mare",
                "née Mary Smith",
                "Nickname: Mimi",
                "Married name: Mary Doe"
            ]
        );
        assert_eq!(c.problems.len(), 2, "{:?}", c.problems);
        assert!(c.problems.iter().all(|p| p.field == "aliases"));
    }

    #[test]
    fn alias_names_shapes() {
        assert_eq!(alias_names(Some(&json!("Bob"))), ["Bob"]);
        assert_eq!(alias_names(Some(&json!(["A", "A", " ", "B"]))), ["A", "B"]);
        assert_eq!(
            alias_names(Some(&json!(["A", {"nickname": "N"}, {"x": 1, "y": 2}]))),
            ["A", "N"]
        );
        assert_eq!(alias_names(Some(&json!({"nickname": "N"}))), ["N"]);
        assert!(alias_names(None).is_empty());
        assert!(alias_names(Some(&json!(null))).is_empty());
    }

    // ----- labeled values -----

    #[test]
    fn labeled_map_list_and_bare_forms() {
        let c = contact(
            "type: person\nemails:\n  work: jane@abc.com\n  home: jane@gmail.com\nphones:\n  - mobile: \"+1 303 555 0100\"\n  - home: \"+1 303 555 0101\"\n  - \"+1 303 555 0199\"\nurls: https://example.com\n",
        );
        let emails: Vec<_> = c
            .emails
            .iter()
            .map(|e| (e.label.as_deref(), e.value.as_str(), e.href.as_deref()))
            .collect();
        assert_eq!(
            emails,
            [
                (Some("work"), "jane@abc.com", Some("mailto:jane@abc.com")),
                (
                    Some("home"),
                    "jane@gmail.com",
                    Some("mailto:jane@gmail.com")
                ),
            ]
        );
        assert_eq!(c.phones.len(), 3);
        assert_eq!(c.phones[0].href.as_deref(), Some("tel:+13035550100"));
        assert_eq!(c.phones[2].label, None);
        assert_eq!(c.urls[0].href.as_deref(), Some("https://example.com"));
        assert!(c.problems.is_empty(), "{:?}", c.problems);
    }

    #[test]
    fn labeled_tolerated_object_form_and_duplicates() {
        let c = contact(
            "type: person\nphones:\n  - {label: work, value: '555 0100'}\n  - work: '555 0101'\n",
        );
        assert_eq!(c.phones.len(), 2);
        assert!(c.phones.iter().all(|p| p.label.as_deref() == Some("work")));
    }

    #[test]
    fn labeled_bad_entries_are_reported_not_fatal() {
        let c = contact("type: person\nemails:\n  - {a: x, b: y}\n  - ok@example.com\n");
        assert_eq!(c.emails.len(), 1);
        assert_eq!(c.problems.len(), 1);
        assert_eq!(c.problems[0].field, "emails");
    }

    #[test]
    fn label_display() {
        assert_eq!(humanize_label("home_fax"), "Home fax");
        assert_eq!(humanize_label("linkedin"), "LinkedIn");
        assert_eq!(humanize_label("first-met"), "First met");
        assert_eq!(humanize_label("My Label"), "My Label");
    }

    #[test]
    fn hrefs_are_an_allowlist() {
        let c = contact(
            "type: person\nurls:\n  evil: \"javascript:alert(1)\"\n  data: \"data:text/html,x\"\n  ok: \"HTTPS://Example.com/x\"\n  www: www.example.com\n  bare: example.com\nemails:\n  bad: \"javascript:alert(1)@x\"\n  worse: \"a@b?subject=x\"\nphones:\n  - \"call me\"\n",
        );
        let hrefs: Vec<_> = c.urls.iter().map(|u| u.href.as_deref()).collect();
        assert_eq!(
            hrefs,
            [
                None,
                None,
                Some("HTTPS://Example.com/x"),
                Some("https://www.example.com"),
                None
            ]
        );
        assert!(c.emails.iter().all(|e| e.href.is_none()));
        assert_eq!(c.phones[0].href, None);
    }

    #[test]
    fn tel_hrefs_strip_layout_and_keep_dial_characters() {
        let cases = [
            ("+1 (303) 555-0100 ext. 12", Some("tel:+13035550100;ext=12")),
            ("+1 303 555 0100", Some("tel:+13035550100")),
            ("303.555.0100", Some("tel:3035550100")),
            ("303-555-0100,,42", Some("tel:3035550100,,42")),
            ("555 0100;123", Some("tel:5550100;123")),
            ("555 0100p12", Some("tel:5550100p12")),
            ("555 0100 W 9", Some("tel:5550100w9")),
            ("555 0100 x7", Some("tel:5550100;ext=7")),
            ("555 0100 Extension 7", Some("tel:5550100;ext=7")),
            ("Ask for Pam 555 0100", None),
            ("call me", None),
            ("12", None),
            ("555 0100 ext. twelve", None),
            ("javascript:alert(1)", None),
        ];
        for (input, want) in cases {
            assert_eq!(tel_href(input).as_deref(), want, "{input:?}");
        }
        // The display text is never rewritten.
        let c = contact("type: person\nphones:\n  work: \"+1 (303) 555-0100 ext. 12\"\n");
        assert_eq!(c.phones[0].value, "+1 (303) 555-0100 ext. 12");
        assert_eq!(c.phones[0].href.as_deref(), Some("tel:+13035550100;ext=12"));
    }

    #[test]
    fn social_handles_stay_text_and_urls_link() {
        let c = contact(
            "type: person\nsocial:\n  twitter: \"@jdoe\"\n  mastodon: https://example.social/@jd\nurls:\n  homepage: \"javascript:alert(document.cookie)\"\nemails:\n  work: jane@abc.example\n",
        );
        assert_eq!(c.social[0].value, "@jdoe");
        assert_eq!(c.social[0].href, None, "no guessed URL templates");
        assert_eq!(
            c.social[1].href.as_deref(),
            Some("https://example.social/@jd")
        );
        assert_eq!(c.urls[0].href, None);
        assert_eq!(c.emails[0].href.as_deref(), Some("mailto:jane@abc.example"));
    }

    // ----- addresses -----

    #[test]
    fn addresses_structured_string_and_list() {
        let c = contact(
            "type: person\naddresses:\n  home:\n    street: \"52 Rue Bonsergent\\nBâtiment B\"\n    city: Paris\n    region: Île-de-France\n    postcode: \"75010\"\n    country: France\n    country_code: FR\n  work: \"1 Main St\\nSpringfield\"\n",
        );
        assert_eq!(c.addresses.len(), 2);
        assert_eq!(
            c.addresses[0].lines,
            [
                "52 Rue Bonsergent",
                "Bâtiment B",
                "Paris, Île-de-France 75010",
                "France"
            ]
        );
        assert_eq!(c.addresses[0].country_code.as_deref(), Some("FR"));
        assert_eq!(c.addresses[1].lines, ["1 Main St", "Springfield"]);
        assert_eq!(c.addresses[1].label_display.as_deref(), Some("Work"));

        let listed = contact(
            "type: person\naddresses:\n  - label: home\n    city: Paris\n  - street: 9 Elm\n",
        );
        assert_eq!(listed.addresses[0].label.as_deref(), Some("home"));
        assert_eq!(listed.addresses[0].lines, ["Paris"]);
        assert_eq!(listed.addresses[1].label, None);
        assert_eq!(listed.addresses[1].lines, ["9 Elm"]);

        let single = contact("type: person\naddresses:\n  city: Paris\n");
        assert_eq!(single.addresses.len(), 1);
        assert_eq!(single.addresses[0].label, None);
    }

    // ----- dates -----

    #[test]
    fn dates_order_and_legacy_merge() {
        let c = contact(
            "type: person\nborn: 1900-01-01\ndied: 2010-08-05\ndates:\n  first_met: 2019-04-02\n  anniversary: 1950-06-10\n  birthday: 1927-03-19\n",
        );
        let got: Vec<_> = c
            .dates
            .iter()
            .map(|d| (d.label.as_str(), d.value.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("birthday", "1927-03-19"),
                ("death", "2010-08-05"),
                ("anniversary", "1950-06-10"),
                ("first_met", "2019-04-02"),
            ]
        );
        assert_eq!(c.dates[3].label_display, "First met");
    }

    #[test]
    fn dates_integer_year_and_prose() {
        let c = contact("type: person\nborn: 1898\ndates:\n  - death: circa 1960\n");
        let birthday = c.dates.iter().find(|d| d.label == "birthday").unwrap();
        assert_eq!(birthday.display, "1898");
        let death = c.dates.iter().find(|d| d.label == "death").unwrap();
        assert_eq!(death.display, "circa 1960");
        assert_eq!(death.date, None);
    }

    #[test]
    fn dates_without_label_is_a_problem() {
        let c = contact("type: person\ndates:\n  - 1927-03-19\n");
        assert!(c.dates.is_empty());
        assert_eq!(c.problems[0].field, "dates");
    }

    // ----- simplifier hook + site.json filter -----

    fn simplified(src: &str) -> SimpleMetadata {
        let y = yaml(src);
        let mut hm = SimpleMetadata::new();
        normalize_simplified(&mut hm, y.as_hash().unwrap());
        hm
    }

    #[test]
    fn normalize_flattens_aliases_for_every_note() {
        let hm = simplified("aliases:\n  - A\n  - nickname: N\n");
        assert_eq!(hm["aliases"], json!(["A", "N"]));
        let scalar = simplified("aliases: Bob\n");
        assert_eq!(scalar["aliases"], json!(["Bob"]));
    }

    #[test]
    fn normalize_dates_only_for_contacts() {
        let hm = simplified("type: person\nborn: 1898\ndates:\n  - anniversary: 03-19\n");
        assert_eq!(hm["dates.birthday"], json!("1898"));
        assert_eq!(hm["dates.anniversary"], json!("--03-19"));
        let other = simplified("type: event\nborn: 1898\n");
        assert!(other.is_empty());
    }

    /// Labels match case-insensitively (`has`, `rank`), so the published key
    /// must not keep the authored case: the charts look up `dates.birthday`.
    #[test]
    fn normalize_writes_lowercase_date_keys() {
        let hm = simplified("type: person\nborn: 1950\ndates:\n  Birthday: 1960-01-02\n");
        assert_eq!(hm["dates.birthday"], json!("1960-01-02"));
        assert!(!hm.contains_key("dates.Birthday"), "{hm:?}");

        let mut flattened = SimpleMetadata::new();
        // What the generic simplifier has already written by the time the hook
        // runs: the nested map flattened with the authored case.
        flattened.insert("dates.Birthday".to_string(), json!("1960-01-02"));
        let y = yaml("type: person\ndates:\n  Birthday: 1960-01-02\n");
        normalize_simplified(&mut flattened, y.as_hash().unwrap());
        assert_eq!(flattened.len(), 1, "{flattened:?}");
        assert_eq!(flattened["dates.birthday"], json!("1960-01-02"));
    }

    #[test]
    fn mixed_case_birthday_beats_legacy_born_on_the_card() {
        let c = contact("type: person\nborn: 1950\ndates:\n  Birthday: 1960-01-02\n");
        let birthdays: Vec<_> = c
            .dates
            .iter()
            .filter(|d| d.label.eq_ignore_ascii_case("birthday"))
            .map(|d| d.value.as_str())
            .collect();
        assert_eq!(birthdays, ["1960-01-02"]);
    }

    #[test]
    fn public_keys_drop_contact_details_and_their_dot_variants() {
        for private in [
            "emails",
            "phones",
            "urls.homepage",
            "social.linkedin",
            "im",
            "addresses.home.city",
        ] {
            assert!(!is_public_frontmatter_key(private), "{private}");
        }
        for public in [
            "title",
            "dates.birthday",
            "aliases",
            "company",
            "emailsx",
            "phone",
        ] {
            assert!(is_public_frontmatter_key(public), "{public}");
        }
    }

    /// Regression: `Emails: x@y` used to be published in `site.json` because
    /// the root was compared case-sensitively.
    #[test]
    fn public_keys_drop_contact_details_in_any_case() {
        for private in [
            "Emails",
            "EMAILS",
            "eMaIlS",
            "Phones",
            "PHONES.work",
            "Urls.Homepage",
            "SOCIAL.linkedin",
            "IM",
            "Im.signal",
            "Addresses.home",
            "ADDRESSES.home.city",
        ] {
            assert!(!is_public_frontmatter_key(private), "{private}");
        }
        for public in ["Title", "Dates.birthday", "EmailsX", "PHONE", "Imx"] {
            assert!(is_public_frontmatter_key(public), "{public}");
        }
    }

    /// End to end through the `site.json` serializer: no case variant of a
    /// private key, flat or dotted, survives.
    #[test]
    fn serialized_frontmatter_omits_private_keys_in_any_case() {
        #[derive(serde::Serialize)]
        struct Wrapper {
            #[serde(serialize_with = "serialize_public_frontmatter")]
            frontmatter: Option<SimpleMetadata>,
        }
        let mut fm = SimpleMetadata::new();
        for (k, v) in [
            ("title", "Ada"),
            ("Emails", "UPPERLEAK@acme.com"),
            ("PHONES", "+1 555 0100"),
            ("Addresses.home", "1 Leak Lane"),
            ("Urls.blog", "https://leak.example"),
        ] {
            fm.insert(k.to_string(), json!(v));
        }
        let out = serde_json::to_string(&Wrapper {
            frontmatter: Some(fm),
        })
        .unwrap();
        assert!(out.contains("\"title\":\"Ada\""), "{out}");
        for leaked in ["UPPERLEAK", "555", "Leak Lane", "leak.example"] {
            assert!(!out.contains(leaked), "{leaked} leaked: {out}");
        }
    }

    // ----- properties -----

    /// Arbitrary YAML-ish JSON, nested a few levels.
    fn arb_value() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(|n| json!(n)),
            ".{0,12}".prop_map(Value::String),
        ];
        leaf.prop_recursive(3, 24, 4, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
                prop::collection::btree_map("[a-z_]{1,8}", inner, 0..4)
                    .prop_map(|m| Value::Object(m.into_iter().collect())),
            ]
        })
    }

    const FIELDS: &[&str] = &[
        "title",
        "first_name",
        "company",
        "aliases",
        "emails",
        "phones",
        "urls",
        "social",
        "im",
        "addresses",
        "dates",
        "born",
        "died",
        "image",
    ];

    proptest! {
        /// Whatever the frontmatter holds, parsing never panics and never
        /// yields an href outside the allowlisted schemes.
        #[test]
        fn never_panics_and_hrefs_stay_safe(values in prop::collection::vec(arb_value(), FIELDS.len())) {
            let mut fm: Map<String, Value> = FIELDS
                .iter()
                .zip(values)
                .map(|(k, v)| (k.to_string(), v))
                .collect();
            fm.insert("type".into(), json!("person"));
            let c = Contact::from_frontmatter(&fm).unwrap();
            for l in c.emails.iter().chain(&c.phones).chain(&c.urls).chain(&c.social).chain(&c.im) {
                if let Some(href) = &l.href {
                    let lower = href.to_ascii_lowercase();
                    prop_assert!(
                        ["mailto:", "tel:", "http://", "https://"].iter().any(|s| lower.starts_with(s)),
                        "unsafe href {href}"
                    );
                }
            }
            let _ = alias_names(fm.get("aliases"));
        }

        /// The map form and the list-of-one-key-maps form read identically
        /// when labels are unique.
        #[test]
        fn map_and_list_forms_agree(entries in prop::collection::btree_map("[a-z]{1,6}", "[a-z0-9@.]{1,10}", 0..5)) {
            let map: Map<String, Value> = entries.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
            let list: Vec<Value> = entries.iter().map(|(k, v)| json!({ k.clone(): v })).collect();
            let as_map = Contact::from_frontmatter(&Map::from_iter([
                ("type".to_string(), json!("person")),
                ("emails".to_string(), Value::Object(map)),
            ])).unwrap();
            let as_list = Contact::from_frontmatter(&Map::from_iter([
                ("type".to_string(), json!("person")),
                ("emails".to_string(), Value::Array(list)),
            ])).unwrap();
            prop_assert_eq!(as_map.emails, as_list.emails);
        }

        /// Every well-formed date survives the iso → parse round trip.
        #[test]
        fn date_iso_round_trips(year in 1000u16..=9999, month in 1u8..=12, day in 1u8..=28) {
            for d in [
                PartialDate::Full { year, month, day },
                PartialDate::YearMonth { year, month },
                PartialDate::Year { year },
                PartialDate::MonthDay { month, day },
            ] {
                prop_assert_eq!(PartialDate::parse(&d.iso()), Some(d));
            }
        }
    }
}
