//! Per-page error detection for server/GUI mode.
//!
//! Validates the problems a reader might care about for a single rendered page:
//!
//! 1. Broken internal links — reuse `OutboundLink` data from `LinkCache` and
//!    resolve each through `path_resolver::resolve_request_path`.
//! 2. Broken media references — parse rendered HTML for `<img>`, `<video>`,
//!    `<audio>`, and `<source>` tags and confirm internal `src` attributes
//!    resolve when interpreted the way a browser would: relative to the
//!    page's canonical URL, then through the same request pipeline the
//!    server uses. Checking never duplicates resolution logic, so it cannot
//!    disagree with what a live request actually serves.
//! 3. Unresolved wikilinks — literal `[[...]]` substrings that escaped
//!    `transform_wikilinks` (see `src/wikilink.rs`). Skipped inside `<code>`
//!    and `<pre>` blocks.
//!
//! Designed to be cheap: each validator is a pure function and is expected to
//! run on-demand for a single page render. The module is never invoked from
//! `src/build.rs`, keeping static-site output untouched.

use regex::Regex;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::link_index::{OutboundLink, resolve_relative_url};
use crate::path_resolver::{
    PathResolverConfig, ResolvedPath, normalize_link_target, resolve_request_path,
};

/// Type of media element whose `src` attribute is broken.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Source,
}

/// A single problem detected on a page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PageError {
    /// A `<a href>` whose internal target does not resolve.
    BrokenInternalLink {
        target: String,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        anchor: Option<String>,
    },
    /// A media element (`<img>`, `<video>`, `<audio>`, `<source>`) whose
    /// internal `src` does not resolve.
    BrokenMediaReference { src: String, kind: MediaKind },
    /// A literal `[[...]]` that was not transformed into a link.
    UnresolvedWikilink { raw: String },
    /// The YAML frontmatter block failed to parse, so the entire frontmatter
    /// (including otherwise-valid fields) was discarded.
    FrontmatterParseError { message: String },
}

/// Response payload for `GET /{page}/errors.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PageErrors {
    /// Canonical URL of the page (e.g., `/docs/guide/`).
    pub page_url: String,
    /// Ordered list of detected problems. Always present; the client uses the
    /// length to decide visibility.
    pub errors: Vec<PageError>,
}

/// Returns `true` if a URL looks external / non-resolvable via the local site.
fn is_external_url(url: &str) -> bool {
    url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("//")
        || url.starts_with("mailto:")
        || url.starts_with("tel:")
        || url.starts_with("javascript:")
        || url.starts_with("data:")
        || url.starts_with("ftp://")
        || url.starts_with("ftps://")
        || url.starts_with("magnet:")
}

/// Validates the internal outbound links for a single page.
///
/// Returns a `BrokenInternalLink` for each `OutboundLink` whose target is
/// internal but does not resolve to any filesystem / tag / directory resource
/// via the path resolver.
pub fn validate_internal_links(
    outbound: &[OutboundLink],
    resolver_config: &PathResolverConfig,
) -> Vec<PageError> {
    let mut errors = Vec::new();

    for link in outbound {
        if !link.internal {
            continue;
        }

        // Fragment-only links (e.g. "#section") cannot be validated without
        // target-page parsing. v1 skips them to avoid false positives.
        if link.to.starts_with('#') || link.to.is_empty() {
            continue;
        }

        // Normalize the authored href (strip anchor/query, percent-decode,
        // trim slashes) exactly as a live HTTP request would be before it
        // reaches the resolver. See `normalize_link_target` for why this must
        // match axum's decoding.
        let request_path = normalize_link_target(&link.to);

        // "" means root, which always resolves when `index_file` exists. We
        // still run it through the resolver to keep behaviour consistent.
        let resolved = resolve_request_path(resolver_config, &request_path);

        if matches!(resolved, ResolvedPath::NotFound) {
            errors.push(PageError::BrokenInternalLink {
                target: link.to.clone(),
                text: link.text.clone(),
                anchor: link.anchor.clone(),
            });
        }
    }

    errors
}

