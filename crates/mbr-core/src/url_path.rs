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
//!
//! It also holds [`srcset_url_spans`] / [`rewrite_srcset`], the one reading of
//! an HTML `srcset` value, shared by the renderer's raw-HTML URL rewrite and
//! QuickLook's attachment collector so the two cannot disagree about which
//! bytes of a `srcset` are a URL.

use std::borrow::Cow;
use std::ops::Range;
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
/// use mbr_core::url_path::path_to_url;
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
/// use mbr_core::url_path::is_external_url;
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

/// Returns `true` when `url` addresses one of mbr's own `?path=`-driven
/// viewer routes (`/.mbr/videos/?path=…`, and the `pdfs` / `audio` / `images`
/// siblings), in any spelling.
///
/// Those routes are addressed by their query, not by a file. A static build
/// does write `.mbr/videos/index.html` and friends, so the canonical spelling
/// happens to land on a real file — but the slashless one a hand author writes
/// (`/.mbr/videos?path=…`) lands on the directory, and a file-based checker
/// calls it a missing trailing slash although every static host redirects it
/// and the server routes it directly. The server's `errors.json` checker never
/// judges them (it skips all of `/.mbr/`, see `page_errors`); this is the
/// narrower rule the static build's checker applies so the two agree on them.
///
/// The match is on a `.mbr` **path segment** rather than a `/.mbr/` prefix
/// because the pipeline emits three spellings of the same link: `/.mbr/videos/`
/// (server mode, site-absolute), `.mbr/videos/` (build, root page) and
/// `../.mbr/videos/` (build, nested page). Requiring the `path=` parameter
/// keeps it narrow: a genuinely broken `/.mbr/whatever/` link carrying no such
/// query is still reported.
///
/// ```
/// use mbr_core::url_path::is_mbr_query_route;
///
/// assert!(is_mbr_query_route("/.mbr/videos/?path=%2Fvideos%2Fdemo.mp4"));
/// assert!(is_mbr_query_route("../.mbr/pdfs?path=/docs/report.pdf"));
/// assert!(!is_mbr_query_route("/.mbr/videos/"));
/// ```
pub fn is_mbr_query_route(url: &str) -> bool {
    // Fragment first, then query: `#` closes a URI (RFC 3986 §3.5), so a `?`
    // to its right is fragment text and never a query.
    let before_fragment = url.split('#').next().unwrap_or_default();

    before_fragment
        .split_once('?')
        .is_some_and(|(path, query)| {
            path.split('/').any(|segment| segment == ".mbr")
                && query
                    .split('&')
                    .any(|param| param.split_once('=').is_some_and(|(key, _)| key == "path"))
        })
}

/// Returns the RFC 3986 scheme of `url` (without the trailing `:`), if any.
///
/// The scheme grammar excludes `/`, `?` and `#`, so a colon that appears after
/// one of those (`docs/a:b.md`) fails the character check and yields `None`
/// without a separate delimiter scan.
///
/// Public because the `mbr` binary's `external_open` needs the *name* of the
/// scheme, not just whether one is present: the GUI navigation policy refuses
/// to hand `javascript:`, `vbscript:` and `data:` to the operating system. It
/// reuses this scan so there is still one definition of what a scheme looks
/// like — the whole point of this module.
pub fn url_scheme(url: &str) -> Option<&str> {
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

/// HTML's "ASCII whitespace": space, tab, LF, FF, CR.
fn is_html_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\x0C' | b'\r')
}

/// The byte ranges of the candidate URLs in an HTML `srcset` value, in order.
///
/// A simplified reading of the HTML "parse a srcset attribute" algorithm:
/// candidates are separated by commas; each is optional whitespace, a URL (a
/// run of non-whitespace), then an optional descriptor (`2x`, `640w`) running
/// to the next comma outside parentheses. Leading commas are skipped, and
/// commas that end the URL's run (`a.png, b.png 2x` or `a.png,`) are the
/// separator, not part of the URL. Descriptors are not validated: only the
/// URLs are of interest, and everything else is kept byte for byte by
/// [`rewrite_srcset`].
///
/// Not supported: URLs containing commas. A comma *inside* the run
/// (`a,b.png`) stays in the URL, as the spec reads it, and the whole run is
/// handed on as one URL; one at the run's end cannot be told from a
/// separator. Neither occurs in a relative file path anyone writes.
///
/// Every range is non-empty, starts and ends on an ASCII byte boundary (so
/// slicing is always valid), holds no whitespace, and neither starts nor ends
/// with a comma.
pub fn srcset_url_spans(srcset: &str) -> Vec<Range<usize>> {
    let bytes = srcset.as_bytes();
    let mut spans = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        // Separators: whitespace and (stray or leading) commas.
        while pos < bytes.len() && (is_html_whitespace(bytes[pos]) || bytes[pos] == b',') {
            pos += 1;
        }
        if pos == bytes.len() {
            break;
        }

        // The URL: a run of non-whitespace, minus any commas ending it.
        let start = pos;
        while pos < bytes.len() && !is_html_whitespace(bytes[pos]) {
            pos += 1;
        }
        let end = start + srcset[start..pos].trim_end_matches(',').len();
        // `bytes[start]` is neither whitespace nor a comma, so `end > start`.
        spans.push(start..end);
        if end < pos {
            // A trailing comma ended this candidate: it has no descriptor.
            continue;
        }

        // The descriptor: up to the next comma outside parentheses.
        let mut depth = 0usize;
        while pos < bytes.len() {
            match bytes[pos] {
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                b',' if depth == 0 => break,
                _ => {}
            }
            pos += 1;
        }
    }
    spans
}

