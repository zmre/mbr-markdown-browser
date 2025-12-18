//! Path resolution logic for the mbr server.
//!
//! This module contains pure functions for determining what resource to serve
//! based on a URL path. By keeping this logic separate from I/O, it becomes
//! easily testable.

use std::path::{Path, PathBuf};

/// The result of resolving a URL path to a resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedPath {
    /// Serve a static file directly (non-markdown)
    StaticFile(PathBuf),
    /// Render a markdown file
    MarkdownFile(PathBuf),
    /// Generate a directory listing
    DirectoryListing(PathBuf),
    /// Resource not found
    NotFound,
}

/// Configuration for path resolution.
#[derive(Debug, Clone)]
pub struct PathResolverConfig<'a> {
    pub base_dir: &'a Path,
    pub static_folder: &'a str,
    pub markdown_extensions: &'a [String],
    pub index_file: &'a str,
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
/// 7. Nothing matches → NotFound
pub fn resolve_request_path(config: &PathResolverConfig, request_path: &str) -> ResolvedPath {
    let candidate_path = config.base_dir.join(request_path);

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

    if let Some(md_path) = find_markdown_file(&candidate_base, config.markdown_extensions) {
        return ResolvedPath::MarkdownFile(md_path);
    }

    // 4. Check static folder
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

    // 7. Nothing found
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
fn find_in_static_folder(config: &PathResolverConfig, request_path: &str) -> Option<PathBuf> {
    let static_dir = config.base_dir.join(config.static_folder).canonicalize().ok()?;
    let candidate = static_dir.join(request_path);
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Test fixture that owns the extensions vector
    struct TestFixture {
        dir: TempDir,
        extensions: Vec<String>,
    }

    impl TestFixture {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            fs::create_dir(dir.path().join("static")).unwrap();
            Self {
                dir,
                extensions: vec![String::from("md")],
            }
        }

        fn with_extensions(extensions: Vec<String>) -> Self {
            let dir = TempDir::new().unwrap();
            fs::create_dir(dir.path().join("static")).unwrap();
            Self { dir, extensions }
        }

        fn config(&self) -> PathResolverConfig<'_> {
            PathResolverConfig {
                base_dir: self.dir.path(),
                static_folder: "static",
                markdown_extensions: &self.extensions,
                index_file: "index.md",
            }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }
    }

    #[test]
    fn test_direct_markdown_file() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("readme.md"), "# Test").unwrap();

        let result = resolve_request_path(&fixture.config(), "readme.md");

        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.path().join("readme.md"))
        );
    }

    #[test]
    fn test_direct_static_file() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("image.png"), "fake image").unwrap();

        let result = resolve_request_path(&fixture.config(), "image.png");

        assert_eq!(
            result,
            ResolvedPath::StaticFile(fixture.path().join("image.png"))
        );
    }

    #[test]
    fn test_directory_with_index() {
        let fixture = TestFixture::new();
        let subdir = fixture.path().join("docs");
        fs::create_dir(&subdir).unwrap();
        fs::write(subdir.join("index.md"), "# Docs").unwrap();

        let result = resolve_request_path(&fixture.config(), "docs");

        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(subdir.join("index.md"))
        );
    }

    #[test]
    fn test_trailing_slash_to_markdown() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("about.md"), "# About").unwrap();

        let result = resolve_request_path(&fixture.config(), "about/");

        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.path().join("about.md"))
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
    fn test_directory_listing() {
        let fixture = TestFixture::new();
        let subdir = fixture.path().join("posts");
        fs::create_dir(&subdir).unwrap();
        // No index file

        let result = resolve_request_path(&fixture.config(), "posts/");

        assert_eq!(result, ResolvedPath::DirectoryListing(subdir));
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

        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(nested.join("index.md"))
        );
    }

    #[test]
    fn test_multiple_markdown_extensions() {
        let fixture =
            TestFixture::with_extensions(vec![String::from("md"), String::from("markdown")]);
        fs::write(fixture.path().join("notes.markdown"), "# Notes").unwrap();

        let result = resolve_request_path(&fixture.config(), "notes/");

        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.path().join("notes.markdown"))
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

        // Should prefer .md (first in list)
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.path().join("test.md"))
        );
    }

    #[test]
    fn test_root_path_empty_string() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("index.md"), "# Home").unwrap();

        let result = resolve_request_path(&fixture.config(), "");

        // Empty path resolves to base_dir, which is a directory with index.md
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.path().join("index.md"))
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
            let config = PathResolverConfig {
                base_dir: dir.path(),
                static_folder: "static",
                markdown_extensions: &extensions,
                index_file: "index.md",
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
            let config = PathResolverConfig {
                base_dir,
                static_folder: "static",
                markdown_extensions: &extensions,
                index_file: "index.md",
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
