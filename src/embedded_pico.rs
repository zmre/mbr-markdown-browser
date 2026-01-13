//! Embedded Pico CSS files for theme support.
//!
//! This module embeds all 42 Pico CSS variant files and provides a function to
//! look up the correct CSS bytes based on a theme configuration string.
//!
//! Theme mapping:
//! - "" or "default" -> pico.classless.min.css
//! - "{color}" (e.g., "amber") -> pico.classless.{color}.min.css
//! - "fluid" -> pico.fluid.classless.min.css
//! - "fluid.{color}" (e.g., "fluid.amber") -> pico.fluid.classless.{color}.min.css
//! - Invalid value -> None (caller should show 404 + warning)

// Default (no color)
const PICO_DEFAULT: &[u8] = include_bytes!("../templates/pico-main/pico.classless.min.css");
const PICO_FLUID_DEFAULT: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.min.css");

// Color variants - standard (20 colors)
const PICO_AMBER: &[u8] = include_bytes!("../templates/pico-main/pico.classless.amber.min.css");
const PICO_BLUE: &[u8] = include_bytes!("../templates/pico-main/pico.classless.blue.min.css");
const PICO_CYAN: &[u8] = include_bytes!("../templates/pico-main/pico.classless.cyan.min.css");
const PICO_FUCHSIA: &[u8] = include_bytes!("../templates/pico-main/pico.classless.fuchsia.min.css");
const PICO_GREEN: &[u8] = include_bytes!("../templates/pico-main/pico.classless.green.min.css");
const PICO_GREY: &[u8] = include_bytes!("../templates/pico-main/pico.classless.grey.min.css");
const PICO_INDIGO: &[u8] = include_bytes!("../templates/pico-main/pico.classless.indigo.min.css");
const PICO_JADE: &[u8] = include_bytes!("../templates/pico-main/pico.classless.jade.min.css");
const PICO_LIME: &[u8] = include_bytes!("../templates/pico-main/pico.classless.lime.min.css");
const PICO_ORANGE: &[u8] = include_bytes!("../templates/pico-main/pico.classless.orange.min.css");
const PICO_PINK: &[u8] = include_bytes!("../templates/pico-main/pico.classless.pink.min.css");
const PICO_PUMPKIN: &[u8] = include_bytes!("../templates/pico-main/pico.classless.pumpkin.min.css");
const PICO_PURPLE: &[u8] = include_bytes!("../templates/pico-main/pico.classless.purple.min.css");
const PICO_RED: &[u8] = include_bytes!("../templates/pico-main/pico.classless.red.min.css");
const PICO_SAND: &[u8] = include_bytes!("../templates/pico-main/pico.classless.sand.min.css");
const PICO_SLATE: &[u8] = include_bytes!("../templates/pico-main/pico.classless.slate.min.css");
const PICO_VIOLET: &[u8] = include_bytes!("../templates/pico-main/pico.classless.violet.min.css");
const PICO_YELLOW: &[u8] = include_bytes!("../templates/pico-main/pico.classless.yellow.min.css");
const PICO_ZINC: &[u8] = include_bytes!("../templates/pico-main/pico.classless.zinc.min.css");

// Color variants - fluid (20 colors)
const PICO_FLUID_AMBER: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.amber.min.css");
const PICO_FLUID_BLUE: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.blue.min.css");
const PICO_FLUID_CYAN: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.cyan.min.css");
const PICO_FLUID_FUCHSIA: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.fuchsia.min.css");
const PICO_FLUID_GREEN: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.green.min.css");
const PICO_FLUID_GREY: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.grey.min.css");
const PICO_FLUID_INDIGO: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.indigo.min.css");
const PICO_FLUID_JADE: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.jade.min.css");
const PICO_FLUID_LIME: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.lime.min.css");
const PICO_FLUID_ORANGE: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.orange.min.css");
const PICO_FLUID_PINK: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.pink.min.css");
const PICO_FLUID_PUMPKIN: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.pumpkin.min.css");
const PICO_FLUID_PURPLE: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.purple.min.css");
const PICO_FLUID_RED: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.red.min.css");
const PICO_FLUID_SAND: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.sand.min.css");
const PICO_FLUID_SLATE: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.slate.min.css");
const PICO_FLUID_VIOLET: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.violet.min.css");
const PICO_FLUID_YELLOW: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.yellow.min.css");
const PICO_FLUID_ZINC: &[u8] =
    include_bytes!("../templates/pico-main/pico.fluid.classless.zinc.min.css");

/// Valid color names for themes.
pub const VALID_COLORS: &[&str] = &[
    "amber", "blue", "cyan", "fuchsia", "green", "grey", "indigo", "jade", "lime", "orange",
    "pink", "pumpkin", "purple", "red", "sand", "slate", "violet", "yellow", "zinc",
];

/// Returns the Pico CSS bytes for a given theme, or None if invalid.
///
/// Theme mapping:
/// - "" or "default" -> default classless CSS
/// - "{color}" (e.g., "amber") -> classless color variant
/// - "fluid" -> fluid classless CSS
/// - "fluid.{color}" (e.g., "fluid.amber") -> fluid classless color variant
pub fn get_pico_css(theme: &str) -> Option<&'static [u8]> {
    match theme.trim() {
        "" | "default" => Some(PICO_DEFAULT),
        "fluid" => Some(PICO_FLUID_DEFAULT),
        theme if theme.starts_with("fluid.") => {
            let color = theme.strip_prefix("fluid.").unwrap();
            get_fluid_color_css(color)
        }
        color if VALID_COLORS.contains(&color) => get_color_css(color),
        _ => None,
    }
}

