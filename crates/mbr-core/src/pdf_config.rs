//! The `[pdf]` configuration table: how `mbr --pdf` and the GUI's
//! **Save as PDF…** lay out a page on paper.
//!
//! Typed and `deny_unknown_fields`, like `[slides]`, so a typo in
//! `.mbr/config.toml` (or `MBR_PDF__*`) fails at startup instead of being
//! silently ignored. Every value that ends up in the page — the margin goes
//! into an `@page` rule, the link base into hrefs — is validated here, so the
//! exporter never has to escape anything it did not check.
//!
//! The exporter itself (hidden webview, platform print APIs) lives in the
//! `mbr` application crate's `pdf_export` module; this module is pure data.

use serde::{Deserialize, Deserializer, Serialize};

/// Millimetres per inch, for custom paper sizes.
const MM_PER_INCH: f64 = 25.4;

/// The smallest and largest custom paper edge accepted, in inches. Anything
/// outside is a typo (`210x297in`), not a sheet of paper.
const MIN_PAPER_INCHES: f64 = 1.0;
const MAX_PAPER_INCHES: f64 = 200.0;

/// Default readiness timeout: how long the exporter waits for the page to say
/// it has finished rendering before exporting anyway.
pub const DEFAULT_PDF_TIMEOUT_MS: u64 = 30_000;

/// Default page margin, as a CSS `margin` shorthand: 0.6in top/bottom,
/// 0.5in left/right. Measured in the macOS spike: with the theme's own
/// `@page { margin: 0 }` pages 2+ had no top/bottom margin at all.
pub const DEFAULT_PDF_MARGIN: &str = "0.6in 0.5in";

/// A paper size: one of the named sizes or a custom `<W>x<H>mm|in`.
///
/// Written and read as a string (`letter`, `a4`, `legal`, `210x297mm`,
/// `8.5x11in`), case-insensitively.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Paper {
    /// US Letter, 8.5 × 11 in.
    Letter,
    /// ISO A4, 210 × 297 mm.
    A4,
    /// US Legal, 8.5 × 14 in.
    Legal,
    /// A custom size, as written (width × height), in inches.
    Custom { width_in: f64, height_in: f64 },
}

impl Paper {
    /// Width and height in inches, as written (portrait for the named sizes).
    pub fn dimensions_in(self) -> (f64, f64) {
        match self {
            Paper::Letter => (8.5, 11.0),
            Paper::A4 => (210.0 / MM_PER_INCH, 297.0 / MM_PER_INCH),
            Paper::Legal => (8.5, 14.0),
            Paper::Custom {
                width_in,
                height_in,
            } => (width_in, height_in),
        }
    }

    /// The named size whose dimensions are within a millimetre of
    /// `(width_in, height_in)` (either way round), else a custom size.
    /// For turning a platform's default paper (points) back into a name.
    pub fn from_dimensions_in(width_in: f64, height_in: f64) -> Paper {
        let close = |a: f64, b: f64| (a - b).abs() * MM_PER_INCH < 1.0;
        [Paper::Letter, Paper::A4, Paper::Legal]
            .into_iter()
            .find(|paper| {
                let (w, h) = paper.dimensions_in();
                (close(w, width_in) && close(h, height_in))
                    || (close(w, height_in) && close(h, width_in))
            })
            .unwrap_or(Paper::Custom {
                width_in,
                height_in,
            })
    }
}

impl std::str::FromStr for Paper {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let normalized: String = raw
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        match normalized.as_str() {
            "letter" => return Ok(Paper::Letter),
            "a4" => return Ok(Paper::A4),
            "legal" => return Ok(Paper::Legal),
            _ => {}
        }
        let invalid = || {
            format!(
                "invalid paper {raw:?}, expected letter, a4, legal or a custom size such as \
                 210x297mm or 8.5x11in"
            )
        };
        let (numbers, per_inch) = if let Some(numbers) = normalized.strip_suffix("mm") {
            (numbers, MM_PER_INCH)
        } else if let Some(numbers) = normalized.strip_suffix("in") {
            (numbers, 1.0)
        } else {
            return Err(invalid());
        };
        let (width, height) = numbers.split_once('x').ok_or_else(invalid)?;
        let parse = |value: &str| -> Result<f64, String> {
            // `f64::from_str` also takes `inf`, `nan` and `1e3`; a paper edge
            // is digits with at most one decimal point.
            let plain = !value.is_empty()
                && value.chars().all(|c| c.is_ascii_digit() || c == '.')
                && value.chars().filter(|&c| c == '.').count() <= 1;
            let inches = value
                .parse::<f64>()
                .ok()
                .filter(|_| plain)
                .map(|v| v / per_inch)
                .ok_or_else(invalid)?;
            if (MIN_PAPER_INCHES..=MAX_PAPER_INCHES).contains(&inches) {
                Ok(inches)
            } else {
                Err(format!(
                    "invalid paper {raw:?}: each side must be between 1in (25.4mm) and 200in"
                ))
            }
        };
        Ok(Paper::Custom {
            width_in: parse(width)?,
            height_in: parse(height)?,
        })
    }
}

