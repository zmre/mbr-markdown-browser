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
//! 4. Frontmatter parse errors — a YAML error discards the *whole* block, so
//!    otherwise-valid fields vanish; the captured message is surfaced verbatim.
//! 5. Relationship data problems — hierarchical (parent/child) cycles and
//!    endpoints that resolved through a name shared by several notes. Both are
//!    detected once per index rebuild (see `src/relationships.rs`) and only
//!    looked up here.
//! 6. Ambiguous body wikilinks — a `[[Name]]` several notes answer to, detected
//!    during the render (see `src/wikilink_index.rs`).
//!
//! Designed to be cheap: each validator is a pure function and is expected to
//! run on-demand for a single page render. The module is never invoked from
//! `src/build.rs`, keeping static-site output untouched.

use regex::Regex;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::link_index::{OutboundLink, resolve_relative_url, resolve_relative_url_checked};
use crate::path_resolver::{
    PathResolverConfig, ResolvedPath, normalize_link_target, resolve_request_path,
};
use crate::relationships::{AmbiguousEndpoint, RelationshipCycle};
use crate::url_path::is_external_url;
use crate::wikilink_index::AmbiguousWikilink;

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
    /// Two or more notes form a parent/child cycle, which is impossible in a real
    /// family tree and makes the genealogy chart unrenderable.
    RelationshipCycle {
        /// Note URLs in the cycle, in traversal order.
        members: Vec<String>,
        /// Canonical relation type whose edges close the cycle (e.g. "child").
        rel_type: String,
    },
    /// A relationship endpoint named a title/alias shared by several notes; mbr
    /// silently resolved it to one of them.
    AmbiguousRelationshipEndpoint {
        /// The endpoint as authored, e.g. "[[John Doe]]".
        raw: String,
        /// The note URL it resolved to.
        resolved_to: String,
        /// The other notes sharing that name.
        candidates: Vec<String>,
    },
    /// A `[[Wikilink]]` in the body named a title/stem shared by several notes;
    /// mbr silently resolved it to one of them.
    AmbiguousWikilink {
        /// The wikilink as authored, e.g. "[[John Doe]]".
        raw: String,
        /// The note URL it resolved to.
        resolved_to: String,
        /// The other notes sharing that name.
        candidates: Vec<String>,
    },
    /// A media file that resolves and is served correctly, but whose track
    /// layout matches a combination implicated in browser decode failures.
    /// Detected by probing the container (see
    /// `video_metadata::probe_playback_compatibility`), so it is server/GUI
    /// only and requires the `media-metadata` feature.
    ///
    /// This is a *heuristic hint*, not a verdict — the combination it looks for
    /// is necessary but not sufficient, with a known false positive (see
    /// `video_metadata::WEBKIT_RISKY_DATA_TAGS`). The browser's own
    /// `MediaError` is the only ground truth for "this did not play", so the
    /// frontend surfaces these entries solely to explain a failure it has
    /// already observed, and never as a standalone warning.
    ///
    /// Unlike [`Self::BrokenMediaReference`], `kind` describes the *media
    /// type* (`video`) rather than the HTML element, because the diagnosis is
    /// about the file, not about which tag referenced it.
    UnplayableMedia {
        /// Matches the `src` attribute the frontend sees on the corresponding
        /// `<source>` / `<mbr-video-extras>` element, with any `#fragment` or
        /// `?query` removed so both spellings compare equal.
        src: String,
        kind: MediaKind,
        /// Human-readable explanation of the most likely cause.
        reason: String,
        /// Copy-pasteable command that resolves the conflict, when one exists.
        #[serde(skip_serializing_if = "Option::is_none")]
        remedy: Option<String>,
        /// Always `true`: marks this entry as a heuristic hint so the frontend
        /// keeps it out of the error badge until a real playback failure is
        /// observed for the same `src`.
        advisory: bool,
    },
}

/// A media reference in the rendered HTML that resolves to a real file on disk.
///
/// Produced by [`collect_media_references`] so callers can inspect the actual
/// bytes (e.g. probe a container for undecodable tracks) while still reporting
/// the `src` string the browser sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaReference {
    /// The element's `src` with `#fragment` / `?query` stripped. This is the
    /// join key the frontend uses to match an element to its diagnosis.
    pub src: String,
    /// Which element carried the reference.
    pub kind: MediaKind,
    /// The file the server would actually serve for this `src`.
    pub path: std::path::PathBuf,
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

