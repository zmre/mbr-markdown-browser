//! Wikilink parsing and transformation module.
//!
//! This module handles two wikilink patterns for tags:
//!
//! 1. `[[Source:value]]` - transformed to `[value](/source/value/)`
//! 2. `[text](Source:value)` - detected and transformed to `[text](/source/value/)`
//!
//! Tag sources are case-insensitive for matching but the URL uses lowercase source names.
//! Tag values are normalized: lowercase with spaces as underscores.

use std::collections::HashSet;

/// Represents a parsed wikilink with source and value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedWikilink {
    /// The source/type of the tag (e.g., "tags", "performers")
    pub source: String,
    /// The tag value (e.g., "rust", "Joshua Jay")
    pub value: String,
    /// Optional custom display text (only for `[text](Source:value)` format)
    pub display_text: Option<String>,
}

impl ParsedWikilink {
    /// Creates a new ParsedWikilink.
    pub fn new(source: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            value: value.into(),
            display_text: None,
        }
    }

    /// Creates a new ParsedWikilink with custom display text.
    pub fn with_display(
        source: impl Into<String>,
        value: impl Into<String>,
        display: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            value: value.into(),
            display_text: Some(display.into()),
        }
    }

    /// Returns the normalized URL source (lowercase).
    pub fn url_source(&self) -> String {
        self.source.to_lowercase()
    }

    /// Returns the normalized URL value (lowercase, spaces as underscores).
    pub fn url_value(&self) -> String {
        normalize_tag_value(&self.value)
    }

    /// Returns the full URL path for this tag link.
    ///
    /// Format: `/{source}/{value}/`
    pub fn url_path(&self) -> String {
        format!("/{}/{}/", self.url_source(), self.url_value())
    }

    /// Returns the display text for this link.
    ///
    /// Priority:
    /// 1. Custom display text (if set)
    /// 2. Original value (preserves case)
    pub fn display(&self) -> &str {
        self.display_text.as_deref().unwrap_or(&self.value)
    }

    /// Converts this wikilink to a markdown link.
    pub fn to_markdown_link(&self) -> String {
        format!("[{}]({})", self.display(), self.url_path())
    }
}

/// Normalizes a tag value for use in URLs.
///
/// - Converts to lowercase
/// - Replaces spaces with underscores
/// - Trims leading/trailing whitespace
///
/// # Examples
///
/// ```
/// use mbr::wikilink::normalize_tag_value;
///
/// assert_eq!(normalize_tag_value("Joshua Jay"), "joshua_jay");
/// assert_eq!(normalize_tag_value("rust"), "rust");
/// assert_eq!(normalize_tag_value("  Spaced  "), "spaced");
/// ```
pub fn normalize_tag_value(value: &str) -> String {
    value.trim().to_lowercase().replace(' ', "_")
}

/// URL schemes that should NOT be treated as tag sources.
const URL_SCHEMES: &[&str] = &[
    "http",
    "https",
    "mailto",
    "tel",
    "ftp",
    "ftps",
    "file",
    "data",
    "javascript",
    "ssh",
    "git",
    "svn",
    "magnet",
];

/// Checks if a source name looks like a URL scheme.
fn is_url_scheme(source: &str) -> bool {
    URL_SCHEMES
        .iter()
        .any(|scheme| source.eq_ignore_ascii_case(scheme))
}

/// Transforms wikilinks in markdown text to standard markdown links.
///
/// Converts `[[Source:value]]` patterns to `[value](/source/value/)` links.
///
/// # Arguments
///
/// * `input` - The markdown text to transform
/// * `valid_sources` - Set of valid tag source names (case-insensitive matching)
///
/// # Returns
///
/// The transformed markdown text with wikilinks converted to standard links.
///
/// # Examples
///
/// ```
/// use std::collections::HashSet;
/// use mbr::wikilink::transform_wikilinks;
///
/// let sources: HashSet<String> = ["tags"].iter().map(|s| s.to_string()).collect();
/// let input = "Check out [[Tags:rust]] and [[Tags:programming]]!";
/// let output = transform_wikilinks(input, &sources);
/// assert_eq!(output, "Check out [rust](/tags/rust/) and [programming](/tags/programming/)!");
/// ```
pub fn transform_wikilinks(input: &str, valid_sources: &HashSet<String>) -> String {
    // Regex pattern for [[Source:value]] where value can contain spaces
    // Match [[Source:value]] but NOT [[Source:value|display]] (Obsidian style)
    let mut result = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = remaining.find("[[") {
        // Add text before the wikilink
        result.push_str(&remaining[..start]);

        // Find the closing ]]
        let after_open = &remaining[start + 2..];
        if let Some(end) = after_open.find("]]") {
            let inner = &after_open[..end];

            // Try to parse as Source:value
            if let Some(wikilink) = parse_wikilink_inner(inner, valid_sources) {
                result.push_str(&wikilink.to_markdown_link());
            } else {
                // Not a valid tag wikilink, keep original
                result.push_str(&remaining[start..start + 4 + end]);
            }

            remaining = &after_open[end + 2..];
        } else {
            // No closing ]], keep original and move past [[
            result.push_str("[[");
            remaining = after_open;
        }
    }

    // Add any remaining text
    result.push_str(remaining);
    result
}

