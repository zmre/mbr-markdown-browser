//! Link rewriting for file moves/renames (server-mode editing endpoints).
//!
//! When a markdown file is moved or renamed, links that reference it break
//! unless every referencing page is updated. This module provides the pure,
//! unit-testable rewrite logic plus repo-wide walkers used by the
//! `/.mbr/move` handler:
//!
//! - [`rewrite_links_for_move`] — the pure seam: rewrite one file's links that
//!   point at the moved page (`old_url` → `new_url`), preserving the original
//!   link style (absolute vs relative vs `./`), `.md`/trailing-slash, `#anchor`,
//!   `?query`, and wiki `|display`.
//! - [`rewrite_moved_file_outbound_links`] — re-express the *moved* file's own
//!   relative links against its new folder (A4-B), including the self-link fix.
//! - [`rewrite_inbound_links_for_move`] — walk the repo and apply
//!   [`rewrite_links_for_move`] to every page that links to the moved file
//!   (A4-A), mirroring `link_grep::find_inbound_links`' per-folder structure.
//! - [`rewrite_bare_wikilinks_for_rename`] — rewrite bare `[[Name]]` links when
//!   the filename stem changes (A4-C), guarded so links that resolve to a
//!   *different* note are left intact.
//!
//! All matching is delimiter-anchored so that moving `/a/guide` never rewrites
//! links to `/a/guide-2`, and every rewrite is idempotent.

use aho_corasick::{AhoCorasickBuilder, MatchKind};
use regex::{Captures, Regex};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::link_grep::{
    compute_patterns_for_folder, compute_relative_path, get_folder_url_path, page_and_folder_urls,
};
use crate::link_index::{is_internal_link, normalize_url_path, resolve_relative_url};
use crate::relationships::normalize_name;
use crate::repo::{is_markdown_extension, should_ignore};
use crate::wikilink_index::WikilinkIndex;

/// The bare link "bases" that could reference `old_url` from `source_folder`:
/// the absolute path, the folder-relative path, and (when applicable) the
/// explicit `./`-prefixed relative path. Longest-first, deduplicated. These are
/// the bare bases underlying [`compute_patterns_for_folder`].
fn collect_move_bases(source_folder: &str, old_url: &str) -> Vec<String> {
    let old_abs = format!("/{}", old_url.trim_matches('/'));
    let old_rel = compute_relative_path(source_folder, old_url);

    let mut bases = vec![old_abs];
    if old_rel != "." {
        bases.push(old_rel.clone());
        if !old_rel.starts_with("../") && !old_rel.starts_with("./") {
            bases.push(format!("./{}", old_rel));
        }
    }

    let mut seen = HashSet::new();
    bases.retain(|b| !b.is_empty() && seen.insert(b.clone()));
    // Longest-first so the alternation prefers the most specific base.
    bases.sort_by_key(|b| std::cmp::Reverse(b.len()));
    bases
}

/// Classifies a matched link target and returns the replacement target that
/// preserves the original absolute-vs-relative style.
fn reclassify_target(matched: &str, new_abs: &str, new_rel: &str) -> String {
    if matched.starts_with('/') {
        new_abs.to_string()
    } else if matched.starts_with("./") {
        format!("./{}", new_rel)
    } else {
        new_rel.to_string()
    }
}

