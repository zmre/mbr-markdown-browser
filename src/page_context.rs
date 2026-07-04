//! Shared template-context assembly for server mode and static site builds.
//!
//! Both `server.rs` (live rendering) and `build.rs` (static site generation)
//! hand a `HashMap<String, serde_json::Value>` to the Tera templates. The two
//! modes insert nearly identical key sets and differ only in:
//!
//! - **Mode flags**: server pages get `server_mode=true` (plus `gui_mode`
//!   and, on some pages, `relative_base="/.mbr/"`); static pages get
//!   `server_mode=false` plus `relative_base`/`relative_root` computed from
//!   the page depth.
//! - **URL style**: server pages use absolute URLs; static pages rewrite
//!   URLs relative to the page depth via `make_relative_url`.
//!
//! The helpers here capture those deltas so each call site only supplies its
//! genuinely page-specific data. Rendered output must remain byte-identical
//! to the previous hand-rolled assembly, so helpers never insert keys a call
//! site did not previously emit.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Value, json};

use crate::build::{relative_base, relative_root};
use crate::config::TagSource;
use crate::link_transform::make_relative_url;
use crate::markdown::HeadingInfo;
use crate::readability::ReadabilityScores;
use crate::server::{Breadcrumb, generate_breadcrumbs, get_current_dir_name};
use crate::tag_index::{TagInfo, TaggedPage};

/// How URLs are emitted into a template context.
pub enum UrlMode {
    /// Server mode: absolute URLs, used as-is.
    Absolute,
    /// Static build: URLs rewritten relative to a page at the given depth.
    RelativeToDepth(usize),
}

impl UrlMode {
    /// Rewrites an absolute URL according to this mode.
    pub fn rewrite(&self, url: &str) -> String {
        match self {
            UrlMode::Absolute => url.to_string(),
            UrlMode::RelativeToDepth(depth) => make_relative_url(url, *depth),
        }
    }

    /// Extracts the `url_path` of a sibling-page JSON object for prev/next
    /// navigation, preserving each mode's historical behavior:
    /// server inserts the raw value (JSON `null` if missing), build falls
    /// back to `/` and relativizes.
    fn nav_url(&self, page: &Value) -> Value {
        match self {
            UrlMode::Absolute => json!(page.get("url_path")),
            UrlMode::RelativeToDepth(depth) => {
                let url = page.get("url_path").and_then(|v| v.as_str()).unwrap_or("/");
                json!(make_relative_url(url, *depth))
            }
        }
    }
}

/// The `server_mode`/`gui_mode`/`relative_base`/`relative_root` key set,
/// which varies per render mode (and per page type within server mode).
pub enum ModeFlags {
    Server {
        /// `Some(flag)` inserts `gui_mode`; `None` omits the key entirely
        /// (tag pages historically don't emit it).
        gui_mode: Option<bool>,
        /// When true, inserts `relative_base: "/.mbr/"` (tag pages and the
        /// media viewer emit it in server mode; other pages don't).
        mbr_base: bool,
    },
    Static {
        /// Page depth used to compute `relative_base`/`relative_root`.
        depth: usize,
    },
}

/// Common page "chrome": mode flags, sidebar configuration, and title
/// prefix/suffix. Every non-markdown assembly site inserts this set.
pub struct PageChrome<'a> {
    pub mode: ModeFlags,
    pub sidebar_style: &'a str,
    pub sidebar_max_items: usize,
    /// `Some((prefix, suffix))` for content pages; `None` for error pages,
    /// which historically omit `title_prefix`/`title_suffix`.
    pub title_affixes: Option<(&'a str, &'a str)>,
}

