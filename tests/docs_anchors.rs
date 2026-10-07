//! Every `#fragment` link in `docs/` must land on an id that page really has.
//!
//! mbr's own link checker (`page_errors.rs`, `--fail-on-broken-links`) strips
//! the fragment before resolving a link, so a link to a renamed or misspelled
//! heading is invisible to it. Heading ids are produced by `markdown::slugify`,
//! whose doubled dashes are frozen (`Definition Lists (FAQ Style)` →
//! `definition-lists--faq-style`), and hand-written links get them wrong.
//!
//! Both halves are read from the **rendered HTML** of each page — the `id`
//! attributes that were actually emitted and the `href`s that were actually
//! emitted — rather than re-derived from the markdown. That way the test can't
//! drift from the renderer: explicit `{#id}` attributes, duplicate-heading
//! suffixes, `--- {#section}` ids and raw-HTML anchors all count, and a link is
//! checked after the same link transform a reader's browser sees.

use mbr::link_transform::LinkTransformConfig;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

fn docs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("docs")
}

/// All markdown files under `docs/`, skipping dot-directories such as `.mbr/`.
fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let hidden = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'));
        if hidden {
            continue;
        }
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

/// The served URL of a docs page under mbr's trailing-slash convention:
/// `markdown/index.md` → `/markdown/`, `modes/review.md` → `/modes/review/`.
fn page_url(relative: &Path) -> (String, bool) {
    let is_index = relative.file_name().is_some_and(|n| n == "index.md");
    let base = if is_index {
        relative.parent().unwrap_or(Path::new(""))
    } else {
        &relative.with_extension("")
    };
    let joined = base
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let url = if joined.is_empty() {
        "/".to_string()
    } else {
        format!("/{joined}/")
    };
    (url, is_index)
}

fn render(file: &Path, url: &str, is_index: bool) -> String {
    mbr::markdown::render_sync(
        file.to_path_buf(),
        &docs_dir(),
        0,
        LinkTransformConfig {
            is_index_file: is_index,
            current_page_url: url.to_string(),
            ..LinkTransformConfig::default()
        },
        None,
        true,
        false,
        HashSet::new(),
        mbr::markdown::ReviewLines::Omit,
        false,
        &[],
        None,
    )
    .unwrap_or_else(|e| panic!("render {}: {e}", file.display()))
    .html
}

/// Values of every `name="…"` attribute in `html`, entity-decoded for `&amp;`.
fn attr_values(html: &str, name: &str) -> Vec<String> {
    let needle = format!(" {name}=\"");
    html.match_indices(&needle)
        .filter_map(|(at, _)| {
            let rest = &html[at + needle.len()..];
            rest.find('"').map(|end| rest[..end].replace("&amp;", "&"))
        })
        .collect()
}

/// Resolves `href` against the directory-style `base` URL, returning the
/// target page URL (no trailing slash, `""` for the root) and the fragment.
/// `None` for anything without a fragment or not on this site.
fn resolve(base: &str, href: &str) -> Option<(String, String)> {
    let (path, fragment) = href.split_once('#')?;
    if path.contains("://") || path.starts_with("mailto:") || path.starts_with("//") {
        return None;
    }
    let path = path.split('?').next().unwrap_or_default();
    let joined = if path.is_empty() {
        base.to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{base}{path}")
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    Some((segments.join("/"), fragment.to_string()))
}

#[test]
fn docs_fragment_links_resolve_to_real_ids() {
    let root = docs_dir();
    let mut files = Vec::new();
    markdown_files(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no docs found in {}", root.display());

    // Render each page once; keep its ids and the fragment links it emits.
    let mut ids_by_page: HashMap<String, HashSet<String>> = HashMap::new();
    let mut links: Vec<(String, String, String, String)> = Vec::new();
    for file in &files {
        let relative = file.strip_prefix(&root).expect("under docs/");
        let (url, is_index) = page_url(relative);
        let html = render(file, &url, is_index);
        let key = url.trim_matches('/').to_string();
        ids_by_page.insert(key, attr_values(&html, "id").into_iter().collect());
        for href in attr_values(&html, "href") {
            if let Some((target, fragment)) = resolve(&url, &href) {
                links.push((relative.display().to_string(), href, target, fragment));
            }
        }
    }

    let broken: Vec<String> = links
        .iter()
        .filter_map(|(source, href, target, fragment)| {
            // Fragments on non-page targets (a video's `#t=10`, an asset) are
            // not anchors and are out of scope here.
            let ids = ids_by_page.get(target)?;
            let resolves = fragment.is_empty() || ids.contains(fragment);
            (!resolves).then(|| format!("{source}: {href}"))
        })
        .collect();

    assert!(
        !links.is_empty(),
        "found no fragment links at all — the href scan is broken"
    );
    assert!(
        broken.is_empty(),
        "{} docs link(s) point at an id their target page does not have:\n  {}",
        broken.len(),
        broken.join("\n  ")
    );
}

#[test]
fn resolve_handles_the_trailing_slash_convention() {
    assert_eq!(
        resolve("/customization/themes/", "../../markdown/#x"),
        Some(("markdown".into(), "x".into()))
    );
    assert_eq!(
        resolve("/markdown/", "#x"),
        Some(("markdown".into(), "x".into()))
    );
    assert_eq!(
        resolve("/", "/reference/cli/#y"),
        Some(("reference/cli".into(), "y".into()))
    );
    assert_eq!(resolve("/a/", "https://example.com/#x"), None);
    assert_eq!(resolve("/a/", "../b/"), None);
}