/// Parses the inner content of a wikilink (`Source:value`).
///
/// Returns `None` if:
/// - No colon found
/// - Source is empty
/// - Source is a URL scheme
/// - Source is not in valid_sources set
fn parse_wikilink_inner(inner: &str, valid_sources: &HashSet<String>) -> Option<ParsedWikilink> {
    // Split on first colon only
    let colon_pos = inner.find(':')?;
    let source = inner[..colon_pos].trim();
    let value = inner[colon_pos + 1..].trim();

    // Validate source
    if source.is_empty() || value.is_empty() {
        return None;
    }

    // Skip URL schemes
    if is_url_scheme(source) {
        return None;
    }

    // Check if source is in valid sources (case-insensitive)
    let source_lower = source.to_lowercase();
    if !valid_sources
        .iter()
        .any(|s| s.to_lowercase() == source_lower)
    {
        return None;
    }

    Some(ParsedWikilink::new(source, value))
}

/// Parses a markdown link destination to check if it's a tag link.
///
/// Detects `Source:value` patterns in link destinations like `[text](Source:value)`.
///
/// # Arguments
///
/// * `dest` - The link destination (the part in parentheses)
/// * `valid_sources` - Set of valid tag source names (case-insensitive matching)
///
/// # Returns
///
/// `Some(ParsedWikilink)` if this is a valid tag link, `None` otherwise.
///
/// # Examples
///
/// ```
/// use std::collections::HashSet;
/// use mbr::wikilink::parse_tag_link;
///
/// let sources: HashSet<String> = ["tags", "performers"].iter().map(|s| s.to_string()).collect();
///
/// // Valid tag link
/// let result = parse_tag_link("Tags:rust", &sources);
/// assert!(result.is_some());
/// assert_eq!(result.unwrap().url_path(), "/tags/rust/");
///
/// // URL scheme - not a tag link
/// assert!(parse_tag_link("https://example.com", &sources).is_none());
///
/// // Unknown source - not a tag link
/// assert!(parse_tag_link("category:books", &sources).is_none());
/// ```
pub fn parse_tag_link(dest: &str, valid_sources: &HashSet<String>) -> Option<ParsedWikilink> {
    parse_wikilink_inner(dest, valid_sources)
}

