//! Path resolution logic for the mbr server.
//!
//! This module contains pure functions for determining what resource to serve
//! based on a URL path. By keeping this logic separate from I/O, it becomes
//! easily testable.

use std::path::{Path, PathBuf};

/// Safely joins a base directory with a request path, preventing path traversal.
///
/// Returns `None` if the resulting path would escape the base directory.
/// The path is canonicalized to resolve symlinks and `..` components.
///
/// # Security
///
/// This function guards against path traversal attacks by:
/// 1. Canonicalizing both the base directory and the joined path
/// 2. Verifying the resolved path starts with the base directory
fn safe_join(
    base_dir: &Path,
    canonical_base_dir: Option<&Path>,
    request_path: &str,
) -> Option<PathBuf> {
    // Use pre-computed canonical base if available, otherwise canonicalize per-call
    let owned_canonical;
    let canonical_base = match canonical_base_dir {
        Some(cached) => cached,
        None => {
            owned_canonical = base_dir.canonicalize().ok()?;
            &owned_canonical
        }
    };

    // Build candidate from canonical_base (not base_dir) to ensure all path
    // construction happens in canonical space. This prevents subtle issues
    // if base_dir itself contains symlinks.
    let candidate = canonical_base.join(request_path);

    // Try to canonicalize - this resolves ".." and symlinks
    // If canonicalize fails (path doesn't exist), try the parent
    if let Ok(canonical) = candidate.canonicalize()
        && canonical.starts_with(canonical_base)
    {
        return Some(canonical);
    }

    // For paths that don't exist yet (checking markdown extensions),
    // we need to verify the parent is safe and construct the full path
    if let Some(parent) = candidate.parent()
        && let Ok(canonical_parent) = parent.canonicalize()
        && canonical_parent.starts_with(canonical_base)
        && let Some(filename) = candidate.file_name()
    {
        return Some(canonical_parent.join(filename));
    }

    None
}

/// The result of resolving a URL path to a resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedPath {
    /// Serve a static file directly (non-markdown)
    StaticFile(PathBuf),
    /// Render a markdown file
    MarkdownFile(PathBuf),
    /// Generate a directory listing
    DirectoryListing(PathBuf),
    /// Render a tag page listing all pages with this tag
    TagPage {
        /// The tag source (e.g., "tags", "performers", "taxonomy.tags")
        source: String,
        /// The normalized tag value (e.g., "rust", "joshua_jay")
        value: String,
    },
    /// Render a tag source index listing all tags from this source
    TagSourceIndex {
        /// The tag source (e.g., "tags", "performers")
        source: String,
    },
    /// Resource not found
    NotFound,
    /// Redirect to canonical URL (e.g., /x/index/ → /x/)
    Redirect(String),
}

/// Configuration for path resolution.
#[derive(Debug, Clone)]
pub struct PathResolverConfig<'a> {
    pub base_dir: &'a Path,
    /// Pre-computed canonical base directory. Avoids calling `canonicalize()` on every request.
    /// If `None`, `safe_join` will canonicalize on each call (backward-compatible fallback).
    pub canonical_base_dir: Option<&'a Path>,
    pub static_folder: &'a str,
    pub markdown_extensions: &'a [String],
    pub index_file: &'a str,
    /// Valid tag source URL identifiers (e.g., ["tags", "performers", "taxonomy.tags"])
    /// Used to detect tag page URLs like /tags/rust/
    pub tag_sources: &'a [String],
}

