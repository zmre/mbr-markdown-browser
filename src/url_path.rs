//! Conversion of filesystem paths into URL paths.
//!
//! URL paths are always `/`-separated, but `std::path` uses the platform
//! separator — `\` on Windows. Stringifying a `Path` with `to_string_lossy()`,
//! `to_str()` or `display()` and splicing the result into a URL therefore
//! produces `/docs\guide/` on Windows. Worse, downstream logic that splits on
//! `/` (index-file collapsing, extension stripping, cache-key matching) then
//! silently stops matching.
//!
//! Everything that turns a path into a URL should go through [`path_to_url`].

use std::borrow::Cow;
use std::path::{Component, Path};

/// Converts a **relative** filesystem path into a `/`-separated URL path body.
///
/// The result has no leading or trailing slash, so callers stay in control of
/// their own delimiters: `docs/guide.md`. An empty or root-only path yields an
/// empty string.
///
/// Component handling:
/// - `Normal` components are joined with `/`.
/// - `ParentDir` (`..`) is preserved, because [`pathdiff::diff_paths`] can
///   legitimately produce upward traversals that the caller depends on.
/// - `CurDir` (`.`), `RootDir` (`/`) and `Prefix` (`C:`) are dropped: a URL
///   path is always rooted at the site root, so they carry no meaning.
///
/// Non-UTF-8 components are lossily converted, matching the previous behavior
/// of the call sites this replaced.
///
/// ```
/// use std::path::Path;
/// use mbr::url_path::path_to_url;
///
/// // Built with `join` so the separator is whatever the host uses natively;
/// // the output is `/`-separated on every platform.
/// let path = Path::new("docs").join("guide.md");
/// assert_eq!(path_to_url(&path), "docs/guide.md");
/// ```
pub fn path_to_url(relative: &Path) -> String {
    let mut url = String::new();

    for component in relative.components() {
        let piece: Cow<'_, str> = match component {
            Component::Normal(name) => name.to_string_lossy(),
            Component::ParentDir => Cow::Borrowed(".."),
            Component::Prefix(_) | Component::RootDir | Component::CurDir => continue,
        };

        if !url.is_empty() {
            url.push('/');
        }
        url.push_str(&piece);
    }

    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Builds a path from components using the platform separator, so these
    /// tests exercise `\` on Windows and `/` on Unix from identical source.
    fn native(parts: &[&str]) -> PathBuf {
        parts.iter().collect()
    }

    #[test]
    fn test_empty_path_is_empty_url() {
        assert_eq!(path_to_url(Path::new("")), "");
    }

    #[test]
    fn test_single_component() {
        assert_eq!(path_to_url(Path::new("README.md")), "README.md");
    }

    #[test]
    fn test_nested_path_uses_forward_slashes() {
        // The core regression guard: a natively-joined path must come back
        // `/`-separated regardless of host platform.
        let path = native(&["docs", "guide", "index.md"]);
        let url = path_to_url(&path);

        assert_eq!(url, "docs/guide/index.md");
        assert!(
            !url.contains('\\'),
            "URL must never contain a backslash, got {url}"
        );
    }

    #[test]
    fn test_deeply_nested_path() {
        let path = native(&["a", "b", "c", "d", "e", "f.png"]);
        assert_eq!(path_to_url(&path), "a/b/c/d/e/f.png");
    }

    #[test]
    fn test_parent_dir_is_preserved() {
        // pathdiff can legitimately produce `..`; dropping it would silently
        // change which file a URL refers to.
        let path = native(&["..", "..", "images", "photo.png"]);
        assert_eq!(path_to_url(&path), "../../images/photo.png");
    }

    #[test]
    fn test_current_dir_is_dropped() {
        assert_eq!(path_to_url(Path::new(".")), "");
        let path = native(&[".", "docs", "guide.md"]);
        assert_eq!(path_to_url(&path), "docs/guide.md");
    }

    #[test]
    fn test_root_component_is_dropped() {
        // Absolute input still yields a relative body; callers add the leading
        // slash themselves.
        assert_eq!(path_to_url(Path::new("/docs/guide.md")), "docs/guide.md");
    }

    #[test]
    fn test_spaces_and_unicode_preserved() {
        let path = native(&["my images", "café \u{1f600}.png"]);
        assert_eq!(path_to_url(&path), "my images/café \u{1f600}.png");
    }

    #[test]
    fn test_no_trailing_or_leading_slash() {
        let url = path_to_url(&native(&["docs", "guide.md"]));
        assert!(!url.starts_with('/'));
        assert!(!url.ends_with('/'));
    }

    /// On Windows a literal backslash string really is a separator, so this
    /// asserts the conversion directly. It cannot run on Unix, where `\` is a
    /// legal filename character and the same literal is a single component.
    #[cfg(windows)]
    #[test]
    fn test_windows_backslash_literal_is_converted() {
        assert_eq!(path_to_url(Path::new(r"docs\guide.md")), "docs/guide.md");
        assert_eq!(
            path_to_url(Path::new(r"C:\notes\docs\guide.md")),
            "notes/docs/guide.md"
        );
    }

    /// Conversely, on Unix a backslash is part of the filename and must be
    /// left alone rather than rewritten into a path separator.
    #[cfg(unix)]
    #[test]
    fn test_unix_backslash_is_a_filename_character() {
        assert_eq!(path_to_url(Path::new(r"weird\name.md")), r"weird\name.md");
    }
}