/// Transforms a tag link destination to a proper URL path.
///
/// If the destination is a tag link (`Source:value`), returns the proper URL path.
/// Otherwise, returns `None` and the original destination should be used.
///
/// # Arguments
///
/// * `dest` - The link destination
/// * `valid_sources` - Set of valid tag source names
///
/// # Returns
///
/// `Some(url_path)` if this is a tag link, `None` otherwise.
pub fn transform_tag_link_dest(dest: &str, valid_sources: &HashSet<String>) -> Option<String> {
    parse_tag_link(dest, valid_sources).map(|wl| wl.url_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sources(sources: &[&str]) -> HashSet<String> {
        sources.iter().map(|s| s.to_string()).collect()
    }

    // normalize_tag_value tests

    #[test]
    fn test_normalize_tag_value_basic() {
        assert_eq!(normalize_tag_value("rust"), "rust");
        assert_eq!(normalize_tag_value("Rust"), "rust");
        assert_eq!(normalize_tag_value("RUST"), "rust");
    }

    #[test]
    fn test_normalize_tag_value_spaces() {
        assert_eq!(normalize_tag_value("Joshua Jay"), "joshua_jay");
        assert_eq!(normalize_tag_value("hello world"), "hello_world");
        assert_eq!(normalize_tag_value("a b c"), "a_b_c");
    }

    #[test]
    fn test_normalize_tag_value_trims() {
        assert_eq!(normalize_tag_value("  rust  "), "rust");
        assert_eq!(normalize_tag_value("\tspaced\t"), "spaced");
    }

    // is_url_scheme tests

    #[test]
    fn test_is_url_scheme() {
        assert!(is_url_scheme("http"));
        assert!(is_url_scheme("HTTP"));
        assert!(is_url_scheme("https"));
        assert!(is_url_scheme("mailto"));
        assert!(is_url_scheme("file"));

        assert!(!is_url_scheme("tags"));
        assert!(!is_url_scheme("performers"));
        assert!(!is_url_scheme("category"));
    }

    // ParsedWikilink tests

    #[test]
    fn test_parsed_wikilink_url_path() {
        let wl = ParsedWikilink::new("Tags", "Rust");
        assert_eq!(wl.url_path(), "/tags/rust/");

        let wl = ParsedWikilink::new("performers", "Joshua Jay");
        assert_eq!(wl.url_path(), "/performers/joshua_jay/");
    }

    #[test]
    fn test_parsed_wikilink_display() {
        let wl = ParsedWikilink::new("tags", "rust");
        assert_eq!(wl.display(), "rust");

        let wl = ParsedWikilink::with_display("tags", "rust", "Rust Programming");
        assert_eq!(wl.display(), "Rust Programming");
    }

    #[test]
    fn test_parsed_wikilink_to_markdown() {
        let wl = ParsedWikilink::new("Tags", "rust");
        assert_eq!(wl.to_markdown_link(), "[rust](/tags/rust/)");

        let wl = ParsedWikilink::new("performers", "Joshua Jay");
        assert_eq!(
            wl.to_markdown_link(),
            "[Joshua Jay](/performers/joshua_jay/)"
        );
    }

    // transform_wikilinks tests

    #[test]
    fn test_transform_wikilinks_basic() {
        let sources = make_sources(&["tags"]);
        let input = "See [[Tags:rust]] for more.";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(output, "See [rust](/tags/rust/) for more.");
    }

    #[test]
    fn test_transform_wikilinks_multiple() {
        let sources = make_sources(&["tags"]);
        let input = "[[Tags:rust]] and [[Tags:programming]] are great.";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(
            output,
            "[rust](/tags/rust/) and [programming](/tags/programming/) are great."
        );
    }

    #[test]
    fn test_transform_wikilinks_with_spaces() {
        let sources = make_sources(&["performers"]);
        let input = "Watch [[performers:Joshua Jay]] perform!";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(
            output,
            "Watch [Joshua Jay](/performers/joshua_jay/) perform!"
        );
    }

    #[test]
    fn test_transform_wikilinks_case_insensitive_source() {
        let sources = make_sources(&["tags"]);

        let input1 = "[[Tags:rust]]";
        let input2 = "[[TAGS:rust]]";
        let input3 = "[[tags:rust]]";

        assert_eq!(transform_wikilinks(input1, &sources), "[rust](/tags/rust/)");
        assert_eq!(transform_wikilinks(input2, &sources), "[rust](/tags/rust/)");
        assert_eq!(transform_wikilinks(input3, &sources), "[rust](/tags/rust/)");
    }

    #[test]
    fn test_transform_wikilinks_unknown_source() {
        let sources = make_sources(&["tags"]);
        let input = "[[category:books]]"; // category not in sources
        let output = transform_wikilinks(input, &sources);
        assert_eq!(output, "[[category:books]]"); // Unchanged
    }

    #[test]
    fn test_transform_wikilinks_url_scheme_not_matched() {
        let sources = make_sources(&["tags", "http"]); // Even if http were a source, it should be skipped
        let input = "[[http://example.com]]";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(output, "[[http://example.com]]"); // Unchanged
    }

    #[test]
    fn test_transform_wikilinks_nested_source() {
        let sources = make_sources(&["taxonomy.tags"]);
        let input = "[[taxonomy.tags:rust]]";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(output, "[rust](/taxonomy.tags/rust/)");
    }

    #[test]
    fn test_transform_wikilinks_no_closing() {
        let sources = make_sources(&["tags"]);
        let input = "[[Tags:rust is broken";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(output, "[[Tags:rust is broken"); // Unchanged
    }

    #[test]
    fn test_transform_wikilinks_empty_value() {
        let sources = make_sources(&["tags"]);
        let input = "[[Tags:]]";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(output, "[[Tags:]]"); // Unchanged (empty value)
    }

    // parse_tag_link tests

    #[test]
    fn test_parse_tag_link_valid() {
        let sources = make_sources(&["tags", "performers"]);

        let result = parse_tag_link("Tags:rust", &sources);
        assert!(result.is_some());
        let wl = result.unwrap();
        assert_eq!(wl.source, "Tags");
        assert_eq!(wl.value, "rust");
        assert_eq!(wl.url_path(), "/tags/rust/");
    }

    #[test]
    fn test_parse_tag_link_with_spaces() {
        let sources = make_sources(&["performers"]);

        let result = parse_tag_link("performers:Joshua Jay", &sources);
        assert!(result.is_some());
        let wl = result.unwrap();
        assert_eq!(wl.value, "Joshua Jay");
        assert_eq!(wl.url_path(), "/performers/joshua_jay/");
    }

    #[test]
    fn test_parse_tag_link_url_scheme() {
        let sources = make_sources(&["tags", "https"]);

        assert!(parse_tag_link("https://example.com", &sources).is_none());
        assert!(parse_tag_link("mailto:test@example.com", &sources).is_none());
        assert!(parse_tag_link("file:///path/to/file", &sources).is_none());
    }

    #[test]
    fn test_parse_tag_link_unknown_source() {
        let sources = make_sources(&["tags"]);

        assert!(parse_tag_link("category:books", &sources).is_none());
    }

    #[test]
    fn test_parse_tag_link_no_colon() {
        let sources = make_sources(&["tags"]);

        assert!(parse_tag_link("just-a-path", &sources).is_none());
        assert!(parse_tag_link("/absolute/path", &sources).is_none());
    }

    // transform_tag_link_dest tests

    #[test]
    fn test_transform_tag_link_dest() {
        let sources = make_sources(&["tags"]);

        assert_eq!(
            transform_tag_link_dest("Tags:rust", &sources),
            Some("/tags/rust/".to_string())
        );

        assert_eq!(
            transform_tag_link_dest("https://example.com", &sources),
            None
        );
        assert_eq!(transform_tag_link_dest("/regular/path/", &sources), None);
    }
}
