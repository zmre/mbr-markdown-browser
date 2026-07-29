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

/// Sanitizes a string for safe use as a path component.
///
/// Removes characters and patterns that could cause path traversal or other
/// filesystem safety issues:
/// - Strips null bytes and control characters
/// - Splits on **both** `/` and `\`, so a Windows-style traversal cannot
///   survive as a single opaque segment (the result always uses `/`, so output
///   is byte-identical on every platform)
/// - Removes `.` and `..` segments
/// - Removes drive-prefixed segments (`C:`, `C:evil`), which `Path::join`
///   treats as absolute or drive-relative on Windows
/// - Strips leading separators and collapses repeated ones
///
/// # Examples
///
/// ```
/// use mbr::wikilink::sanitize_path_component;
///
/// assert_eq!(sanitize_path_component("rust"), "rust");
/// assert_eq!(sanitize_path_component("/etc/passwd"), "etc/passwd");
/// assert_eq!(sanitize_path_component("../../secret"), "secret");
/// assert_eq!(sanitize_path_component("foo/../bar"), "foo/bar");
/// assert_eq!(sanitize_path_component(r"..\..\Users\victim"), "Users/victim");
/// assert_eq!(sanitize_path_component(r"C:\Windows\evil"), "Windows/evil");
/// ```
pub fn sanitize_path_component(value: &str) -> String {
    value
        // Remove null bytes and control characters
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        // Split on both separators, drop empty/relative/drive segments, rejoin
        .split(['/', '\\'])
        .filter(|seg| !seg.is_empty() && *seg != ".." && *seg != "." && !has_drive_prefix(seg))
        .collect::<Vec<_>>()
        .join("/")
}

/// True when a path segment starts with a Windows drive prefix (`C:`).
///
/// Such a segment must be dropped rather than kept: `Path::join` on Windows
/// treats both `C:\x` (absolute) and `C:x` (drive-relative) as replacing the
/// base path, which would let a tag value escape the build output directory.
fn has_drive_prefix(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Normalizes a tag value for use in URLs.
///
/// - Converts to lowercase
/// - Replaces spaces with underscores
/// - Trims leading/trailing whitespace
/// - Sanitizes against path traversal
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
    sanitize_path_component(&value.trim().to_lowercase().replace(' ', "_"))
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
/// # Code regions are left alone
///
/// This is a raw-text prepass that runs *before* pulldown-cmark parses the
/// document, so it has to recognise code itself. Wikilinks are left verbatim
/// inside:
///
/// - fenced code blocks — backtick and tilde fences, any fence length, with or
///   without an info string, including fences written inside a blockquote;
/// - indented code blocks — four or more columns of indentation opening a block
///   outside a list;
/// - inline code spans — including multi-backtick spans and spans that wrap
///   onto the following line.
///
/// An unclosed fence runs to the end of the input, which is what CommonMark
/// (and therefore pulldown-cmark) does too: the remainder of the file really is
/// code, so leaving its wikilinks literal keeps this pass and the parser in
/// agreement.
///
/// Detection is a single left-to-right line scan with no regexes; documents
/// containing no `[[` at all skip it entirely.
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
///
/// // Code samples are documentation, not links.
/// let input = "```\n[[Tags:rust]]\n```";
/// assert_eq!(transform_wikilinks(input, &sources), input);
/// ```
pub fn transform_wikilinks(input: &str, valid_sources: &HashSet<String>) -> String {
    // Fast path: most documents contain no wikilink at all, so skip the block
    // scan entirely and pay only one substring search.
    if !input.contains("[[") {
        return input.to_string();
    }

    let mut result = String::with_capacity(input.len());
    let mut blocks = BlockScanner::new();
    // Start of the pending run of consecutive non-code lines. Runs are
    // transformed as a unit so inline code spans that wrap across lines are
    // still recognised.
    let mut text_start = 0;
    let mut pos = 0;

    while pos < input.len() {
        let line_len = input[pos..]
            .find('\n')
            .map_or(input.len() - pos, |idx| idx + 1);
        let line_end = pos + line_len;
        if blocks.classify(&input[pos..line_end]) == LineKind::Code {
            push_transformed(&mut result, &input[text_start..pos], valid_sources);
            result.push_str(&input[pos..line_end]);
            text_start = line_end;
        }
        pos = line_end;
    }
    push_transformed(&mut result, &input[text_start..], valid_sources);
    result
}

/// Whether a source line belongs to a code block (copied verbatim) or to
/// ordinary text (scanned for wikilinks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    Text,
    Code,
}