impl TryFrom<String> for Paper {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl From<Paper> for String {
    fn from(paper: Paper) -> Self {
        paper.to_string()
    }
}

impl std::fmt::Display for Paper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Paper::Letter => f.write_str("letter"),
            Paper::A4 => f.write_str("a4"),
            Paper::Legal => f.write_str("legal"),
            Paper::Custom {
                width_in,
                height_in,
            } => write!(f, "{width_in}x{height_in}in"),
        }
    }
}

/// Page orientation. `landscape` swaps the paper's width and height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

/// A validated CSS `margin` shorthand: one to four non-negative lengths
/// (`0`, or a number with one of the units `in`, `cm`, `mm`, `pt`, `pc`,
/// `px`), stored with single spaces.
///
/// Validated because it is written verbatim into an `@page` rule in the
/// exported page; anything else (`;`, `}`, `calc()`, `auto`) is refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MarginRaw", into = "String")]
pub struct Margin(String);

/// A margin as TOML or the environment may spell it: `"0.5in"`, or a bare
/// `0` (which figment reads from `MBR_PDF__MARGIN=0` as a number).
#[derive(Deserialize)]
#[serde(untagged)]
enum MarginRaw {
    Number(f64),
    Text(String),
}

impl TryFrom<MarginRaw> for Margin {
    type Error = String;

    fn try_from(raw: MarginRaw) -> Result<Self, Self::Error> {
        match raw {
            MarginRaw::Number(0.0) => Ok(Margin("0".to_owned())),
            MarginRaw::Number(n) => Err(format!(
                "invalid margin {n}: a non-zero margin needs a unit, such as \"{n}in\" or \"{n}mm\""
            )),
            MarginRaw::Text(text) => text.parse(),
        }
    }
}

impl std::str::FromStr for Margin {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        const UNITS: [&str; 6] = ["in", "cm", "mm", "pt", "pc", "px"];
        let invalid = || {
            format!(
                "invalid margin {raw:?}, expected one to four lengths such as \"0.5in\" or \
                 \"0.6in 0.5in\" (units: in, cm, mm, pt, pc, px)"
            )
        };
        let parts: Vec<&str> = raw.split_whitespace().collect();
        if parts.is_empty() || parts.len() > 4 {
            return Err(invalid());
        }
        let valid_length = |part: &str| {
            if part == "0" {
                return true;
            }
            UNITS.iter().any(|unit| {
                part.strip_suffix(unit).is_some_and(|number| {
                    !number.is_empty()
                        && number.chars().all(|c| c.is_ascii_digit() || c == '.')
                        && number.chars().filter(|&c| c == '.').count() <= 1
                        && number != "."
                })
            })
        };
        if parts.iter().all(|part| valid_length(part)) {
            Ok(Margin(parts.join(" ")))
        } else {
            Err(invalid())
        }
    }
}

impl From<Margin> for String {
    fn from(margin: Margin) -> Self {
        margin.0
    }
}

impl Margin {
    /// The shorthand, ready for a CSS `margin:` declaration.
    pub fn as_css(&self) -> &str {
        &self.0
    }
}

impl Default for Margin {
    fn default() -> Self {
        Margin(DEFAULT_PDF_MARGIN.to_owned())
    }
}

/// Where internal links point in an exported PDF, when not stripped: an
/// `http(s)` URL ending in `/` (a published copy of the repository).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LinkBase(String);

impl std::str::FromStr for LinkBase {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let trimmed = raw.trim();
        let parsed =
            url::Url::parse(trimmed).map_err(|e| format!("invalid link_base {raw:?}: {e}"))?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err(format!(
                "invalid link_base {raw:?}: expected an http(s) URL such as https://notes.example.com/"
            ));
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(format!(
                "invalid link_base {raw:?}: a base URL cannot have a query or fragment"
            ));
        }
        let mut base = parsed.to_string();
        if !base.ends_with('/') {
            base.push('/');
        }
        Ok(LinkBase(base))
    }
}