/// Rewrites every link in `content` that points at `old_url` so it points at
/// `new_url`, as seen from a file living in `source_folder` (a folder URL such
/// as `/docs/`).
///
/// Handles inline links `[t](target)`, reference definitions `[ref]: target`,
/// and path-style wiki links `[[dir/target]]`. The original link style
/// (absolute / relative / `./`), any `.md` extension, trailing slash,
/// `#anchor`, `?query`, and wiki `|display` are preserved verbatim. Bare
/// `[[Name]]` links are intentionally left alone (see
/// [`rewrite_bare_wikilinks_for_rename`]). Idempotent.
pub fn rewrite_links_for_move(
    old_url: &str,
    new_url: &str,
    source_folder: &str,
    content: &str,
) -> String {
    if old_url.trim_matches('/').is_empty() {
        return content.to_string();
    }
    let bases = collect_move_bases(source_folder, old_url);
    if bases.is_empty() {
        return content.to_string();
    }

    let new_abs = format!("/{}", new_url.trim_matches('/'));
    let new_rel = compute_relative_path(source_folder, new_url);
    let reclass = |t: &str| reclassify_target(t, &new_abs, &new_rel);

    let alt = bases
        .iter()
        .map(|b| regex::escape(b))
        .collect::<Vec<_>>()
        .join("|");

    let mut out = content.to_string();

    // Inline: [text](target(.md)?(/)?(#anchor|?query)?)
    if let Ok(re) = Regex::new(&format!(
        r"(?P<pre>\[[^\]]*\]\()(?P<t>{alt})(?P<ext>(?:\.md)?)(?P<slash>/?)(?P<suf>(?:[#?][^)]*)?)(?P<post>\))"
    )) {
        out = re
            .replace_all(&out, |c: &Captures| {
                format!(
                    "{}{}{}{}{}{}",
                    &c["pre"],
                    reclass(&c["t"]),
                    &c["ext"],
                    &c["slash"],
                    &c["suf"],
                    &c["post"]
                )
            })
            .into_owned();
    }

    // Reference definition: [ref]: target(.md)?(/)?(#anchor)?  (line-anchored).
    // The trailing boundary is a captured char class / `$` anchor (the `regex`
    // crate has no lookahead) so `/docs/guide` never matches inside
    // `/docs/guide-2`; the boundary char is re-emitted verbatim.
    if let Ok(re) = Regex::new(&format!(
        r"(?m)(?P<pre>^[ \t]*\[[^\]]+\]:[ \t]*)(?P<t>{alt})(?P<ext>(?:\.md)?)(?P<slash>/?)(?P<suf>(?:[#?]\S*)?)(?P<b>[ \t]|$)"
    )) {
        out = re
            .replace_all(&out, |c: &Captures| {
                format!(
                    "{}{}{}{}{}{}",
                    &c["pre"],
                    reclass(&c["t"]),
                    &c["ext"],
                    &c["slash"],
                    &c["suf"],
                    &c["b"]
                )
            })
            .into_owned();
    }

    // Path-style wiki: [[dir/target(.md)?(/)?(#anchor)?(|display)?]]
    let wiki_bases: Vec<&String> = bases.iter().filter(|b| b.contains('/')).collect();
    if !wiki_bases.is_empty() {
        let walt = wiki_bases
            .iter()
            .map(|b| regex::escape(b))
            .collect::<Vec<_>>()
            .join("|");
        if let Ok(re) = Regex::new(&format!(
            r"(?i)(?P<pre>\[\[)(?P<t>{walt})(?P<ext>(?:\.md)?)(?P<slash>/?)(?P<anchor>(?:#[^\]|]*)?)(?P<disp>(?:\|[^\]]*)?)(?P<post>\]\])"
        )) {
            out = re
                .replace_all(&out, |c: &Captures| {
                    format!(
                        "{}{}{}{}{}{}{}",
                        &c["pre"],
                        reclass(&c["t"]),
                        &c["ext"],
                        &c["slash"],
                        &c["anchor"],
                        &c["disp"],
                        &c["post"]
                    )
                })
                .into_owned();
        }
    }

    out
}

/// The folder a page lives in, as a folder URL ending in `/`.
///
/// For an index page the URL already denotes the directory; for a non-index
/// page the last URL segment is the file stem, so the folder is its parent.
fn folder_of(url: &str, is_index: bool) -> String {
    if is_index {
        let trimmed = url.trim_end_matches('/');
        if trimmed.is_empty() {
            "/".to_string()
        } else {
            format!("{}/", trimmed)
        }
    } else {
        get_folder_url_path(url)
    }
}

/// Returns the original markdown extension suffix (e.g. `.md`) if the last
/// path segment of `path` ends in a configured markdown extension.
fn markdown_ext_suffix(path: &str, exts: &[String]) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or(trimmed);
    let (_, ext) = last.rsplit_once('.')?;
    if is_markdown_extension(&ext.to_lowercase(), exts) {
        Some(format!(".{ext}"))
    } else {
        None
    }
}

