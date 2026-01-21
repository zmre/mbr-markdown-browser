//! Attribute Parser Module
//!
//! Parses attribute blocks like `{#id .class key=value}` following the
//! pulldown-cmark heading attributes spec. Reusable for horizontal rules,
//! images, and other elements.
//!
//! # Syntax
//!
//! ```text
//! {#id .class1 .class2 key=value key2="value with spaces"}
//! ```
//!
//! - `#id` - ID attribute (last one wins if multiple)
//! - `.class` - CSS class (multiple allowed, no deduplication)
//! - `key=value` - Custom attribute
//! - `key="quoted value"` - Custom attribute with spaces

use regex::Regex;
use std::sync::LazyLock;

/// Regex patterns for attribute parsing
/// ID pattern: match #id at start or after whitespace (avoid matching #fff in "color=#fff")
static ID_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)#([\w-]+)").unwrap());
/// Class pattern: match .class at start or after whitespace (avoid matching .5 in "opacity=0.5")
static CLASS_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)\.([\w-]+)").unwrap());
/// Attr pattern: match key=value or key="value" (supports both straight " and curly "" quotes)
static ATTR_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    // Match: key="value" or key="value" (curly quotes U+201C/U+201D) or key=unquoted
    // Using explicit Unicode escape sequences for curly quotes
    Regex::new(r#"([\w-]+)=(?:"([^"]*)"|[\u{201C}]([^\u{201D}]*)[\u{201D}]|(\S+))"#).unwrap()
});

/// Parsed attributes from `{#id .class key=value}` syntax.
/// Reusable for horizontal rules, images, and other elements.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedAttrs {
    /// Optional ID attribute (last one wins if multiple specified)
    pub id: Option<String>,
    /// CSS classes (multiple allowed)
    pub classes: Vec<String>,
    /// Custom attributes as key-value pairs
    pub attrs: Vec<(String, Option<String>)>,
}

impl ParsedAttrs {
    /// Parse attribute block: `{#id .class key=value}`
    ///
    /// Returns `Some` if input is a valid attrs block starting with `{` and ending with `}`.
    /// Returns `None` if input doesn't match the expected format.
    ///
    /// # Examples
    ///
    /// ```
    /// use mbr::attrs::ParsedAttrs;
    ///
    /// let attrs = ParsedAttrs::parse("{#my-id .highlight}").unwrap();
    /// assert_eq!(attrs.id, Some("my-id".to_string()));
    /// assert_eq!(attrs.classes, vec!["highlight"]);
    /// ```
    pub fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();