/// Validates `<img>`, `<video>`, `<audio>` and `<source>` `src` attributes in
/// the rendered HTML.
///
/// `page_url` is the page's canonical directory-style URL (e.g.
/// `/docs/guide/`; `/` for the root page). The rendered HTML contains srcs
/// that the link transform has already rewritten relative to that URL, so
/// each internal `src` is resolved against it exactly the way a browser
/// would, then checked through the same pipeline a live HTTP request hits. An
/// error is recorded only when the server would actually 404 the request.
pub fn validate_media_references(
    html: &str,
    resolver_config: &PathResolverConfig,
    page_url: &str,
) -> Vec<PageError> {
    // Parsing the HTML document is ~microseconds for typical page sizes; the
    // selectors below compile once per call which keeps the API ergonomic.
    let doc = Html::parse_document(html);

    let specs: [(&str, MediaKind); 4] = [
        ("img[src]", MediaKind::Image),
        ("video[src]", MediaKind::Video),
        ("audio[src]", MediaKind::Audio),
        ("source[src]", MediaKind::Source),
    ];

    let mut errors = Vec::new();

    for (selector_str, kind) in specs {
        let Ok(selector) = Selector::parse(selector_str) else {
            continue;
        };
        for el in doc.select(&selector) {
            let Some(src) = el.value().attr("src") else {
                continue;
            };

            if src.is_empty() || is_external_url(src) {
                continue;
            }

            if media_reference_resolves(src, resolver_config, page_url) {
                continue;
            }

            errors.push(PageError::BrokenMediaReference {
                src: src.to_string(),
                kind: kind.clone(),
            });
        }
    }

    errors
}

/// Resolves a media `src` against the page's canonical URL and checks whether
/// the server would serve it, mimicking exactly what a browser + live request
/// does:
///
/// 1. Strip the fragment / query from the (still percent-encoded) src.
/// 2. Resolve the remaining path against `page_url` with browser semantics.
///    The page URL is directory-style (ends in `/`), so all of its segments
///    are kept while `.` / `..` segments are applied — which is
///    [`resolve_relative_url`] with `is_index_file = true`. Absolute srcs
///    (`/foo.png`) pass through unchanged.
/// 3. Normalize (percent-decode, trim slashes — this also drops the trailing
///    slash step 2 appends) via [`normalize_link_target`] and resolve via
///    [`resolve_request_path`] — the identical pipeline a live HTTP request
///    hits. If the resolver reports `NotFound` the browser would 404 too, so
///    flagging the reference is always correct.
fn media_reference_resolves(
    src: &str,
    resolver_config: &PathResolverConfig,
    page_url: &str,
) -> bool {
    // Fragment / query stripping must happen before relative resolution so
    // `#` / `?` payloads never participate in `.` / `..` segment handling.
    let path_part = src.split(['#', '?']).next().unwrap_or_default();

    // A fragment- or query-only src refers to the page itself.
    if path_part.is_empty() {
        return true;
    }

    let absolute_url = resolve_relative_url(page_url, path_part, true);
    let request_path = normalize_link_target(&absolute_url);

    // "" means the site root; the resolver handles it like any live request.
    !matches!(
        resolve_request_path(resolver_config, &request_path),
        ResolvedPath::NotFound
    )
}

/// Matches a literal `[[...]]` that survived `transform_wikilinks`. We exclude
/// `]` inside the match so we correctly stop at the first `]]` and do not
/// greedily consume nested brackets.
static WIKILINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\[\]\n]+)\]\]").expect("static wikilink regex is valid"));

/// Masks out content inside `<code>` and `<pre>` blocks so the wikilink scan
/// does not report examples that readers intentionally wrote in markdown
/// code samples. The `regex` crate does not support backreferences, so we
/// handle the two tags independently.
fn mask_code_blocks(html: &str) -> String {
    static CODE_BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?is)<code\b[^>]*>.*?</code>|<pre\b[^>]*>.*?</pre>")
            .expect("static code-block regex is valid")
    });

    CODE_BLOCK_RE
        .replace_all(html, |caps: &regex::Captures| {
            // Preserve length so downstream match offsets stay sensible; the
            // only thing that matters is that bracket characters are gone.
            " ".repeat(caps[0].len())
        })
        .into_owned()
}

