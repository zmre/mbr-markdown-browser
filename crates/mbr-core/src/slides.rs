//! Deck-wide Reveal.js settings for slide notes.
//!
//! One curated, typed set of options, read from two places:
//!
//! - the `[slides]` table of `.mbr/config.toml` (and `MBR_SLIDES__*`), the
//!   repository's default for every deck, validated by serde at startup like
//!   every other option; and
//! - a note's `slides:` frontmatter map, which overrides the default key by
//!   key. A note's value is the author's, per page, so a bad one must not break
//!   the page: [`SlideSettings::from_frontmatter`] keeps the keys that parse and
//!   reports the rest, which the server lists in `errors.json`.
//!
//! Every field is optional: `None` means "not set here", so the browser
//! (`components/src/slide-settings.ts`) can merge config, then frontmatter, on
//! top of its own defaults — which are exactly the settings mbr hardcoded
//! before these options existed. Merging happens there, not here, because a
//! custom `.mbr/_head.html` that predates these options emits neither object
//! and must still get today's deck.
//!
//! Reveal's own per-slide attributes (`--- {data-transition="zoom"}`) beat
//! both: Reveal reads them per slide, over its global configuration.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use yaml_rust2::Yaml;

/// A Reveal.js slide (or background) transition style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transition {
    None,
    Fade,
    Slide,
    Convex,
    Concave,
    Zoom,
}

/// Reveal.js `transitionSpeed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransitionSpeed {
    Default,
    Fast,
    Slow,
}

/// Reveal.js `slideNumber`: off, on (Reveal's default `h.v` format), or one of
/// Reveal's named formats.
///
/// Written as `true`/`false` or the format string, exactly as Reveal takes it
/// (a quoted `"true"`/`"false"` too, which is what a form that writes strings
/// produces); anything else is an error naming the accepted values (a plain
/// untagged enum would only say "data did not match any variant"). Always
/// serialized as Reveal takes it: a boolean or a format string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SlideNumberRaw", into = "SlideNumberRaw")]
pub enum SlideNumber {
    Off,
    On,
    /// `h.v`: horizontal . vertical slide number.
    HorizontalDotVertical,
    /// `h/v`: horizontal / vertical slide number.
    HorizontalSlashVertical,
    /// `c`: flattened slide number.
    Current,
    /// `c/t`: flattened slide number / total slides.
    CurrentOfTotal,
}

/// The wire shape of [`SlideNumber`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum SlideNumberRaw {
    Shown(bool),
    Format(String),
}

impl TryFrom<SlideNumberRaw> for SlideNumber {
    type Error = String;

    fn try_from(raw: SlideNumberRaw) -> Result<Self, Self::Error> {
        match raw {
            SlideNumberRaw::Shown(false) => Ok(Self::Off),
            SlideNumberRaw::Shown(true) => Ok(Self::On),
            SlideNumberRaw::Format(format) => match format.as_str() {
                // A quoted boolean, as a form that writes strings produces.
                "false" => Ok(Self::Off),
                "true" => Ok(Self::On),
                "h.v" => Ok(Self::HorizontalDotVertical),
                "h/v" => Ok(Self::HorizontalSlashVertical),
                "c" => Ok(Self::Current),
                "c/t" => Ok(Self::CurrentOfTotal),
                other => Err(format!(
                    "invalid slide_number {other:?}, expected true, false, \"h.v\", \"h/v\", \"c\" or \"c/t\""
                )),
            },
        }
    }
}

impl From<SlideNumber> for SlideNumberRaw {
    fn from(number: SlideNumber) -> Self {
        match number {
            SlideNumber::Off => Self::Shown(false),
            SlideNumber::On => Self::Shown(true),
            SlideNumber::HorizontalDotVertical => Self::Format("h.v".into()),
            SlideNumber::HorizontalSlashVertical => Self::Format("h/v".into()),
            SlideNumber::Current => Self::Format("c".into()),
            SlideNumber::CurrentOfTotal => Self::Format("c/t".into()),
        }
    }
}

/// Reveal.js `data-background-opacity`: a number from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct BackgroundOpacity(f64);

impl BackgroundOpacity {
    pub fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for BackgroundOpacity {
    type Error = String;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        if (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(format!(
                "invalid background_opacity {value}, expected a number from 0 to 1"
            ))
        }
    }
}