/// What is wrong with one link, as judged from the href a browser would
/// actually follow.
///
/// All three surface as [`PageError::BrokenInternalLink`]: the wire format is a
/// pinned contract with `components/src/mbr-page-errors.ts`, and a variant that
/// file does not know about would be counted by nothing and rendered by
/// nothing — an error that "exists" but is invisible is worse than no error.
/// Every one of these does break a reader's navigation, so the label is honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkDefect {
    /// The target does not resolve to anything the server would serve.
    Missing,
    /// A `..` climbed above the repository root. Browsers clamp this (RFC 3986
    /// §5.2.4), so the reader silently lands on an unrelated page instead of
    /// the one the author meant.
    EscapesRoot,
    /// The target is a *markdown page*, but the href does not end in `/`.
    /// Markdown pages live at directory-style URLs; without the slash the
    /// browser resolves the target page's own relative links one directory too
    /// high, so the damage lands on the *next* click, not this one. In server
    /// mode a redirect repairs it; in a static build nothing does.
    ///
    /// Deliberately **markdown-only**. Every other thing the resolver serves at
    /// a directory-style URL — a directory listing, a tag page, a tag source
    /// index — is rendered from a template whose links are site-absolute
    /// (server mode) or root-anchored `../`-chains (build mode), and neither
    /// form depends on the base's trailing slash. Only a markdown *body*
    /// carries links authored relative to the page's own location, so only a
    /// markdown target can suffer the next-click damage this variant predicts.
    /// See [`classify_emitted_href`] for why widening it also breaks the
    /// checker's central invariant.
    NonCanonical,
}

/// Judges one **already-emitted** href exactly as a browser would.
///
/// The distinction from validating the *authored* markdown destination is the
/// whole point: a link transform bug — the wrong number of `../`, a missing
/// trailing slash — is invisible to a checker that re-derives the target from
/// the source text, because it re-applies the same (buggy) rules. This reads
/// what actually went into the HTML.
///
/// `page_url` is the page's canonical directory-style URL, so relative
/// resolution uses `is_index_file = true`: every segment of a URL ending in `/`
/// is a real directory component. That is the same rationale
/// [`media_reference_resolves`] documents for media srcs.
///
/// # The trailing-slash check must use the *renderer's* notion of "page"
///
/// Only one thing puts a trailing slash on an emitted href:
/// `link_transform::transform_link`, via
/// [`crate::link_transform::LinkTransformConfig::markdown_page_probe`], which
/// answers yes for [`ResolvedPath::MarkdownFile`] and nothing else. So
/// [`LinkDefect::NonCanonical`] is checked for exactly that one variant. Asking
/// for a slash on any other kind reports a defect the pipeline is structurally
/// incapable of producing — the checker flagging its own renderer's output,
/// which is the failure this module's header claims cannot happen.
///
/// It would also be wrong on the merits. `/tags/rust`, `/tags` and a directory
/// listing are all served **200 in place**, with no redirect
/// (`path_resolver::canonical_page_redirect` fires only for markdown pages), and
/// the templates behind them emit site-absolute links in server mode and
/// root-anchored `../`-chains in build mode. Both forms resolve identically
/// whether or not the base URL ends in `/`, so there is no next-click damage to
/// warn about. [`ResolvedPath::Redirect`] is likewise safe: the browser is sent
/// to the canonical URL before it resolves anything.
fn classify_emitted_href(
    href: &str,
    resolver_config: &PathResolverConfig,
    page_url: &str,
) -> Option<LinkDefect> {
    // Fragment-only links address the current page; validating them would need
    // target-page parsing, so they are skipped to avoid false positives.
    if href.is_empty() || href.starts_with('#') {
        return None;
    }

    // Off-site targets are not ours to resolve. Anything with a scheme
    // (`ftp://…`, `magnet:…`) would otherwise be joined onto the base directory
    // and reported as a false broken link.
    if is_external_url(href) {
        return None;
    }

    // Fragment / query must come off before relative resolution so their
    // payloads never participate in `.` / `..` segment handling.
    let path_part = href.split(['#', '?']).next().unwrap_or_default();
    if path_part.is_empty() {
        return None;
    }

    // `_checked`, not the clamping resolver: an above-root `..` is a real
    // authoring defect and clamping it turns the link into one that points at
    // whatever page happens to sit at the clamped location.
    let Some(absolute_url) = resolve_relative_url_checked(page_url, path_part, true) else {
        return Some(LinkDefect::EscapesRoot);
    };

    // Normalize (percent-decode, trim slashes) and resolve through the same
    // pipeline a live HTTP request hits, so this can never disagree with what
    // the server serves. See `normalize_link_target` for why the decoding has
    // to match axum's.
    let request_path = normalize_link_target(&absolute_url);
    match resolve_request_path(resolver_config, &request_path) {
        ResolvedPath::NotFound => Some(LinkDefect::Missing),
        // The one target whose body links are authored relative to its own
        // location, and the one target the renderer can spell canonically.
        // `normalize_link_target` deliberately trims slashes, which makes the
        // two spellings indistinguishable at that layer — so the check has to
        // be made here, on the raw href.
        ResolvedPath::MarkdownFile(_) => {
            (!path_part.ends_with('/')).then_some(LinkDefect::NonCanonical)
        }
        // A static file is addressed by its exact name and must NOT gain a
        // trailing slash. Directory listings, tag pages, tag source indexes and
        // `/x/index`-style redirects are all canonical however they were
        // written — see the doc comment above.
        _ => None,
    }
}

