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

use std::sync::Arc;

/// Answers "does this absolute, percent-decoded site URL address a markdown
/// page?" for an authored link target that carries no markdown extension.
///
/// See [`LinkTransformConfig::markdown_page_probe`] for why this is a
/// caller-supplied predicate rather than a guess.
pub type MarkdownPageProbe = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Builds the probe used in server and build mode: ask the *path resolver* —
/// the very code a live HTTP request goes through — whether the URL resolves to
/// a markdown file.
///
/// Reusing the resolver rather than re-deriving candidate filenames is the
/// point: this cannot disagree with what the server actually serves, which is
/// the class of bug that produced the trailing-slash defect in the first place.
///
/// The cost is paid only for *extension-less relative* link targets — `.md`
/// links, root-relative URLs, anchors and external URLs never reach it — and is
/// a handful of `stat` calls.
pub fn filesystem_markdown_page_probe(
    resolver: crate::path_resolver::OwnedPathResolverConfig,
) -> MarkdownPageProbe {
    Arc::new(move |absolute_url: &str| {
        let request_path = crate::path_resolver::normalize_link_target(absolute_url);
        matches!(
            crate::path_resolver::resolve_request_path(&resolver.as_config(), &request_path),
            crate::path_resolver::ResolvedPath::MarkdownFile(_)
        )
    })
}

/// Configuration for link transformation.
#[derive(Clone)]
pub struct LinkTransformConfig {
    /// Markdown file extensions (e.g., ["md", "markdown"])
    pub markdown_extensions: Vec<String>,
    /// Index filename (e.g., "index.md")
    pub index_file: String,
    /// Whether the current file is an index file (affects ../ prefix)
    pub is_index_file: bool,
    /// Page depth for converting root-relative URLs to relative (build mode).
    /// None = leave root-relative URLs unchanged (server mode).
    pub url_depth: Option<usize>,
    /// The canonical URL of the page being rendered (e.g. `/docs/guide/`).
    ///
    /// Used together with `is_index_file` for Obsidian-style body-wikilink
    /// resolution (current folder first, else global). Empty when there is no
    /// current page context (e.g. CLI/QuickLook paths, which never resolve
    /// wikilinks globally).
    pub current_page_url: String,
    /// Decides whether an **extension-less** relative target names a markdown
    /// page (`[x](../folder/file)`) or a genuinely extension-less static file
    /// (`[license](../LICENSE)`).
    ///
    /// Markdown pages are served at directory-style URLs, so a link to one must
    /// end in `/`; a static file must not. Nothing in the href distinguishes
    /// the two and guessing either way corrupts the other, so the repository
    /// has to answer. `None` means "no repository context" — CLI/QuickLook
    /// rendering, link-grep and unit tests — and preserves the historical
    /// behaviour of treating an extension-less target as a static file.
    ///
    /// Called with the target's **absolute** site URL, already resolved against
    /// [`Self::current_page_url`], so a probe never reimplements relative-path
    /// arithmetic.
    pub markdown_page_probe: Option<MarkdownPageProbe>,
}

/// Hand-written because [`MarkdownPageProbe`] is a closure and cannot derive
/// `Debug`; only its presence is worth printing.
impl std::fmt::Debug for LinkTransformConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkTransformConfig")
            .field("markdown_extensions", &self.markdown_extensions)
            .field("index_file", &self.index_file)
            .field("is_index_file", &self.is_index_file)
            .field("url_depth", &self.url_depth)
            .field("current_page_url", &self.current_page_url)
            .field("markdown_page_probe", &self.markdown_page_probe.is_some())
            .finish()
    }
}

impl Default for LinkTransformConfig {
    fn default() -> Self {
        Self {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
        }
    }
}