impl From<BackgroundOpacity> for f64 {
    fn from(opacity: BackgroundOpacity) -> Self {
        opacity.0
    }
}

/// Deck-wide slide settings; every field `None` unless set.
///
/// `deny_unknown_fields`, so a misspelt key in `[slides]` fails at startup
/// rather than being ignored, and a misspelt key in a note's `slides:` map is
/// reported for that note.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SlideSettings {
    /// Reveal `transition`. mbr's default: `slide`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
    /// Reveal `transitionSpeed`. Reveal's default: `default`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_speed: Option<TransitionSpeed>,
    /// Reveal `backgroundTransition`. Reveal's default: `fade`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_transition: Option<Transition>,
    /// Reveal `slideNumber`. Reveal's default: off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slide_number: Option<SlideNumber>,
    /// Reveal `controls`. mbr's default: shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controls: Option<bool>,
    /// Reveal `progress`. mbr's default: shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<bool>,
    /// A background image for every top-level slide that has no background of
    /// its own (`data-background*`), applied as that slide's
    /// `data-background-image`. A config value is a site URL (relative to the
    /// repository root); a frontmatter value is relative to the note, like
    /// `![](…)` on the same page. See `page_context::insert_slide_settings`
    /// and `markdown::finalize_render` for where each is resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_image: Option<String>,
    /// `data-background-size` for the default background (CSS
    /// `background-size`; Reveal's default: `cover`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_size: Option<String>,
    /// `data-background-opacity` for the default background (0–1; Reveal's
    /// default: 1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_opacity: Option<BackgroundOpacity>,
}

/// One `slides:` frontmatter entry that was ignored, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlideSettingProblem {
    /// The key at fault, e.g. `slides.transition`.
    pub field: String,
    pub message: String,
}

/// A note's `slides:` frontmatter map: the settings that parsed, and the
/// entries that did not.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlideFrontmatter {
    pub settings: SlideSettings,
    pub problems: Vec<SlideSettingProblem>,
}

impl SlideSettings {
    /// Nothing set; the same value as `SlideSettings::default()`, usable where
    /// a `'static` reference is needed.
    pub const UNSET: Self = Self {
        transition: None,
        transition_speed: None,
        background_transition: None,
        slide_number: None,
        controls: None,
        progress: None,
        background_image: None,
        background_size: None,
        background_opacity: None,
    };