/// Validates every `<a href>` in the page's **rendered** HTML.
///
/// This is the layer that can see link-transform defects, because it reads the
/// href that was emitted rather than re-deriving one from the markdown source.
/// Prefer it over [`validate_internal_links`] wherever the rendered body is
/// available.
pub fn validate_rendered_links(
    html: &str,
    resolver_config: &PathResolverConfig,
    page_url: &str,
) -> Vec<PageError> {
    let Ok(selector) = Selector::parse("a[href]") else {
        return Vec::new();
    };
    let doc = Html::parse_document(html);
    let mut seen = std::collections::HashSet::new();

    doc.select(&selector)
        .filter_map(|element| {
            let href = element.value().attr("href")?;
            if !seen.insert(href.to_string()) {
                return None;
            }
            classify_emitted_href(href, resolver_config, page_url)?;
            let text = element.text().collect::<String>().trim().to_string();
            let (_, anchor) = crate::link_index::split_url_anchor(href);
            Some(PageError::BrokenInternalLink {
                target: href.to_string(),
                text,
                anchor,
            })
        })
        .collect()
}

/// Validates the internal outbound links for a single page.
///
/// Used where there is no rendered body to read hrefs from — tag pages and tag
/// indexes, whose outbound links are synthesized as absolute site URLs rather
/// than authored. For markdown pages use [`validate_rendered_links`], which
/// sees what the renderer actually emitted.
pub fn validate_internal_links(
    outbound: &[OutboundLink],
    resolver_config: &PathResolverConfig,
) -> Vec<PageError> {
    outbound
        .iter()
        .filter(|link| link.internal)
        .filter(|link| {
            // These targets are already absolute site URLs, so the site root is
            // the correct base to resolve them against.
            classify_emitted_href(&link.to, resolver_config, "/").is_some_and(|defect| {
                // Only genuine 404s are reported here. `OutboundLink.to` for a
                // markdown page holds the *authored* destination resolved with
                // markdown semantics (`beta.md` -> `/docs/beta.md/`), which is
                // not the href a browser follows — judging its canonicality
                // would be judging a string nobody navigates to.
                matches!(defect, LinkDefect::Missing | LinkDefect::EscapesRoot)
            })
        })
        .map(|link| PageError::BrokenInternalLink {
            target: link.to.clone(),
            text: link.text.clone(),
            anchor: link.anchor.clone(),
        })
        .collect()
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
    scan_media_elements(html)
        .into_iter()
        .filter(|(src, _)| !media_reference_resolves(src, resolver_config, page_url))
        .map(|(src, kind)| PageError::BrokenMediaReference { src, kind })
        .collect()
}