/// Line-at-a-time scanner for the block-level code regions that
/// [`transform_wikilinks`] must not rewrite inside.
struct BlockScanner {
    /// Open fence as (marker byte, marker run length), if any.
    fence: Option<(u8, usize)>,
    /// Previous line was blank; the start of the input counts as blank.
    after_blank: bool,
    /// Currently inside an indented code block.
    indented: bool,
    /// Inside a list, where four-column indentation is item content rather than
    /// code. Approximated from the last unindented line, which keeps loose list
    /// items (`- a`, blank, four-space continuation paragraph) rewritable.
    in_list: bool,
}

impl BlockScanner {
    const fn new() -> Self {
        Self {
            fence: None,
            after_blank: true,
            indented: false,
            in_list: false,
        }
    }

    /// Classifies one line (trailing newline included) and advances the state.
    fn classify(&mut self, line: &str) -> LineKind {
        let content = line.trim_end_matches(['\n', '\r']);

        if let Some((marker, len)) = self.fence {
            if is_fence_close(content, marker, len) {
                self.fence = None;
            }
            return LineKind::Code;
        }

        if content.trim().is_empty() {
            self.after_blank = true;
            self.indented = false;
            return LineKind::Text;
        }

        let indent = indent_width(content);
        if indent >= 4 && !self.in_list && (self.after_blank || self.indented) {
            self.indented = true;
            self.after_blank = false;
            return LineKind::Code;
        }
        self.indented = false;
        self.after_blank = false;

        let bare = strip_block_markers(content);
        if indent == 0 {
            self.in_list = starts_list_item(bare);
        }
        match fence_open(bare) {
            Some(fence) => {
                self.fence = Some(fence);
                LineKind::Code
            }
            None => LineKind::Text,
        }
    }
}

/// Columns of leading indentation, counting a tab as four columns.
fn indent_width(line: &str) -> usize {
    line.bytes()
        .take_while(|b| *b == b' ' || *b == b'\t')
        .map(|b| if b == b'\t' { 4 } else { 1 })
        .sum()
}

/// Strips leading indentation and any blockquote markers, so a fence written
/// inside a blockquote is still recognised as a fence.
fn strip_block_markers(line: &str) -> &str {
    let mut rest = line.trim_start_matches([' ', '\t']);
    while let Some(after) = rest.strip_prefix('>') {
        rest = after.trim_start_matches([' ', '\t']);
    }
    rest
}

/// Length of the leading run of `marker` bytes.
fn marker_run(text: &str, marker: u8) -> usize {
    text.bytes().take_while(|b| *b == marker).count()
}

/// Returns the fence marker and run length when `line` (block markers already
/// stripped) opens a fenced code block.
fn fence_open(line: &str) -> Option<(u8, usize)> {
    let marker = *line.as_bytes().first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let len = marker_run(line, marker);
    // A backtick info string may not contain a backtick (CommonMark), which is
    // what keeps ``code`` spans from being mistaken for fences.
    if len < 3 || (marker == b'`' && line[len..].contains('`')) {
        return None;
    }
    Some((marker, len))
}