/// `srcset` with each candidate URL replaced by what `rewrite_url` returns
/// for it; `None` keeps that URL as authored.
///
/// Only the URLs (see [`srcset_url_spans`]) change: descriptors, separators
/// and whitespace, newlines included, are kept byte for byte. When
/// `rewrite_url` returns `None` for every candidate the value is returned
/// borrowed, so callers can tell "nothing to do" without comparing strings.
/// Any `Some`, even one equal to the URL, rebuilds the value.
///
/// ```
/// use mbr_core::url_path::rewrite_srcset;
///
/// let out = rewrite_srcset("a.png, b.png 2x", |url| (url == "b.png").then(|| "../b.png".into()));
/// assert_eq!(out, "a.png, ../b.png 2x");
/// ```
pub fn rewrite_srcset<F>(srcset: &str, mut rewrite_url: F) -> Cow<'_, str>
where
    F: FnMut(&str) -> Option<String>,
{
    let mut rewritten: Option<String> = None;
    let mut copied_to = 0;
    for span in srcset_url_spans(srcset) {
        let Some(replacement) = rewrite_url(&srcset[span.clone()]) else {
            continue;
        };
        let out = rewritten.get_or_insert_with(|| String::with_capacity(srcset.len() + 16));
        out.push_str(&srcset[copied_to..span.start]);
        out.push_str(&replacement);
        copied_to = span.end;
    }
    match rewritten {
        Some(mut out) => {
            out.push_str(&srcset[copied_to..]);
            Cow::Owned(out)
        }
        None => Cow::Borrowed(srcset),
    }
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
            // Application schemes. `message:` is the shape that motivated the
            // GUI navigation handler: mail clients address a message by its
            // `Message-ID`, whose angle brackets arrive percent-encoded, and
            // both the `//` and the bare forms are in the wild.
            (
                "message://%3CCAEn-OzgEreVRuNkdnb9gdFNLXByerYCLraYJGRvjSvXw9chVMQ@mail.gmail.com%3E",
                true,
            ),
            ("message:%3Cabc123@mail.example.com%3E", true),
            ("zoommtg://zoom.us/join?confno=1234567890", true),
            ("x-devonthink-item://8A3B0C1D-2E4F", true),
            // Percent-encoding does not change the answer either way: it is not
            // a scheme character, so it can neither create nor hide one.
            ("https://example.com/a%20b?c=%26d#e%2Ff", true),
            ("docs/my%20file.md", false),
            ("/docs/caf%C3%A9/", false),
            ("./%3Cbrackets%3E.md", false),
            // An encoded colon is just a character in a filename.
            ("Notes%3A2024.md", false),
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
    /// The exemption has to cover every spelling the pipeline emits and
    /// nothing beyond it, so both halves of the predicate are pinned here:
    /// a `.mbr` path segment *and* a `path=` query parameter.
    #[test]
    fn test_is_mbr_query_route_table() {
        let cases: &[(&str, bool)] = &[
            // The three spellings of one link: site-absolute (server mode),
            // root-relative and nested-relative (static build).
            ("/.mbr/videos/?path=%2Fvideos%2Fdemo.mp4", true),
            (".mbr/videos/?path=%2Fvideos%2Fdemo.mp4", true),
            ("../.mbr/videos/?path=%2Fvideos%2Fdemo.mp4", true),
            ("../../.mbr/audio/?path=/music/track.mp3", true),
            // All four viewer routes, plus the slashless spelling a hand
            // author writes (which is what the build used to flag).
            ("/.mbr/pdfs/?path=/docs/report.pdf", true),
            ("/.mbr/images/?path=/img/photo.png", true),
            ("/.mbr/videos?path=/videos/demo.mp4", true),
            // Extra parameters and ordering do not change the answer.
            ("/.mbr/videos/?t=30&path=/videos/demo.mp4", true),
            ("/.mbr/videos/?path=/videos/demo.mp4#t=30", true),
            // A `.mbr` route with no query is a genuinely broken link and
            // stays reportable — that is what keeps this narrow.
            ("/.mbr/videos/", false),
            ("/.mbr/whatever/", false),
            // An unrelated query on an mbr route is not a viewer link.
            ("/.mbr/videos/?v=1", false),
            // `path` must be a parameter, not a prefix of one.
            ("/.mbr/videos/?pathological=1", false),
            // A `path=` query on an ordinary page is an ordinary page link.
            ("/docs/page/?path=x", false),
            // `.mbr` must be a whole segment, not a substring.
            ("/notes/.mbrconfig/?path=x", false),
            ("/my.mbr/videos/?path=x", false),
            // `#` closes the URI, so this `?path=` is fragment text and there
            // is no query at all.
            ("/.mbr/videos/#frag?path=/videos/demo.mp4", false),
            // Fragment-only and empty hrefs.
            ("#section", false),
            ("", false),
        ];

        for (url, expected) in cases {
            assert_eq!(
                is_mbr_query_route(url),
                *expected,
                "is_mbr_query_route({url:?}) should be {expected}"
            );
        }
    }

    // ============ srcset ============

    fn urls(srcset: &str) -> Vec<&str> {
        srcset_url_spans(srcset)
            .into_iter()
            .map(|span| &srcset[span])
            .collect()
    }

    #[test]
    fn srcset_single_url() {
        assert_eq!(urls("docs/hero-dark.png"), ["docs/hero-dark.png"]);
    }

    #[test]
    fn srcset_url_with_density_descriptor() {
        assert_eq!(urls("a.png 2x"), ["a.png"]);
    }

    #[test]
    fn srcset_multiple_candidates_with_width_descriptors() {
        assert_eq!(
            urls("small.jpg 480w, medium.jpg 800w, large.jpg 1200w"),
            ["small.jpg", "medium.jpg", "large.jpg"]
        );
    }

    #[test]
    fn srcset_extra_whitespace_and_newlines() {
        let value = "\n    a.png   1x ,\n\t b.png\t2x\n  ";
        assert_eq!(urls(value), ["a.png", "b.png"]);
        assert_eq!(
            rewrite_srcset(value, |url| Some(format!("../{url}"))),
            "\n    ../a.png   1x ,\n\t ../b.png\t2x\n  "
        );
    }

    #[test]
    fn srcset_trailing_comma_is_a_separator() {
        assert_eq!(urls("a.png,"), ["a.png"]);
        assert_eq!(urls("a.png, b.png"), ["a.png", "b.png"]);
        assert_eq!(urls("a.png,, b.png 2x,"), ["a.png", "b.png"]);
        assert_eq!(
            rewrite_srcset("a.png,b.png 2x", |url| Some(format!("/{url}"))),
            "/a.png,b.png 2x",
            "a comma inside the run is part of the URL"
        );
        assert_eq!(
            rewrite_srcset("a.png, b.png 2x", |url| Some(format!("/{url}"))),
            "/a.png, /b.png 2x"
        );
    }

    #[test]
    fn srcset_leading_commas_are_skipped() {
        assert_eq!(urls(" ,, a.png 1x"), ["a.png"]);
    }

    #[test]
    fn srcset_descriptor_parentheses_hide_commas() {
        assert_eq!(urls("a.png (x, y), b.png"), ["a.png", "b.png"]);
    }

    #[test]
    fn srcset_empty_and_blank() {
        for value in ["", "   ", ",", " , \n"] {
            assert!(urls(value).is_empty(), "{value:?}");
            assert!(
                matches!(rewrite_srcset(value, |_| Some("x".into())), Cow::Borrowed(v) if v == value)
            );
        }
    }

    #[test]
    fn srcset_unchanged_is_borrowed() {
        let value = "a.png 1x, b.png 2x";
        assert!(matches!(rewrite_srcset(value, |_| None), Cow::Borrowed(v) if v == value));
    }

    #[test]
    fn srcset_rewrites_only_chosen_urls() {
        assert_eq!(
            rewrite_srcset("https://x/a.png 1x, b.png 2x", |url| {
                (!url.starts_with("https:")).then(|| format!("cid:{url}"))
            }),
            "https://x/a.png 1x, cid:b.png 2x"
        );
    }

    proptest::proptest! {
        /// Rebuilding with every URL replaced by itself gives the input back,
        /// byte for byte: nothing outside the URLs is touched.
        #[test]
        fn srcset_identity_rebuild_is_byte_identical(value in "[a-z0-9./ ,\t\n()xw]{0,48}") {
            let rebuilt = rewrite_srcset(&value, |url| Some(url.to_string()));
            proptest::prop_assert_eq!(rebuilt.as_ref(), value.as_str());
        }

        /// Spans are ordered, disjoint, non-empty URLs with no whitespace and
        /// no comma at either end.
        #[test]
        fn srcset_spans_are_well_formed(value in "[a-z0-9./ ,\t\n()xw]{0,48}") {
            let spans = srcset_url_spans(&value);
            let mut previous_end = 0;
            for span in spans {
                proptest::prop_assert!(span.start >= previous_end && span.start < span.end);
                let url = &value[span.clone()];
                proptest::prop_assert!(!url.bytes().any(is_html_whitespace), "{:?}", url);
                proptest::prop_assert!(!url.starts_with(',') && !url.ends_with(','), "{:?}", url);
                previous_end = span.end;
            }
        }
    }
}