/// Inserts the shared chrome key set into a template context.
pub fn insert_page_chrome(ctx: &mut HashMap<String, Value>, chrome: &PageChrome<'_>) {
    match &chrome.mode {
        ModeFlags::Server { gui_mode, mbr_base } => {
            ctx.insert("server_mode".to_string(), json!(true));
            if let Some(gui) = gui_mode {
                ctx.insert("gui_mode".to_string(), json!(gui));
            }
            if *mbr_base {
                ctx.insert("relative_base".to_string(), json!("/.mbr/"));
            }
        }
        ModeFlags::Static { depth } => {
            ctx.insert("server_mode".to_string(), json!(false));
            ctx.insert("relative_base".to_string(), json!(relative_base(*depth)));
            ctx.insert("relative_root".to_string(), json!(relative_root(*depth)));
        }
    }
    ctx.insert("sidebar_style".to_string(), json!(chrome.sidebar_style));
    ctx.insert(
        "sidebar_max_items".to_string(),
        json!(chrome.sidebar_max_items),
    );
    if let Some((prefix, suffix)) = chrome.title_affixes {
        ctx.insert("title_prefix".to_string(), json!(prefix));
        ctx.insert("title_suffix".to_string(), json!(suffix));
    }
}

/// Serializes tag-source configuration as a JSON string for safe template
/// rendering in a JavaScript context (used by frontend tag linking).
pub fn tag_sources_json(tag_sources: &[TagSource]) -> String {
    serde_json::to_string(
        &tag_sources
            .iter()
            .map(|ts| {
                json!({
                    "field": ts.field,
                    "urlSource": ts.url_source(),
                    "label": ts.singular_label(),
                    "labelPlural": ts.plural_label()
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string())
}

/// Converts breadcrumbs to their JSON template representation, rewriting
/// URLs per the given mode.
pub fn breadcrumbs_to_json(breadcrumbs: &[Breadcrumb], url_mode: &UrlMode) -> Vec<Value> {
    breadcrumbs
        .iter()
        .map(|b| json!({"name": b.name, "url": url_mode.rewrite(&b.url)}))
        .collect()
}

/// Resolves singular/plural labels for a tag source. Falls back to
/// `fallback_base` / `{fallback_base}s` when the source is not configured
/// (server capitalizes the fallback, build uses it verbatim).
pub fn tag_labels(
    tag_sources: &[TagSource],
    source: &str,
    fallback_base: &str,
) -> (String, String) {
    tag_sources
        .iter()
        .find(|ts| ts.url_source() == source)
        .map(|ts| (ts.singular_label(), ts.plural_label()))
        .unwrap_or_else(|| (fallback_base.to_string(), format!("{}s", fallback_base)))
}

/// Inserts the shared key set for a tag page (`tag.html`).
pub fn insert_tag_page_keys(
    ctx: &mut HashMap<String, Value>,
    source: &str,
    display_value: &str,
    label: &str,
    label_plural: &str,
    pages: &[TaggedPage],
    url_mode: &UrlMode,
) {
    ctx.insert("tag_source".to_string(), json!(source));
    ctx.insert("tag_display_value".to_string(), json!(display_value));
    ctx.insert("tag_label".to_string(), json!(label));
    ctx.insert("tag_label_plural".to_string(), json!(label_plural));
    let pages_json: Vec<Value> = pages
        .iter()
        .map(|p| {
            json!({
                "url_path": url_mode.rewrite(&p.url_path),
                "title": p.title,
                "description": p.description,
            })
        })
        .collect();
    ctx.insert("pages".to_string(), json!(pages_json));
    ctx.insert("page_count".to_string(), json!(pages.len()));
}

/// Inserts the shared key set for a tag source index page (`tag_index.html`).
pub fn insert_tag_index_keys(
    ctx: &mut HashMap<String, Value>,
    source: &str,
    label: &str,
    label_plural: &str,
    tags: &[TagInfo],
) {
    ctx.insert("tag_source".to_string(), json!(source));
    ctx.insert("tag_label".to_string(), json!(label));
    ctx.insert("tag_label_plural".to_string(), json!(label_plural));
    let tags_json: Vec<Value> = tags
        .iter()
        .map(|t| {
            json!({
                "url_value": t.normalized,
                "display_value": t.display,
                "page_count": t.count,
            })
        })
        .collect();
    ctx.insert("tags".to_string(), json!(tags_json));
    ctx.insert("tag_count".to_string(), json!(tags.len()));
}

/// Inserts the shared key set for an error page (`error.html`).
/// `error_message` is omitted when `None` (matches historical behavior).
pub fn insert_error_keys(
    ctx: &mut HashMap<String, Value>,
    error_code: u16,
    error_title: &str,
    error_message: Option<&str>,
) {
    ctx.insert("error_code".to_string(), json!(error_code));
    ctx.insert("error_title".to_string(), json!(error_title));
    if let Some(msg) = error_message {
        ctx.insert("error_message".to_string(), json!(msg));
    }
}

/// Page-specific inputs for the shared markdown-page context builder.
///
/// Sibling computation stays with the caller (server memoizes via a cache,
/// build uses a pre-built index); this struct only carries the results.
pub struct MarkdownPageParams<'a> {
    /// URL-shaped path used for breadcrumbs and `current_dir_name`.
    pub breadcrumb_path: &'a Path,
    pub headings: &'a [HeadingInfo],
    pub has_h1: bool,
    pub word_count: usize,
    pub readability: &'a ReadabilityScores,
    /// File path relative to the repository root.
    pub file_path: &'a str,
    /// File mtime as seconds since the Unix epoch, if available.
    pub modified_secs: Option<u64>,
    /// Absolute URL of this page, used to locate it among its siblings.
    pub current_url: &'a str,
    /// Sorted sibling pages (JSON objects with `url_path`/`title`).
    pub siblings: &'a [Value],
}

/// Repository-level configuration for the markdown-page context builder.
pub struct MarkdownContextOptions<'a> {
    pub tag_sources: &'a [TagSource],
    pub sidebar_style: &'a str,
    pub sidebar_max_items: usize,
    pub title_prefix: &'a str,
    pub title_suffix: &'a str,
}

/// Builds the `extra_context` map shared by server-mode and static-build
/// markdown rendering.
///
/// Mode flags (`server_mode`/`gui_mode`) are *not* inserted here: markdown
/// pages carry them in frontmatter (so they appear in `frontmatter_json`),
/// which remains the caller's responsibility. In static mode
/// (`UrlMode::RelativeToDepth`) this also inserts `relative_base` and
/// `relative_root`.
pub fn markdown_extra_context(
    params: &MarkdownPageParams<'_>,
    opts: &MarkdownContextOptions<'_>,
    url_mode: &UrlMode,
) -> HashMap<String, Value> {
    let mut ctx = HashMap::new();

    // Navigation: breadcrumbs and current directory name
    let breadcrumbs = generate_breadcrumbs(params.breadcrumb_path);
    ctx.insert(
        "breadcrumbs".to_string(),
        json!(breadcrumbs_to_json(&breadcrumbs, url_mode)),
    );
    ctx.insert(
        "current_dir_name".to_string(),
        json!(get_current_dir_name(params.breadcrumb_path)),
    );

    // Heading TOC
    ctx.insert("headings".to_string(), json!(params.headings));
    ctx.insert("has_h1".to_string(), json!(params.has_h1));

    // Tag sources configuration for frontend tag linking (pre-serialized as
    // a JSON string for safe template rendering in a JavaScript context)
    ctx.insert(
        "tag_sources".to_string(),
        json!(tag_sources_json(opts.tag_sources)),
    );

    // Word count and reading time (200 words per minute)
    let reading_time_minutes = params
        .word_count
        .div_ceil(crate::constants::WORDS_PER_MINUTE);
    ctx.insert("word_count".to_string(), json!(params.word_count));
    ctx.insert(
        "reading_time_minutes".to_string(),
        json!(reading_time_minutes),
    );

    // Readability scores (Flesch Reading Ease + Flesch-Kincaid Grade Level).
    // `None` values serialize as JSON `null`, which the template outputs
    // literally so the frontend can guard on `!= null`.
    ctx.insert(
        "flesch_reading_ease".to_string(),
        json!(params.readability.flesch_reading_ease),
    );
    ctx.insert(
        "flesch_kincaid_grade".to_string(),
        json!(params.readability.flesch_kincaid_grade),
    );

    // File path (relative to root) for reference
    ctx.insert("file_path".to_string(), json!(params.file_path));

    // Sidebar navigation configuration and title affixes
    ctx.insert("sidebar_style".to_string(), json!(opts.sidebar_style));
    ctx.insert(
        "sidebar_max_items".to_string(),
        json!(opts.sidebar_max_items),
    );
    ctx.insert("title_prefix".to_string(), json!(opts.title_prefix));
    ctx.insert("title_suffix".to_string(), json!(opts.title_suffix));

    // Modified date from file metadata
    if let Some(secs) = params.modified_secs {
        ctx.insert("modified_timestamp".to_string(), json!(secs));
    }

    // Prev/next sibling pages for navigation
    if let Some(current_idx) = params.siblings.iter().position(|f| {
        f.get("url_path")
            .and_then(|v| v.as_str())
            .is_some_and(|p| p == params.current_url)
    }) {
        if current_idx > 0
            && let Some(prev) = params.siblings.get(current_idx - 1)
        {
            ctx.insert(
                "prev_page".to_string(),
                json!({
                    "url": url_mode.nav_url(prev),
                    "title": prev.get("title").and_then(|v| v.as_str()).unwrap_or("Previous")
                }),
            );
        }
        if let Some(next) = params.siblings.get(current_idx + 1) {
            ctx.insert(
                "next_page".to_string(),
                json!({
                    "url": url_mode.nav_url(next),
                    "title": next.get("title").and_then(|v| v.as_str()).unwrap_or("Next")
                }),
            );
        }
    }

    // Relative path variables for static builds
    if let UrlMode::RelativeToDepth(depth) = url_mode {
        ctx.insert("relative_base".to_string(), json!(relative_base(*depth)));
        ctx.insert("relative_root".to_string(), json!(relative_root(*depth)));
    }

    ctx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_source(field: &str) -> TagSource {
        TagSource {
            field: field.to_string(),
            label: None,
            label_plural: None,
        }
    }

    #[test]
    fn test_url_mode_rewrite_absolute_is_identity() {
        assert_eq!(UrlMode::Absolute.rewrite("/docs/guide/"), "/docs/guide/");
    }

    #[test]
    fn test_url_mode_rewrite_relative_matches_make_relative_url() {
        assert_eq!(
            UrlMode::RelativeToDepth(2).rewrite("/docs/guide/"),
            make_relative_url("/docs/guide/", 2)
        );
    }

    #[test]
    fn test_insert_page_chrome_server_full() {
        let mut ctx = HashMap::new();
        insert_page_chrome(
            &mut ctx,
            &PageChrome {
                mode: ModeFlags::Server {
                    gui_mode: Some(true),
                    mbr_base: true,
                },
                sidebar_style: "auto",
                sidebar_max_items: 10,
                title_affixes: Some(("pre ", " suf")),
            },
        );
        assert_eq!(ctx.get("server_mode"), Some(&json!(true)));
        assert_eq!(ctx.get("gui_mode"), Some(&json!(true)));
        assert_eq!(ctx.get("relative_base"), Some(&json!("/.mbr/")));
        assert_eq!(ctx.get("sidebar_style"), Some(&json!("auto")));
        assert_eq!(ctx.get("sidebar_max_items"), Some(&json!(10)));
        assert_eq!(ctx.get("title_prefix"), Some(&json!("pre ")));
        assert_eq!(ctx.get("title_suffix"), Some(&json!(" suf")));
        assert!(!ctx.contains_key("relative_root"));
    }

    #[test]
    fn test_insert_page_chrome_server_omits_optional_keys() {
        let mut ctx = HashMap::new();
        insert_page_chrome(
            &mut ctx,
            &PageChrome {
                mode: ModeFlags::Server {
                    gui_mode: None,
                    mbr_base: false,
                },
                sidebar_style: "auto",
                sidebar_max_items: 10,
                title_affixes: None,
            },
        );
        assert_eq!(ctx.get("server_mode"), Some(&json!(true)));
        assert!(!ctx.contains_key("gui_mode"));
        assert!(!ctx.contains_key("relative_base"));
        assert!(!ctx.contains_key("title_prefix"));
        assert!(!ctx.contains_key("title_suffix"));
    }

    #[test]
    fn test_insert_page_chrome_static() {
        let mut ctx = HashMap::new();
        insert_page_chrome(
            &mut ctx,
            &PageChrome {
                mode: ModeFlags::Static { depth: 2 },
                sidebar_style: "auto",
                sidebar_max_items: 5,
                title_affixes: Some(("", "")),
            },
        );
        assert_eq!(ctx.get("server_mode"), Some(&json!(false)));
        assert_eq!(ctx.get("relative_base"), Some(&json!("../../.mbr/")));
        assert_eq!(ctx.get("relative_root"), Some(&json!("../../")));
        assert!(!ctx.contains_key("gui_mode"));
    }

    #[test]
    fn test_tag_sources_json_format() {
        let sources = vec![tag_source("tags")];
        let json_str = tag_sources_json(&sources);
        let parsed: Vec<Value> = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["field"], "tags");
        assert_eq!(parsed[0]["urlSource"], "tags");
        assert!(parsed[0]["label"].is_string());
        assert!(parsed[0]["labelPlural"].is_string());
    }

    #[test]
    fn test_tag_sources_json_empty() {
        assert_eq!(tag_sources_json(&[]), "[]");
    }

    #[test]
    fn test_breadcrumbs_to_json_absolute() {
        let crumbs = generate_breadcrumbs(Path::new("docs/guide"));
        let json_crumbs = breadcrumbs_to_json(&crumbs, &UrlMode::Absolute);
        assert_eq!(json_crumbs.len(), crumbs.len());
        for (jc, c) in json_crumbs.iter().zip(crumbs.iter()) {
            assert_eq!(jc["name"], json!(c.name));
            assert_eq!(jc["url"], json!(c.url));
        }
    }

    #[test]
    fn test_breadcrumbs_to_json_relative() {
        let crumbs = generate_breadcrumbs(Path::new("docs/guide"));
        let json_crumbs = breadcrumbs_to_json(&crumbs, &UrlMode::RelativeToDepth(2));
        for (jc, c) in json_crumbs.iter().zip(crumbs.iter()) {
            assert_eq!(jc["url"], json!(make_relative_url(&c.url, 2)));
        }
    }

    #[test]
    fn test_tag_labels_configured_source() {
        let sources = vec![tag_source("tags")];
        let (label, plural) = tag_labels(&sources, "tags", "fallback");
        assert_eq!(label, sources[0].singular_label());
        assert_eq!(plural, sources[0].plural_label());
    }

    #[test]
    fn test_tag_labels_fallback() {
        let (label, plural) = tag_labels(&[], "performers", "Performers");
        assert_eq!(label, "Performers");
        assert_eq!(plural, "Performerss");

        let (label, plural) = tag_labels(&[], "performers", "performers");
        assert_eq!(label, "performers");
        assert_eq!(plural, "performerss");
    }

    #[test]
    fn test_insert_tag_page_keys() {
        let mut ctx = HashMap::new();
        let pages = vec![TaggedPage::with_description(
            "/docs/rust/",
            "Rust Guide",
            "A guide",
            "Rust",
        )];
        insert_tag_page_keys(
            &mut ctx,
            "tags",
            "Rust",
            "Tag",
            "Tags",
            &pages,
            &UrlMode::Absolute,
        );
        assert_eq!(ctx.get("tag_source"), Some(&json!("tags")));
        assert_eq!(ctx.get("tag_display_value"), Some(&json!("Rust")));
        assert_eq!(ctx.get("tag_label"), Some(&json!("Tag")));
        assert_eq!(ctx.get("tag_label_plural"), Some(&json!("Tags")));
        assert_eq!(ctx.get("page_count"), Some(&json!(1)));
        let pages_val = ctx.get("pages").unwrap();
        assert_eq!(pages_val[0]["url_path"], "/docs/rust/");
        assert_eq!(pages_val[0]["title"], "Rust Guide");
        assert_eq!(pages_val[0]["description"], "A guide");
    }

    #[test]
    fn test_insert_tag_page_keys_null_description_and_relative_url() {
        let mut ctx = HashMap::new();
        let pages = vec![TaggedPage::new("/docs/rust/", "Rust Guide", "Rust")];
        insert_tag_page_keys(
            &mut ctx,
            "tags",
            "Rust",
            "Tag",
            "Tags",
            &pages,
            &UrlMode::RelativeToDepth(2),
        );
        let pages_val = ctx.get("pages").unwrap();
        // Missing description serializes as JSON null (historical behavior)
        assert!(pages_val[0]["description"].is_null());
        assert_eq!(
            pages_val[0]["url_path"],
            json!(make_relative_url("/docs/rust/", 2))
        );
    }

    #[test]
    fn test_insert_tag_index_keys() {
        let mut ctx = HashMap::new();
        let tags = vec![TagInfo {
            normalized: "rust".to_string(),
            display: "Rust".to_string(),
            count: 3,
        }];
        insert_tag_index_keys(&mut ctx, "tags", "Tag", "Tags", &tags);
        assert_eq!(ctx.get("tag_source"), Some(&json!("tags")));
        assert_eq!(ctx.get("tag_count"), Some(&json!(1)));
        let tags_val = ctx.get("tags").unwrap();
        assert_eq!(tags_val[0]["url_value"], "rust");
        assert_eq!(tags_val[0]["display_value"], "Rust");
        assert_eq!(tags_val[0]["page_count"], 3);
    }

    #[test]
    fn test_insert_error_keys_with_and_without_message() {
        let mut ctx = HashMap::new();
        insert_error_keys(&mut ctx, 404, "Not Found", Some("gone"));
        assert_eq!(ctx.get("error_code"), Some(&json!(404)));
        assert_eq!(ctx.get("error_title"), Some(&json!("Not Found")));
        assert_eq!(ctx.get("error_message"), Some(&json!("gone")));

        let mut ctx = HashMap::new();
        insert_error_keys(&mut ctx, 500, "Internal Server Error", None);
        assert!(!ctx.contains_key("error_message"));
    }

    fn markdown_opts(sources: &[TagSource]) -> MarkdownContextOptions<'_> {
        MarkdownContextOptions {
            tag_sources: sources,
            sidebar_style: "auto",
            sidebar_max_items: 10,
            title_prefix: "",
            title_suffix: "",
        }
    }

    #[test]
    fn test_markdown_extra_context_server_mode() {
        let scores = crate::readability::ReadabilityScores {
            flesch_reading_ease: Some(65.0),
            flesch_kincaid_grade: Some(8.0),
        };
        let siblings = vec![
            json!({"url_path": "/docs/a/", "title": "A"}),
            json!({"url_path": "/docs/b/", "title": "B"}),
            json!({"url_path": "/docs/c/", "title": "C"}),
        ];
        let headings = vec![HeadingInfo {
            level: 2,
            text: "Intro".to_string(),
            id: "intro".to_string(),
        }];
        let params = MarkdownPageParams {
            breadcrumb_path: Path::new("docs/b"),
            headings: &headings,
            has_h1: true,
            word_count: 401,
            readability: &scores,
            file_path: "docs/b.md",
            modified_secs: Some(1700000000),
            current_url: "/docs/b/",
            siblings: &siblings,
        };
        let ctx = markdown_extra_context(&params, &markdown_opts(&[]), &UrlMode::Absolute);

        assert_eq!(ctx.get("has_h1"), Some(&json!(true)));
        assert_eq!(ctx.get("word_count"), Some(&json!(401)));
        // 401 words at 200 wpm rounds up to 3 minutes
        assert_eq!(ctx.get("reading_time_minutes"), Some(&json!(3)));
        assert_eq!(ctx.get("flesch_reading_ease"), Some(&json!(65.0)));
        assert_eq!(ctx.get("file_path"), Some(&json!("docs/b.md")));
        assert_eq!(ctx.get("modified_timestamp"), Some(&json!(1700000000u64)));
        assert_eq!(ctx.get("prev_page").unwrap()["url"], "/docs/a/");
        assert_eq!(ctx.get("prev_page").unwrap()["title"], "A");
        assert_eq!(ctx.get("next_page").unwrap()["url"], "/docs/c/");
        // Server mode: no relative path variables
        assert!(!ctx.contains_key("relative_base"));
        assert!(!ctx.contains_key("relative_root"));
        // Mode flags are the caller's responsibility (frontmatter)
        assert!(!ctx.contains_key("server_mode"));
        assert!(!ctx.contains_key("gui_mode"));
    }

    #[test]
    fn test_markdown_extra_context_static_mode() {
        let scores = crate::readability::ReadabilityScores {
            flesch_reading_ease: None,
            flesch_kincaid_grade: None,
        };
        let siblings = vec![
            json!({"url_path": "/docs/a/", "title": "A"}),
            json!({"url_path": "/docs/b/", "title": "B"}),
        ];
        let params = MarkdownPageParams {
            breadcrumb_path: Path::new("/docs/b/"),
            headings: &[],
            has_h1: false,
            word_count: 0,
            readability: &scores,
            file_path: "docs/b.md",
            modified_secs: None,
            current_url: "/docs/b/",
            siblings: &siblings,
        };
        let ctx =
            markdown_extra_context(&params, &markdown_opts(&[]), &UrlMode::RelativeToDepth(2));

        // None scores serialize as JSON null
        assert_eq!(ctx.get("flesch_reading_ease"), Some(&json!(null)));
        assert!(!ctx.contains_key("modified_timestamp"));
        // Static mode: relative path variables present
        assert_eq!(ctx.get("relative_base"), Some(&json!("../../.mbr/")));
        assert_eq!(ctx.get("relative_root"), Some(&json!("../../")));
        // Prev URL is relativized; no next page (current is last)
        assert_eq!(
            ctx.get("prev_page").unwrap()["url"],
            json!(make_relative_url("/docs/a/", 2))
        );
        assert!(!ctx.contains_key("next_page"));
    }

    #[test]
    fn test_markdown_extra_context_no_siblings_match() {
        let scores = crate::readability::ReadabilityScores {
            flesch_reading_ease: None,
            flesch_kincaid_grade: None,
        };
        let params = MarkdownPageParams {
            breadcrumb_path: Path::new("docs/b"),
            headings: &[],
            has_h1: false,
            word_count: 10,
            readability: &scores,
            file_path: "docs/b.md",
            modified_secs: None,
            current_url: "/not-in-list/",
            siblings: &[],
        };
        let ctx = markdown_extra_context(&params, &markdown_opts(&[]), &UrlMode::Absolute);
        assert!(!ctx.contains_key("prev_page"));
        assert!(!ctx.contains_key("next_page"));
    }
}