/// True when `line` closes an open fence of `marker` repeated at least `len`
/// times, with nothing but whitespace after the run.
fn is_fence_close(line: &str, marker: u8, len: usize) -> bool {
    let bare = strip_block_markers(line);
    let run = marker_run(bare, marker);
    run >= len && bare[run..].trim().is_empty()
}

/// True when `line` (block markers stripped) starts a list item.
fn starts_list_item(line: &str) -> bool {
    let bytes = line.as_bytes();
    match bytes.first() {
        Some(b'-' | b'*' | b'+') => matches!(bytes.get(1), None | Some(b' ' | b'\t')),
        Some(b'0'..=b'9') => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            matches!(bytes.get(digits), Some(b'.' | b')'))
                && matches!(bytes.get(digits + 1), None | Some(b' ' | b'\t'))
        }
        _ => false,
    }
}

/// Rewrites the wikilinks in a run of non-code lines, stepping over inline code
/// spans. Untransformed stretches are copied in one slice each, so a run with
/// no wikilink costs a single substring search and one `push_str`.
fn push_transformed(result: &mut String, text: &str, valid_sources: &HashSet<String>) {
    if !text.contains("[[") {
        result.push_str(text);
        return;
    }

    let bytes = text.as_bytes();
    // Everything before `copied` has already been written to `result`.
    let mut copied = 0;
    let mut pos = 0;

    while pos < bytes.len() {
        match bytes[pos] {
            b'`' => {
                let run = marker_run(&text[pos..], b'`');
                pos += run;
                // Unclosed backticks are literal text, so scanning simply
                // continues after them.
                if let Some(end) = code_span_end(&text[pos..], run) {
                    pos += end;
                }
            }
            b'[' if bytes.get(pos + 1) == Some(&b'[') => {
                let inner_start = pos + 2;
                // With no `]]` left in the run, no later wikilink can close
                // either, so the rest is copied verbatim.
                let Some(offset) = text[inner_start..].find("]]") else {
                    break;
                };
                let inner = &text[inner_start..inner_start + offset];
                if let Some(wikilink) = parse_wikilink_inner(inner, valid_sources) {
                    result.push_str(&text[copied..pos]);
                    result.push_str(&wikilink.to_markdown_link());
                    copied = inner_start + offset + 2;
                }
                pos = inner_start + offset + 2;
            }
            _ => pos += 1,
        }
    }

    result.push_str(&text[copied..]);
}