/// Re-expresses a single relative link `target` (from the moved file's old
/// location) so it points at the same destination from the new folder.
///
/// Returns `None` (leave the link unchanged) for absolute, anchor-only, or
/// external targets. Applies the self-link fix: a link that resolves to the
/// moved file itself is re-pointed at `new_url`.
fn relocate_link_target(
    target: &str,
    old_url: &str,
    old_is_index: bool,
    new_folder: &str,
    new_url: &str,
    exts: &[String],
) -> Option<String> {
    if target.is_empty()
        || target.starts_with('/')
        || target.starts_with('#')
        || !is_internal_link(target)
    {
        return None;
    }

    // Split off any #anchor / ?query suffix.
    let (path_part, suffix) = match target.find(['#', '?']) {
        Some(i) => (&target[..i], &target[i..]),
        None => (target, ""),
    };
    if path_part.is_empty() {
        return None;
    }

    let had_dot = path_part.starts_with("./");
    let trailing_slash = path_part.ends_with('/');
    let md_ext = markdown_ext_suffix(path_part, exts);

    // Convert a markdown path (`intro.md`) to its URL form (`intro/`) so the
    // resolver treats it as a page rather than a literal segment.
    let rel_for_resolve = match &md_ext {
        Some(ext) => {
            let base = path_part.strip_suffix(ext.as_str()).unwrap_or(path_part);
            format!("{}/", base)
        }
        None => path_part.to_string(),
    };

    let mut abs = resolve_relative_url(old_url, &rel_for_resolve, old_is_index);
    if normalize_url_path(&abs) == normalize_url_path(old_url) {
        abs = normalize_url_path(new_url);
    }

    let new_core = compute_relative_path(new_folder, &abs);
    if new_core == "." {
        return None;
    }

    let mut out = String::new();
    if had_dot && !new_core.starts_with("..") {
        out.push_str("./");
    }
    out.push_str(&new_core);
    if let Some(ext) = &md_ext {
        out.push_str(ext);
    } else if trailing_slash {
        out.push('/');
    }
    out.push_str(suffix);
    Some(out)
}