/// Scans rendered HTML for every internal media reference, yielding
/// `(src, kind)` pairs with the `src` verbatim as authored.
///
/// External (`https:`, `data:`, …) and empty `src` values are dropped because
/// nothing local can be said about them. Parsing the document is
/// ~microseconds for typical page sizes; the selectors compile once per call,
/// which keeps the API ergonomic and the endpoint off any hot path.
fn scan_media_elements(html: &str) -> Vec<(String, MediaKind)> {
    let doc = Html::parse_document(html);

    let specs: [(&str, MediaKind); 4] = [
        ("img[src]", MediaKind::Image),
        ("video[src]", MediaKind::Video),
        ("audio[src]", MediaKind::Audio),
        ("source[src]", MediaKind::Source),
    ];

    specs
        .into_iter()
        .filter_map(|(selector_str, kind)| {
            Selector::parse(selector_str).ok().map(|sel| (sel, kind))
        })
        .flat_map(|(selector, kind)| {
            doc.select(&selector)
                .filter_map(|el| el.value().attr("src"))
                .filter(|src| !src.is_empty() && !is_external_url(src))
                .map(|src| (src.to_string(), kind.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Resolves every internal media reference in `html` to the file the server
/// would actually serve, dropping the ones that do not resolve to a static
/// file (those are already reported by [`validate_media_references`]).
///
/// The returned `src` has its `#fragment` / `?query` stripped, so a
/// `<source src='…mp4#t=30,200'>` and the sibling
/// `<mbr-video-extras src='…mp4'>` produce the same join key for the frontend.
///
/// References are deduplicated by `src`, so a file embedded twice on one page
/// yields one entry and callers never probe the same bytes twice.
pub fn collect_media_references(
    html: &str,
    resolver_config: &PathResolverConfig,
    page_url: &str,
) -> Vec<MediaReference> {
    let mut seen = std::collections::HashSet::new();

    scan_media_elements(html)
        .into_iter()
        .filter_map(|(src, kind)| {
            let path_part = src.split(['#', '?']).next().unwrap_or_default();
            if path_part.is_empty() || !seen.insert(path_part.to_string()) {
                return None;
            }
            let absolute_url = resolve_relative_url(page_url, path_part, true);
            let request_path = normalize_link_target(&absolute_url);
            match resolve_request_path(resolver_config, &request_path) {
                ResolvedPath::StaticFile(path) => Some(MediaReference {
                    src: path_part.to_string(),
                    kind,
                    path,
                }),
                _ => None,
            }
        })
        .collect()
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

/// Wraps the hierarchical relationship cycles a page is a member of (from
/// [`crate::relationships::RelationshipIndex::cycles_for`]) into the page-error
/// list.
///
/// Attached to every note *in* the cycle rather than to whichever note declared
/// the closing edge: the data is wrong across the whole loop, and any member is
/// a valid place to break it.
pub fn relationship_cycle_errors(cycles: &[RelationshipCycle]) -> Vec<PageError> {
    cycles
        .iter()
        .map(|cycle| PageError::RelationshipCycle {
            members: cycle.members.clone(),
            rel_type: cycle.rel_type.clone(),
        })
        .collect()
}

/// Wraps the ambiguous relationship endpoints a page *declared* (from
/// [`crate::relationships::RelationshipIndex::ambiguous_endpoints_for`]) into
/// the page-error list.
pub fn ambiguous_relationship_endpoint_errors(endpoints: &[AmbiguousEndpoint]) -> Vec<PageError> {
    endpoints
        .iter()
        .map(|endpoint| PageError::AmbiguousRelationshipEndpoint {
            raw: endpoint.raw.clone(),
            resolved_to: endpoint.resolved_to.clone(),
            candidates: endpoint.candidates.clone(),
        })
        .collect()
}

/// Wraps the ambiguous body wikilinks found while rendering a page (from
/// [`crate::markdown::MarkdownRenderResult::ambiguous_wikilinks`]) into the
/// page-error list.
pub fn ambiguous_wikilink_errors(wikilinks: &[AmbiguousWikilink]) -> Vec<PageError> {
    wikilinks
        .iter()
        .map(|wikilink| PageError::AmbiguousWikilink {
            raw: wikilink.raw.clone(),
            resolved_to: wikilink.resolved_to.clone(),
            candidates: wikilink.candidates.clone(),
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
    fn external_scheme_link_marked_internal_is_ignored() {
        // Regression: `is_internal_link` did not know these schemes, so the
        // renderer stored them with `internal: true`; this validator then ran
        // them through the path resolver, got `NotFound`, and showed a false
        // "broken internal link" in the page-errors panel.
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let outbound = [
            "ftp://example.com/data.zip",
            "magnet:?xt=urn:btih:abc",
            "sms:+15555550123",
        ]
        .iter()
        .map(|to| OutboundLink {
            to: (*to).to_string(),
            text: "ext".to_string(),
            anchor: None,
            // Deliberately mislabelled, as the old renderer did.
            internal: true,
        })
        .collect::<Vec<_>>();

        let errs = validate_internal_links(&outbound, &cfg);
        assert!(errs.is_empty(), "expected no errors, got: {:?}", errs);
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

    // --- validate_rendered_links -------------------------------------------

    /// The layout from the reported bug: `docs/guide.md` links across to
    /// `folder/file.md`, which has a sibling.
    fn rendered_link_setup() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("docs")).unwrap();
        std::fs::create_dir_all(base.join("folder")).unwrap();
        std::fs::write(base.join("docs/guide.md"), "# guide").unwrap();
        std::fs::write(base.join("folder/file.md"), "# file").unwrap();
        std::fs::write(base.join("folder/sibling.md"), "# sibling").unwrap();
        std::fs::write(base.join("LICENSE"), "MIT").unwrap();
        (dir, base)
    }

    fn rendered_link_errors(html: &str, base: &Path, page_url: &str) -> Vec<PageError> {
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(base, &exts, "index.md", &tags);
        validate_rendered_links(html, &cfg, page_url)
    }

    /// The reported defect. `/folder/file` serves 200, so nothing downstream
    /// looked wrong — but the browser then resolves that page's own relative
    /// links against `/folder/` and every one of them 404s. The href, not the
    /// authored destination, is the only place this is visible.
    #[test]
    fn page_link_without_trailing_slash_is_reported() {
        let (_guard, base) = rendered_link_setup();
        let html = r#"<a href="../../folder/file">file</a>"#;
        let errs = rendered_link_errors(html, &base, "/docs/guide/");

        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(matches!(
            &errs[0],
            PageError::BrokenInternalLink { target, text, .. }
                if target == "../../folder/file" && text == "file"
        ));
    }

    #[test]
    fn page_link_with_trailing_slash_is_ignored() {
        let (_guard, base) = rendered_link_setup();
        let html = r#"<a href="../../folder/file/">file</a>"#;
        assert!(
            rendered_link_errors(html, &base, "/docs/guide/").is_empty(),
            "the canonical spelling must not be flagged"
        );
    }

    /// A static file is addressed by its exact name; demanding a trailing slash
    /// would be the mirror-image bug.
    #[test]
    fn extensionless_static_file_link_is_ignored() {
        let (_guard, base) = rendered_link_setup();
        let html = r#"<a href="../../LICENSE">License</a>"#;
        assert!(
            rendered_link_errors(html, &base, "/docs/guide/").is_empty(),
            "an extension-less static file must not be asked for a trailing slash"
        );
    }

    /// Regression: `normalize_link_target` trims slashes, so `/folder/file` and
    /// `/folder/file/` collapse to the same string and the defect was
    /// structurally invisible to anything checking only "does it resolve".
    #[test]
    fn resolving_alone_cannot_tell_the_two_spellings_apart() {
        assert_eq!(
            normalize_link_target("/folder/file"),
            normalize_link_target("/folder/file/"),
            "if this ever differs, the trailing-slash check can move earlier"
        );
    }

    /// A `..` that climbs out of the repository produced no diagnostic at all:
    /// `resolve_relative_url` popped an empty segment stack and discarded the
    /// `None`, so the link silently became one to a real page.
    #[test]
    fn above_root_link_is_reported() {
        let (_guard, base) = rendered_link_setup();
        let html = r#"<a href="../../../../escape/target/">escape</a>"#;
        let errs = rendered_link_errors(html, &base, "/docs/guide/");

        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(matches!(
            &errs[0],
            PageError::BrokenInternalLink { target, .. } if target == "../../../../escape/target/"
        ));
    }

    /// An above-root link whose *clamped* destination happens to exist is the
    /// nastiest shape: the reader lands on a real, wrong page and nothing
    /// complains.
    #[test]
    fn above_root_link_to_an_existing_clamped_target_is_still_reported() {
        let (_guard, base) = rendered_link_setup();
        // From /docs/guide/, `../../../folder/file/` climbs one level too far;
        // a browser clamps it and serves /folder/file/, which exists.
        let html = r#"<a href="../../../folder/file/">file</a>"#;
        let errs = rendered_link_errors(html, &base, "/docs/guide/");
        assert_eq!(
            errs.len(),
            1,
            "a clamped above-root link must still be reported: {errs:?}"
        );
    }

    #[test]
    fn missing_target_is_reported() {
        let (_guard, base) = rendered_link_setup();
        let html = r#"<a href="../../folder/gone/">gone</a>"#;
        assert_eq!(rendered_link_errors(html, &base, "/docs/guide/").len(), 1);
    }

    #[test]
    fn external_fragment_and_empty_hrefs_are_ignored() {
        let (_guard, base) = rendered_link_setup();
        let html = r##"
            <a href="https://example.com/x">ext</a>
            <a href="mailto:a@b.c">mail</a>
            <a href="#section">anchor</a>
            <a href="">empty</a>
            <a href="?q=1">query only</a>
        "##;
        let errs = rendered_link_errors(html, &base, "/docs/guide/");
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn anchor_and_query_do_not_hide_a_page_link_defect() {
        let (_guard, base) = rendered_link_setup();
        // The path part is what decides canonicality; the suffix must neither
        // mask a defect nor create one.
        let bad = r#"<a href="../../folder/file#top">f</a>"#;
        assert_eq!(rendered_link_errors(bad, &base, "/docs/guide/").len(), 1);

        let good = r#"<a href="../../folder/file/?x=1#top">f</a>"#;
        assert!(rendered_link_errors(good, &base, "/docs/guide/").is_empty());
    }

    #[test]
    fn percent_encoded_href_resolves_before_it_is_judged() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("docs")).unwrap();
        std::fs::write(base.join("docs/guide.md"), "# g").unwrap();
        std::fs::write(base.join("My Page.md"), "# m").unwrap();

        let html = r#"<a href="../../My%20Page/">My Page</a>"#;
        assert!(rendered_link_errors(html, &base, "/docs/guide/").is_empty());
    }

    #[test]
    fn repeated_identical_hrefs_are_deduped() {
        let (_guard, base) = rendered_link_setup();
        let html = r#"
            <a href="../../folder/gone/">a</a>
            <a href="../../folder/gone/">b</a>
        "#;
        assert_eq!(rendered_link_errors(html, &base, "/docs/guide/").len(), 1);
    }

    /// Regression (false positive introduced with `validate_rendered_links`):
    /// a link to a *directory* without a trailing slash was reported broken.
    ///
    /// Nothing in the pipeline can emit the "canonical" spelling —
    /// `markdown_page_probe` answers yes only for a markdown file, so
    /// `transform_link` leaves an extension-less directory target exactly as
    /// authored — and the server answers `/folder` with 200 in place, no
    /// redirect. The listing it renders links site-absolutely, so the
    /// next-click damage `NonCanonical` exists to predict cannot occur.
    #[test]
    fn directory_listing_link_without_trailing_slash_is_ignored() {
        let (_guard, base) = rendered_link_setup();
        // `folder/` holds file.md and sibling.md but no index, so it resolves
        // to a DirectoryListing.
        let html = r#"<a href="../../folder">folder</a>"#;
        assert!(
            rendered_link_errors(html, &base, "/docs/guide/").is_empty(),
            "a directory listing is served in place and must not be flagged"
        );
    }

    /// Same defect via an absolute href, which is how the browse UI and hand-
    /// written navigation spell directory links.
    #[test]
    fn absolute_directory_link_without_trailing_slash_is_ignored() {
        let (_guard, base) = rendered_link_setup();
        let html = r#"<a href="/folder">folder</a>"#;
        assert!(
            rendered_link_errors(html, &base, "/docs/guide/").is_empty(),
            "an absolute directory link must not be flagged"
        );
    }

    /// Tag pages and tag source indexes have no filesystem existence at all and
    /// are served 200 at either spelling. Their templates emit absolute URLs, so
    /// the trailing slash cannot change what a subsequent click resolves to.
    #[test]
    fn tag_urls_without_trailing_slash_are_ignored() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("docs")).unwrap();
        std::fs::write(base.join("docs/guide.md"), "# g").unwrap();
        let exts = vec!["md".to_string()];
        let tags = vec!["tags".to_string(), "people".to_string()];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"
            <a href="/tags/rust">tag page</a>
            <a href="/tags">tag index</a>
            <a href="/people/jane_doe">person</a>
        "#;
        let errs = validate_rendered_links(html, &cfg, "/docs/guide/");
        assert!(errs.is_empty(), "expected no errors, got: {errs:?}");
    }

    /// The canonical spellings must stay clean too — a fix that silenced the
    /// slashless form by silencing the whole variant would pass the tests above
    /// and this one, so the markdown case below is what pins the behaviour.
    #[test]
    fn tag_and_directory_urls_with_trailing_slash_are_ignored() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("docs")).unwrap();
        std::fs::create_dir_all(base.join("folder")).unwrap();
        std::fs::write(base.join("docs/guide.md"), "# g").unwrap();
        std::fs::write(base.join("folder/file.md"), "# f").unwrap();
        let exts = vec!["md".to_string()];
        let tags = vec!["tags".to_string()];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"
            <a href="/tags/rust/">tag page</a>
            <a href="/tags/">tag index</a>
            <a href="/folder/">folder</a>
        "#;
        let errs = validate_rendered_links(html, &cfg, "/docs/guide/");
        assert!(errs.is_empty(), "expected no errors, got: {errs:?}");
    }

    /// The narrowing must not reach the defect the variant was added for: a
    /// *markdown* page link with no trailing slash is still reported. Paired
    /// with the tests above, this pins `NonCanonical` to exactly the set
    /// `markdown_page_probe` can canonicalize.
    #[test]
    fn narrowing_keeps_reporting_markdown_page_links() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("docs")).unwrap();
        std::fs::create_dir_all(base.join("folder")).unwrap();
        std::fs::write(base.join("docs/guide.md"), "# g").unwrap();
        std::fs::write(base.join("folder/file.md"), "# f").unwrap();
        let exts = vec!["md".to_string()];
        let tags = vec!["tags".to_string()];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<a href="/folder/file">file</a>"#;
        assert_eq!(
            validate_rendered_links(html, &cfg, "/docs/guide/").len(),
            1,
            "a markdown page link without its trailing slash is still a defect"
        );
    }

    /// An href that still carries `.md` is a defect, in every spelling.
    ///
    /// `link_transform::strip_markdown_extension` normally removes the
    /// extension and adds the slash, so a surviving `.md` means the transform
    /// did not fire — and the result is exactly what [`LinkDefect::NonCanonical`]
    /// exists for: the server 301s `/docs/guide.md` to `/docs/guide/`, but a
    /// static build emits no such redirect, so the link dies there.
    ///
    /// Pinned separately from the extension-less case because the narrowing to
    /// [`ResolvedPath::MarkdownFile`] is what keeps these reported, and an
    /// over-eager future widening of the `_ => None` arm would silence them.
    ///
    /// Asserts the exact [`LinkDefect`] rather than "something was reported".
    /// The distinction matters: an href that merely fails to *resolve* also
    /// produces an error, so a count-based assertion goes green even when the
    /// canonicality rule has stopped working. (Written the sloppy way first,
    /// this test passed while proving nothing — `../folder/file.md` from
    /// `/docs/guide/` is `/docs/folder/file.md`, which is simply `Missing`.)
    #[test]
    fn a_markdown_href_keeping_its_md_extension_is_not_canonical() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("docs")).unwrap();
        std::fs::create_dir_all(base.join("folder")).unwrap();
        std::fs::write(base.join("docs/guide.md"), "# g").unwrap();
        std::fs::write(base.join("folder/file.md"), "# f").unwrap();
        let exts = vec!["md".to_string()];
        let tags = vec!["tags".to_string()];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        // `../../` because the page itself sits one directory down: from
        // `/docs/guide/`, `../` is `/docs/` and `../../` is the root.
        for href in [
            "/folder/file.md",      // site-absolute
            "../../folder/file.md", // page-relative
            "/folder/file.md#top",  // a fragment must not excuse it
            "/folder/file.md?v=2",  // nor a query
        ] {
            assert_eq!(
                classify_emitted_href(href, &cfg, "/docs/guide/"),
                Some(LinkDefect::NonCanonical),
                "`{href}` still names the file, so it needs the trailing slash"
            );
        }

        // Same rule for an extension-less page link, which is the shape the
        // transform actually emits when its probe fails.
        assert_eq!(
            classify_emitted_href("/folder/file", &cfg, "/docs/guide/"),
            Some(LinkDefect::NonCanonical)
        );
    }

    /// The trailing slash is the whole difference: the same targets, spelled
    /// canonically, are silent.
    ///
    /// Guards the other direction of the narrowing — repairing the false
    /// positives must not start reporting correct links.
    #[test]
    fn a_markdown_href_with_its_trailing_slash_is_silent() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("docs")).unwrap();
        std::fs::create_dir_all(base.join("folder")).unwrap();
        std::fs::write(base.join("docs/guide.md"), "# g").unwrap();
        std::fs::write(base.join("folder/file.md"), "# f").unwrap();
        let exts = vec!["md".to_string()];
        let tags = vec!["tags".to_string()];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        for href in [
            "/folder/file/",
            "../../folder/file/",
            "/folder/file/#top",
            "/folder/file/?v=2",
        ] {
            assert_eq!(
                classify_emitted_href(href, &cfg, "/docs/guide/"),
                None,
                "`{href}` is exactly what the renderer emits and must be silent"
            );
        }
    }

    /// A missing target and a non-canonical one are different defects, and the
    /// relative arithmetic decides which. Pins that they cannot be confused —
    /// the trap the two tests above were written to avoid.
    #[test]
    fn a_relative_md_href_that_resolves_nowhere_is_missing_not_non_canonical() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("docs")).unwrap();
        std::fs::create_dir_all(base.join("folder")).unwrap();
        std::fs::write(base.join("docs/guide.md"), "# g").unwrap();
        std::fs::write(base.join("folder/file.md"), "# f").unwrap();
        let exts = vec!["md".to_string()];
        let tags = vec!["tags".to_string()];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        // One `../` short: this addresses /docs/folder/file.md, which is not there.
        assert_eq!(
            classify_emitted_href("../folder/file.md", &cfg, "/docs/guide/"),
            Some(LinkDefect::Missing)
        );
    }

    /// A genuinely missing target keeps being reported no matter how it is
    /// spelled — the narrowing touches canonicality only, never existence.
    #[test]
    fn narrowing_does_not_hide_missing_targets() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("docs")).unwrap();
        std::fs::write(base.join("docs/guide.md"), "# g").unwrap();
        let exts = vec!["md".to_string()];
        let tags = vec!["tags".to_string()];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        // `/nope` is neither a file, a directory, nor a configured tag source.
        let html = r#"<a href="/nope">gone</a><a href="/nope/">gone too</a>"#;
        assert_eq!(validate_rendered_links(html, &cfg, "/docs/guide/").len(), 2);
    }

    /// The index-page variant: `/docs/` keeps every segment, so a sibling link
    /// carries no `../`.
    #[test]
    fn index_page_sibling_link_is_ignored() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("docs")).unwrap();
        std::fs::write(base.join("docs/index.md"), "# i").unwrap();
        std::fs::write(base.join("docs/guide.md"), "# g").unwrap();

        let html = r#"<a href="guide/">Guide</a>"#;
        assert!(rendered_link_errors(html, &base, "/docs/").is_empty());
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

    // --- collect_media_references -----------------------------------------

    fn video_setup() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("Projects")).unwrap();
        std::fs::write(base.join("Projects/note.md"), "# x").unwrap();
        std::fs::write(base.join("Projects/Foo Bar.mp4"), b"\x00\x00\x00\x18ftyp").unwrap();
        (dir, base)
    }

    #[test]
    fn collect_media_references_resolves_source_to_disk_path() {
        let (_guard, base) = video_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<video><source src="../Foo%20Bar.mp4" type="video/mp4"></video>"#;
        let refs = collect_media_references(html, &cfg, "/Projects/note/");

        assert_eq!(refs.len(), 1, "{:?}", refs);
        assert_eq!(refs[0].src, "../Foo%20Bar.mp4");
        assert_eq!(refs[0].kind, MediaKind::Source);
        assert_eq!(refs[0].path, base.join("Projects/Foo Bar.mp4"));
    }

    #[test]
    fn collect_media_references_strips_time_fragment() {
        // A `{{ vid(start=…) }}` embed puts `#t=30,200` on the <source> but not
        // on the sibling <mbr-video-extras>. Stripping makes both spellings
        // produce the same join key for the frontend.
        let (_guard, base) = video_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<video><source src="../Foo%20Bar.mp4#t=30,200"></video>"#;
        let refs = collect_media_references(html, &cfg, "/Projects/note/");

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].src, "../Foo%20Bar.mp4");
    }

    #[test]
    fn collect_media_references_dedupes_repeated_embeds() {
        // reid-video-issue.md embeds the same clip twice (percent-encoded and
        // angle-bracketed); the probe must not run twice.
        let (_guard, base) = video_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"
            <video><source src="../Foo%20Bar.mp4"></video>
            <video><source src="../Foo%20Bar.mp4#t=5"></video>
        "#;
        let refs = collect_media_references(html, &cfg, "/Projects/note/");
        assert_eq!(refs.len(), 1, "{:?}", refs);
    }

    #[test]
    fn collect_media_references_skips_missing_and_external() {
        let (_guard, base) = video_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"
            <video><source src="../Gone.mp4"></video>
            <video><source src="https://example.com/x.mp4"></video>
        "#;
        let refs = collect_media_references(html, &cfg, "/Projects/note/");
        assert!(refs.is_empty(), "{:?}", refs);
    }

    #[test]
    fn collect_media_references_finds_images_and_audio_too() {
        // The collector is media-type agnostic; callers filter by extension.
        let (_guard, base) = media_setup();
        let exts = vec!["md".to_string()];
        let tags: Vec<String> = vec![];
        let cfg = make_config(&base, &exts, "index.md", &tags);

        let html = r#"<img src="photo.png"><audio src="photo.png"></audio>"#;
        let refs = collect_media_references(html, &cfg, "/");
        // Deduped by src, so the second reference to photo.png is dropped.
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].kind, MediaKind::Image);
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

    // The three variants below are a pinned wire contract shared with
    // `mbr-page-errors.ts`. The assertions spell out the exact JSON on purpose:
    // renaming a field here silently breaks the frontend, and a full-string
    // comparison is what makes that impossible to miss.

    #[test]
    fn relationship_cycle_serializes_to_the_pinned_shape() {
        let err = PageError::RelationshipCycle {
            members: vec!["/a/".to_string(), "/b/".to_string()],
            rel_type: "child".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"type":"relationship_cycle","members":["/a/","/b/"],"rel_type":"child"}"#
        );
    }

    #[test]
    fn ambiguous_relationship_endpoint_serializes_to_the_pinned_shape() {
        let err = PageError::AmbiguousRelationshipEndpoint {
            raw: "[[John Doe]]".to_string(),
            resolved_to: "/people/john-jr/".to_string(),
            candidates: vec!["/people/john-sr/".to_string()],
        };
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"type":"ambiguous_relationship_endpoint","raw":"[[John Doe]]","resolved_to":"/people/john-jr/","candidates":["/people/john-sr/"]}"#
        );
    }

    #[test]
    fn ambiguous_wikilink_serializes_to_the_pinned_shape() {
        let err = PageError::AmbiguousWikilink {
            raw: "[[John Doe]]".to_string(),
            resolved_to: "/people/john-jr/".to_string(),
            candidates: vec!["/people/john-sr/".to_string()],
        };
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"type":"ambiguous_wikilink","raw":"[[John Doe]]","resolved_to":"/people/john-jr/","candidates":["/people/john-sr/"]}"#
        );
    }

    #[test]
    fn relationship_cycle_errors_maps_every_cycle() {
        let cycles = vec![
            RelationshipCycle {
                members: vec!["/a/".to_string(), "/b/".to_string()],
                rel_type: "child".to_string(),
            },
            RelationshipCycle {
                members: vec!["/x/".to_string(), "/y/".to_string()],
                rel_type: "employee".to_string(),
            },
        ];
        let errs = relationship_cycle_errors(&cycles);
        assert_eq!(errs.len(), 2);
        assert!(matches!(
            &errs[0],
            PageError::RelationshipCycle { rel_type, members }
                if rel_type == "child" && members.len() == 2
        ));
        assert!(relationship_cycle_errors(&[]).is_empty());
    }

    #[test]
    fn ambiguous_relationship_endpoint_errors_preserves_all_candidates() {
        let endpoints = vec![AmbiguousEndpoint {
            raw: "[[Sam]]".to_string(),
            resolved_to: "/a/sam/".to_string(),
            candidates: vec!["/m/sam/".to_string(), "/z/sam/".to_string()],
        }];
        let errs = ambiguous_relationship_endpoint_errors(&endpoints);
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            PageError::AmbiguousRelationshipEndpoint { raw, resolved_to, candidates }
                if raw == "[[Sam]]" && resolved_to == "/a/sam/" && candidates.len() == 2
        ));
        assert!(ambiguous_relationship_endpoint_errors(&[]).is_empty());
    }

    #[test]
    fn ambiguous_wikilink_errors_preserves_all_candidates() {
        let wikilinks = vec![AmbiguousWikilink {
            raw: "[[Sam]]".to_string(),
            resolved_to: "/a/sam/".to_string(),
            candidates: vec!["/z/sam/".to_string()],
        }];
        let errs = ambiguous_wikilink_errors(&wikilinks);
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            PageError::AmbiguousWikilink { raw, resolved_to, candidates }
                if raw == "[[Sam]]" && resolved_to == "/a/sam/" && candidates == &["/z/sam/".to_string()]
        ));
        assert!(ambiguous_wikilink_errors(&[]).is_empty());
    }

    #[test]
    fn unplayable_media_serializes_to_the_frontend_contract() {
        // This exact shape is a contract with `components/src/`. Changing any
        // key or tag value here is a breaking frontend change.
        let err = PageError::UnplayableMedia {
            src: "../Foo%20Bar.mp4".to_string(),
            kind: MediaKind::Video,
            reason: "likely cause".to_string(),
            remedy: Some(
                "ffmpeg -i in.mp4 -map 0 -c copy -dn -movflags +faststart out.mp4".to_string(),
            ),
            advisory: true,
        };

        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "unplayable_media",
                "src": "../Foo%20Bar.mp4",
                "kind": "video",
                "reason": "likely cause",
                "remedy": "ffmpeg -i in.mp4 -map 0 -c copy -dn -movflags +faststart out.mp4",
                "advisory": true
            })
        );
    }

    #[test]
    fn unplayable_media_omits_absent_remedy() {
        let err = PageError::UnplayableMedia {
            src: "../x.mp4".to_string(),
            kind: MediaKind::Video,
            reason: "nope".to_string(),
            remedy: None,
            advisory: true,
        };
        let json = serde_json::to_string(&err).unwrap();
        assert!(!json.contains("remedy"), "{}", json);
    }

    #[test]
    fn unplayable_media_round_trips() {
        let err = PageError::UnplayableMedia {
            src: "../Foo%20Bar.mp4".to_string(),
            kind: MediaKind::Video,
            reason: "r".to_string(),
            remedy: Some("cmd".to_string()),
            advisory: true,
        };
        let json = serde_json::to_string(&err).unwrap();
        let back: PageError = serde_json::from_str(&json).unwrap();
        assert_eq!(back, err);
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
