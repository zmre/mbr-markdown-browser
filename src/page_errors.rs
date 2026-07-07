//! Per-page error detection for server/GUI mode.
//!
//! Validates the problems a reader might care about for a single rendered page:
//!
//! 1. Broken internal links — reuse `OutboundLink` data from `LinkCache` and
//!    resolve each through `path_resolver::resolve_request_path`.
//! 2. Broken media references — parse rendered HTML for `<img>`, `<video>`,
//!    `<audio>`, and `<source>` tags and confirm internal `src` attributes
//!    resolve.
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
use std::path::Path;
use std::sync::LazyLock;

use crate::link_index::OutboundLink;
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
/// For each internal `src`, we strip anchors / queries, URL-decode, and resolve
/// relative paths against the markdown file's directory. A match is recorded
/// when the target neither exists on disk nor is served by the resolver.
pub fn validate_media_references(
    html: &str,
    resolver_config: &PathResolverConfig,
    markdown_dir: &Path,
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

            if media_reference_resolves(src, resolver_config, markdown_dir) {
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

/// Resolves a media `src` to check existence. Tries three strategies in order:
///
/// 1. Path resolver (covers files in base_dir, static folder, tag pages).
/// 2. Direct filesystem check against the markdown file's parent directory
///    (handles relative paths that live next to the source markdown).
/// 3. Direct filesystem check against the base directory (handles absolute
///    site-root paths).
fn media_reference_resolves(
    src: &str,
    resolver_config: &PathResolverConfig,
    markdown_dir: &Path,
) -> bool {
    // Normalize (strip anchor/query, percent-decode, trim slashes) exactly as
    // a live HTTP request would be. See `normalize_link_target`.
    let cleaned = normalize_link_target(src);

    if cleaned.is_empty() {
        return true;
    }

    // Strategy 1: path resolver (this handles static folder overlays, index
    // files, etc.). `resolve_request_path` expects no leading slash.
    match resolve_request_path(resolver_config, &cleaned) {
        ResolvedPath::StaticFile(_)
        | ResolvedPath::MarkdownFile(_)
        | ResolvedPath::TagPage { .. }
        | ResolvedPath::TagSourceIndex { .. }
        | ResolvedPath::Redirect(_)
        | ResolvedPath::DirectoryListing(_) => return true,
        ResolvedPath::NotFound => {}
    }

    // Strategy 2 / 3: explicit filesystem probes. These guard against edge
    // cases in the resolver (e.g. hidden / dotfiles not served). The leading
    // slash on the authored src (checked before normalization trims it)
    // decides whether the path is site-root-absolute or relative to the
    // markdown file.
    let candidate = if src.starts_with('/') {
        resolver_config.base_dir.join(&cleaned)
    } else {
        markdown_dir.join(&cleaned)
    };
    candidate.exists()
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
    use std::path::PathBuf;
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

        let html = r#"<p><img src="./missing.png" alt="x"></p>"#;
        let errs = validate_media_references(html, &cfg, &base);
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
        let errs = validate_media_references(html, &cfg, &base);
        assert!(errs.is_empty());
    }

    #[test]
    fn absolute_path_image_under_static_is_ignored() {
        let (_guard, base) = media_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        // "/images/ok.png" should resolve via the static folder overlay.
        let html = r#"<p><img src="/images/ok.png" alt="x"></p>"#;
        let errs = validate_media_references(html, &cfg, &base);
        assert!(errs.is_empty());
    }

    #[test]
    fn external_image_is_ignored() {
        let (_guard, base) = media_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<p><img src="https://example.com/a.png"></p>"#;
        let errs = validate_media_references(html, &cfg, &base);
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
        let errs = validate_media_references(html, &cfg, &base);

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
        // Guards the media_reference_resolves refactor: encoded relative srcs
        // must decode and probe the markdown file's directory.
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::write(base.join("my photo.png"), b"\x89PNG").unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<p><img src="./my%20photo.png" alt="x"></p>"#;
        let errs = validate_media_references(html, &cfg, &base);
        assert!(errs.is_empty(), "expected no errors, got: {:?}", errs);
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