/// Rewrites the *moved* file's own relative outbound links so they resolve to
/// the same destinations from its new folder (A4-B). Absolute, anchor-only,
/// external, and bare `[[Name]]` links are left untouched; self-links are
/// re-pointed at `new_url`.
pub fn rewrite_moved_file_outbound_links(
    old_url: &str,
    old_is_index: bool,
    new_url: &str,
    new_is_index: bool,
    exts: &[String],
    content: &str,
) -> String {
    let new_folder = folder_of(new_url, new_is_index);
    let mut out = content.to_string();

    // Inline links: [text](url optional-title)
    if let Ok(re) =
        Regex::new(r#"(?P<pre>\[[^\]]*\]\()(?P<url>[^)\s]+)(?P<rest>(?:\s+[^)]*)?)(?P<post>\))"#)
    {
        out = re
            .replace_all(&out, |c: &Captures| {
                match relocate_link_target(
                    &c["url"],
                    old_url,
                    old_is_index,
                    &new_folder,
                    new_url,
                    exts,
                ) {
                    Some(n) => format!("{}{}{}{}", &c["pre"], n, &c["rest"], &c["post"]),
                    None => c[0].to_string(),
                }
            })
            .into_owned();
    }

    // Reference definitions: [ref]: url optional-title
    if let Ok(re) = Regex::new(r"(?m)(?P<pre>^[ \t]*\[[^\]]+\]:[ \t]*)(?P<url>\S+)(?P<post>.*)$") {
        out = re
            .replace_all(&out, |c: &Captures| {
                match relocate_link_target(
                    &c["url"],
                    old_url,
                    old_is_index,
                    &new_folder,
                    new_url,
                    exts,
                ) {
                    Some(n) => format!("{}{}{}", &c["pre"], n, &c["post"]),
                    None => c[0].to_string(),
                }
            })
            .into_owned();
    }

    // Path-style wiki links: [[dir/target#anchor|display]] (bare names skipped)
    if let Ok(re) = Regex::new(
        r"(?P<pre>\[\[)(?P<url>[^\]|#]+)(?P<anchor>(?:#[^\]|]*)?)(?P<disp>(?:\|[^\]]*)?)(?P<post>\]\])",
    ) {
        out = re
            .replace_all(&out, |c: &Captures| {
                let url = &c["url"];
                if !url.contains('/') {
                    return c[0].to_string();
                }
                match relocate_link_target(url, old_url, old_is_index, &new_folder, new_url, exts) {
                    Some(n) => format!(
                        "{}{}{}{}{}",
                        &c["pre"], n, &c["anchor"], &c["disp"], &c["post"]
                    ),
                    None => c[0].to_string(),
                }
            })
            .into_owned();
    }

    out
}

/// Rewrites a single bare wiki link name `[[old_name]]` to `[[new_stem]]`,
/// preserving any `#anchor` and `|display`. Case-insensitive and
/// delimiter-anchored (so `[[guide]]` never rewrites `[[guide-2]]`). Path-style
/// names (containing `/`) are left to [`rewrite_moved_file_outbound_links`].
pub fn rewrite_bare_wikilink(content: &str, old_name: &str, new_stem: &str) -> String {
    if old_name.contains('/') || old_name.trim().is_empty() {
        return content.to_string();
    }
    let esc = regex::escape(old_name.trim());
    let re = match Regex::new(&format!(
        r"(?i)(?P<pre>\[\[)\s*{esc}\s*(?P<anchor>(?:#[^\]|]*)?)(?P<disp>(?:\|[^\]]*)?)(?P<post>\]\])"
    )) {
        Ok(re) => re,
        Err(_) => return content.to_string(),
    };
    re.replace_all(content, |c: &Captures| {
        format!(
            "{}{}{}{}{}",
            &c["pre"], new_stem, &c["anchor"], &c["disp"], &c["post"]
        )
    })
    .into_owned()
}

/// Writes `bytes` to `path` atomically via a temp file in the same directory
/// followed by a rename.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file.md");
    let tmp = parent.join(format!(".{file_name}.mbr-tmp"));
    std::fs::write(&tmp, bytes)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Iterates markdown files under `root_dir`, skipping ignored directories and
/// the paths in `skip_abs`. Yields `(path, page_url, folder_url)` for each, as
/// computed by [`page_and_folder_urls`] — so with `index_file` supplied the
/// page URL is the canonical one (`docs/index.md` → `/docs/`) while the folder
/// URL stays the one relative links resolve against (`/docs/`).
fn markdown_files<'a>(
    root_dir: &'a Path,
    markdown_extensions: &'a [String],
    ignore_dirs: &'a [String],
    ignore_globs: &'a [String],
    index_file: Option<&'a str>,
    skip_abs: &'a HashSet<PathBuf>,
) -> impl Iterator<Item = (PathBuf, String, String)> + 'a {
    WalkDir::new(root_dir)
        .follow_links(true)
        .into_iter()
        .filter_entry(move |e| {
            let path = e.path();
            if path.is_dir()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                return !ignore_dirs.contains(&name.to_string());
            }
            true
        })
        .filter_map(|e| e.ok())
        .filter_map(move |entry| {
            let path = entry.path();
            if !path.is_file() || skip_abs.contains(path) {
                return None;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !markdown_extensions.contains(&ext) {
                return None;
            }
            if should_ignore(path, ignore_dirs, ignore_globs) {
                return None;
            }
            let (page_url, folder_url) =
                page_and_folder_urls(path, root_dir, markdown_extensions, index_file);
            Some((path.to_path_buf(), page_url, folder_url))
        })
}