/// Detects literal `[[...]]` strings left in the rendered HTML by failed
/// wikilink transformation.
pub fn detect_unresolved_wikilinks(html: &str) -> Vec<PageError> {
    let masked = mask_code_blocks(html);
    let mut errors = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for cap in WIKILINK_RE.captures_iter(&masked) {
        let raw = cap
            .get(0)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        if raw.is_empty() {
            continue;
        }
        // De-dup identical literals (common when the same bad wikilink appears
        // more than once on a page).
        if seen.insert(raw.clone()) {
            errors.push(PageError::UnresolvedWikilink { raw });
        }
    }

    errors
}

/// Wraps a captured YAML frontmatter parse error (from
/// [`crate::markdown::MarkdownRenderResult::frontmatter_error`]) into the
/// page-error list. Returns an empty vec when there was no error.
pub fn frontmatter_parse_errors(err: &Option<String>) -> Vec<PageError> {
    err.iter()
        .map(|message| PageError::FrontmatterParseError {
            message: message.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn make_config<'a>(
        base_dir: &'a Path,
        exts: &'a [String],
        index_file: &'a str,
        tag_sources: &'a [String],
    ) -> PathResolverConfig<'a> {
        PathResolverConfig {
            base_dir,
            canonical_base_dir: None,
            static_folder: "static",
            markdown_extensions: exts,
            index_file,
            tag_sources,
        }
    }

    // --- validate_internal_links -------------------------------------------

    #[test]
    fn broken_internal_link_is_reported() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let outbound = vec![OutboundLink {
            to: "/nonexistent/".to_string(),
            text: "bad".to_string(),
            anchor: None,
            internal: true,
        }];

        let errs = validate_internal_links(&outbound, &cfg);
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            PageError::BrokenInternalLink { target, .. } if target == "/nonexistent/"
        ));
    }

    #[test]
    fn valid_internal_link_is_ignored() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("page.md"), "# x").unwrap();
        let base = dir.path().canonicalize().unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let outbound = vec![OutboundLink {
            to: "/page/".to_string(),
            text: "ok".to_string(),
            anchor: None,
            internal: true,
        }];

        let errs = validate_internal_links(&outbound, &cfg);
        assert!(errs.is_empty());
    }

    #[test]
    fn external_link_is_ignored() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let outbound = vec![OutboundLink {
            to: "https://example.com/anything".to_string(),
            text: "ext".to_string(),
            anchor: None,
            internal: false,
        }];

        let errs = validate_internal_links(&outbound, &cfg);
        assert!(errs.is_empty());
    }

    #[test]
    fn fragment_only_link_is_ignored() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let outbound = vec![OutboundLink {
            to: "#section".to_string(),
            text: "anchor".to_string(),
            anchor: Some("#section".to_string()),
            internal: true,
        }];

        let errs = validate_internal_links(&outbound, &cfg);
        assert!(errs.is_empty());
    }

    #[test]
    fn percent_encoded_link_to_existing_file_is_not_reported() {
        // Regression: axum percent-decodes live request paths, so an authored
        // href like /IronCore%20Swag%20T-shirts%20Gifts must be decoded before
        // resolution or the checker reports a bogus 404.
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("IronCore Swag T-shirts Gifts.md"), "# x").unwrap();
        let base = dir.path().canonicalize().unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let outbound = vec![
            OutboundLink {
                to: "/IronCore%20Swag%20T-shirts%20Gifts".to_string(),
                text: "no trailing slash".to_string(),
                anchor: None,
                internal: true,
            },
            OutboundLink {
                to: "/IronCore%20Swag%20T-shirts%20Gifts/".to_string(),
                text: "trailing slash".to_string(),
                anchor: None,
                internal: true,
            },
        ];

        let errs = validate_internal_links(&outbound, &cfg);
        assert!(errs.is_empty(), "expected no errors, got: {:?}", errs);
    }

    #[test]
    fn percent_encoded_apostrophe_link_is_not_reported() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("World's Best.md"), "# x").unwrap();
        let base = dir.path().canonicalize().unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let outbound = vec![OutboundLink {
            to: "/World%27s%20Best/".to_string(),
            text: "apostrophe".to_string(),
            anchor: None,
            internal: true,
        }];

        let errs = validate_internal_links(&outbound, &cfg);
        assert!(errs.is_empty(), "expected no errors, got: {:?}", errs);
    }

    #[test]
    fn percent_encoded_unicode_link_is_not_reported() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("café.md"), "# x").unwrap();
        let base = dir.path().canonicalize().unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let outbound = vec![OutboundLink {
            to: "/caf%C3%A9/".to_string(),
            text: "unicode".to_string(),
            anchor: None,
            internal: true,
        }];

        let errs = validate_internal_links(&outbound, &cfg);
        assert!(errs.is_empty(), "expected no errors, got: {:?}", errs);
    }

    #[test]
    fn percent_encoded_link_with_anchor_and_query_is_not_reported() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("IronCore Swag T-shirts Gifts.md"), "# x").unwrap();
        let base = dir.path().canonicalize().unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let outbound = vec![OutboundLink {
            to: "/IronCore%20Swag%20T-shirts%20Gifts/?x=1#top".to_string(),
            text: "anchor and query".to_string(),
            anchor: Some("#top".to_string()),
            internal: true,
        }];

        let errs = validate_internal_links(&outbound, &cfg);
        assert!(errs.is_empty(), "expected no errors, got: {:?}", errs);
    }

    #[test]
    fn missing_percent_encoded_target_is_still_reported() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let outbound = vec![OutboundLink {
            to: "/Nope%20Missing/".to_string(),
            text: "gone".to_string(),
            anchor: None,
            internal: true,
        }];

        let errs = validate_internal_links(&outbound, &cfg);
        assert_eq!(errs.len(), 1);
        // The error payload preserves the authored (still-encoded) target.
        assert!(matches!(
            &errs[0],
            PageError::BrokenInternalLink { target, .. } if target == "/Nope%20Missing/"
        ));
    }

    // --- validate_media_references -----------------------------------------

    fn media_setup() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        // Pre-create a known image so valid media resolves.
        std::fs::write(base.join("photo.png"), b"\x89PNG").unwrap();
        std::fs::create_dir_all(base.join("static/images")).unwrap();
        std::fs::write(base.join("static/images/ok.png"), b"\x89PNG").unwrap();
        (dir, base)
    }

    #[test]
    fn broken_img_is_reported() {
        let (_guard, base) = media_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        // Root-level page: srcs resolve against the site root URL "/".
        let html = r#"<p><img src="./missing.png" alt="x"></p>"#;
        let errs = validate_media_references(html, &cfg, "/");
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            PageError::BrokenMediaReference { kind: MediaKind::Image, src } if src == "./missing.png"
        ));
    }

    #[test]
    fn valid_img_is_ignored() {
        let (_guard, base) = media_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<p><img src="photo.png" alt="x"></p>"#;
        let errs = validate_media_references(html, &cfg, "/");
        assert!(errs.is_empty());
    }

    #[test]
    fn absolute_path_image_under_static_is_ignored() {
        let (_guard, base) = media_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        // "/images/ok.png" should resolve via the static folder overlay,
        // regardless of which page references it.
        let html = r#"<p><img src="/images/ok.png" alt="x"></p>"#;
        let errs = validate_media_references(html, &cfg, "/docs/guide/");
        assert!(errs.is_empty());
    }

    #[test]
    fn external_image_is_ignored() {
        let (_guard, base) = media_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<p><img src="https://example.com/a.png"></p>"#;
        let errs = validate_media_references(html, &cfg, "/");
        assert!(errs.is_empty());
    }

    #[test]
    fn broken_video_audio_and_source_are_reported() {
        let (_guard, base) = media_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"
            <video src="./gone.mp4"></video>
            <audio src="./gone.mp3"></audio>
            <video><source src="./gone.webm"></video>
        "#;
        let errs = validate_media_references(html, &cfg, "/");

        assert!(
            errs.iter().any(|e| matches!(
                e,
                PageError::BrokenMediaReference {
                    kind: MediaKind::Video,
                    ..
                }
            )),
            "{:?}",
            errs
        );
        assert!(errs.iter().any(|e| matches!(
            e,
            PageError::BrokenMediaReference {
                kind: MediaKind::Audio,
                ..
            }
        )));
        assert!(errs.iter().any(|e| matches!(
            e,
            PageError::BrokenMediaReference {
                kind: MediaKind::Source,
                ..
            }
        )));
    }

    #[test]
    fn percent_encoded_relative_img_next_to_markdown_is_ignored() {
        // Encoded relative srcs must resolve against the page URL first and
        // percent-decode afterwards, matching axum's live-request decoding.
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::write(base.join("my photo.png"), b"\x89PNG").unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<p><img src="./my%20photo.png" alt="x"></p>"#;
        let errs = validate_media_references(html, &cfg, "/");
        assert!(errs.is_empty(), "expected no errors, got: {:?}", errs);
    }

    /// Layout matching the real-world false-positive report: an attachments
    /// folder sitting next to the markdown file, referenced from a page
    /// served at a directory-style URL. The server-mode link transform
    /// rewrites the src to `../<attachments>/...` relative to the page URL.
    fn attachments_setup() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let attachments = base.join("Projects/Ideas/hledger-web-gui_attachments");
        std::fs::create_dir_all(&attachments).unwrap();
        std::fs::write(base.join("Projects/Ideas/hledger-web-gui.md"), "# x").unwrap();
        std::fs::write(attachments.join("img.png"), b"\x89PNG").unwrap();
        std::fs::write(attachments.join("my photo.png"), b"\x89PNG").unwrap();
        (dir, base)
    }

    #[test]
    fn parent_relative_img_next_to_markdown_is_ignored() {
        // Regression: the checker used to resolve `../` srcs against the
        // markdown file's directory (one level too high) and falsely flag
        // images that the browser loads fine via the page URL.
        let (_guard, base) = attachments_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<p><img src="../hledger-web-gui_attachments/img.png" alt="x"></p>"#;
        let errs = validate_media_references(html, &cfg, "/Projects/Ideas/hledger-web-gui/");
        assert!(errs.is_empty(), "expected no errors, got: {:?}", errs);
    }

    #[test]
    fn parent_relative_percent_encoded_img_is_ignored() {
        let (_guard, base) = attachments_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<p><img src="../hledger-web-gui_attachments/my%20photo.png" alt="x"></p>"#;
        let errs = validate_media_references(html, &cfg, "/Projects/Ideas/hledger-web-gui/");
        assert!(errs.is_empty(), "expected no errors, got: {:?}", errs);
    }

    #[test]
    fn index_page_sibling_img_is_ignored() {
        // docs/index.md is served at /docs/, so a plain "img.png" src loads
        // docs/img.png in the browser.
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("docs")).unwrap();
        std::fs::write(base.join("docs/index.md"), "# x").unwrap();
        std::fs::write(base.join("docs/img.png"), b"\x89PNG").unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<p><img src="img.png" alt="x"></p>"#;
        let errs = validate_media_references(html, &cfg, "/docs/");
        assert!(errs.is_empty(), "expected no errors, got: {:?}", errs);
    }

    #[test]
    fn parent_relative_missing_img_is_reported() {
        // A genuinely missing file reached via `../` must still be flagged.
        let (_guard, base) = attachments_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<p><img src="../hledger-web-gui_attachments/nope.png" alt="x"></p>"#;
        let errs = validate_media_references(html, &cfg, "/Projects/Ideas/hledger-web-gui/");
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            PageError::BrokenMediaReference { kind: MediaKind::Image, src }
                if src == "../hledger-web-gui_attachments/nope.png"
        ));
    }

    // --- detect_unresolved_wikilinks --------------------------------------

    #[test]
    fn literal_wikilink_in_body_is_reported() {
        let html = "<p>See [[never-a-real-page]] for more.</p>";
        let errs = detect_unresolved_wikilinks(html);
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            PageError::UnresolvedWikilink { raw } if raw == "[[never-a-real-page]]"
        ));
    }

    #[test]
    fn transformed_wikilink_yields_no_match() {
        // Once `transform_wikilinks` has resolved it, only a normal anchor
        // remains and the regex must not match.
        let html = r#"<p><a href="/tags/rust/">rust</a></p>"#;
        let errs = detect_unresolved_wikilinks(html);
        assert!(errs.is_empty());
    }

    #[test]
    fn wikilink_inside_code_block_is_ignored() {
        let html =
            "<p>Regular</p><pre><code>This is a literal [[bracket]] inside code</code></pre>";
        let errs = detect_unresolved_wikilinks(html);
        assert!(
            errs.is_empty(),
            "expected no wikilink errors inside code/pre, got: {:?}",
            errs
        );
    }

    #[test]
    fn wikilink_inside_inline_code_is_ignored() {
        let html = "<p>See <code>[[foo]]</code> for the literal syntax.</p>";
        let errs = detect_unresolved_wikilinks(html);
        assert!(errs.is_empty());
    }

    #[test]
    fn repeated_wikilink_is_deduped() {
        let html = "<p>[[bad]] again [[bad]] and [[bad]]</p>";
        let errs = detect_unresolved_wikilinks(html);
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn wikilink_with_display_text_is_reported_verbatim() {
        let html = "<p>[[Target|Display Text]]</p>";
        let errs = detect_unresolved_wikilinks(html);
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            PageError::UnresolvedWikilink { raw } if raw == "[[Target|Display Text]]"
        ));
    }

    // --- Serialization ----------------------------------------------------

    #[test]
    fn page_error_serializes_with_snake_case_type_tag() {
        let err = PageError::BrokenInternalLink {
            target: "/x/".to_string(),
            text: "x".to_string(),
            anchor: None,
        };
        let json = serde_json::to_string(&err).unwrap();
        assert!(
            json.contains("\"type\":\"broken_internal_link\""),
            "{}",
            json
        );
        // anchor is None, so it should be skipped
        assert!(!json.contains("\"anchor\""), "{}", json);
    }

    #[test]
    fn media_kind_serializes_as_snake_case() {
        let err = PageError::BrokenMediaReference {
            src: "./x.png".to_string(),
            kind: MediaKind::Image,
        };
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("\"kind\":\"image\""), "{}", json);
        assert!(
            json.contains("\"type\":\"broken_media_reference\""),
            "{}",
            json
        );
    }

    #[test]
    fn unresolved_wikilink_serializes() {
        let err = PageError::UnresolvedWikilink {
            raw: "[[foo]]".to_string(),
        };
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("\"type\":\"unresolved_wikilink\""));
        assert!(json.contains("\"raw\":\"[[foo]]\""));
    }

    #[test]
    fn frontmatter_parse_error_serializes() {
        let err = PageError::FrontmatterParseError {
            message: "mapping values are not allowed".to_string(),
        };
        let json = serde_json::to_string(&err).unwrap();
        assert!(
            json.contains("\"type\":\"frontmatter_parse_error\""),
            "{}",
            json
        );
        assert!(json.contains("mapping values are not allowed"), "{}", json);
    }

    #[test]
    fn frontmatter_parse_errors_none_is_empty() {
        assert!(frontmatter_parse_errors(&None).is_empty());
    }

    #[test]
    fn frontmatter_parse_errors_some_yields_one() {
        let errs = frontmatter_parse_errors(&Some("bad yaml".to_string()));
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            PageError::FrontmatterParseError { message } if message == "bad yaml"
        ));
    }

    #[test]
    fn page_errors_empty_default_serializes() {
        let pe = PageErrors {
            page_url: "/x/".to_string(),
            errors: vec![],
        };
        let json = serde_json::to_string(&pe).unwrap();
        assert!(json.contains("\"page_url\":\"/x/\""));
        assert!(json.contains("\"errors\":[]"));
    }
}