    /// True when nothing is set.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Reads the `slides:` map of a note's raw frontmatter, key by key: a key
    /// that does not parse (unknown name, value out of its set) is left out and
    /// reported, the others still apply.
    ///
    /// `None` when the note has no `slides` key, or when its value is not a
    /// map. The latter is not reported: `slides: https://…` on a talk's note
    /// is a perfectly reasonable link to the slides, not a broken setting.
    pub fn from_frontmatter(frontmatter: &Yaml) -> Option<SlideFrontmatter> {
        let entries = frontmatter
            .as_hash()?
            .get(&Yaml::String("slides".to_string()))?
            .as_hash()?;

        // Walked in the author's order, so problems are listed as written.
        let (valid, problems) = entries.iter().fold(
            (Map::new(), Vec::new()),
            |(mut valid, mut problems), (key, value)| {
                let key = key
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{key:?}"));
                let single =
                    Map::from_iter([(key.clone(), crate::relationships::yaml_to_json(value))]);
                match serde_json::from_value::<SlideSettings>(Value::Object(single.clone())) {
                    Ok(_) => valid.extend(single),
                    Err(e) => problems.push(SlideSettingProblem {
                        field: format!("slides.{key}"),
                        message: format!("{e}; ignored"),
                    }),
                }
                (valid, problems)
            },
        );
        // Every entry parsed on its own and the fields are independent, so
        // the union parses too; the fallback only guards that reasoning.
        let settings = serde_json::from_value(Value::Object(valid)).unwrap_or_default();
        Some(SlideFrontmatter { settings, problems })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frontmatter(yaml: &str) -> Yaml {
        yaml_rust2::YamlLoader::load_from_str(yaml)
            .expect("test YAML parses")
            .into_iter()
            .next()
            .expect("one document")
    }

    #[test]
    fn default_sets_nothing_and_serializes_empty() {
        let settings = SlideSettings::default();
        assert!(settings.is_empty());
        assert_eq!(settings, SlideSettings::UNSET);
        assert_eq!(serde_json::to_value(&settings).unwrap(), json!({}));
    }

    #[test]
    fn every_field_round_trips_through_json() {
        let value = json!({
            "transition": "zoom",
            "transition_speed": "fast",
            "background_transition": "none",
            "slide_number": "c/t",
            "controls": false,
            "progress": false,
            "background_image": "images/bg.jpg",
            "background_size": "contain",
            "background_opacity": 0.4,
        });
        let settings: SlideSettings = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(settings.transition, Some(Transition::Zoom));
        assert_eq!(settings.slide_number, Some(SlideNumber::CurrentOfTotal));
        assert_eq!(
            settings.background_opacity.map(BackgroundOpacity::get),
            Some(0.4)
        );
        assert_eq!(serde_json::to_value(&settings).unwrap(), value);
    }

    #[test]
    fn slide_number_accepts_bools_and_reveal_formats_only() {
        for (written, expected) in [
            (json!(true), SlideNumber::On),
            (json!(false), SlideNumber::Off),
            (json!("h.v"), SlideNumber::HorizontalDotVertical),
            (json!("h/v"), SlideNumber::HorizontalSlashVertical),
            (json!("c"), SlideNumber::Current),
            (json!("c/t"), SlideNumber::CurrentOfTotal),
        ] {
            let parsed: SlideNumber = serde_json::from_value(written.clone()).unwrap();
            assert_eq!(parsed, expected, "{written}");
            assert_eq!(serde_json::to_value(parsed).unwrap(), written);
        }
        assert_eq!(
            serde_json::from_value::<SlideNumber>(json!("true")).unwrap(),
            SlideNumber::On
        );
        assert_eq!(
            serde_json::from_value::<SlideNumber>(json!("false")).unwrap(),
            SlideNumber::Off
        );
        let err = serde_json::from_value::<SlideNumber>(json!("x/y")).unwrap_err();
        assert!(err.to_string().contains("\"h.v\""), "{err}");
    }

    #[test]
    fn background_opacity_must_be_between_zero_and_one() {
        assert!(serde_json::from_value::<BackgroundOpacity>(json!(0)).is_ok());
        assert!(serde_json::from_value::<BackgroundOpacity>(json!(1)).is_ok());
        assert!(serde_json::from_value::<BackgroundOpacity>(json!(1.5)).is_err());
        assert!(serde_json::from_value::<BackgroundOpacity>(json!(-0.1)).is_err());
    }

    #[test]
    fn unknown_key_and_bad_value_are_rejected() {
        assert!(serde_json::from_value::<SlideSettings>(json!({"transiton": "fade"})).is_err());
        assert!(serde_json::from_value::<SlideSettings>(json!({"transition": "spin"})).is_err());
    }

    #[test]
    fn frontmatter_without_slides_map_is_none() {
        assert_eq!(
            SlideSettings::from_frontmatter(&frontmatter("title: x")),
            None
        );
        // A link to a talk's slides is not a broken setting.
        assert_eq!(
            SlideSettings::from_frontmatter(&frontmatter("slides: https://example.com/deck")),
            None
        );
    }

    #[test]
    fn frontmatter_keeps_valid_keys_and_reports_the_rest() {
        let parsed = SlideSettings::from_frontmatter(&frontmatter(
            "type: slides\nslides:\n  transition: zom\n  slide_number: true\n  \
             controls: nope\n  progresss: false\n  background_opacity: 0.5\n",
        ))
        .expect("a slides map");
        assert_eq!(
            parsed.settings,
            SlideSettings {
                slide_number: Some(SlideNumber::On),
                background_opacity: Some(BackgroundOpacity(0.5)),
                ..SlideSettings::default()
            }
        );
        let fields: Vec<&str> = parsed.problems.iter().map(|p| p.field.as_str()).collect();
        assert_eq!(
            fields,
            ["slides.transition", "slides.controls", "slides.progresss"]
        );
        assert!(
            parsed.problems[0].message.contains("zom"),
            "{:?}",
            parsed.problems
        );
        assert!(parsed.problems[0].message.ends_with("ignored"));
    }

    #[test]
    fn frontmatter_yaml_none_is_the_none_transition() {
        let parsed =
            SlideSettings::from_frontmatter(&frontmatter("slides:\n  transition: none\n")).unwrap();
        assert_eq!(parsed.settings.transition, Some(Transition::None));
        assert!(parsed.problems.is_empty());
    }
}