        // Must start with { and end with }
        if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
            return None;
        }

        // Extract content between braces
        let content = &trimmed[1..trimmed.len() - 1];

        let mut result = ParsedAttrs::default();

        // Parse IDs (last one wins)
        for cap in ID_PATTERN.captures_iter(content) {
            result.id = Some(cap[1].to_string());
        }

        // Parse classes
        result.classes = CLASS_PATTERN
            .captures_iter(content)
            .map(|cap| cap[1].to_string())
            .collect();

        // Parse key=value attributes
        result.attrs = ATTR_PATTERN
            .captures_iter(content)
            .map(|cap| {
                let key = cap[1].to_string();
                // Group 2: straight quoted, Group 3: curly quoted, Group 4: unquoted
                let value = cap
                    .get(2)
                    .or_else(|| cap.get(3))
                    .or_else(|| cap.get(4))
                    .map(|m| m.as_str().to_string());
                (key, value)
            })
            .collect();

        Some(result)
    }

    /// Render as HTML attribute string.
    ///
    /// Returns a string like ` id="x" class="a b" key="val"` (note leading space).
    /// Returns empty string if no attributes are present.
    ///
    /// # Examples
    ///
    /// ```
    /// use mbr::attrs::ParsedAttrs;
    ///
    /// let attrs = ParsedAttrs::parse("{#intro .highlight}").unwrap();
    /// assert_eq!(attrs.to_html_attr_string(), r#" id="intro" class="highlight""#);
    /// ```
    pub fn to_html_attr_string(&self) -> String {
        if self.is_empty() {
            return String::new();
        }

        let mut parts = Vec::new();

        // Add ID if present
        if let Some(id) = &self.id {
            parts.push(format!(r#"id="{}""#, html_escape(id)));
        }

        // Add classes if present
        if !self.classes.is_empty() {
            let classes_str = self
                .classes
                .iter()
                .map(|c| html_escape(c))
                .collect::<Vec<_>>()
                .join(" ");
            parts.push(format!(r#"class="{}""#, classes_str));
        }

        // Add custom attributes
        for (key, value) in &self.attrs {
            match value {
                Some(v) => parts.push(format!(r#"{}="{}""#, html_escape(key), html_escape(v))),
                None => parts.push(html_escape(key)),
            }
        }

        if parts.is_empty() {
            String::new()
        } else {
            format!(" {}", parts.join(" "))
        }
    }

    /// Check if any attributes are present.
    pub fn is_empty(&self) -> bool {
        self.id.is_none() && self.classes.is_empty() && self.attrs.is_empty()
    }
}

/// Simple HTML attribute value escaping.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_id() {
        let attrs = ParsedAttrs::parse("{#my-id}").unwrap();
        assert_eq!(attrs.id, Some("my-id".to_string()));
        assert!(attrs.classes.is_empty());
        assert!(attrs.attrs.is_empty());
    }

    #[test]
    fn test_parse_classes() {
        let attrs = ParsedAttrs::parse("{.foo .bar}").unwrap();
        assert_eq!(attrs.id, None);
        assert_eq!(attrs.classes, vec!["foo", "bar"]);
        assert!(attrs.attrs.is_empty());
    }

    #[test]
    fn test_parse_single_class() {
        let attrs = ParsedAttrs::parse("{.highlight}").unwrap();
        assert_eq!(attrs.classes, vec!["highlight"]);
    }

    #[test]
    fn test_parse_mixed() {
        let attrs = ParsedAttrs::parse(r#"{#intro .highlight data-x="y"}"#).unwrap();
        assert_eq!(attrs.id, Some("intro".to_string()));
        assert_eq!(attrs.classes, vec!["highlight"]);
        assert_eq!(
            attrs.attrs,
            vec![("data-x".to_string(), Some("y".to_string()))]
        );
    }

    #[test]
    fn test_parse_unquoted_value() {
        let attrs = ParsedAttrs::parse("{data-transition=slide}").unwrap();
        assert_eq!(
            attrs.attrs,
            vec![("data-transition".to_string(), Some("slide".to_string()))]
        );
    }

    #[test]
    fn test_parse_quoted_value_with_spaces() {
        let attrs = ParsedAttrs::parse(r#"{title="Hello World"}"#).unwrap();
        assert_eq!(
            attrs.attrs,
            vec![("title".to_string(), Some("Hello World".to_string()))]
        );
    }

    #[test]
    fn test_last_id_wins() {
        let attrs = ParsedAttrs::parse("{#first #second}").unwrap();
        assert_eq!(attrs.id, Some("second".to_string()));
    }

    #[test]
    fn test_empty_attrs() {
        let attrs = ParsedAttrs::parse("{}").unwrap();
        assert!(attrs.is_empty());
    }

    #[test]
    fn test_non_attrs_returns_none() {
        assert!(ParsedAttrs::parse("hello world").is_none());
        assert!(ParsedAttrs::parse("{incomplete").is_none());
        assert!(ParsedAttrs::parse("no braces").is_none());
        assert!(ParsedAttrs::parse("missing}").is_none());
    }

    #[test]
    fn test_to_html_attr_string_id_only() {
        let attrs = ParsedAttrs::parse("{#my-id}").unwrap();
        assert_eq!(attrs.to_html_attr_string(), r#" id="my-id""#);
    }

    #[test]
    fn test_to_html_attr_string_classes_only() {
        let attrs = ParsedAttrs::parse("{.foo .bar}").unwrap();
        assert_eq!(attrs.to_html_attr_string(), r#" class="foo bar""#);
    }

    #[test]
    fn test_to_html_attr_string_mixed() {
        let attrs = ParsedAttrs::parse(r#"{#intro .highlight data-bg="blue"}"#).unwrap();
        assert_eq!(
            attrs.to_html_attr_string(),
            r#" id="intro" class="highlight" data-bg="blue""#
        );
    }

    #[test]
    fn test_to_html_attr_string_empty() {
        let attrs = ParsedAttrs::parse("{}").unwrap();
        assert_eq!(attrs.to_html_attr_string(), "");
    }

    #[test]
    fn test_html_escaping() {
        let attrs = ParsedAttrs::parse(r#"{#test data-val="<script>"}"#).unwrap();
        assert_eq!(
            attrs.to_html_attr_string(),
            r#" id="test" data-val="&lt;script&gt;""#
        );
    }

    #[test]
    fn test_whitespace_handling() {
        // Leading/trailing whitespace should be handled
        let attrs = ParsedAttrs::parse("  {#my-id}  ").unwrap();
        assert_eq!(attrs.id, Some("my-id".to_string()));
    }

    #[test]
    fn test_complex_attrs() {
        let attrs = ParsedAttrs::parse(
            r##"{#section-1 .slide .center data-transition="slide" data-background-color="#fff"}"##,
        )
        .unwrap();
        assert_eq!(attrs.id, Some("section-1".to_string()));
        assert_eq!(attrs.classes, vec!["slide", "center"]);
        assert_eq!(attrs.attrs.len(), 2);
    }

    #[test]
    fn test_curly_quotes() {
        // Test curly quotes (U+201C " and U+201D ") as used by pulldown-cmark smart punctuation
        // Build the string with explicit Unicode escapes
        let input = "{data-transition=\u{201C}slide\u{201D}}";
        let attrs = ParsedAttrs::parse(input).unwrap();
        assert_eq!(
            attrs.attrs,
            vec![("data-transition".to_string(), Some("slide".to_string()))]
        );
    }

    #[test]
    fn test_curly_quotes_html_output() {
        // Curly quotes should produce straight quotes in HTML output
        let input = "{data-bg=\u{201C}blue\u{201D}}";
        let attrs = ParsedAttrs::parse(input).unwrap();
        assert_eq!(attrs.to_html_attr_string(), r#" data-bg="blue""#);
    }
}