/// Walks the repo and rewrites every page that links to the moved file
/// (`old_url` → `new_url`), returning the absolute paths that were changed
/// (A4-A).
///
/// Files are bucketed by folder URL and gated by a case-insensitive
/// Aho-Corasick automaton (mirroring [`crate::link_grep::find_inbound_links`]);
/// only gate hits run the full [`rewrite_links_for_move`] rewrite and an atomic
/// write.
pub fn rewrite_inbound_links_for_move(
    old_url: &str,
    new_url: &str,
    root_dir: &Path,
    markdown_extensions: &[String],
    ignore_dirs: &[String],
    ignore_globs: &[String],
    skip_abs: &HashSet<PathBuf>,
) -> std::io::Result<Vec<PathBuf>> {
    if old_url.trim_matches('/').is_empty() {
        return Ok(Vec::new());
    }
    let new_norm = new_url.trim_end_matches('/');

    // Bucket files by folder URL so per-folder patterns are computed once.
    let mut folder_files: HashMap<String, Vec<PathBuf>> = HashMap::new();
    // No index file here: this walker's caller does not carry one, and the
    // moved file itself is already excluded by `skip_abs`, so the positional
    // page URL below is only a defensive second check.
    for (path, source_url, folder) in markdown_files(
        root_dir,
        markdown_extensions,
        ignore_dirs,
        ignore_globs,
        None,
        skip_abs,
    ) {
        // Defensive: never rewrite the moved file itself.
        if source_url.trim_end_matches('/') == new_norm {
            continue;
        }
        folder_files.entry(folder).or_default().push(path);
    }

    let mut changed = Vec::new();
    for (folder, files) in &folder_files {
        let patterns = compute_patterns_for_folder(folder, old_url);
        if patterns.is_empty() {
            continue;
        }
        let Ok(ac) = AhoCorasickBuilder::new()
            .ascii_case_insensitive(true)
            .match_kind(MatchKind::LeftmostFirst)
            .build(&patterns)
        else {
            continue;
        };

        for path in files {
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            if !ac.is_match(&content) {
                continue;
            }
            let rewritten = rewrite_links_for_move(old_url, new_url, folder, &content);
            if rewritten != content {
                atomic_write(path, rewritten.as_bytes())?;
                changed.push(path.clone());
            }
        }
    }
    Ok(changed)
}