/// Resolves a URL path to determine what resource should be served.
///
/// This is a pure function that performs filesystem checks but no I/O operations
/// like reading file contents. It determines the type of resource to serve.
///
/// # Resolution Order
///
/// 1. Direct file match in base_dir → StaticFile
/// 2. Directory with configured index file (e.g., index.md) → MarkdownFile
/// 3. Path with trailing slash matching a markdown file (e.g., /foo/ → foo.md) → MarkdownFile
/// 4. File in static folder → StaticFile
/// 5. Directory with index.{markdown_ext} → MarkdownFile
/// 6. Directory without index → DirectoryListing
/// 7. Tag source index (e.g., /tags/) → TagSourceIndex (if source matches config)
/// 8. Tag page (e.g., /tags/rust/) → TagPage (if source matches config)
/// 9. Nothing matches → NotFound
///
/// Note: Filesystem paths (steps 1-6) always take precedence over tag URLs (steps 7-8).
/// If a file or directory named "tags" exists, it will be served instead of the tag index.
///
/// # Security
///
/// Path traversal attacks (e.g., `../../../etc/passwd`) are blocked by validating
/// that all resolved paths remain within the configured base directory.
pub fn resolve_request_path(config: &PathResolverConfig, request_path: &str) -> ResolvedPath {
    // Use safe_join to prevent path traversal attacks
    // If the path would escape base_dir, skip to tag resolution or NotFound
    if let Some(candidate_path) =
        safe_join(config.base_dir, config.canonical_base_dir, request_path)
    {
        // 1. Direct file match
        if candidate_path.is_file() {
            return if is_markdown_file(&candidate_path, config.markdown_extensions) {
                ResolvedPath::MarkdownFile(candidate_path)
            } else {
                ResolvedPath::StaticFile(candidate_path)
            };
        }

        // 2. Directory with configured index file
        if candidate_path.is_dir() {
            let index_path = candidate_path.join(config.index_file);
            if index_path.is_file() {
                return ResolvedPath::MarkdownFile(index_path);
            }
        }

        // 3. Try markdown extensions on base path (for /foo/ → foo.md)
        let candidate_base = strip_trailing_separator(&candidate_path);

        // 3a. Check for non-canonical index URL (e.g., /x/index/ should redirect to /x/)
        // This must come before step 3 to catch URLs like /docs/index/ before they resolve
        let index_stem = Path::new(config.index_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("index");

        if let Some(file_name) = candidate_base.file_name().and_then(|f| f.to_str())
            && file_name == index_stem
        {
            // Check if parent directory contains the actual index file
            if let Some(parent) = candidate_base.parent() {
                let index_path = parent.join(config.index_file);
                if index_path.is_file() {
                    // Build canonical URL: /x/index/ → /x/
                    // Use pre-computed canonical base if available
                    let owned_base;
                    let canonical_base = match config.canonical_base_dir {
                        Some(cached) => Some(cached),
                        None => {
                            owned_base = config.base_dir.canonicalize().ok();
                            owned_base.as_deref()
                        }
                    };
                    let canonical = canonical_base
                        .and_then(|base| pathdiff::diff_paths(parent, base))
                        .map(|p| {
                            let s = p.to_string_lossy();
                            if s.is_empty() {
                                "/".to_string()
                            } else {
                                format!("/{}/", s)
                            }
                        })
                        .unwrap_or_else(|| "/".to_string());
                    return ResolvedPath::Redirect(canonical);
                }
            }
        }

        if let Some(md_path) = find_markdown_file(&candidate_base, config.markdown_extensions) {
            return ResolvedPath::MarkdownFile(md_path);
        }

        // 4. Check static folder (has its own path traversal protection)
        if let Some(static_path) = find_in_static_folder(config, request_path) {
            return ResolvedPath::StaticFile(static_path);
        }

        // 5. Directory with index.{markdown_ext}
        if candidate_base.is_dir() {
            let index_base = candidate_base.join("index");
            if let Some(md_path) = find_markdown_file(&index_base, config.markdown_extensions) {
                return ResolvedPath::MarkdownFile(md_path);
            }

            // 6. Directory without index → listing
            return ResolvedPath::DirectoryListing(candidate_base);
        }
    }

    // 4b. Static folder check - ALSO check here for paths not in base_dir
    // This handles the case where the path doesn't exist in base_dir but exists in static folder
    // (e.g., /images/blog/photo.png where images/ only exists under static/)
    if let Some(static_path) = find_in_static_folder(config, request_path) {
        return ResolvedPath::StaticFile(static_path);
    }

    // 7-8. Check for tag URLs (only if nothing matched in filesystem)
    // This is also reached if safe_join returned None (path traversal blocked)
    if let Some(tag_result) = try_resolve_tag_url(request_path, config.tag_sources) {
        return tag_result;
    }

    // 9. Nothing found
    ResolvedPath::NotFound
}

/// Checks if a path is a markdown file based on configured extensions.
fn is_markdown_file(path: &Path, extensions: &[String]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| extensions.iter().any(|md_ext| md_ext == ext))
        .unwrap_or(false)
}

/// Strips trailing path separator from a path.
fn strip_trailing_separator(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    let trimmed = s.trim_end_matches(std::path::MAIN_SEPARATOR);
    PathBuf::from(trimmed)
}

/// Finds a markdown file by trying each configured extension.
fn find_markdown_file(base_path: &Path, extensions: &[String]) -> Option<PathBuf> {
    extensions
        .iter()
        .map(|ext| {
            let mut path = base_path.to_path_buf();
            path.set_extension(ext);
            path
        })
        .find(|path| path.is_file())
}

/// Finds a file in the static folder.
///
/// # Security
///
/// This function guards against path traversal attacks by canonicalizing
/// the resolved path and verifying it remains within the static directory.
fn find_in_static_folder(config: &PathResolverConfig, request_path: &str) -> Option<PathBuf> {
    // Build static_dir from canonical base if available, otherwise canonicalize
    let static_dir = match config.canonical_base_dir {
        Some(cached) => {
            let dir = cached.join(config.static_folder);
            // Still need to verify it exists (canonicalize checks this)
            dir.canonicalize().ok()?
        }
        None => config
            .base_dir
            .join(config.static_folder)
            .canonicalize()
            .ok()?,
    };
    let candidate = static_dir.join(request_path);

    // Canonicalize to resolve any ".." or symlinks, then verify containment
    let canonical = candidate.canonicalize().ok()?;
    if canonical.starts_with(&static_dir) && canonical.is_file() {
        Some(canonical)
    } else {
        None
    }
}

