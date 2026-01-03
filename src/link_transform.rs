//! Link transformation for trailing-slash URL convention.
//!
//! When markdown files are served with trailing-slash URLs (e.g., `docs/guide.md` → `/docs/guide/`),
//! relative links in the markdown need to be adjusted so they resolve correctly from the browser's
//! perspective.
//!
//! ## Problem
//!
//! A link `[other](other.md)` in `docs/guide.md`:
//! - Filesystem: refers to `docs/other.md` (sibling file)
//! - From URL `/docs/guide/`: browser resolves `other.md` as `/docs/guide/other.md` (WRONG)
//! - Correct URL: `/docs/other/`
//!
//! ## Solution
//!
//! Transform relative links by:
//! 1. Adding `../` prefix for regular markdown files (not index files)
//! 2. Replacing markdown extensions with trailing slash
//! 3. Collapsing index file references to their directory

/// Configuration for link transformation.
#[derive(Debug, Clone)]
pub struct LinkTransformConfig {
    /// Markdown file extensions (e.g., ["md", "markdown"])
    pub markdown_extensions: Vec<String>,
    /// Index filename (e.g., "index.md")
    pub index_file: String,
    /// Whether the current file is an index file (affects ../ prefix)
    pub is_index_file: bool,
}

impl Default for LinkTransformConfig {
    fn default() -> Self {
        Self {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
        }
    }
}

/// Transform a relative link URL for the trailing-slash URL convention.
///
/// # Rules
///
/// 1. Absolute URLs (`http://`, `https://`, `//`) → unchanged
/// 2. Root-relative URLs (starts with `/`) → unchanged
/// 3. Anchor-only links (`#...`) → unchanged
/// 4. Data/javascript URLs → unchanged
/// 5. Relative markdown links → prepend `../` (if not index file), replace extension with `/`
/// 6. Relative static files → prepend `../` (if not index file)
///
/// # Examples
///
/// ```
/// use mbr::link_transform::{transform_link, LinkTransformConfig};
///
/// let config = LinkTransformConfig {
///     markdown_extensions: vec!["md".to_string()],
///     index_file: "index.md".to_string(),
///     is_index_file: false,
/// };
///
/// // Regular markdown file: add ../ and trailing slash
/// assert_eq!(transform_link("other.md", &config), "../other/");
///
/// // Index file config: no ../ prefix
/// let index_config = LinkTransformConfig { is_index_file: true, ..config.clone() };
/// assert_eq!(transform_link("other.md", &index_config), "other/");
///
/// // Absolute URLs unchanged
/// assert_eq!(transform_link("https://example.com", &config), "https://example.com");
/// ```
pub fn transform_link(url: &str, config: &LinkTransformConfig) -> String {
    // Empty or whitespace-only
    if url.is_empty() || url.trim().is_empty() {
        return url.to_string();
    }

    // Anchor-only links
    if url.starts_with('#') {
        return url.to_string();
    }

    // Absolute URLs (http://, https://, //)
    if is_absolute_url(url) {
        return url.to_string();
    }

    // Root-relative URLs
    if url.starts_with('/') {
        return url.to_string();
    }

    // Data URLs and javascript URLs
    if url.starts_with("data:") || url.starts_with("javascript:") {
        return url.to_string();
    }

    // Mailto links (these are handled specially elsewhere, but be safe)
    if url.starts_with("mailto:") {
        return url.to_string();
    }

    // Split into path and suffix (anchor/query)
    let (path, suffix) = split_url_parts(url);

    // Empty path after splitting (e.g., just "?query" or malformed)
    if path.is_empty() {
        return url.to_string();
    }

    // Normalize: strip leading "./"
    let path = path.strip_prefix("./").unwrap_or(&path);

    // Count and strip existing "../" prefixes
    let (parent_count, remaining_path) = count_parent_traversals(path);

    // If nothing remains after stripping ../, just return with adjusted parents
    if remaining_path.is_empty() {
        let prefix = if config.is_index_file {
            "../".repeat(parent_count)
        } else {
            "../".repeat(parent_count + 1)
        };
        return format!("{}{}", prefix, suffix);
    }

    // Check if it's a markdown file
    if let Some(base_path) = strip_markdown_extension(remaining_path, &config.markdown_extensions) {
        // Check if it ends with index file (without extension)
        let index_stem = config
            .index_file
            .strip_suffix(".md")
            .or_else(|| config.index_file.strip_suffix(".markdown"))
            .unwrap_or(&config.index_file);

        let final_path = if base_path.ends_with(index_stem) {
            // Collapse index file to directory
            let stripped = base_path
                .strip_suffix(index_stem)
                .unwrap_or(base_path)
                .trim_end_matches('/');
            if stripped.is_empty() {
                // Just "index.md" -> "./" for index files, "../" for regular
                "".to_string()
            } else {
                format!("{}/", stripped)
            }
        } else {
            format!("{}/", base_path)
        };

        // Build prefix based on parent count and whether current file is index
        let prefix = if config.is_index_file {
            "../".repeat(parent_count)
        } else {
            "../".repeat(parent_count + 1)
        };

        // Handle edge case: if final_path is empty and we have no prefix, use "./"
        if final_path.is_empty() && prefix.is_empty() {
            return format!("./{}", suffix);
        }

        return format!("{}{}{}", prefix, final_path, suffix);
    }

    // Static file: just add ../ prefix
    let prefix = if config.is_index_file {
        "../".repeat(parent_count)
    } else {
        "../".repeat(parent_count + 1)
    };

    format!("{}{}{}", prefix, remaining_path, suffix)
}