/// Transform a relative link URL for the trailing-slash URL convention.
///
/// # Rules
///
/// 1. Anchor-only links (`#...`) → unchanged
/// 2. External URLs — anything with a scheme (`https:`, `mailto:`, `magnet:`,
///    `blob:`, …) or protocol-relative (`//host`) → unchanged, as decided by
///    [`crate::url_path::is_external_url`]
/// 3. Root-relative URLs (starts with `/`) → unchanged
/// 4. Relative markdown links → prepend `../` (if not index file), replace extension with `/`
/// 5. Relative **extension-less** targets → prepend `../` (if not index file),
///    and append `/` when [`LinkTransformConfig::markdown_page_probe`] says the
///    target is a markdown page
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
///     url_depth: None,
///     current_page_url: String::new(),
///     markdown_page_probe: None,
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

    // Anything that leaves the site is opaque to us: absolute and
    // protocol-relative URLs, but equally `mailto:`, `tel:`, `sms:`,
    // `magnet:`, `data:`, `javascript:` and `blob:` (what the Crepe editor
    // mints for a freshly pasted image before upload — a bogus `../` prefix
    // would break the in-editor preview). One predicate covers them all, so a
    // scheme can no longer be handled here but missed by link tracking.
    if crate::url_path::is_external_url(url) {
        return url.to_string();
    }

    // Root-relative URLs — convert to relative in build mode
    if url.starts_with('/') {
        return match config.url_depth {
            Some(depth) => make_relative_url(url, depth),
            None => url.to_string(),
        };
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

        // The link targets an index file only when the *final path segment* is
        // exactly the index stem — mirroring `repo::build_markdown_url_path`.
        // A plain `ends_with` also matched "site-index", "myindex" and
        // "subindex", rewriting them to "../site-/", "../my/" and "../sub/":
        // dead links, or worse, a live link to a different page.
        let is_index_target =
            base_path == index_stem || base_path.ends_with(&format!("/{}", index_stem));

        let final_path = if is_index_target {
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

    // Extension-less target, or a static file. `strip_markdown_extension` is
    // the *only* markdown detector above, so an extension-less link to a
    // markdown page (`[x](../folder/file)` — how Obsidian and zk write links)
    // lands here and used to be emitted verbatim, without the trailing slash
    // the URL convention requires. The page still resolved, but at a
    // non-canonical URL, and every relative href on *that* page then resolved
    // one directory too high. Ask the repository which kind of target this is;
    // when nobody can answer, keep the historical static-file behaviour.
    let prefix = if config.is_index_file {
        "../".repeat(parent_count)
    } else {
        "../".repeat(parent_count + 1)
    };

    if !remaining_path.ends_with('/') && resolves_to_markdown_page(path, config) {
        return format!("{}{}/{}", prefix, remaining_path, suffix);
    }

    format!("{}{}{}", prefix, remaining_path, suffix)
}

/// Asks [`LinkTransformConfig::markdown_page_probe`] whether `authored_path`
/// (relative, `./` already stripped, no anchor/query) names a markdown page.
///
/// The authored path is resolved against the page URL with *markdown*
/// semantics — `is_index_file` decides whether the page URL's last segment is a
/// file stem or a real directory — because that is what the author wrote it
/// relative to. The transform's own `../` compensation happens afterwards and
/// must not be applied twice.
fn resolves_to_markdown_page(authored_path: &str, config: &LinkTransformConfig) -> bool {
    let Some(probe) = &config.markdown_page_probe else {
        return false;
    };
    // Without a page URL there is nothing to resolve against, and guessing
    // would append slashes to root-level static files.
    if config.current_page_url.is_empty() {
        return false;
    }
    match crate::link_index::resolve_relative_url_checked(
        &config.current_page_url,
        authored_path,
        config.is_index_file,
    ) {
        // A target outside the repository is nobody's markdown page; leaving it
        // untouched keeps the defect visible to the link validators.
        None => false,
        Some(absolute) => probe(&absolute),
    }
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

/// Convert an absolute URL path to a relative URL from the given depth.
///
/// Examples (from depth 2):
/// - "/" → "../../"
/// - "/docs/" → "../../docs/"
/// - "/docs/guide/" → "../../docs/guide/"
pub fn make_relative_url(absolute_url: &str, depth: usize) -> String {
    let target = absolute_url.trim_start_matches('/');
    if target.is_empty() {
        // Link to root
        if depth == 0 {
            "./".to_string()
        } else {
            "../".repeat(depth)
        }
    } else {
        // Go up to root, then down to target
        if depth == 0 {
            target.to_string()
        } else {
            format!("{}{}", "../".repeat(depth), target)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regular_config() -> LinkTransformConfig {
        LinkTransformConfig {
            markdown_extensions: vec!["md".to_string(), "markdown".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
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

    /// Regression (mirrors `repo::test_build_markdown_url_path_myindex_not_
    /// treated_as_index`): only a *whole* final segment equal to the index
    /// stem collapses to a directory. `ends_with` used to mangle these into
    /// `../site-/`, `../my/`, `../re/` and `../sub/` — the last of which can
    /// silently point at a real but different page.
    #[test]
    fn test_index_lookalike_stems_keep_their_own_url() {
        let cases = [
            ("site-index.md", "../site-index/"),
            ("myindex.md", "../myindex/"),
            ("reindex.md", "../reindex/"),
            ("subindex.md", "../subindex/"),
            ("docs/site-index.md", "../docs/site-index/"),
        ];

        for (input, expected) in cases {
            assert_eq!(transform_link(input, &regular_config()), expected);
        }

        // Index files themselves still collapse to the folder URL.
        assert_eq!(
            transform_link("docs/index.md", &regular_config()),
            "../docs/"
        );
        assert_eq!(transform_link("index.md", &regular_config()), "../");
    }

    #[test]
    fn test_index_lookalike_stems_from_index_page() {
        assert_eq!(transform_link("subindex.md", &index_config()), "subindex/");
        assert_eq!(transform_link("docs/index.md", &index_config()), "docs/");
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
    fn test_data_image_url_unchanged() {
        // A realistic data: image URL must pass through untouched (no ../).
        let url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ";
        assert_eq!(transform_link(url, &regular_config()), url);
        assert_eq!(transform_link(url, &index_config()), url);
    }

    #[test]
    fn test_blob_url_unchanged() {
        // The Crepe editor mints blob: URLs for pasted images before upload;
        // rewriting one to `../blob:...` would break the in-editor preview.
        let url = "blob:http://localhost:5220/550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(transform_link(url, &regular_config()), url);
        assert_eq!(transform_link(url, &index_config()), url);
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

    #[test]
    fn test_scheme_urls_unchanged() {
        // Regression: `magnet:` was rewritten to `../magnet:?…` and `ftps:`
        // to `../ftps:/…` because each scheme had to be enumerated by hand.
        for url in [
            "ftps://ftp.example.com/file.txt",
            "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9",
            "sms:+15555550123",
            "callto:+15555550123",
            "ssh://git@example.com/repo.git",
        ] {
            assert_eq!(transform_link(url, &regular_config()), url);
            assert_eq!(transform_link(url, &index_config()), url);
        }
    }

    #[test]
    fn test_colon_in_relative_path_is_still_transformed() {
        // A colon after a slash is not a scheme, so the link must still be
        // rewritten for the trailing-slash URL convention.
        assert_eq!(
            transform_link("docs/a:b.md", &regular_config()),
            "../docs/a:b/"
        );
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

    // =========================================================================
    // Root-relative URL transformation with url_depth (build mode)
    // =========================================================================

    fn build_config(depth: usize) -> LinkTransformConfig {
        LinkTransformConfig {
            url_depth: Some(depth),
            ..regular_config()
        }
    }

    #[test]
    fn test_root_relative_with_depth_0() {
        assert_eq!(
            transform_link("/videos/demo.mp4", &build_config(0)),
            "videos/demo.mp4"
        );
    }

    #[test]
    fn test_root_relative_with_depth_1() {
        assert_eq!(
            transform_link("/videos/demo.mp4", &build_config(1)),
            "../videos/demo.mp4"
        );
    }

    #[test]
    fn test_root_relative_with_depth_2() {
        assert_eq!(
            transform_link("/videos/demo.mp4", &build_config(2)),
            "../../videos/demo.mp4"
        );
    }

    #[test]
    fn test_root_relative_to_root_with_depth() {
        assert_eq!(transform_link("/", &build_config(0)), "./");
        assert_eq!(transform_link("/", &build_config(1)), "../");
        assert_eq!(transform_link("/", &build_config(2)), "../../");
    }

    #[test]
    fn test_root_relative_tag_link_with_depth() {
        assert_eq!(
            transform_link("/tags/rust/", &build_config(2)),
            "../../tags/rust/"
        );
    }

    #[test]
    fn test_root_relative_unchanged_without_depth() {
        // Server mode (url_depth: None) leaves root-relative unchanged
        assert_eq!(
            transform_link("/videos/demo.mp4", &regular_config()),
            "/videos/demo.mp4"
        );
        assert_eq!(
            transform_link("/tags/rust/", &regular_config()),
            "/tags/rust/"
        );
    }

    // =========================================================================
    // make_relative_url
    // =========================================================================

    #[test]
    fn test_make_relative_url_to_root() {
        assert_eq!(make_relative_url("/", 0), "./");
        assert_eq!(make_relative_url("/", 1), "../");
        assert_eq!(make_relative_url("/", 2), "../../");
    }

    #[test]
    fn test_make_relative_url_to_path() {
        assert_eq!(make_relative_url("/docs/", 0), "docs/");
        assert_eq!(make_relative_url("/docs/guide/", 0), "docs/guide/");
        assert_eq!(make_relative_url("/docs/", 1), "../docs/");
        assert_eq!(make_relative_url("/other/", 1), "../other/");
        assert_eq!(make_relative_url("/docs/", 2), "../../docs/");
        assert_eq!(make_relative_url("/docs/guide/", 2), "../../docs/guide/");
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
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
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

        /// Root-relative URLs are unchanged when url_depth is None (server mode)
        #[test]
        fn prop_root_relative_unchanged(path in "/[a-zA-Z0-9./_-]*") {
            let config = regular_config();
            prop_assert_eq!(transform_link(&path, &config), path);
        }

        /// Root-relative URLs are relativized when url_depth is Some (build mode)
        #[test]
        fn prop_root_relative_relativized(
            path in "[a-zA-Z][a-zA-Z0-9/_-]{0,20}",
            depth in 0usize..5
        ) {
            let url = format!("/{}", path);
            let mut config = regular_config();
            config.url_depth = Some(depth);
            let result = transform_link(&url, &config);
            // Result should NOT start with /
            prop_assert!(!result.starts_with('/'), "Should be relative: {}", result);
            // Result should contain the original path (without leading /)
            prop_assert!(result.ends_with(&path), "Should end with path {}: {}", path, result);
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

/// End-to-end link resolution: author an href, run the **real** transform, then
/// resolve the emitted href the way a browser would against the page's real
/// served URL, and assert the final absolute URL.
///
/// This is the layer whose absence let the reported bug ship. The unit tests
/// above are string-in/string-out with no page URL at all, so they cannot tell
/// `../other/` emitted from `/docs/guide/` (correct) from the same string
/// emitted from `/docs/` (one level too high), and nothing anywhere authored a
/// `../` href against a real repository. Every case here therefore states the
/// *destination*, which is the only thing a reader experiences.
#[cfg(test)]
mod browser_resolution_tests {
    use super::*;
    use crate::path_resolver::OwnedPathResolverConfig;
    use std::path::Path;
    use tempfile::TempDir;

    // ---------------------------------------------------------------------
    // RFC 3986 §5.2 relative resolution — a browser, in ~40 lines.
    // ---------------------------------------------------------------------

    /// RFC 3986 §5.2.4 `remove_dot_segments`, verbatim.
    ///
    /// Written out rather than approximated because the whole point of these
    /// tests is that `.`/`..` handling matches a browser exactly — including
    /// the clamp on an above-root `..`, which is precisely the behaviour that
    /// let a defective link look like a working one.
    fn remove_dot_segments(path: &str) -> String {
        let mut input = path.to_string();
        let mut output = String::new();

        fn pop_last_segment(output: &mut String) {
            match output.rfind('/') {
                Some(index) => output.truncate(index),
                None => output.clear(),
            }
        }

        while !input.is_empty() {
            if let Some(rest) = input.strip_prefix("../") {
                input = rest.to_string();
            } else if let Some(rest) = input.strip_prefix("./") {
                input = rest.to_string();
            } else if let Some(rest) = input.strip_prefix("/./") {
                input = format!("/{rest}");
            } else if input == "/." {
                input = "/".to_string();
            } else if let Some(rest) = input.strip_prefix("/../") {
                input = format!("/{rest}");
                pop_last_segment(&mut output);
            } else if input == "/.." {
                input = "/".to_string();
                pop_last_segment(&mut output);
            } else if input == "." || input == ".." {
                input.clear();
            } else {
                // Move the first path segment (with its leading "/") to output.
                let end = if let Some(rest) = input.strip_prefix('/') {
                    rest.find('/').map(|i| i + 1).unwrap_or(input.len())
                } else {
                    input.find('/').unwrap_or(input.len())
                };
                output.push_str(&input[..end]);
                input = input[end..].to_string();
            }
        }

        output
    }

    /// RFC 3986 §5.2.3 `merge`, for a base that has an authority (a site URL
    /// always does): everything up to and including the base's last `/`.
    fn merge(base_path: &str, reference_path: &str) -> String {
        match base_path.rfind('/') {
            Some(index) => format!("{}{}", &base_path[..=index], reference_path),
            None => format!("/{reference_path}"),
        }
    }

    /// Resolve `reference` against the absolute `base` URL path, as a browser
    /// does when the user clicks a link on the page served at `base`.
    fn resolve_in_browser(base: &str, reference: &str) -> String {
        let (without_fragment, fragment) = match reference.split_once('#') {
            Some((head, tail)) => (head, format!("#{tail}")),
            None => (reference, String::new()),
        };
        let (ref_path, query) = match without_fragment.split_once('?') {
            Some((head, tail)) => (head, format!("?{tail}")),
            None => (without_fragment, String::new()),
        };

        let target = if ref_path.is_empty() {
            base.to_string()
        } else if ref_path.starts_with('/') {
            remove_dot_segments(ref_path)
        } else {
            remove_dot_segments(&merge(base, ref_path))
        };

        format!("{target}{query}{fragment}")
    }

    // ---------------------------------------------------------------------
    // The repository every case is authored against.
    // ---------------------------------------------------------------------

    /// A layout with a page and an extension-less static file at each depth
    /// that matters, plus two siblings so the cascade case has somewhere to go.
    fn fixture() -> TempDir {
        let dir = TempDir::new().expect("temp repo");
        let root = dir.path();
        for folder in ["docs", "folder", "a/b", "static"] {
            std::fs::create_dir_all(root.join(folder)).expect("create dir");
        }
        for page in [
            "index.md",
            "root.md",
            "docs/index.md",
            "docs/guide.md",
            "docs/other.md",
            "folder/file.md",
            "folder/sibling.md",
            "a/b/c.md",
            "a/b/d.md",
        ] {
            std::fs::write(root.join(page), "# page").expect("write page");
        }
        // Genuinely extension-less *files*: appending a trailing slash to a
        // link pointing at one of these would corrupt it.
        for asset in ["LICENSE", "docs/Makefile", "folder/Dockerfile"] {
            std::fs::write(root.join(asset), "text").expect("write asset");
        }
        std::fs::write(root.join("docs/photo.png"), b"\x89PNG").expect("write image");
        dir
    }

    fn transform_config(root: &Path, source_rel: &str) -> LinkTransformConfig {
        let source = root.join(source_rel);
        let is_index_file = source
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f == "index.md");
        LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file,
            url_depth: None,
            current_page_url: crate::repo::build_markdown_url_path(&source, root, "index.md"),
            markdown_page_probe: Some(filesystem_markdown_page_probe(OwnedPathResolverConfig {
                base_dir: root.to_path_buf(),
                canonical_base_dir: root.canonicalize().ok(),
                static_folder: "static".to_string(),
                markdown_extensions: vec!["md".to_string()],
                index_file: "index.md".to_string(),
                tag_sources: Vec::new(),
            })),
        }
    }

    /// The page URL `source_rel` is served at.
    fn page_url(root: &Path, source_rel: &str) -> String {
        crate::repo::build_markdown_url_path(&root.join(source_rel), root, "index.md")
    }

    /// Author `href` in `source_rel`, transform it, and follow it like a
    /// browser. Returns `(emitted href, final absolute URL)`.
    fn follow(root: &Path, source_rel: &str, href: &str) -> (String, String) {
        let config = transform_config(root, source_rel);
        let emitted = transform_link(href, &config);
        let landed = resolve_in_browser(&config.current_page_url, &emitted);
        (emitted, landed)
    }

    fn assert_lands(root: &Path, source_rel: &str, href: &str, expected: &str) {
        let (emitted, landed) = follow(root, source_rel, href);
        assert_eq!(
            landed, expected,
            "[{source_rel}] `{href}` emitted `{emitted}` and landed on `{landed}`, \
             expected `{expected}`"
        );
    }

    // ---------------------------------------------------------------------
    // Sanity: the browser model itself.
    // ---------------------------------------------------------------------

    #[test]
    fn browser_model_matches_rfc_3986_examples() {
        // The normal examples from RFC 3986 §5.4.1, path-only.
        assert_eq!(resolve_in_browser("/b/c/d;p", "g"), "/b/c/g");
        assert_eq!(resolve_in_browser("/b/c/d;p", "./g"), "/b/c/g");
        assert_eq!(resolve_in_browser("/b/c/d;p", "g/"), "/b/c/g/");
        assert_eq!(resolve_in_browser("/b/c/d;p", "/g"), "/g");
        assert_eq!(resolve_in_browser("/b/c/d;p", "../g"), "/b/g");
        assert_eq!(resolve_in_browser("/b/c/d;p", "../../g"), "/g");
        // §5.4.2: excess `..` is discarded, not an error. This clamp is exactly
        // why an above-root link looks like it works.
        assert_eq!(resolve_in_browser("/b/c/d;p", "../../../g"), "/g");
        assert_eq!(resolve_in_browser("/b/c/d;p", "../../../../g"), "/g");
        // Query and fragment ride along untouched.
        assert_eq!(resolve_in_browser("/b/c/d;p", "g?y"), "/b/c/g?y");
        assert_eq!(resolve_in_browser("/b/c/d;p", "g#s"), "/b/c/g#s");
        assert_eq!(resolve_in_browser("/b/c/d;p", "g?y#s"), "/b/c/g?y#s");
        // A directory-style base keeps all of its segments.
        assert_eq!(
            resolve_in_browser("/docs/guide/", "../other/"),
            "/docs/other/"
        );
    }

    // ---------------------------------------------------------------------
    // The table: every depth × every link form × both page kinds.
    // ---------------------------------------------------------------------

    #[test]
    fn every_link_form_lands_on_its_canonical_url() {
        let dir = fixture();
        let root = dir.path();

        // (source file, authored href, final absolute URL)
        let cases: &[(&str, &str, &str)] = &[
            // --- depth 0, non-index page (/root/) ---
            ("root.md", "docs/guide.md", "/docs/guide/"),
            ("root.md", "docs/guide", "/docs/guide/"),
            ("root.md", "docs/guide/", "/docs/guide/"),
            ("root.md", "./docs/guide.md", "/docs/guide/"),
            ("root.md", "docs/index.md", "/docs/"),
            ("root.md", "docs/guide.md#anchor", "/docs/guide/#anchor"),
            ("root.md", "docs/guide.md?q=1", "/docs/guide/?q=1"),
            ("root.md", "LICENSE", "/LICENSE"),
            ("root.md", "docs/photo.png", "/docs/photo.png"),
            // --- depth 0, index page (/) ---
            ("index.md", "docs/guide.md", "/docs/guide/"),
            ("index.md", "docs/guide", "/docs/guide/"),
            ("index.md", "root.md", "/root/"),
            ("index.md", "LICENSE", "/LICENSE"),
            // --- depth 1, non-index page (/docs/guide/) ---
            ("docs/guide.md", "other.md", "/docs/other/"),
            ("docs/guide.md", "other", "/docs/other/"),
            ("docs/guide.md", "other/", "/docs/other/"),
            ("docs/guide.md", "./other.md", "/docs/other/"),
            ("docs/guide.md", "index.md", "/docs/"),
            ("docs/guide.md", "../root.md", "/root/"),
            ("docs/guide.md", "../root", "/root/"),
            // The exact reported repro: an extension-less `../` link to a
            // markdown page two folders away.
            ("docs/guide.md", "../folder/file.md", "/folder/file/"),
            ("docs/guide.md", "../folder/file", "/folder/file/"),
            ("docs/guide.md", "../index.md", "/"),
            ("docs/guide.md", "other.md#anchor", "/docs/other/#anchor"),
            ("docs/guide.md", "other.md?q=1", "/docs/other/?q=1"),
            ("docs/guide.md", "other#anchor", "/docs/other/#anchor"),
            // Extension-less STATIC targets must NOT gain a slash.
            ("docs/guide.md", "Makefile", "/docs/Makefile"),
            ("docs/guide.md", "../LICENSE", "/LICENSE"),
            (
                "docs/guide.md",
                "../folder/Dockerfile",
                "/folder/Dockerfile",
            ),
            ("docs/guide.md", "photo.png", "/docs/photo.png"),
            // --- depth 1, index page at the same depth (/docs/) ---
            ("docs/index.md", "guide.md", "/docs/guide/"),
            ("docs/index.md", "guide", "/docs/guide/"),
            ("docs/index.md", "guide/", "/docs/guide/"),
            ("docs/index.md", "./guide.md", "/docs/guide/"),
            ("docs/index.md", "../root.md", "/root/"),
            ("docs/index.md", "../root", "/root/"),
            ("docs/index.md", "../folder/file", "/folder/file/"),
            ("docs/index.md", "Makefile", "/docs/Makefile"),
            ("docs/index.md", "../LICENSE", "/LICENSE"),
            ("docs/index.md", "guide.md#anchor", "/docs/guide/#anchor"),
            // --- depth 3, non-index page (/a/b/c/) ---
            ("a/b/c.md", "d.md", "/a/b/d/"),
            ("a/b/c.md", "d", "/a/b/d/"),
            ("a/b/c.md", "./d.md", "/a/b/d/"),
            ("a/b/c.md", "../../root.md", "/root/"),
            ("a/b/c.md", "../../root", "/root/"),
            ("a/b/c.md", "../../docs/guide.md", "/docs/guide/"),
            ("a/b/c.md", "../../docs/guide", "/docs/guide/"),
            ("a/b/c.md", "../../LICENSE", "/LICENSE"),
            ("a/b/c.md", "../../index.md", "/"),
            ("a/b/c.md", "d.md?q=1#anchor", "/a/b/d/?q=1#anchor"),
        ];

        for (source, href, expected) in cases {
            assert_lands(root, source, href, expected);
        }
    }

    /// The regression, stated as the thing a reader actually experiences: the
    /// extension-less form must land in the same place as the `.md` form.
    #[test]
    fn extensionless_markdown_link_lands_where_the_dot_md_form_does() {
        let dir = fixture();
        let root = dir.path();

        let (with_ext, landed_with_ext) = follow(root, "docs/guide.md", "../folder/file.md");
        let (without_ext, landed_without_ext) = follow(root, "docs/guide.md", "../folder/file");

        assert_eq!(with_ext, "../../folder/file/");
        assert_eq!(
            without_ext, "../../folder/file/",
            "an extension-less markdown target must get the trailing slash its \
             canonical URL has"
        );
        assert_eq!(landed_with_ext, "/folder/file/");
        assert_eq!(landed_without_ext, "/folder/file/");
    }

    /// The cascade — the test that would have caught the reported bug. Follow
    /// the link, then follow a link *on the page it lands on*. A missing
    /// trailing slash is invisible on the first hop (the page still serves) and
    /// only breaks the second.
    #[test]
    fn links_on_the_landed_page_still_resolve() {
        let dir = fixture();
        let root = dir.path();

        for authored in ["../folder/file.md", "../folder/file", "../folder/file/"] {
            let (_, landed) = follow(root, "docs/guide.md", authored);
            assert_eq!(
                landed,
                page_url(root, "folder/file.md"),
                "`{authored}` must land on the target's canonical URL"
            );

            // Now stand on that page and click its own sibling link.
            let (emitted, second) = follow(root, "folder/file.md", "sibling.md");
            assert_eq!(
                second, "/folder/sibling/",
                "after following `{authored}`, `sibling.md` (emitted `{emitted}`) \
                 must still reach /folder/sibling/ — this is the hop the \
                 trailing-slash defect breaks"
            );

            // And the second hop is only correct because the first one landed
            // canonically: resolving the *same* emitted href against the
            // non-canonical URL goes somewhere else entirely.
            let non_canonical = landed.trim_end_matches('/');
            assert_ne!(
                resolve_in_browser(non_canonical, &emitted),
                "/folder/sibling/",
                "sanity: a slashless landing URL must break the next hop, or \
                 this test proves nothing"
            );
        }
    }

    /// Without a probe (CLI/QuickLook, link-grep, unit tests) the historical
    /// behaviour is preserved exactly: an extension-less target is treated as a
    /// static file. Appending a slash on a guess would corrupt `LICENSE`.
    #[test]
    fn extensionless_target_is_unchanged_without_a_probe() {
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: "/docs/guide/".to_string(),
            markdown_page_probe: None,
        };
        assert_eq!(
            transform_link("../folder/file", &config),
            "../../folder/file"
        );
        assert_eq!(transform_link("../LICENSE", &config), "../../LICENSE");
    }

    /// A probe is also ignored when there is no page to resolve against, so a
    /// context without a current page URL can never invent a slash.
    #[test]
    fn probe_is_not_consulted_without_a_current_page_url() {
        let dir = fixture();
        let root = dir.path();
        let mut config = transform_config(root, "docs/guide.md");
        config.current_page_url = String::new();
        assert_eq!(
            transform_link("../folder/file", &config),
            "../../folder/file"
        );
    }

    /// A link that climbs above the repository root is left exactly as authored
    /// (minus the usual `../` compensation): there is no markdown page out
    /// there to justify a trailing slash, and leaving it alone keeps the defect
    /// visible to the link validators instead of dressing it up as a page link.
    #[test]
    fn above_root_traversal_is_not_dressed_up_as_a_page() {
        let dir = fixture();
        let root = dir.path();

        let (emitted, landed) = follow(root, "docs/guide.md", "../../escape/target");
        assert_eq!(emitted, "../../../escape/target");
        assert!(
            !emitted.ends_with('/'),
            "an above-root target must not be given a page's trailing slash: {emitted}"
        );
        // The browser clamps the excess `..`, which is exactly why this needs a
        // diagnostic rather than a silent repair — see
        // `page_errors::validate_rendered_links`.
        assert_eq!(landed, "/escape/target");
    }

    /// Build mode relativizes root-relative URLs by page depth; that arithmetic
    /// must land on the same absolute URL a server-mode reader gets.
    #[test]
    fn build_mode_root_relative_links_land_on_the_same_url() {
        let dir = fixture();
        let root = dir.path();

        for source in [
            "root.md",
            "index.md",
            "docs/guide.md",
            "docs/index.md",
            "a/b/c.md",
        ] {
            let base = page_url(root, source);
            let depth = base
                .trim_matches('/')
                .split('/')
                .filter(|s| !s.is_empty())
                .count();
            let mut config = transform_config(root, source);
            config.url_depth = Some(depth);

            for target in ["/docs/guide/", "/folder/file/", "/"] {
                let emitted = transform_link(target, &config);
                assert_eq!(
                    resolve_in_browser(&base, &emitted),
                    target,
                    "[{source}] root-relative `{target}` emitted `{emitted}`"
                );
            }
        }
    }
}
