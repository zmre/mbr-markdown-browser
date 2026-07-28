//! URL path primitives shared by the link pipeline.
//!
//! Two concerns live here because every consumer needs both and duplicating
//! either produces silent, platform- or scheme-specific breakage:
//!
//! 1. [`path_to_url`] — conversion of filesystem paths into URL paths. URL
//!    paths are always `/`-separated, but `std::path` uses the platform
//!    separator — `\` on Windows. Stringifying a `Path` with
//!    `to_string_lossy()`, `to_str()` or `display()` and splicing the result
//!    into a URL therefore produces `/docs\guide/` on Windows. Worse,
//!    downstream logic that splits on `/` (index-file collapsing, extension
//!    stripping, cache-key matching) then silently stops matching.
//! 2. [`is_external_url`] — the single answer to "does this URL leave the
//!    site?", used by link transformation, link tracking and link validation.

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

/// Returns `true` when `url` addresses something outside this site.
///
/// This is the one place that answers the question. Link transformation
/// ([`crate::link_transform::transform_link`]), link tracking
/// ([`crate::link_index::is_internal_link`]) and link validation
/// ([`crate::page_errors`]) all call it, so an href can never be rewritten as
/// a relative path by one and reported as a broken internal link by another.
///
/// A URL is external when it is either:
/// - **protocol-relative** (`//cdn.example.com/x.js`), which inherits the
///   page's scheme and leaves the site without naming one; or
/// - **scheme-qualified** — it starts with an RFC 3986 scheme followed by
///   `:` (`ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`).
///
/// Testing the *shape* rather than an allowlist of known schemes is
/// deliberate: enumerating schemes is what made `ftp://`, `ftps://`,
/// `magnet:`, `sms:`, `callto:` and `blob:` get mangled into relative paths
/// and reported as broken internal links, one scheme list at a time.
///
/// Everything else is site-relative and therefore internal: absolute paths
/// (`/docs/`), relative paths (`guide.md`), fragment-only (`#top`) and
/// query-only (`?q=1`) references.
///
/// Two shapes are intentionally *not* read as schemes:
/// - **Windows drive letters** (`C:/notes/x.md`). RFC 3986 permits one-letter
///   schemes but none exist in practice, so a single letter before the colon
///   is a drive, matching what every URL parser does.
/// - **A colon in a later path segment** (`docs/a:b.md`), because `/`, `?`
///   and `#` are not legal scheme characters and end the scan. A colon in the
///   *first* segment (`Notes:2024.md`) does look like a scheme; RFC 3986
///   requires such a reference to be written `./Notes:2024.md`, which this
///   function correctly reports as internal.
///
/// ```
/// use mbr::url_path::is_external_url;
///
/// assert!(is_external_url("magnet:?xt=urn:btih:abc"));
/// assert!(is_external_url("//cdn.example.com/x.js"));
/// assert!(!is_external_url("docs/guide.md"));
/// assert!(!is_external_url("C:/notes/guide.md"));
/// ```
pub fn is_external_url(url: &str) -> bool {
    // Protocol-relative first: it starts with `/` but is not site-relative.
    if url.starts_with("//") {
        return true;
    }

    // Absolute site paths, fragment-only and query-only references can never
    // carry a scheme, so stop before scanning for a colon.
    if url.starts_with(['/', '#', '?']) {
        return false;
    }

    url_scheme(url).is_some()
}

/// Returns the RFC 3986 scheme of `url` (without the trailing `:`), if any.
///
/// The scheme grammar excludes `/`, `?` and `#`, so a colon that appears after
/// one of those (`docs/a:b.md`) fails the character check and yields `None`
/// without a separate delimiter scan.
fn url_scheme(url: &str) -> Option<&str> {
    let colon = url.find(':')?;
    let scheme = &url[..colon];

    // Single letter before the colon is a Windows drive, not a scheme.
    if scheme.len() < 2 {
        return None;
    }

    let mut chars = scheme.chars();
    let starts_with_alpha = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let rest_is_scheme_chars =
        chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));

    (starts_with_alpha && rest_is_scheme_chars).then_some(scheme)
}

/// Serde helper: serializes a **relative** path as a `/`-separated string.
///
/// Use via `#[serde(serialize_with = "crate::url_path::serialize_as_url")]` on
/// any `Path`/`PathBuf` field that crosses into JSON consumed by the frontend,
/// so the wire format stays platform independent even though the in-memory
/// value uses native separators.
pub fn serialize_as_url<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&path_to_url(path))
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

    /// The whole point of the shared predicate: one table, one answer. The
    /// `ftp*`/`magnet`/`sms`/`callto`/`blob` rows are the regression — each
    /// was missing from at least one of the four hand-rolled scheme lists this
    /// function replaced, so those links were rewritten into relative paths
    /// and reported as broken internal links.
    #[test]
    fn test_is_external_url_table() {
        let cases: &[(&str, bool)] = &[
            // Schemes that used to be missing from one or more copies.
            ("ftp://ftp.example.com/pub/file.zip", true),
            ("ftps://ftp.example.com/pub/file.zip", true),
            ("magnet:?xt=urn:btih:c12fe1c06bba254a9dc9", true),
            ("sms:+15555550123", true),
            ("callto:+15555550123", true),
            ("blob:http://localhost:5220/550e8400-e29b", true),
            // Schemes every copy already knew about.
            ("mailto:test@example.com", true),
            ("tel:+15555550123", true),
            ("http://example.com/path", true),
            ("https://example.com/path", true),
            ("data:image/png;base64,iVBORw0KGgo", true),
            ("javascript:void(0)", true),
            ("file:///etc/hosts", true),
            // Protocol-relative: no scheme named, still leaves the site.
            ("//cdn.example.com/script.js", true),
            // Site-relative shapes.
            ("/abs/path", false),
            ("relative/path.md", false),
            ("./relative/path.md", false),
            ("../sibling/path.md", false),
            ("#anchor", false),
            ("?q=1", false),
            ("", false),
            // A Windows drive letter is not a one-letter scheme.
            ("C:/win/path", false),
            ("c:/win/path", false),
            // A colon in a later path segment does not make a scheme.
            ("docs/a:b.md", false),
            ("docs/a?x:y", false),
            // Digits may not start a scheme (`12:30 notes.md` is a filename).
            ("12:30 notes.md", false),
            // RFC 3986's escape hatch for a colon in the first segment.
            ("./Notes:2024.md", false),
        ];

        for (url, expected) in cases {
            assert_eq!(
                is_external_url(url),
                *expected,
                "is_external_url({url:?}) should be {expected}"
            );
        }
    }
}