/// Check if a URL is absolute (has protocol or is protocol-relative).
fn is_absolute_url(url: &str) -> bool {
    url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("//")
        || url.starts_with("ftp://")
        || url.starts_with("file://")
}

/// Split a URL into path and suffix (anchor # or query ?).
/// Returns (path, suffix) where suffix includes the delimiter.
fn split_url_parts(url: &str) -> (String, String) {
    // Find first occurrence of # or ?
    let anchor_pos = url.find('#');
    let query_pos = url.find('?');

    let split_pos = match (anchor_pos, query_pos) {
        (Some(a), Some(q)) => Some(a.min(q)),
        (Some(a), None) => Some(a),
        (None, Some(q)) => Some(q),
        (None, None) => None,
    };

    match split_pos {
        Some(pos) => (url[..pos].to_string(), url[pos..].to_string()),
        None => (url.to_string(), String::new()),
    }
}

/// Count leading "../" sequences and return (count, remaining_path).
fn count_parent_traversals(path: &str) -> (usize, &str) {
    let mut count = 0;
    let mut remaining = path;

    while let Some(rest) = remaining.strip_prefix("../") {
        count += 1;
        remaining = rest;
    }

    (count, remaining)
}

/// Strip markdown extension if present, returning the base path.
fn strip_markdown_extension<'a>(path: &'a str, extensions: &[String]) -> Option<&'a str> {
    for ext in extensions {
        let suffix = format!(".{}", ext);
        if path.ends_with(&suffix) {
            return Some(&path[..path.len() - suffix.len()]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regular_config() -> LinkTransformConfig {
        LinkTransformConfig {
            markdown_extensions: vec!["md".to_string(), "markdown".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
        }
    }

    fn index_config() -> LinkTransformConfig {
        LinkTransformConfig {
            is_index_file: true,
            ..regular_config()
        }
    }

    // =========================================================================
    // Regular markdown files (is_index_file: false)
    // =========================================================================

    #[test]
    fn test_simple_relative_md() {
        assert_eq!(transform_link("other.md", &regular_config()), "../other/");
    }

    #[test]
    fn test_subdirectory_md() {
        assert_eq!(
            transform_link("sub/doc.md", &regular_config()),
            "../sub/doc/"
        );
    }

    #[test]
    fn test_parent_traversal() {
        assert_eq!(
            transform_link("../other.md", &regular_config()),
            "../../other/"
        );
    }

    #[test]
    fn test_double_parent() {
        assert_eq!(
            transform_link("../../root.md", &regular_config()),
            "../../../root/"
        );
    }

    #[test]
    fn test_index_collapse() {
        assert_eq!(
            transform_link("folder/index.md", &regular_config()),
            "../folder/"
        );
    }

    #[test]
    fn test_nested_index_collapse() {
        assert_eq!(transform_link("a/b/index.md", &regular_config()), "../a/b/");
    }

    #[test]
    fn test_just_index_md() {
        // Link to index.md in same directory
        assert_eq!(transform_link("index.md", &regular_config()), "../");
    }

    #[test]
    fn test_static_file() {
        assert_eq!(
            transform_link("image.png", &regular_config()),
            "../image.png"
        );
    }

    #[test]
    fn test_nested_static() {
        assert_eq!(
            transform_link("assets/img.png", &regular_config()),
            "../assets/img.png"
        );
    }

    #[test]
    fn test_md_with_anchor() {
        assert_eq!(
            transform_link("other.md#section", &regular_config()),
            "../other/#section"
        );
    }

    #[test]
    fn test_md_with_query() {
        assert_eq!(
            transform_link("other.md?foo=bar", &regular_config()),
            "../other/?foo=bar"
        );
    }

    #[test]
    fn test_md_with_query_and_anchor() {
        assert_eq!(
            transform_link("other.md?foo=bar#section", &regular_config()),
            "../other/?foo=bar#section"
        );
    }

    #[test]
    fn test_explicit_current_dir() {
        assert_eq!(transform_link("./other.md", &regular_config()), "../other/");
    }

    #[test]
    fn test_alternate_extension() {
        assert_eq!(
            transform_link("other.markdown", &regular_config()),
            "../other/"
        );
    }

    #[test]
    fn test_parent_static_file() {
        assert_eq!(
            transform_link("../image.png", &regular_config()),
            "../../image.png"
        );
    }

    // =========================================================================
    // Index files (is_index_file: true)
    // =========================================================================

    #[test]
    fn test_index_simple_relative_md() {
        assert_eq!(transform_link("other.md", &index_config()), "other/");
    }

    #[test]
    fn test_index_subdirectory_md() {
        assert_eq!(transform_link("sub/doc.md", &index_config()), "sub/doc/");
    }

    #[test]
    fn test_index_parent_traversal() {
        assert_eq!(transform_link("../other.md", &index_config()), "../other/");
    }

    #[test]
    fn test_index_double_parent() {
        assert_eq!(
            transform_link("../../root.md", &index_config()),
            "../../root/"
        );
    }

    #[test]
    fn test_index_static_file() {
        // Index files don't need ../ for siblings
        assert_eq!(transform_link("image.png", &index_config()), "image.png");
    }

    #[test]
    fn test_index_nested_static() {
        assert_eq!(
            transform_link("assets/img.png", &index_config()),
            "assets/img.png"
        );
    }

    #[test]
    fn test_index_md_with_anchor() {
        assert_eq!(
            transform_link("other.md#section", &index_config()),
            "other/#section"
        );
    }

    #[test]
    fn test_index_parent_static() {
        assert_eq!(
            transform_link("../image.png", &index_config()),
            "../image.png"
        );
    }

    #[test]
    fn test_index_to_index_collapse() {
        assert_eq!(
            transform_link("folder/index.md", &index_config()),
            "folder/"
        );
    }

    // =========================================================================
    // URLs that should be unchanged (both modes)
    // =========================================================================

    #[test]
    fn test_absolute_https() {
        let url = "https://example.com/path";
        assert_eq!(transform_link(url, &regular_config()), url);
        assert_eq!(transform_link(url, &index_config()), url);
    }

    #[test]
    fn test_absolute_http() {
        let url = "http://example.com/path";
        assert_eq!(transform_link(url, &regular_config()), url);
        assert_eq!(transform_link(url, &index_config()), url);
    }

    #[test]
    fn test_protocol_relative() {
        let url = "//cdn.example.com/file.js";
        assert_eq!(transform_link(url, &regular_config()), url);
        assert_eq!(transform_link(url, &index_config()), url);
    }

    #[test]
    fn test_root_relative() {
        let url = "/docs/guide/";
        assert_eq!(transform_link(url, &regular_config()), url);
        assert_eq!(transform_link(url, &index_config()), url);
    }

    #[test]
    fn test_anchor_only() {
        let url = "#section";
        assert_eq!(transform_link(url, &regular_config()), url);
        assert_eq!(transform_link(url, &index_config()), url);
    }

    #[test]
    fn test_empty_link() {
        assert_eq!(transform_link("", &regular_config()), "");
        assert_eq!(transform_link("", &index_config()), "");
    }

    #[test]
    fn test_data_url() {
        let url = "data:image/png;base64,abc123";
        assert_eq!(transform_link(url, &regular_config()), url);
    }

    #[test]
    fn test_javascript_url() {
        let url = "javascript:void(0)";
        assert_eq!(transform_link(url, &regular_config()), url);
    }

    #[test]
    fn test_mailto_url() {
        let url = "mailto:test@example.com";
        assert_eq!(transform_link(url, &regular_config()), url);
    }

    #[test]
    fn test_ftp_url() {
        let url = "ftp://ftp.example.com/file.txt";
        assert_eq!(transform_link(url, &regular_config()), url);
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_file_with_dots_in_name() {
        // my.file.md should only strip the final .md
        assert_eq!(
            transform_link("my.file.md", &regular_config()),
            "../my.file/"
        );
    }

    #[test]
    fn test_non_md_extension() {
        assert_eq!(
            transform_link("readme.txt", &regular_config()),
            "../readme.txt"
        );
    }

    #[test]
    fn test_just_query() {
        // Edge case: just a query string
        assert_eq!(transform_link("?foo=bar", &regular_config()), "?foo=bar");
    }

    #[test]
    fn test_deeply_nested_path() {
        assert_eq!(
            transform_link("a/b/c/d/file.md", &regular_config()),
            "../a/b/c/d/file/"
        );
    }

    #[test]
    fn test_mixed_traversal_and_descent() {
        assert_eq!(
            transform_link("../sibling/doc.md", &regular_config()),
            "../../sibling/doc/"
        );
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    fn regular_config() -> LinkTransformConfig {
        LinkTransformConfig {
            markdown_extensions: vec!["md".to_string(), "markdown".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
        }
    }

    fn index_config() -> LinkTransformConfig {
        LinkTransformConfig {
            is_index_file: true,
            ..regular_config()
        }
    }

    proptest! {
        /// Transformation is deterministic
        #[test]
        fn prop_deterministic(url in ".*") {
            let config = regular_config();
            let r1 = transform_link(&url, &config);
            let r2 = transform_link(&url, &config);
            prop_assert_eq!(r1, r2);
        }

        /// Absolute HTTPS URLs are never modified
        #[test]
        fn prop_https_unchanged(path in "[a-zA-Z0-9./_-]*") {
            let url = format!("https://example.com/{}", path);
            let config = regular_config();
            prop_assert_eq!(transform_link(&url, &config), url);
        }

        /// Absolute HTTP URLs are never modified
        #[test]
        fn prop_http_unchanged(path in "[a-zA-Z0-9./_-]*") {
            let url = format!("http://example.com/{}", path);
            let config = regular_config();
            prop_assert_eq!(transform_link(&url, &config), url);
        }

        /// Protocol-relative URLs are never modified
        #[test]
        fn prop_protocol_relative_unchanged(path in "[a-zA-Z0-9./_-]*") {
            let url = format!("//cdn.example.com/{}", path);
            let config = regular_config();
            prop_assert_eq!(transform_link(&url, &config), url);
        }

        /// Root-relative URLs are never modified
        #[test]
        fn prop_root_relative_unchanged(path in "/[a-zA-Z0-9./_-]*") {
            let config = regular_config();
            prop_assert_eq!(transform_link(&path, &config), path);
        }

        /// Anchor-only links are never modified
        #[test]
        fn prop_anchor_only_unchanged(anchor in "#[a-zA-Z0-9_-]*") {
            let config = regular_config();
            prop_assert_eq!(transform_link(&anchor, &config), anchor);
        }

        /// Empty links are never modified
        #[test]
        fn prop_empty_unchanged(_dummy in 0..1i32) {
            let config = regular_config();
            prop_assert_eq!(transform_link("", &config), "");
        }

        /// Regular markdown links always get ../ prepended
        #[test]
        fn prop_regular_md_gets_parent(name in "[a-zA-Z][a-zA-Z0-9_-]{0,20}") {
            let url = format!("{}.md", name);
            let config = regular_config();
            let result = transform_link(&url, &config);
            prop_assert!(result.starts_with("../"), "Expected ../ prefix: {}", result);
        }

        /// Index file markdown links don't get extra ../
        #[test]
        fn prop_index_md_no_extra_parent(name in "[a-zA-Z][a-zA-Z0-9_-]{0,20}") {
            let url = format!("{}.md", name);
            let config = index_config();
            let result = transform_link(&url, &config);
            prop_assert!(!result.starts_with("../"), "Should not have ../ prefix: {}", result);
        }

        /// Transformed markdown links end with /
        #[test]
        fn prop_md_ends_with_slash(name in "[a-zA-Z][a-zA-Z0-9_-]{0,20}") {
            let url = format!("{}.md", name);
            let config = regular_config();
            let result = transform_link(&url, &config);
            // Strip any anchor/query to check the path
            let base = result.split(&['?', '#'][..]).next().unwrap();
            prop_assert!(base.ends_with('/'), "Path should end with /: {}", base);
        }

        /// Anchors are preserved through transformation
        #[test]
        fn prop_anchor_preserved(
            name in "[a-zA-Z][a-zA-Z0-9_-]{0,10}",
            anchor in "[a-zA-Z][a-zA-Z0-9_-]{0,10}"
        ) {
            let url = format!("{}.md#{}", name, anchor);
            let config = regular_config();
            let result = transform_link(&url, &config);
            prop_assert!(result.contains(&format!("#{}", anchor)), "Anchor not preserved: {}", result);
        }

        /// Query strings are preserved through transformation
        #[test]
        fn prop_query_preserved(
            name in "[a-zA-Z][a-zA-Z0-9_-]{0,10}",
            query in "[a-zA-Z][a-zA-Z0-9_=-]{0,10}"
        ) {
            let url = format!("{}.md?{}", name, query);
            let config = regular_config();
            let result = transform_link(&url, &config);
            prop_assert!(result.contains(&format!("?{}", query)), "Query not preserved: {}", result);
        }
    }
}