/// Attempts to resolve a URL path as a tag URL.
///
/// Matches patterns like:
/// - `{source}/` → TagSourceIndex (e.g., "tags/" → list all tags)
/// - `{source}/{value}/` → TagPage (e.g., "tags/rust/" → pages tagged "rust")
///
/// The source must match one of the configured tag sources (case-insensitive).
/// Returns `None` if the path doesn't match a tag URL pattern.
fn try_resolve_tag_url(request_path: &str, tag_sources: &[String]) -> Option<ResolvedPath> {
    // Skip if no tag sources configured
    if tag_sources.is_empty() {
        return None;
    }

    // Normalize path: strip leading and trailing slashes
    let path = request_path.trim_matches('/');

    // Empty path is not a tag URL
    if path.is_empty() {
        return None;
    }

    // Split path into segments
    let segments: Vec<&str> = path.split('/').collect();

    match segments.len() {
        // Single segment: might be a tag source index (e.g., "tags")
        1 => {
            let source = segments[0].to_lowercase();
            if tag_sources.iter().any(|s| s.to_lowercase() == source) {
                Some(ResolvedPath::TagSourceIndex { source })
            } else {
                None
            }
        }
        // Two segments: might be a tag page (e.g., "tags/rust")
        2 => {
            let source = segments[0].to_lowercase();
            let value = segments[1].to_lowercase();

            // Don't match empty values
            if value.is_empty() {
                return None;
            }

            if tag_sources.iter().any(|s| s.to_lowercase() == source) {
                Some(ResolvedPath::TagPage { source, value })
            } else {
                None
            }
        }
        // More than 2 segments: not a tag URL
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Test fixture that owns the extensions and tag_sources vectors
    struct TestFixture {
        dir: TempDir,
        canonical: PathBuf,
        extensions: Vec<String>,
        tag_sources: Vec<String>,
    }

    impl TestFixture {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            fs::create_dir(dir.path().join("static")).unwrap();
            let canonical = dir.path().canonicalize().unwrap();
            Self {
                dir,
                canonical,
                extensions: vec![String::from("md")],
                tag_sources: vec![],
            }
        }

        fn with_extensions(extensions: Vec<String>) -> Self {
            let dir = TempDir::new().unwrap();
            fs::create_dir(dir.path().join("static")).unwrap();
            let canonical = dir.path().canonicalize().unwrap();
            Self {
                dir,
                canonical,
                extensions,
                tag_sources: vec![],
            }
        }

        fn with_tag_sources(tag_sources: Vec<String>) -> Self {
            let dir = TempDir::new().unwrap();
            fs::create_dir(dir.path().join("static")).unwrap();
            let canonical = dir.path().canonicalize().unwrap();
            Self {
                dir,
                canonical,
                extensions: vec![String::from("md")],
                tag_sources,
            }
        }

        fn config(&self) -> PathResolverConfig<'_> {
            PathResolverConfig {
                base_dir: self.dir.path(),
                canonical_base_dir: Some(&self.canonical),
                static_folder: "static",
                markdown_extensions: &self.extensions,
                index_file: "index.md",
                tag_sources: &self.tag_sources,
            }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        /// Returns the canonicalized base path (resolves symlinks like /var -> /private/var on macOS)
        fn canonical_path(&self) -> PathBuf {
            self.dir.path().canonicalize().unwrap()
        }
    }

    #[test]
    fn test_direct_markdown_file() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("readme.md"), "# Test").unwrap();

        let result = resolve_request_path(&fixture.config(), "readme.md");

        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("readme.md"))
        );
    }

    #[test]
    fn test_direct_static_file() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("image.png"), "fake image").unwrap();

        let result = resolve_request_path(&fixture.config(), "image.png");

        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::StaticFile(fixture.canonical_path().join("image.png"))
        );
    }

    #[test]
    fn test_directory_with_index() {
        let fixture = TestFixture::new();
        let subdir = fixture.path().join("docs");
        fs::create_dir(&subdir).unwrap();
        fs::write(subdir.join("index.md"), "# Docs").unwrap();

        let result = resolve_request_path(&fixture.config(), "docs");

        // safe_join returns canonicalized paths
        let expected = fixture.canonical_path().join("docs/index.md");
        assert_eq!(result, ResolvedPath::MarkdownFile(expected));
    }

    #[test]
    fn test_trailing_slash_to_markdown() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("about.md"), "# About").unwrap();

        let result = resolve_request_path(&fixture.config(), "about/");

        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("about.md"))
        );
    }

    #[test]
    fn test_static_folder_file() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("static/style.css"), "body {}").unwrap();

        let result = resolve_request_path(&fixture.config(), "style.css");

        // The static file path is canonicalized
        let expected = fixture
            .path()
            .join("static/style.css")
            .canonicalize()
            .unwrap();
        assert_eq!(result, ResolvedPath::StaticFile(expected));
    }

    #[test]
    fn test_static_folder_nested_path() {
        let fixture = TestFixture::new();
        fs::create_dir_all(fixture.path().join("static/images/blog")).unwrap();
        fs::write(
            fixture.path().join("static/images/blog/photo.png"),
            "fake image",
        )
        .unwrap();

        // Request for /images/blog/photo.png should find static/images/blog/photo.png
        let result = resolve_request_path(&fixture.config(), "images/blog/photo.png");

        let expected = fixture
            .path()
            .join("static/images/blog/photo.png")
            .canonicalize()
            .unwrap();
        assert_eq!(result, ResolvedPath::StaticFile(expected));
    }

    #[test]
    fn test_directory_listing() {
        let fixture = TestFixture::new();
        let subdir = fixture.path().join("posts");
        fs::create_dir(&subdir).unwrap();
        // No index file

        let result = resolve_request_path(&fixture.config(), "posts/");

        // safe_join returns canonicalized paths
        let expected = fixture.canonical_path().join("posts");
        assert_eq!(result, ResolvedPath::DirectoryListing(expected));
    }

    #[test]
    fn test_not_found() {
        let fixture = TestFixture::new();

        let result = resolve_request_path(&fixture.config(), "nonexistent");

        assert_eq!(result, ResolvedPath::NotFound);
    }

    #[test]
    fn test_nested_directory_with_index() {
        let fixture = TestFixture::new();
        let nested = fixture.path().join("blog/2024");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("index.md"), "# Blog 2024").unwrap();

        let result = resolve_request_path(&fixture.config(), "blog/2024");

        // safe_join returns canonicalized paths
        let expected = fixture.canonical_path().join("blog/2024/index.md");
        assert_eq!(result, ResolvedPath::MarkdownFile(expected));
    }

    #[test]
    fn test_multiple_markdown_extensions() {
        let fixture =
            TestFixture::with_extensions(vec![String::from("md"), String::from("markdown")]);
        fs::write(fixture.path().join("notes.markdown"), "# Notes").unwrap();

        let result = resolve_request_path(&fixture.config(), "notes/");

        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("notes.markdown"))
        );
    }

    #[test]
    fn test_prefers_first_extension() {
        let fixture =
            TestFixture::with_extensions(vec![String::from("md"), String::from("markdown")]);
        // Create both .md and .markdown files
        fs::write(fixture.path().join("test.md"), "# MD").unwrap();
        fs::write(fixture.path().join("test.markdown"), "# Markdown").unwrap();

        let result = resolve_request_path(&fixture.config(), "test/");

        // Should prefer .md (first in list), safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("test.md"))
        );
    }

    #[test]
    fn test_root_path_empty_string() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("index.md"), "# Home").unwrap();

        let result = resolve_request_path(&fixture.config(), "");

        // Empty path resolves to base_dir, which is a directory with index.md
        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("index.md"))
        );
    }

    #[test]
    fn test_is_markdown_file() {
        let extensions = vec![String::from("md"), String::from("markdown")];

        assert!(is_markdown_file(Path::new("test.md"), &extensions));
        assert!(is_markdown_file(Path::new("test.markdown"), &extensions));
        assert!(!is_markdown_file(Path::new("test.txt"), &extensions));
        assert!(!is_markdown_file(Path::new("test"), &extensions));
    }

    #[test]
    fn test_strip_trailing_separator() {
        assert_eq!(
            strip_trailing_separator(Path::new("/foo/bar/")),
            PathBuf::from("/foo/bar")
        );
        assert_eq!(
            strip_trailing_separator(Path::new("/foo/bar")),
            PathBuf::from("/foo/bar")
        );
        assert_eq!(
            strip_trailing_separator(Path::new("relative/")),
            PathBuf::from("relative")
        );
    }

    // ==================== Tag URL Resolution Tests ====================

    #[test]
    fn test_tag_source_index() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);
        let result = resolve_request_path(&fixture.config(), "tags/");

        assert_eq!(
            result,
            ResolvedPath::TagSourceIndex {
                source: "tags".to_string()
            }
        );
    }

    #[test]
    fn test_tag_source_index_without_trailing_slash() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);
        let result = resolve_request_path(&fixture.config(), "tags");

        assert_eq!(
            result,
            ResolvedPath::TagSourceIndex {
                source: "tags".to_string()
            }
        );
    }

    #[test]
    fn test_tag_page() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);
        let result = resolve_request_path(&fixture.config(), "tags/rust/");

        assert_eq!(
            result,
            ResolvedPath::TagPage {
                source: "tags".to_string(),
                value: "rust".to_string()
            }
        );
    }

    #[test]
    fn test_tag_page_without_trailing_slash() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);
        let result = resolve_request_path(&fixture.config(), "tags/rust");

        assert_eq!(
            result,
            ResolvedPath::TagPage {
                source: "tags".to_string(),
                value: "rust".to_string()
            }
        );
    }

    #[test]
    fn test_tag_url_case_insensitive_source() {
        let fixture = TestFixture::with_tag_sources(vec!["Tags".to_string()]);

        // Uppercase in URL should match lowercase config
        let result = resolve_request_path(&fixture.config(), "TAGS/rust/");

        assert_eq!(
            result,
            ResolvedPath::TagPage {
                source: "tags".to_string(),
                value: "rust".to_string()
            }
        );
    }

    #[test]
    fn test_tag_url_unknown_source_not_matched() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);

        // "categories" is not a configured tag source
        let result = resolve_request_path(&fixture.config(), "categories/rust/");

        assert_eq!(result, ResolvedPath::NotFound);
    }

    #[test]
    fn test_tag_url_no_sources_configured() {
        let fixture = TestFixture::new(); // Empty tag_sources
        let result = resolve_request_path(&fixture.config(), "tags/rust/");

        assert_eq!(result, ResolvedPath::NotFound);
    }

    #[test]
    fn test_tag_url_multiple_sources() {
        let fixture = TestFixture::with_tag_sources(vec![
            "tags".to_string(),
            "performers".to_string(),
            "taxonomy.categories".to_string(),
        ]);

        // All sources should be recognized
        assert_eq!(
            resolve_request_path(&fixture.config(), "tags/rust/"),
            ResolvedPath::TagPage {
                source: "tags".to_string(),
                value: "rust".to_string()
            }
        );
        assert_eq!(
            resolve_request_path(&fixture.config(), "performers/joshua_jay/"),
            ResolvedPath::TagPage {
                source: "performers".to_string(),
                value: "joshua_jay".to_string()
            }
        );
        assert_eq!(
            resolve_request_path(&fixture.config(), "taxonomy.categories/"),
            ResolvedPath::TagSourceIndex {
                source: "taxonomy.categories".to_string()
            }
        );
    }

    #[test]
    fn test_file_takes_precedence_over_tag_url() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);
        // Create a real markdown file at "tags.md"
        fs::write(fixture.path().join("tags.md"), "# Real Tags Page").unwrap();

        // File should take precedence over tag source index
        let result = resolve_request_path(&fixture.config(), "tags/");

        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("tags.md"))
        );
    }

    #[test]
    fn test_directory_takes_precedence_over_tag_url() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);
        // Create a real directory "tags/"
        fs::create_dir(fixture.path().join("tags")).unwrap();

        // Directory listing should take precedence
        let result = resolve_request_path(&fixture.config(), "tags/");

        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::DirectoryListing(fixture.canonical_path().join("tags"))
        );
    }

    #[test]
    fn test_nested_tag_value_not_matched() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);

        // More than 2 segments is not a valid tag URL
        let result = resolve_request_path(&fixture.config(), "tags/rust/advanced/");

        assert_eq!(result, ResolvedPath::NotFound);
    }

    #[test]
    fn test_try_resolve_tag_url_directly() {
        let sources = vec!["tags".to_string(), "performers".to_string()];

        // Tag source index
        assert_eq!(
            try_resolve_tag_url("tags/", &sources),
            Some(ResolvedPath::TagSourceIndex {
                source: "tags".to_string()
            })
        );

        // Tag page
        assert_eq!(
            try_resolve_tag_url("tags/rust", &sources),
            Some(ResolvedPath::TagPage {
                source: "tags".to_string(),
                value: "rust".to_string()
            })
        );

        // Unknown source
        assert_eq!(try_resolve_tag_url("unknown/value", &sources), None);

        // Empty path
        assert_eq!(try_resolve_tag_url("", &sources), None);

        // Empty sources
        assert_eq!(try_resolve_tag_url("tags/rust", &[]), None);
    }

    // ==================== Non-Canonical Index URL Redirect Tests ====================

    #[test]
    fn test_non_canonical_index_redirects() {
        let fixture = TestFixture::new();
        let docs = fixture.path().join("docs");
        fs::create_dir(&docs).unwrap();
        fs::write(docs.join("index.md"), "# Docs Index").unwrap();

        // /docs/index/ should redirect to /docs/
        let result = resolve_request_path(&fixture.config(), "docs/index/");
        assert_eq!(result, ResolvedPath::Redirect("/docs/".to_string()));
    }

    #[test]
    fn test_root_index_redirects() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("index.md"), "# Home").unwrap();

        // /index/ should redirect to /
        let result = resolve_request_path(&fixture.config(), "index/");
        assert_eq!(result, ResolvedPath::Redirect("/".to_string()));
    }

    #[test]
    fn test_nested_index_redirects() {
        let fixture = TestFixture::new();
        let nested = fixture.path().join("a/b/c");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("index.md"), "# Nested").unwrap();

        // /a/b/c/index/ should redirect to /a/b/c/
        let result = resolve_request_path(&fixture.config(), "a/b/c/index/");
        assert_eq!(result, ResolvedPath::Redirect("/a/b/c/".to_string()));
    }

    #[test]
    fn test_regular_file_named_index_no_redirect() {
        let fixture = TestFixture::new();
        // Create a regular file that happens to be named index.md (not in a directory with index)
        fs::write(fixture.path().join("index.md"), "# Regular Index").unwrap();

        // But also create docs/readme.md as a standalone file (no parent index)
        let docs = fixture.path().join("docs");
        fs::create_dir(&docs).unwrap();
        fs::write(docs.join("readme.md"), "# Readme").unwrap();

        // /docs/readme/ should NOT redirect (readme is not the index file)
        let result = resolve_request_path(&fixture.config(), "docs/readme/");
        assert!(matches!(result, ResolvedPath::MarkdownFile(_)));
    }

    #[test]
    fn test_index_without_trailing_slash_redirects() {
        let fixture = TestFixture::new();
        let docs = fixture.path().join("docs");
        fs::create_dir(&docs).unwrap();
        fs::write(docs.join("index.md"), "# Docs Index").unwrap();

        // /docs/index (without trailing slash) should also redirect to /docs/
        let result = resolve_request_path(&fixture.config(), "docs/index");
        assert_eq!(result, ResolvedPath::Redirect("/docs/".to_string()));
    }

    // ==================== Path Traversal Security Tests ====================

    #[test]
    fn test_path_traversal_blocked_with_dotdot() {
        let fixture = TestFixture::new();
        // Create a file outside the temp directory (simulating /etc/passwd)
        // We can't actually create /etc/passwd, so we test that path traversal returns NotFound

        // Various path traversal attempts should all return NotFound
        let attacks = vec![
            "../../../etc/passwd",
            "..%2F..%2F..%2Fetc/passwd",
            "foo/../../../etc/passwd",
            "foo/bar/../../../etc/passwd",
            "....//....//etc/passwd",
        ];

        for attack in attacks {
            let result = resolve_request_path(&fixture.config(), attack);
            assert_eq!(
                result,
                ResolvedPath::NotFound,
                "Path traversal should be blocked for: {}",
                attack
            );
        }
    }

    #[test]
    fn test_path_traversal_blocked_in_static_folder() {
        let fixture = TestFixture::new();
        // Create a file in static folder
        fs::write(fixture.path().join("static/safe.txt"), "safe content").unwrap();

        // Path traversal within static folder should be blocked
        let attacks = vec![
            "../readme.md",     // Try to escape static to base_dir
            "../../etc/passwd", // Try to escape completely
            "foo/../../../etc/passwd",
        ];

        for attack in &attacks {
            let result = find_in_static_folder(&fixture.config(), attack);
            assert!(
                result.is_none(),
                "Static folder path traversal should be blocked for: {}",
                attack
            );
        }

        // But valid file should still work
        let valid = find_in_static_folder(&fixture.config(), "safe.txt");
        assert!(valid.is_some(), "Valid static file should be found");
    }

    #[test]
    fn test_safe_join_blocks_traversal() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create a file inside
        fs::write(base.join("inside.txt"), "inside").unwrap();

        // Valid path should work
        let valid = safe_join(base, None, "inside.txt");
        assert!(valid.is_some(), "Valid path should work");
        assert!(valid.unwrap().ends_with("inside.txt"));

        // Path traversal should be blocked
        let attack = safe_join(base, None, "../../../etc/passwd");
        assert!(attack.is_none(), "Path traversal should be blocked");

        // Complex traversal should be blocked
        let attack2 = safe_join(base, None, "foo/../../../etc/passwd");
        assert!(
            attack2.is_none(),
            "Complex path traversal should be blocked"
        );
    }

    #[test]
    fn test_safe_join_allows_internal_dotdot() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create nested structure
        fs::create_dir_all(base.join("foo/bar")).unwrap();
        fs::write(base.join("foo/sibling.txt"), "sibling").unwrap();

        // Going up and back down within base_dir should work
        let valid = safe_join(base, None, "foo/bar/../sibling.txt");
        assert!(valid.is_some(), "Internal navigation should work");
        let resolved = valid.unwrap();
        assert!(
            resolved.ends_with("sibling.txt"),
            "Should resolve to sibling.txt, got: {:?}",
            resolved
        );
    }

    #[test]
    fn test_path_traversal_returns_not_found_not_error() {
        let fixture = TestFixture::new();

        // Path traversal should cleanly return NotFound, not panic or error
        let result = resolve_request_path(&fixture.config(), "../../../../etc/passwd");

        // Should be NotFound, not a panic or file access
        assert_eq!(result, ResolvedPath::NotFound);
    }

    #[test]
    fn test_symlink_escape_blocked() {
        // This test verifies that symlinks pointing outside base_dir are blocked
        let dir = TempDir::new().unwrap();
        let base = dir.path();
        fs::create_dir(base.join("static")).unwrap();

        // Create a symlink in static folder pointing outside
        // (This is OS-dependent and may not work on all systems)
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link_path = base.join("static/escape");
            // Try to create symlink to /tmp (which exists on most Unix systems)
            if symlink("/tmp", &link_path).is_ok() {
                let extensions = vec![String::from("md")];
                let tag_sources: Vec<String> = vec![];
                let config = PathResolverConfig {
                    base_dir: base,
                    canonical_base_dir: None,
                    static_folder: "static",
                    markdown_extensions: &extensions,
                    index_file: "index.md",
                    tag_sources: &tag_sources,
                };

                // Following the symlink should be blocked
                let result = find_in_static_folder(&config, "escape/some_file");
                assert!(result.is_none(), "Symlink escape should be blocked");
            }
        }
    }

    // ==================== Static Folder Tests ====================

    #[test]
    fn test_precedence_base_dir_over_static() {
        // Request /image.png with file in BOTH locations
        // Should prefer base_dir (step 1 wins over step 4b)
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("image.png"), "direct").unwrap();
        fs::write(fixture.path().join("static/image.png"), "static").unwrap();

        let result = resolve_request_path(&fixture.config(), "image.png");

        // Should return base_dir file, not static folder
        assert_eq!(
            result,
            ResolvedPath::StaticFile(fixture.canonical_path().join("image.png"))
        );

        // Verify the correct file would be served by checking content
        let resolved_path = match result {
            ResolvedPath::StaticFile(p) => p,
            _ => panic!("Expected StaticFile"),
        };
        let content = fs::read_to_string(resolved_path).unwrap();
        assert_eq!(
            content, "direct",
            "Should serve file from base_dir, not static folder"
        );
    }

    #[test]
    fn test_safe_join_failure_static_fallback() {
        // Request /images/blog/photo.png where:
        // - base_dir/images/ does NOT exist (safe_join fails)
        // - static/images/blog/photo.png DOES exist
        // This is the exact regression case
        let fixture = TestFixture::new();
        fs::create_dir_all(fixture.path().join("static/images/blog")).unwrap();
        fs::write(fixture.path().join("static/images/blog/photo.png"), "image").unwrap();
        // Note: base_dir/images/ does NOT exist

        let result = resolve_request_path(&fixture.config(), "images/blog/photo.png");

        let expected = fixture
            .path()
            .join("static/images/blog/photo.png")
            .canonicalize()
            .unwrap();
        assert_eq!(result, ResolvedPath::StaticFile(expected));
    }

    #[test]
    fn test_empty_static_folder_config() {
        // Config with static_folder = ""
        // Static folder lookup should be skipped
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("static")).unwrap();
        fs::write(dir.path().join("static/file.txt"), "content").unwrap();

        let extensions = vec![String::from("md")];
        let tag_sources: Vec<String> = vec![];
        let config = PathResolverConfig {
            base_dir: dir.path(),
            canonical_base_dir: None,
            static_folder: "", // Empty!
            markdown_extensions: &extensions,
            index_file: "index.md",
            tag_sources: &tag_sources,
        };

        let result = resolve_request_path(&config, "file.txt");
        assert_eq!(result, ResolvedPath::NotFound);
    }

    #[test]
    fn test_deeply_nested_static_path() {
        // Test 5+ levels of nesting
        let fixture = TestFixture::new();
        fs::create_dir_all(fixture.path().join("static/a/b/c/d/e")).unwrap();
        fs::write(fixture.path().join("static/a/b/c/d/e/deep.png"), "deep").unwrap();

        let result = resolve_request_path(&fixture.config(), "a/b/c/d/e/deep.png");

        let expected = fixture
            .path()
            .join("static/a/b/c/d/e/deep.png")
            .canonicalize()
            .unwrap();
        assert_eq!(result, ResolvedPath::StaticFile(expected));
    }

    #[test]
    fn test_static_folder_with_trailing_slash_request() {
        // Request "images/photo.png/" with trailing slash
        // Behavior is platform-dependent:
        // - macOS: canonicalize() tolerates trailing slashes on file paths
        // - Linux: canonicalize() rejects trailing slashes on file paths
        let fixture = TestFixture::new();
        fs::create_dir_all(fixture.path().join("static/images")).unwrap();
        fs::write(fixture.path().join("static/images/photo.png"), "img").unwrap();

        let result = resolve_request_path(&fixture.config(), "images/photo.png/");

        #[cfg(target_os = "macos")]
        {
            // macOS tolerates trailing slash on file paths
            let expected = fixture
                .path()
                .join("static/images/photo.png")
                .canonicalize()
                .unwrap();
            assert_eq!(result, ResolvedPath::StaticFile(expected));
        }

        #[cfg(target_os = "linux")]
        {
            // Linux rejects trailing slash on file paths (stricter behavior)
            assert_eq!(result, ResolvedPath::NotFound);
        }
    }

    #[test]
    fn test_static_folder_url_encoded_spaces() {
        // Test that paths with spaces work through static folder
        let fixture = TestFixture::new();
        fs::create_dir_all(fixture.path().join("static/my images")).unwrap();
        fs::write(
            fixture.path().join("static/my images/photo file.jpg"),
            "img",
        )
        .unwrap();

        // URL-decoded path (as server would provide after decoding)
        let result = resolve_request_path(&fixture.config(), "my images/photo file.jpg");

        let expected = fixture
            .path()
            .join("static/my images/photo file.jpg")
            .canonicalize()
            .unwrap();
        assert_eq!(result, ResolvedPath::StaticFile(expected));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;
    use std::fs;
    use tempfile::TempDir;

    // Strategy for valid path component names
    fn path_component_strategy() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_-]{1,12}"
    }

    // Strategy for valid extensions
    fn extension_strategy() -> impl Strategy<Value = String> {
        "[a-z]{1,5}"
    }

    proptest! {
        /// is_markdown_file is deterministic
        #[test]
        fn prop_is_markdown_file_deterministic(
            filename in path_component_strategy(),
            ext in extension_strategy(),
            extensions in proptest::collection::vec(extension_strategy(), 1..4)
        ) {
            let path = PathBuf::from(format!("{}.{}", filename, ext));
            let result1 = is_markdown_file(&path, &extensions);
            let result2 = is_markdown_file(&path, &extensions);
            prop_assert_eq!(result1, result2);
        }

        /// is_markdown_file returns true when extension matches
        #[test]
        fn prop_is_markdown_file_matches_extension(
            filename in path_component_strategy(),
            extensions in proptest::collection::vec(extension_strategy(), 1..4)
        ) {
            // Use the first extension from the list
            if let Some(ext) = extensions.first() {
                let path = PathBuf::from(format!("{}.{}", filename, ext));
                prop_assert!(is_markdown_file(&path, &extensions));
            }
        }

        /// strip_trailing_separator is idempotent
        #[test]
        fn prop_strip_trailing_separator_idempotent(
            components in proptest::collection::vec(path_component_strategy(), 1..5)
        ) {
            let path_str = format!("/{}/", components.join("/"));
            let path = Path::new(&path_str);

            let once = strip_trailing_separator(path);
            let twice = strip_trailing_separator(&once);

            prop_assert_eq!(once, twice);
        }

        /// strip_trailing_separator never ends with separator (except for root)
        #[test]
        fn prop_strip_trailing_separator_no_trailing(
            components in proptest::collection::vec(path_component_strategy(), 1..5)
        ) {
            let path_str = format!("/{}/", components.join("/"));
            let path = Path::new(&path_str);
            let result = strip_trailing_separator(path);
            let result_str = result.to_string_lossy();

            prop_assert!(
                !result_str.ends_with('/'),
                "Result {:?} should not end with /",
                result_str
            );
        }

        /// Path resolution is deterministic for the same filesystem state
        #[test]
        fn prop_path_resolution_deterministic(
            request_path in proptest::collection::vec(path_component_strategy(), 0..3)
        ) {
            let dir = TempDir::new().unwrap();
            fs::create_dir(dir.path().join("static")).unwrap();

            // Create a markdown file
            fs::write(dir.path().join("test.md"), "# Test").unwrap();

            let extensions = vec![String::from("md")];
            let tag_sources: Vec<String> = vec![];
            let config = PathResolverConfig {
                base_dir: dir.path(),
                canonical_base_dir: None,
                static_folder: "static",
                markdown_extensions: &extensions,
                index_file: "index.md",
                tag_sources: &tag_sources,
            };

            let path_str = request_path.join("/");

            let result1 = resolve_request_path(&config, &path_str);
            let result2 = resolve_request_path(&config, &path_str);

            prop_assert_eq!(result1, result2);
        }

        /// Path traversal with ".." in paths doesn't cause panics
        /// and returns deterministic results
        #[test]
        fn prop_path_traversal_no_panic(
            prefix in proptest::collection::vec(path_component_strategy(), 0..2),
            suffix in proptest::collection::vec(path_component_strategy(), 0..2)
        ) {
            let dir = TempDir::new().unwrap();
            let base_dir = dir.path();
            fs::create_dir(base_dir.join("static")).unwrap();

            let extensions = vec![String::from("md")];
            let tag_sources: Vec<String> = vec![];
            let config = PathResolverConfig {
                base_dir,
                canonical_base_dir: None,
                static_folder: "static",
                markdown_extensions: &extensions,
                index_file: "index.md",
                tag_sources: &tag_sources,
            };

            // Try various path traversal patterns
            let attack_paths = vec![
                format!("{}/../{}", prefix.join("/"), suffix.join("/")),
                format!("../{}", suffix.join("/")),
                format!("{}/../../{}", prefix.join("/"), suffix.join("/")),
            ];

            for attack_path in attack_paths {
                // Should not panic and should return consistent results
                let result1 = resolve_request_path(&config, &attack_path);
                let result2 = resolve_request_path(&config, &attack_path);
                prop_assert_eq!(result1, result2, "Results should be deterministic for {:?}", attack_path);
            }
        }
    }
}