/// Walks the repo and rewrites bare `[[Name]]` links for a rename, mapping each
/// old name in `delta` to `new_stem` (A4-C). Only rewrites a name in a file
/// when the **pre-move** wikilink index resolves that name (from that file) to
/// `old_url`, so links pointing at a *different* note are left intact.
///
/// Returns the absolute paths that were changed. `delta` is a list of
/// `(old_name, new_stem)` pairs; typically just the changed filename stem.
#[allow(clippy::too_many_arguments)]
pub fn rewrite_bare_wikilinks_for_rename(
    delta: &[(String, String)],
    old_url: &str,
    root_dir: &Path,
    markdown_extensions: &[String],
    ignore_dirs: &[String],
    ignore_globs: &[String],
    index_file: &str,
    wikilink_index: &WikilinkIndex,
    skip_abs: &HashSet<PathBuf>,
) -> std::io::Result<Vec<PathBuf>> {
    if delta.is_empty() {
        return Ok(Vec::new());
    }
    let old_norm = normalize_url_path(old_url);
    let mut changed = Vec::new();

    // Index-aware page URLs for the resolution guard (the wikilink index is
    // keyed on index-stripped URLs).
    for (path, file_url, _folder) in markdown_files(
        root_dir,
        markdown_extensions,
        ignore_dirs,
        ignore_globs,
        Some(index_file),
        skip_abs,
    ) {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let file_is_index = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == index_file);

        let mut new_content = content.clone();
        let mut file_changed = false;
        for (name, new_stem) in delta {
            if normalize_name(name) == normalize_name(new_stem) {
                continue;
            }
            // Guard: only rewrite names that (pre-move) resolved to old_url.
            match wikilink_index.resolve_wikilink(name, &file_url, file_is_index) {
                Some(u) if normalize_url_path(&u) == old_norm => {}
                _ => continue,
            }
            let updated = rewrite_bare_wikilink(&new_content, name, new_stem);
            if updated != new_content {
                new_content = updated;
                file_changed = true;
            }
        }
        if file_changed {
            atomic_write(&path, new_content.as_bytes())?;
            changed.push(path);
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== rewrite_links_for_move: inline links =====

    #[test]
    fn absolute_inline_link_rewritten() {
        // A sibling folder links absolutely; style preserved.
        let content = "See [Guide](/docs/guide/).";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/notes/", content);
        assert_eq!(out, "See [Guide](/docs/manual/).");
    }

    #[test]
    fn relative_inline_link_rewritten() {
        // Same-folder relative link: /docs/ folder → `guide/`.
        let content = "See [Guide](guide/).";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/docs/", content);
        assert_eq!(out, "See [Guide](manual/).");
    }

    #[test]
    fn dot_slash_relative_link_rewritten() {
        let content = "See [Guide](./guide/).";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/docs/", content);
        assert_eq!(out, "See [Guide](./manual/).");
    }

    #[test]
    fn parent_traversal_relative_link_rewritten() {
        // From /docs/sub/ a link to /docs/guide/ is `../guide/`.
        let content = "See [Guide](../guide/).";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/docs/sub/", content);
        assert_eq!(out, "See [Guide](../manual/).");
    }

    #[test]
    fn md_extension_preserved() {
        let content = "See [Guide](guide.md).";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/docs/", content);
        assert_eq!(out, "See [Guide](manual.md).");
    }

    #[test]
    fn trailing_slash_and_anchor_preserved() {
        let content = "See [Guide](/docs/guide/#intro).";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/notes/", content);
        assert_eq!(out, "See [Guide](/docs/manual/#intro).");
    }

    #[test]
    fn query_string_preserved() {
        let content = "See [Guide](/docs/guide?tab=1).";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/notes/", content);
        assert_eq!(out, "See [Guide](/docs/manual?tab=1).");
    }

    #[test]
    fn md_extension_with_anchor_preserved() {
        let content = "See [Guide](guide.md#intro).";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/docs/", content);
        assert_eq!(out, "See [Guide](manual.md#intro).");
    }

    // ===== rewrite_links_for_move: prefix-collision negative =====

    #[test]
    fn prefix_collision_not_rewritten() {
        // Moving /docs/guide must NOT touch links to /docs/guide-2.
        let content = "A [x](/docs/guide-2/) and [y](/docs/guide/).";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/notes/", content);
        assert_eq!(out, "A [x](/docs/guide-2/) and [y](/docs/manual/).");
    }

    #[test]
    fn prefix_collision_relative_not_rewritten() {
        let content = "A [x](guide-2/) and [y](guide/).";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/docs/", content);
        assert_eq!(out, "A [x](guide-2/) and [y](manual/).");
    }

    // ===== rewrite_links_for_move: reference definitions =====

    #[test]
    fn reference_definition_rewritten_use_untouched() {
        // The [ref]: url definition is rewritten; the [text][ref] use is not a URL.
        let content = "Intro [see][g] more.\n\n[g]: /docs/guide/\n";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/notes/", content);
        assert_eq!(out, "Intro [see][g] more.\n\n[g]: /docs/manual/\n");
    }

    #[test]
    fn reference_definition_prefix_collision_not_rewritten() {
        let content = "[g]: /docs/guide-2/\n";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/notes/", content);
        assert_eq!(out, "[g]: /docs/guide-2/\n");
    }

    #[test]
    fn reference_definition_with_title_rewritten() {
        let content = "[g]: /docs/guide \"The Guide\"\n";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/notes/", content);
        assert_eq!(out, "[g]: /docs/manual \"The Guide\"\n");
    }

    // ===== rewrite_links_for_move: path-style wiki links =====

    #[test]
    fn path_wiki_with_display_rewritten() {
        let content = "See [[docs/guide|The Guide]].";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/", content);
        assert_eq!(out, "See [[docs/manual|The Guide]].");
    }

    #[test]
    fn path_wiki_with_anchor_rewritten() {
        let content = "See [[docs/guide#intro]].";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/", content);
        assert_eq!(out, "See [[docs/manual#intro]].");
    }

    #[test]
    fn bare_wiki_left_untouched_by_move() {
        // A bare [[guide]] (no slash) is handled by the rename path, not here.
        let content = "See [[guide]].";
        let out = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/docs/", content);
        assert_eq!(out, "See [[guide]].");
    }

    // ===== rewrite_links_for_move: idempotence =====

    #[test]
    fn rewrite_is_idempotent() {
        let content = "See [Guide](/docs/guide/) and [rel](guide/) and [[docs/guide]].";
        let once = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/docs/", content);
        let twice = rewrite_links_for_move("/docs/guide/", "/docs/manual/", "/docs/", &once);
        assert_eq!(once, twice);
    }

    // ===== rewrite_moved_file_outbound_links (A4-B) =====

    #[test]
    fn moved_file_same_folder_leaves_relative_links() {
        // Rename within the same folder: sibling links are unchanged...
        let exts = vec!["md".to_string()];
        let content = "See [Other](other/) and [abs](/x/).";
        let out = rewrite_moved_file_outbound_links(
            "/docs/guide/",
            false,
            "/docs/manual/",
            false,
            &exts,
            content,
        );
        assert_eq!(out, "See [Other](other/) and [abs](/x/).");
    }

    #[test]
    fn moved_file_self_link_repointed() {
        // ...but a self-link is re-pointed at the new name.
        let exts = vec!["md".to_string()];
        let content = "Back to [me](guide/).";
        let out = rewrite_moved_file_outbound_links(
            "/docs/guide/",
            false,
            "/docs/manual/",
            false,
            &exts,
            content,
        );
        assert_eq!(out, "Back to [me](manual/).");
    }

    #[test]
    fn moved_file_cross_folder_rebases_relative_links() {
        // Move /a/guide.md → /b/guide.md: sibling link `other/` (meaning /a/other/)
        // must become `../a/other/`.
        let exts = vec!["md".to_string()];
        let content = "See [Other](other/).";
        let out = rewrite_moved_file_outbound_links(
            "/a/guide/",
            false,
            "/b/guide/",
            false,
            &exts,
            content,
        );
        assert_eq!(out, "See [Other](../a/other/).");
    }

    #[test]
    fn moved_file_absolute_and_external_untouched() {
        let exts = vec!["md".to_string()];
        let content = "A [abs](/x/y/) and [ext](https://example.com/) and [anchor](#top).";
        let out = rewrite_moved_file_outbound_links(
            "/a/guide/",
            false,
            "/b/guide/",
            false,
            &exts,
            content,
        );
        assert_eq!(out, content);
    }

    #[test]
    fn moved_index_file_self_link_repointed() {
        // Index page /docs/ (docs/index.md) moving to /guides/ (guides/index.md).
        // A self link `./` or `../docs/` resolving to itself is repointed.
        let exts = vec!["md".to_string()];
        let content = "See [child](child/).";
        let out =
            rewrite_moved_file_outbound_links("/docs/", true, "/guides/", true, &exts, content);
        // child/ (meaning /docs/child/) becomes ../docs/child/ from /guides/.
        assert_eq!(out, "See [child](../docs/child/).");
    }

    // ===== rewrite_bare_wikilink (A4-C pure) =====

    #[test]
    fn bare_wikilink_rewritten_case_insensitive() {
        let out = rewrite_bare_wikilink("See [[Guide]] now.", "guide", "manual");
        assert_eq!(out, "See [[manual]] now.");
    }

    #[test]
    fn bare_wikilink_preserves_anchor_and_display() {
        let out = rewrite_bare_wikilink("See [[guide#intro|The Guide]].", "guide", "manual");
        assert_eq!(out, "See [[manual#intro|The Guide]].");
    }

    #[test]
    fn bare_wikilink_prefix_collision_untouched() {
        let out = rewrite_bare_wikilink("See [[guide-2]] and [[guide]].", "guide", "manual");
        assert_eq!(out, "See [[guide-2]] and [[manual]].");
    }

    #[test]
    fn bare_wikilink_multiword_name() {
        let out = rewrite_bare_wikilink("See [[Patrick Walsh]].", "Patrick Walsh", "pw");
        assert_eq!(out, "See [[pw]].");
    }

    // ===== collect_move_bases / index-vs-nonindex =====

    #[test]
    fn index_base_computation_nonindex_folder() {
        // From folder /docs/ the same-folder page uses `guide` relative form.
        let bases = collect_move_bases("/docs/", "/docs/guide/");
        assert!(bases.contains(&"/docs/guide".to_string()));
        assert!(bases.contains(&"guide".to_string()));
        assert!(bases.contains(&"./guide".to_string()));
    }

    #[test]
    fn index_target_absolute_link_rewritten() {
        // Moving an index page /docs/ → /guides/: an absolute link is rewritten.
        let content = "Home [docs](/docs/).";
        let out = rewrite_links_for_move("/docs/", "/guides/", "/notes/", content);
        assert_eq!(out, "Home [docs](/guides/).");
    }
}