/// Byte offset just past the closing backtick run of a code span opened with
/// `open` backticks, or `None` when the span never closes.
///
/// A code span cannot contain a blank line (it lives inside one paragraph), so
/// a blank line ends the search and the opening backticks stay literal text.
fn code_span_end(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut pos = 0;

    while pos < bytes.len() {
        match bytes[pos] {
            b'`' => {
                // Only an exactly equal run closes the span; longer or shorter
                // runs are span content.
                let run = marker_run(&text[pos..], b'`');
                pos += run;
                if run == open {
                    return Some(pos);
                }
            }
            b'\n' => {
                let next = pos + 1;
                let blank = text[next..]
                    .bytes()
                    .take_while(|b| *b != b'\n')
                    .all(|b| b.is_ascii_whitespace());
                if blank {
                    return None;
                }
                pos = next;
            }
            _ => pos += 1,
        }
    }
    None
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

    // transform_wikilinks code-region tests
    //
    // These run as a raw-text prepass before pulldown-cmark, so every code
    // construct the parser recognises has to be recognised here too, else the
    // documentation of the wikilink syntax rewrites itself.

    #[test]
    fn test_transform_wikilinks_skips_fenced_code_block() {
        let sources = make_sources(&["tags"]);
        let input =
            "Before [[Tags:rust]].\n\n```markdown\n[[Tags:rust]]\n```\n\nAfter [[Tags:rust]].";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(
            output,
            "Before [rust](/tags/rust/).\n\n```markdown\n[[Tags:rust]]\n```\n\nAfter [rust](/tags/rust/)."
        );
    }

    #[test]
    fn test_transform_wikilinks_skips_tilde_fence() {
        let sources = make_sources(&["tags"]);
        let input = "~~~\n[[Tags:rust]]\n~~~\n[[Tags:rust]]";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(output, "~~~\n[[Tags:rust]]\n~~~\n[rust](/tags/rust/)");
    }

    #[test]
    fn test_transform_wikilinks_skips_longer_fence_containing_shorter_one() {
        // A four-backtick fence is only closed by four backticks, so the inner
        // three-backtick lines are content.
        let sources = make_sources(&["tags"]);
        let input = "````\n```\n[[Tags:rust]]\n```\n````\n[[Tags:rust]]";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(
            output,
            "````\n```\n[[Tags:rust]]\n```\n````\n[rust](/tags/rust/)"
        );
    }

    #[test]
    fn test_transform_wikilinks_skips_fence_inside_blockquote() {
        let sources = make_sources(&["tags"]);
        let input = "> ```\n> [[Tags:rust]]\n> ```\n\n[[Tags:rust]]";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(
            output,
            "> ```\n> [[Tags:rust]]\n> ```\n\n[rust](/tags/rust/)"
        );
    }

    #[test]
    fn test_transform_wikilinks_unclosed_fence_runs_to_end_of_input() {
        // Documented behaviour: CommonMark treats everything after an unclosed
        // fence as code, so this pass does too — staying consistent with the
        // parser matters more than rewriting text the reader sees as code.
        let sources = make_sources(&["tags"]);
        let input = "[[Tags:rust]]\n\n```\n[[Tags:rust]]\nstill code\n[[Tags:rust]]";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(
            output,
            "[rust](/tags/rust/)\n\n```\n[[Tags:rust]]\nstill code\n[[Tags:rust]]"
        );
    }

    #[test]
    fn test_transform_wikilinks_skips_inline_code_span() {
        let sources = make_sources(&["tags"]);
        let input = "Write `[[Tags:rust]]` to link [[Tags:rust]].";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(output, "Write `[[Tags:rust]]` to link [rust](/tags/rust/).");
    }

    #[test]
    fn test_transform_wikilinks_skips_multi_backtick_span() {
        let sources = make_sources(&["tags"]);
        let input = "Use ``[[Tags:rust]] and `x` `` then [[Tags:rust]].";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(
            output,
            "Use ``[[Tags:rust]] and `x` `` then [rust](/tags/rust/)."
        );
    }

    #[test]
    fn test_transform_wikilinks_skips_span_wrapping_to_next_line() {
        let sources = make_sources(&["tags"]);
        let input = "Use `code\n[[Tags:rust]]` here, but [[Tags:rust]] links.";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(
            output,
            "Use `code\n[[Tags:rust]]` here, but [rust](/tags/rust/) links."
        );
    }

    #[test]
    fn test_transform_wikilinks_unmatched_backtick_is_literal() {
        // A stray backtick must not suppress the rest of the document: a code
        // span cannot cross a blank line, so the paragraph after it is text.
        let sources = make_sources(&["tags"]);
        let input = "A stray ` tick [[Tags:rust]].\n\n[[Tags:rust]] again.";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(
            output,
            "A stray ` tick [rust](/tags/rust/).\n\n[rust](/tags/rust/) again."
        );
    }

    #[test]
    fn test_transform_wikilinks_skips_indented_code_block() {
        let sources = make_sources(&["tags"]);
        let input = "Example:\n\n    [[Tags:rust]]\n    more code\n\nBack to [[Tags:rust]].";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(
            output,
            "Example:\n\n    [[Tags:rust]]\n    more code\n\nBack to [rust](/tags/rust/)."
        );
    }

    #[test]
    fn test_transform_wikilinks_tab_indented_code_block() {
        let sources = make_sources(&["tags"]);
        let input = "Example:\n\n\t[[Tags:rust]]\n\nText [[Tags:rust]].";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(
            output,
            "Example:\n\n\t[[Tags:rust]]\n\nText [rust](/tags/rust/)."
        );
    }

    #[test]
    fn test_transform_wikilinks_indented_paragraph_continuation_is_text() {
        // Four-space indentation directly under a paragraph line is a lazy
        // continuation, not a code block.
        let sources = make_sources(&["tags"]);
        let input = "A paragraph\n    with [[Tags:rust]] wrapped.";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(output, "A paragraph\n    with [rust](/tags/rust/) wrapped.");
    }

    #[test]
    fn test_transform_wikilinks_indented_list_continuation_is_text() {
        // Inside a list, four-space indentation is item content.
        let sources = make_sources(&["tags"]);
        let input = "- item\n\n    Continued [[Tags:rust]] text.";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(output, "- item\n\n    Continued [rust](/tags/rust/) text.");
    }

    #[test]
    fn test_transform_wikilinks_indented_code_after_list_ends() {
        // A top-level paragraph closes the list, so indentation is code again.
        let sources = make_sources(&["tags"]);
        let input = "- item\n\ntext\n\n    [[Tags:rust]]\n";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(output, "- item\n\ntext\n\n    [[Tags:rust]]\n");
    }

    #[test]
    fn test_transform_wikilinks_indented_first_line_is_code() {
        // The start of the document behaves like the line after a blank line.
        let sources = make_sources(&["tags"]);
        let input = "    [[Tags:rust]]\n";
        assert_eq!(transform_wikilinks(input, &sources), input);
    }

    #[test]
    fn test_transform_wikilinks_crlf_fence() {
        let sources = make_sources(&["tags"]);
        let input = "```\r\n[[Tags:rust]]\r\n```\r\n[[Tags:rust]]\r\n";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(
            output,
            "```\r\n[[Tags:rust]]\r\n```\r\n[rust](/tags/rust/)\r\n"
        );
    }

    #[test]
    fn test_transform_wikilinks_inline_code_in_fence_info_is_not_a_fence() {
        // ``x`` is a code span, not a fence: an info string may not contain a
        // backtick, so the line stays text and the later wikilink is rewritten.
        let sources = make_sources(&["tags"]);
        let input = "``x`` starts a line\n[[Tags:rust]]";
        let output = transform_wikilinks(input, &sources);
        assert_eq!(output, "``x`` starts a line\n[rust](/tags/rust/)");
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

    // sanitize_path_component tests

    #[test]
    fn test_sanitize_path_component_normal_values() {
        assert_eq!(sanitize_path_component("rust"), "rust");
        assert_eq!(sanitize_path_component("hello_world"), "hello_world");
        assert_eq!(sanitize_path_component("foo/bar"), "foo/bar");
    }

    #[test]
    fn test_sanitize_path_component_strips_leading_slash() {
        assert_eq!(sanitize_path_component("/etc/passwd"), "etc/passwd");
        assert_eq!(sanitize_path_component("//absolute"), "absolute");
        assert_eq!(sanitize_path_component("///triple"), "triple");
    }

    #[test]
    fn test_sanitize_path_component_removes_dotdot() {
        assert_eq!(sanitize_path_component("../../secret"), "secret");
        assert_eq!(sanitize_path_component("foo/../bar"), "foo/bar");
        assert_eq!(sanitize_path_component("../.."), "");
        assert_eq!(sanitize_path_component("a/../../b"), "a/b");
    }

    #[test]
    fn test_sanitize_path_component_removes_single_dot() {
        assert_eq!(sanitize_path_component("./foo"), "foo");
        assert_eq!(sanitize_path_component("foo/./bar"), "foo/bar");
    }

    #[test]
    fn test_sanitize_path_component_null_bytes() {
        assert_eq!(sanitize_path_component("foo\0bar"), "foobar");
        assert_eq!(sanitize_path_component("\0"), "");
    }

    #[test]
    fn test_sanitize_path_component_control_chars() {
        assert_eq!(sanitize_path_component("foo\x01bar"), "foobar");
        assert_eq!(sanitize_path_component("hello\nworld"), "helloworld");
    }

    #[test]
    fn test_sanitize_path_component_complex_attacks() {
        // Wikipedia-style attack path from goodwiki
        assert_eq!(sanitize_path_component("/pol/_phenomena"), "pol/_phenomena");
        // Multiple traversals
        assert_eq!(
            sanitize_path_component("/../../../etc/shadow"),
            "etc/shadow"
        );
    }

    #[test]
    fn test_sanitize_path_component_empty() {
        assert_eq!(sanitize_path_component(""), "");
        assert_eq!(sanitize_path_component("/"), "");
        assert_eq!(sanitize_path_component("//"), "");
    }

    #[test]
    fn test_sanitize_path_component_backslash_separators() {
        // `\` is a separator on Windows, so a backslash traversal must not
        // survive as one opaque segment that the build joins onto --output.
        assert_eq!(
            sanitize_path_component(r"..\..\..\Users\victim\pwned"),
            "Users/victim/pwned"
        );
        assert_eq!(sanitize_path_component(r"foo\bar"), "foo/bar");
        assert_eq!(sanitize_path_component(r"\absolute"), "absolute");
        assert_eq!(sanitize_path_component(r"a\..\b"), "a/b");
        assert_eq!(
            sanitize_path_component(r"mixed/back\slash"),
            "mixed/back/slash"
        );
        // UNC prefixes collapse to plain relative segments.
        assert_eq!(
            sanitize_path_component(r"\\server\share\evil"),
            "server/share/evil"
        );
    }

    #[test]
    fn test_sanitize_path_component_drops_drive_prefix() {
        assert_eq!(sanitize_path_component(r"C:\Windows\evil"), "Windows/evil");
        // Drive-relative (`C:evil`) also replaces the base path on Windows.
        assert_eq!(sanitize_path_component("C:evil"), "");
        assert_eq!(sanitize_path_component("z:"), "");
        // Only a single-letter drive letter counts; ordinary values keep colons.
        assert_eq!(sanitize_path_component("ab:cd"), "ab:cd");
        assert_eq!(sanitize_path_component("9:30"), "9:30");
    }

    #[cfg(windows)]
    #[test]
    fn test_sanitize_path_component_join_stays_inside_output_dir() {
        use std::path::{Component, Path};

        let output = Path::new(r"C:\build");
        for payload in [
            r"..\..\..\Users\victim\pwned",
            r"C:\Windows\System32\evil",
            r"C:evil",
            r"\\server\share\evil",
        ] {
            let sanitized = sanitize_path_component(payload);
            assert!(
                !Path::new(&sanitized).components().any(|component| matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )),
                "{payload} kept an escaping component: {sanitized}"
            );
            let joined = output.join(&sanitized);
            assert!(
                joined.starts_with(output),
                "{payload} escaped --output: {joined:?}"
            );
        }
    }

    // normalize_tag_value with path traversal

    #[test]
    fn test_normalize_tag_value_sanitizes_paths() {
        assert_eq!(normalize_tag_value("/etc/passwd"), "etc/passwd");
        assert_eq!(normalize_tag_value("../../secret"), "secret");
        assert_eq!(normalize_tag_value("/pol/_phenomena"), "pol/_phenomena");
    }

    #[test]
    fn test_normalize_tag_value_sanitizes_windows_paths() {
        // The frontmatter payload from the tag-page build path.
        assert_eq!(
            normalize_tag_value(r"..\..\..\Users\victim\pwned"),
            "users/victim/pwned"
        );
        assert_eq!(normalize_tag_value(r"C:\Windows\Temp"), "windows/temp");
    }
}