impl TryFrom<String> for LinkBase {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl From<LinkBase> for String {
    fn from(base: LinkBase) -> Self {
        base.0
    }
}

impl LinkBase {
    /// The URL, always ending in `/`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The `[pdf]` table. Every key is optional; see each field for its default.
///
/// Environment: `MBR_PDF__PAPER=a4`, `MBR_PDF__PAGE_NUMBERS=true`, … (figment
/// splits on `__`). Command-line flags (`--pdf-paper`, …) beat both.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PdfConfig {
    /// Paper size. Unset: the platform's default (macOS: the system's default
    /// paper; elsewhere Letter in US/Canadian locales, else A4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper: Option<Paper>,
    /// `portrait` (default) or `landscape`.
    pub orientation: Orientation,
    /// Page margin, a CSS `margin` shorthand. Default `0.6in 0.5in`.
    pub margin: Margin,
    /// Print the platform's header and footer (title, date, "Page N of M").
    /// macOS and Windows only; ignored on Linux. Default off.
    pub page_numbers: bool,
    /// Rewrite internal links onto this published site instead of removing
    /// them. Default: removed (their text is kept).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_base: Option<LinkBase>,
    /// How long to wait for the page to finish rendering before exporting
    /// anyway (with a warning), in milliseconds. Default 30000; must be > 0.
    #[serde(deserialize_with = "deserialize_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for PdfConfig {
    fn default() -> Self {
        PdfConfig {
            paper: None,
            orientation: Orientation::Portrait,
            margin: Margin::default(),
            page_numbers: false,
            link_base: None,
            timeout_ms: DEFAULT_PDF_TIMEOUT_MS,
        }
    }
}

fn deserialize_timeout_ms<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let value = u64::deserialize(deserializer)?;
    if value == 0 {
        Err(serde::de::Error::custom(
            "invalid timeout_ms 0, expected a number of milliseconds greater than 0",
        ))
    } else {
        Ok(value)
    }
}

/// The paper a locale implies: Letter for the United States and Canada, A4
/// everywhere else. `locale` is a POSIX or BCP 47 name (`en_US.UTF-8`,
/// `fr-CA`, `de_DE@euro`); one without a region (`C`, `POSIX`, `en`) is A4.
pub fn paper_for_locale(locale: &str) -> Paper {
    let base = locale.split(['.', '@']).next().unwrap_or_default();
    let region = base
        .split(['_', '-'])
        .skip(1)
        .find(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()));
    match region.map(|r| r.to_ascii_uppercase()).as_deref() {
        Some("US" | "CA") => Paper::Letter,
        _ => Paper::A4,
    }
}

