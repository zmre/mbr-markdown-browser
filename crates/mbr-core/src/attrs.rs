//! Attribute Parser Module
//!
//! Parses attribute blocks like `{#id .class key=value}` in the shape of
//! pulldown-cmark's heading attributes, plus quoted values (which
//! pulldown-cmark's heading attributes do not have). Reusable for horizontal
//! rules, images, and other elements.
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
//! - `key="quoted value"` / `key='quoted value'` - Custom attribute whose
//!   value may contain spaces (curly `“…”` / `‘…’` quotes are accepted too)
//!
//! The block is split into whitespace-separated tokens, except that a quoted
//! value runs to its closing quote. So `style="color: #fff"` is one attribute:
//! the `#fff` inside the value is not an id, nor `.b` in `title="a .b"` a class.
//! Tokens that are none of the three forms are ignored.
//!
//! Values are taken verbatim: no backslash escapes, entities or smart
//! punctuation, which is why the renderer slices the block from the markdown
//! *source* rather than from rendered text (where `--` would already be a
//! dash and `'` a curly quote).

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
    /// use mbr_core::attrs::ParsedAttrs;
    ///
    /// let attrs = ParsedAttrs::parse("{#my-id .highlight}").unwrap();
    /// assert_eq!(attrs.id, Some("my-id".to_string()));
    /// assert_eq!(attrs.classes, vec!["highlight"]);
    /// ```
    pub fn parse(input: &str) -> Option<Self> {
        let content = input
            .trim()
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))?;

        let mut result = ParsedAttrs::default();
        let mut rest = content;
        loop {
            rest = rest.trim_start();
            let Some(first) = rest.chars().next() else {
                break;
            };
            rest = match first {
                '#' | '.' => {
                    let (name, after) = split_name(&rest[1..]);
                    if !name.is_empty() {
                        if first == '#' {
                            result.id = Some(name.to_string());
                        } else {
                            result.classes.push(name.to_string());
                        }
                    }
                    skip_token(after)
                }
                _ => {
                    let (key, after) = split_name(rest);
                    match after.strip_prefix('=') {
                        Some(value_start) if !key.is_empty() => {
                            let (value, after) = split_value(value_start);
                            result
                                .attrs
                                .push((key.to_string(), Some(value.to_string())));
                            after
                        }
                        _ => skip_token(after),
                    }
                }
            };
        }

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
    /// use mbr_core::attrs::ParsedAttrs;
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

/// Quote pairs a value may be wrapped in: straight, and the curly forms
/// smart punctuation produces (accepted for text typed with them).
const QUOTES: [(char, char); 4] = [
    ('"', '"'),
    ('\'', '\''),
    ('\u{201C}', '\u{201D}'),
    ('\u{2018}', '\u{2019}'),
];

/// A name character: a word character or `-`, as in `[\w-]`.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// Splits the leading run of name characters off `s`.
fn split_name(s: &str) -> (&str, &str) {
    let end = s.find(|c| !is_name_char(c)).unwrap_or(s.len());
    s.split_at(end)
}

/// Drops everything up to the next whitespace.
fn skip_token(s: &str) -> &str {
    s.find(char::is_whitespace).map_or("", |i| &s[i..])
}

/// Splits an attribute value off `s` (the text right after `=`), returning
/// the value without its quotes and the text after it.
///
/// A quoted value runs to its closing quote. One that is never closed is read
/// like an unquoted value — up to the next whitespace, opening quote included —
/// which is what the attribute meant before quotes were tokenized.
fn split_value(s: &str) -> (&str, &str) {
    let quoted = QUOTES.iter().find_map(|&(open, close)| {
        let inner = s.strip_prefix(open)?;
        let end = inner.find(close)?;
        Some((&inner[..end], &inner[end + close.len_utf8()..]))
    });
    quoted.unwrap_or_else(|| {
        let end = s.find(char::is_whitespace).unwrap_or(s.len());
        s.split_at(end)
    })
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

    #[test]
    fn single_quoted_value() {
        let attrs = ParsedAttrs::parse("{data-x='a b' .c}").unwrap();
        assert_eq!(attrs.attrs, vec![("data-x".into(), Some("a b".into()))]);
        assert_eq!(attrs.classes, vec!["c"]);
    }

    #[test]
    fn curly_single_quoted_value() {
        let attrs = ParsedAttrs::parse("{data-x=\u{2018}a b\u{2019}}").unwrap();
        assert_eq!(attrs.attrs, vec![("data-x".into(), Some("a b".into()))]);
    }

    #[test]
    fn mixed_quotes_nest_the_other_kind() {
        let attrs = ParsedAttrs::parse(r#"{a="it's" b='say "hi"'}"#).unwrap();
        assert_eq!(
            attrs.attrs,
            vec![
                ("a".into(), Some("it's".into())),
                ("b".into(), Some(r#"say "hi""#.into())),
            ]
        );
    }

    #[test]
    fn dashes_and_ellipses_are_verbatim() {
        let attrs = ParsedAttrs::parse(r#"{style="--pico-color:cyan;" t=a--b...}"#).unwrap();
        assert_eq!(
            attrs.attrs,
            vec![
                ("style".into(), Some("--pico-color:cyan;".into())),
                ("t".into(), Some("a--b...".into())),
            ]
        );
    }

    #[test]
    fn id_and_class_markers_inside_a_quoted_value_are_value() {
        let attrs = ParsedAttrs::parse(r#"{#real style="color: #fff" title="a .b"}"#).unwrap();
        assert_eq!(attrs.id, Some("real".into()));
        assert!(attrs.classes.is_empty(), "{attrs:?}");
        assert_eq!(
            attrs.attrs,
            vec![
                ("style".into(), Some("color: #fff".into())),
                ("title".into(), Some("a .b".into())),
            ]
        );
    }

    #[test]
    fn unterminated_quote_reads_as_unquoted() {
        let attrs = ParsedAttrs::parse(r#"{a="x b=y}"#).unwrap();
        assert_eq!(
            attrs.attrs,
            vec![
                ("a".into(), Some("\"x".into())),
                ("b".into(), Some("y".into())),
            ]
        );
    }

    #[test]
    fn unknown_tokens_are_ignored() {
        let attrs = ParsedAttrs::parse("{bare # . =v #ok}").unwrap();
        assert_eq!(attrs.id, Some("ok".into()));
        assert!(
            attrs.classes.is_empty() && attrs.attrs.is_empty(),
            "{attrs:?}"
        );
    }

    proptest::proptest! {
        /// Any value without its own quote character comes back verbatim,
        /// whichever quote wraps it and whatever surrounds it.
        #[test]
        fn quoted_values_round_trip(
            value in "[^\"]{0,40}",
            single in "[^']{0,40}",
            id in "[A-Za-z0-9_-]{1,12}",
        ) {
            let input = format!("{{#{id} k=\"{value}\" j='{single}' .tail}}");
            let attrs = ParsedAttrs::parse(&input).unwrap();
            proptest::prop_assert_eq!(attrs.id, Some(id));
            proptest::prop_assert_eq!(attrs.classes, vec!["tail".to_string()]);
            proptest::prop_assert_eq!(
                attrs.attrs,
                vec![("k".to_string(), Some(value)), ("j".to_string(), Some(single))]
            );
        }
    }
}