/// Returns the standard classless CSS for a color variant.
fn get_color_css(color: &str) -> Option<&'static [u8]> {
    match color {
        "amber" => Some(PICO_AMBER),
        "blue" => Some(PICO_BLUE),
        "cyan" => Some(PICO_CYAN),
        "fuchsia" => Some(PICO_FUCHSIA),
        "green" => Some(PICO_GREEN),
        "grey" => Some(PICO_GREY),
        "indigo" => Some(PICO_INDIGO),
        "jade" => Some(PICO_JADE),
        "lime" => Some(PICO_LIME),
        "orange" => Some(PICO_ORANGE),
        "pink" => Some(PICO_PINK),
        "pumpkin" => Some(PICO_PUMPKIN),
        "purple" => Some(PICO_PURPLE),
        "red" => Some(PICO_RED),
        "sand" => Some(PICO_SAND),
        "slate" => Some(PICO_SLATE),
        "violet" => Some(PICO_VIOLET),
        "yellow" => Some(PICO_YELLOW),
        "zinc" => Some(PICO_ZINC),
        _ => None,
    }
}

/// Returns the fluid classless CSS for a color variant.
fn get_fluid_color_css(color: &str) -> Option<&'static [u8]> {
    match color {
        "amber" => Some(PICO_FLUID_AMBER),
        "blue" => Some(PICO_FLUID_BLUE),
        "cyan" => Some(PICO_FLUID_CYAN),
        "fuchsia" => Some(PICO_FLUID_FUCHSIA),
        "green" => Some(PICO_FLUID_GREEN),
        "grey" => Some(PICO_FLUID_GREY),
        "indigo" => Some(PICO_FLUID_INDIGO),
        "jade" => Some(PICO_FLUID_JADE),
        "lime" => Some(PICO_FLUID_LIME),
        "orange" => Some(PICO_FLUID_ORANGE),
        "pink" => Some(PICO_FLUID_PINK),
        "pumpkin" => Some(PICO_FLUID_PUMPKIN),
        "purple" => Some(PICO_FLUID_PURPLE),
        "red" => Some(PICO_FLUID_RED),
        "sand" => Some(PICO_FLUID_SAND),
        "slate" => Some(PICO_FLUID_SLATE),
        "violet" => Some(PICO_FLUID_VIOLET),
        "yellow" => Some(PICO_FLUID_YELLOW),
        "zinc" => Some(PICO_FLUID_ZINC),
        _ => None,
    }
}

/// Returns a formatted string of valid theme values for error messages.
pub fn valid_themes_display() -> String {
    let mut themes = vec!["default".to_string(), "fluid".to_string()];
    themes.extend(VALID_COLORS.iter().map(|c| (*c).to_string()));
    themes.extend(VALID_COLORS.iter().map(|c| format!("fluid.{}", c)));
    themes.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_theme() {
        assert!(get_pico_css("").is_some());
        assert!(get_pico_css("default").is_some());
        // Both should return same CSS
        assert_eq!(get_pico_css(""), get_pico_css("default"));
    }

    #[test]
    fn test_color_themes() {
        for color in VALID_COLORS {
            assert!(
                get_pico_css(color).is_some(),
                "Missing color theme: {}",
                color
            );
        }
    }

    #[test]
    fn test_fluid_theme() {
        assert!(get_pico_css("fluid").is_some());
    }

    #[test]
    fn test_fluid_color_themes() {
        for color in VALID_COLORS {
            let theme = format!("fluid.{}", color);
            assert!(
                get_pico_css(&theme).is_some(),
                "Missing fluid color theme: {}",
                theme
            );
        }
    }

    #[test]
    fn test_invalid_theme() {
        assert!(get_pico_css("invalid").is_none());
        assert!(get_pico_css("fluid.invalid").is_none());
        assert!(get_pico_css("notacolor").is_none());
        assert!(get_pico_css("fluid.notacolor").is_none());
    }

    #[test]
    fn test_whitespace_handling() {
        assert!(get_pico_css("  default  ").is_some());
        assert!(get_pico_css("  amber  ").is_some());
        assert!(get_pico_css("  fluid  ").is_some());
        assert!(get_pico_css("  fluid.amber  ").is_some());
    }

    #[test]
    fn test_valid_themes_display() {
        let display = valid_themes_display();
        assert!(display.contains("default"));
        assert!(display.contains("fluid"));
        assert!(display.contains("amber"));
        assert!(display.contains("fluid.amber"));
        assert!(display.contains("zinc"));
        assert!(display.contains("fluid.zinc"));
    }

    #[test]
    fn test_css_content_is_valid() {
        // Verify that the CSS content is not empty and starts with expected content
        let default_css = get_pico_css("default").unwrap();
        assert!(!default_css.is_empty());
        // Pico CSS should contain some recognizable content
        let css_str = std::str::from_utf8(default_css).unwrap();
        assert!(
            css_str.contains("pico") || css_str.contains(":root"),
            "CSS should contain Pico-related content"
        );
    }

    #[test]
    fn test_different_themes_return_different_css() {
        let default_css = get_pico_css("default").unwrap();
        let amber_css = get_pico_css("amber").unwrap();
        let fluid_css = get_pico_css("fluid").unwrap();
        let fluid_amber_css = get_pico_css("fluid.amber").unwrap();

        // All should be non-empty
        assert!(!default_css.is_empty());
        assert!(!amber_css.is_empty());
        assert!(!fluid_css.is_empty());
        assert!(!fluid_amber_css.is_empty());

        // They should be different from each other
        assert_ne!(default_css, amber_css, "default should differ from amber");
        assert_ne!(default_css, fluid_css, "default should differ from fluid");
        assert_ne!(
            fluid_css, fluid_amber_css,
            "fluid should differ from fluid.amber"
        );
    }
}