/// The locale that decides paper size, from environment variables read
/// through `var`: `LC_ALL`, then `LC_PAPER`, then `LANG`, skipping empty
/// values (POSIX precedence). `None` when none is set.
pub fn paper_locale_from_env(var: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["LC_ALL", "LC_PAPER", "LANG"]
        .into_iter()
        .filter_map(var)
        .find(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inches(paper: Paper) -> (f64, f64) {
        let (w, h) = paper.dimensions_in();
        ((w * 1000.0).round() / 1000.0, (h * 1000.0).round() / 1000.0)
    }

    #[test]
    fn named_papers_parse_case_insensitively() {
        assert_eq!("letter".parse(), Ok(Paper::Letter));
        assert_eq!("A4".parse(), Ok(Paper::A4));
        assert_eq!(" Legal ".parse(), Ok(Paper::Legal));
        assert_eq!(inches(Paper::A4), (8.268, 11.693));
    }

    #[test]
    fn custom_papers_parse_in_mm_and_inches() {
        assert_eq!(inches("210x297mm".parse().unwrap()), (8.268, 11.693));
        assert_eq!(inches("8.5x11in".parse().unwrap()), (8.5, 11.0));
        assert_eq!(inches("8.5 x 11 IN".parse().unwrap()), (8.5, 11.0));
        // Landscape-shaped custom sizes are kept as written.
        assert_eq!(inches("11x8.5in".parse().unwrap()), (11.0, 8.5));
    }

    #[test]
    fn bad_papers_are_refused() {
        for bad in [
            "",
            "tabloid",
            "210x297",
            "210x297cm",
            "x297mm",
            "210xmm",
            "210x297x3mm",
            "-210x297mm",
            "infxinfin",
            "nanx11in",
            "1e3x11in",
            "0.5x11in",
            "210x297in",
            "8..5x11in",
        ] {
            assert!(bad.parse::<Paper>().is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn paper_round_trips_through_its_string_form() {
        for paper in [Paper::Letter, Paper::A4, Paper::Legal] {
            assert_eq!(paper.to_string().parse(), Ok(paper));
        }
        let custom: Paper = "8.5x14in".parse().unwrap();
        assert_eq!(custom.to_string().parse::<Paper>(), Ok(custom));
    }

    #[test]
    fn from_dimensions_names_the_standard_sizes() {
        assert_eq!(Paper::from_dimensions_in(8.5, 11.0), Paper::Letter);
        assert_eq!(Paper::from_dimensions_in(11.0, 8.5), Paper::Letter);
        assert_eq!(
            Paper::from_dimensions_in(595.28 / 72.0, 841.89 / 72.0),
            Paper::A4
        );
        assert_eq!(Paper::from_dimensions_in(8.5, 14.0), Paper::Legal);
        assert!(matches!(
            Paper::from_dimensions_in(5.0, 7.0),
            Paper::Custom { .. }
        ));
    }

    #[test]
    fn margins_accept_one_to_four_lengths() {
        assert_eq!("0.5in".parse::<Margin>().unwrap().as_css(), "0.5in");
        assert_eq!(
            "  0.6in   0.5in ".parse::<Margin>().unwrap().as_css(),
            "0.6in 0.5in"
        );
        assert_eq!(
            "10mm 1cm 12pt 0".parse::<Margin>().unwrap().as_css(),
            "10mm 1cm 12pt 0"
        );
        assert_eq!(Margin::default().as_css(), DEFAULT_PDF_MARGIN);
    }

    /// The margin is written into an `@page` rule: nothing that could close
    /// the declaration or the rule may pass.
    #[test]
    fn margins_refuse_anything_but_plain_lengths() {
        for bad in [
            "",
            "0.5",
            "1 2 3 4 5",
            "0.5in;",
            "0.5in} body{color:red",
            "calc(1in)",
            "auto",
            "-1in",
            "1em",
            ".in",
            "1..2in",
            "1in 2in 3in 4in 5in",
        ] {
            assert!(bad.parse::<Margin>().is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn link_base_requires_http_and_gains_a_trailing_slash() {
        assert_eq!(
            "https://notes.example.com"
                .parse::<LinkBase>()
                .unwrap()
                .as_str(),
            "https://notes.example.com/"
        );
        assert_eq!(
            "http://example.com/docs"
                .parse::<LinkBase>()
                .unwrap()
                .as_str(),
            "http://example.com/docs/"
        );
        for bad in [
            "notes.example.com",
            "ftp://example.com/",
            "javascript:alert(1)",
            "file:///etc/",
            "https://example.com/?q=1",
            "https://example.com/#x",
        ] {
            assert!(
                bad.parse::<LinkBase>().is_err(),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn defaults_are_the_documented_ones() {
        let config = PdfConfig::default();
        assert_eq!(config.paper, None);
        assert_eq!(config.orientation, Orientation::Portrait);
        assert_eq!(config.margin.as_css(), "0.6in 0.5in");
        assert!(!config.page_numbers);
        assert_eq!(config.link_base, None);
        assert_eq!(config.timeout_ms, 30_000);
    }

    #[test]
    fn locale_decides_letter_or_a4() {
        for letter in ["en_US.UTF-8", "en-US", "fr_CA", "en_us", "es_US@x"] {
            assert_eq!(paper_for_locale(letter), Paper::Letter, "{letter}");
        }
        for a4 in ["en_GB.UTF-8", "de_DE@euro", "C", "POSIX", "", "en", "fr-FR"] {
            assert_eq!(paper_for_locale(a4), Paper::A4, "{a4}");
        }
    }

    #[test]
    fn paper_locale_follows_posix_precedence() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert_eq!(
            paper_locale_from_env(env(&[("LANG", "en_US.UTF-8"), ("LC_PAPER", "de_DE")])),
            Some("de_DE".to_owned())
        );
        assert_eq!(
            paper_locale_from_env(env(&[("LC_ALL", ""), ("LANG", "fr_CA")])),
            Some("fr_CA".to_owned())
        );
        assert_eq!(paper_locale_from_env(env(&[])), None);
    }
}
