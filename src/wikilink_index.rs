//! Global name index for body-wikilink (`[[Name]]`) resolution.
//!
//! Obsidian-style resolution: a bare `[[Name]]` in a page body resolves to a
//! file with that name in the **current folder** first; if none exists there,
//! it falls back to the **first matching file anywhere** in the repo. Only
//! `404`s when nothing matches. Regular `[text](path.md)` links are unaffected.
//!
//! This index provides only the *rewrite* lookup:
//! [`WikilinkIndex::resolve_wikilink`] returns `Some(url)` **only** when a
//! rewrite to an absolute URL is needed, so a same-folder link the author
//! spelled exactly as the file is named keeps the renderer's default relative
//! transform byte-for-byte.
//!
//! Resolution semantics mirror [`crate::relationships`]: names are matched
//! case-insensitively (via [`crate::relationships::normalize_name`]) against
//! note titles, then aliases, then filename stems, with ambiguities resolved
//! deterministically to the lexicographically-smallest URL.
//!
//! Modelled on [`crate::tag_index::TagIndex`] and
//! [`crate::relationships::RelationshipIndex`]: papaya-backed, rebuilt after a
//! scan (and on live file changes in server mode), held behind an `Arc` on
//! [`crate::repo::Repo`].

use std::collections::HashMap;

use papaya::HashMap as ConcurrentHashMap;

use crate::relationships::{NoteRelInput, normalize_name};

/// Thread-safe global name index for `[[Name]]` body-wikilink resolution.
pub struct WikilinkIndex {
    /// normalized title -> url (first insertion wins in sorted-URL order).
    by_title: ConcurrentHashMap<String, String>,
    /// normalized alias -> url.
    by_alias: ConcurrentHashMap<String, String>,
    /// normalized filename stem -> url.
    by_stem: ConcurrentHashMap<String, String>,
    /// (normalized folder, normalized stem) -> url, for current-folder-first.
    by_dir_stem: ConcurrentHashMap<(String, String), String>,
}

impl Default for WikilinkIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl WikilinkIndex {
    /// Creates a new empty index.
    pub fn new() -> Self {
        Self {
            by_title: ConcurrentHashMap::new(),
            by_alias: ConcurrentHashMap::new(),
            by_stem: ConcurrentHashMap::new(),
            by_dir_stem: ConcurrentHashMap::new(),
        }
    }

    /// Rebuilds the index from the given notes.
    ///
    /// Notes are visited in sorted-URL order so ambiguous names resolve
    /// deterministically to the lexicographically-smallest URL (first insertion
    /// wins), matching [`crate::relationships`]' name resolution.
    pub fn rebuild(&self, notes: &[NoteRelInput]) {
        let mut sorted: Vec<&NoteRelInput> = notes.iter().collect();
        sorted.sort_by(|a, b| a.url.cmp(&b.url));

        let mut by_title: HashMap<String, String> = HashMap::new();
        let mut by_alias: HashMap<String, String> = HashMap::new();
        let mut by_stem: HashMap<String, String> = HashMap::new();
        let mut by_dir_stem: HashMap<(String, String), String> = HashMap::new();

        for note in &sorted {
            by_title
                .entry(normalize_name(&note.title))
                .or_insert_with(|| note.url.clone());
            for alias in &note.aliases {
                by_alias
                    .entry(normalize_name(alias))
                    .or_insert_with(|| note.url.clone());
            }
            let stem_key = normalize_name(&note.stem);
            by_stem
                .entry(stem_key.clone())
                .or_insert_with(|| note.url.clone());
            by_dir_stem
                .entry((page_folder(&note.url, note.is_index), stem_key))
                .or_insert_with(|| note.url.clone());
        }

        swap_in(&self.by_title, by_title);
        swap_in(&self.by_alias, by_alias);
        swap_in(&self.by_stem, by_stem);
        swap_in(&self.by_dir_stem, by_dir_stem);
    }

    /// Resolves a bare body-wikilink name to an absolute URL **only** when a
    /// global-fallback rewrite is required; returns `None` when the renderer's
    /// default relative transform already resolves (or nothing matches).
    ///
    /// `name` is the raw wikilink target (bare — the caller guards on `/`); any
    /// trailing `#anchor` is split off, the base name resolved, and the anchor
    /// re-appended to the returned URL.
    ///
    /// - **Current-folder-first, spelled exactly** — if a file with this stem
    ///   exists in the current page's folder *and* the wikilink text matches
    ///   that file's URL segment character for character, returns `None` (the
    ///   default `../Name/` transform already points at it, so behaviour is
    ///   unchanged).
    /// - **Current-folder-first, spelled differently** — stems match
    ///   case-insensitively, but the default transform emits the author's
    ///   spelling verbatim, so `[[japan]]` next to `Japan.md` would produce
    ///   `../japan/`: a 404 on any case-sensitive filesystem and a lost
    ///   backlink. Those return `Some(absolute_url)` so the href points at the
    ///   real page.
    /// - **Global fallback** — otherwise resolves `name` against title, then
    ///   alias, then stem, and returns `Some(absolute_url)`.
    /// - **Not found anywhere** — returns `None` (caller keeps default → 404).
    ///
    /// Two notes in one folder whose stems differ only by case (`Japan.md` and
    /// `japan.md`) collide on this index's `(folder, normalized stem)` key, so
    /// the module-wide ambiguity rule applies: the lexicographically-smallest
    /// URL wins and *every* spelling resolves to it.
    pub fn resolve_wikilink(
        &self,
        name: &str,
        current_page_url: &str,
        current_is_index: bool,
    ) -> Option<String> {
        let (base, anchor) = match name.split_once('#') {
            Some((base, anchor)) => (base.trim(), Some(anchor)),
            None => (name.trim(), None),
        };
        if base.is_empty() {
            return None;
        }
        let key = normalize_name(base);

        // Current-folder-first: a file with this stem in the current page's
        // folder is already the target of the renderer's default relative
        // transform — but only when the author spelled it the way the URL does,
        // since that transform passes the raw text straight through.
        let current_folder = page_folder(current_page_url, current_is_index);
        {
            let by_dir_stem = self.by_dir_stem.pin();
            if let Some(url) = by_dir_stem.get(&(current_folder, key.clone())) {
                return (last_segment(url) != base).then(|| with_anchor(url.clone(), anchor));
            }
        }

        // Global fallback: title -> alias -> stem.
        let url = {
            let title = self.by_title.pin();
            let alias = self.by_alias.pin();
            let stem = self.by_stem.pin();
            title
                .get(&key)
                .or_else(|| alias.get(&key))
                .or_else(|| stem.get(&key))
                .cloned()
        }?;

        Some(with_anchor(url, anchor))
    }

    /// Clears the index.
    pub fn clear(&self) {
        self.by_title.pin().clear();
        self.by_alias.pin().clear();
        self.by_stem.pin().clear();
        self.by_dir_stem.pin().clear();
    }

    /// Returns true when the index holds no entries.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.by_title.pin().is_empty()
            && self.by_alias.pin().is_empty()
            && self.by_stem.pin().is_empty()
            && self.by_dir_stem.pin().is_empty()
    }
}

/// Clears `target` and repopulates it from `source`. Mirrors the
/// clear-then-insert rebuild used by [`crate::relationships::RelationshipIndex`].
fn swap_in<K>(target: &ConcurrentHashMap<K, String>, source: HashMap<K, String>)
where
    K: Clone + Eq + std::hash::Hash + Send + Sync + 'static,
{
    let guard = target.pin();
    guard.clear();
    for (key, value) in source {
        guard.insert(key, value);
    }
}

/// The last non-empty segment of a site URL: the text a bare wikilink has to
/// use for the renderer's default relative transform to reach that page.
fn last_segment(url: &str) -> &str {
    url.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("")
}

/// Re-appends the anchor that [`WikilinkIndex::resolve_wikilink`] split off.
fn with_anchor(url: String, anchor: Option<&str>) -> String {
    match anchor {
        Some(anchor) => format!("{url}#{anchor}"),
        None => url,
    }
}

/// The folder a note (or the current page) lives in, normalized for
/// `by_dir_stem` keys and current-folder comparison.
///
/// Mirrors the base-directory logic in
/// [`crate::link_index::resolve_relative_url`]: for a non-index page the last
/// URL segment is the file stem, so the folder is its parent; for an index page
/// every segment is a real directory component.
fn page_folder(url: &str, is_index: bool) -> String {
    let segments: Vec<&str> = url
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let folder = if is_index || segments.is_empty() {
        segments.join("/")
    } else {
        segments[..segments.len() - 1].join("/")
    };
    normalize_name(&folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(url: &str, title: &str, stem: &str, is_index: bool) -> NoteRelInput {
        NoteRelInput {
            url: url.to_string(),
            title: title.to_string(),
            stem: stem.to_string(),
            aliases: Vec::new(),
            is_index,
            relationships: Vec::new(),
        }
    }

    #[test]
    fn same_folder_stem_returns_none() {
        // A file whose stem matches lives in the referencing page's folder, so
        // the default relative transform already resolves — no rewrite.
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note(
                "/notes/patrick-walsh/",
                "Patrick Walsh",
                "patrick-walsh",
                false,
            ),
            note("/notes/family/", "Family", "family", false),
        ]);
        assert_eq!(
            idx.resolve_wikilink("patrick-walsh", "/notes/family/", false),
            None
        );
    }

    #[test]
    fn same_folder_case_mismatch_rewrites_to_the_real_url() {
        // `notes/Japan.md` with a sibling writing `[[japan]]`: the default
        // transform would emit `../japan/`, which 404s on a case-sensitive
        // filesystem and drops the backlink from the static build.
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/notes/Japan/", "Japan", "Japan", false),
            note("/notes/other/", "Other", "other", false),
        ]);
        assert_eq!(
            idx.resolve_wikilink("japan", "/notes/other/", false),
            Some("/notes/Japan/".to_string())
        );
        // Exact spelling still needs no rewrite.
        assert_eq!(idx.resolve_wikilink("Japan", "/notes/other/", false), None);
    }

    #[test]
    fn same_folder_case_mismatch_preserves_anchor() {
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/notes/Japan/", "Japan", "Japan", false),
            note("/notes/other/", "Other", "other", false),
        ]);
        assert_eq!(
            idx.resolve_wikilink("japan#food", "/notes/other/", false),
            Some("/notes/Japan/#food".to_string())
        );
    }

    #[test]
    fn same_folder_case_variants_all_resolve_to_smallest_url() {
        // Two notes in one folder differing only by case collide on the
        // (folder, normalized stem) key, so the module-wide ambiguity rule
        // applies: `/notes/Japan/` sorts first and wins for every spelling.
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/notes/japan/", "japan", "japan", false),
            note("/notes/Japan/", "Japan", "Japan", false),
            note("/notes/other/", "Other", "other", false),
        ]);
        // Exact match on the winner: default relative transform already works.
        assert_eq!(idx.resolve_wikilink("Japan", "/notes/other/", false), None);
        // Any other spelling is rewritten to the same winner.
        assert_eq!(
            idx.resolve_wikilink("japan", "/notes/other/", false),
            Some("/notes/Japan/".to_string())
        );
        assert_eq!(
            idx.resolve_wikilink("JAPAN", "/notes/other/", false),
            Some("/notes/Japan/".to_string())
        );
    }

    #[test]
    fn same_folder_index_note_resolves_to_the_folder_url() {
        // `notes/index.md` is served at `/notes/`, so `[[index]]` from a
        // sibling cannot use the default `../index/` transform.
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/notes/", "Notes", "index", true),
            note("/notes/other/", "Other", "other", false),
        ]);
        assert_eq!(
            idx.resolve_wikilink("index", "/notes/other/", false),
            Some("/notes/".to_string())
        );
    }

    #[test]
    fn global_fallback_returns_absolute_url() {
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note(
                "/walsh/patrick-walsh/",
                "Patrick Walsh",
                "patrick-walsh",
                false,
            ),
            note("/notes/family/", "Family", "family", false),
        ]);
        // From /notes/family/, `[[Patrick Walsh]]` is not in /notes/, so it
        // resolves globally to the matching file's absolute URL.
        assert_eq!(
            idx.resolve_wikilink("Patrick Walsh", "/notes/family/", false),
            Some("/walsh/patrick-walsh/".to_string())
        );
    }

    #[test]
    fn ambiguous_name_resolves_to_smallest_url() {
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/z/sam/", "Sam", "sam", false),
            note("/a/sam/", "Sam", "sam", false),
        ]);
        assert_eq!(
            idx.resolve_wikilink("Sam", "/other/page/", false),
            Some("/a/sam/".to_string())
        );
    }

    #[test]
    fn resolution_is_case_insensitive_title_alias_stem() {
        let idx = WikilinkIndex::new();
        let mut mary = note("/people/mary/", "Mary Smith", "mary", false);
        mary.aliases = vec!["Mary Doe".to_string()];
        idx.rebuild(&[mary]);
        // Title match (different case).
        assert_eq!(
            idx.resolve_wikilink("mary smith", "/x/", false),
            Some("/people/mary/".to_string())
        );
        // Alias match (different case).
        assert_eq!(
            idx.resolve_wikilink("MARY DOE", "/x/", false),
            Some("/people/mary/".to_string())
        );
        // Stem match.
        assert_eq!(
            idx.resolve_wikilink("Mary", "/x/", false),
            Some("/people/mary/".to_string())
        );
    }

    #[test]
    fn missing_name_returns_none() {
        let idx = WikilinkIndex::new();
        idx.rebuild(&[note("/a/", "A", "a", false)]);
        assert_eq!(idx.resolve_wikilink("Nonexistent", "/x/", false), None);
    }

    #[test]
    fn anchor_is_preserved() {
        let idx = WikilinkIndex::new();
        idx.rebuild(&[note(
            "/walsh/patrick-walsh/",
            "Patrick Walsh",
            "patrick-walsh",
            false,
        )]);
        assert_eq!(
            idx.resolve_wikilink("Patrick Walsh#early-life", "/notes/x/", false),
            Some("/walsh/patrick-walsh/#early-life".to_string())
        );
    }

    #[test]
    fn index_page_folder_is_the_directory_itself() {
        // An index note at /people/ (index.md) sits in folder "people"; a
        // sibling by stem is treated as same-folder.
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/people/", "People", "index", true),
            note("/people/john/", "John", "john", false),
        ]);
        // From the index page /people/, `[[john]]` is a same-folder sibling.
        assert_eq!(idx.resolve_wikilink("john", "/people/", true), None);
    }

    #[test]
    fn rebuild_replaces_prior_contents() {
        let idx = WikilinkIndex::new();
        idx.rebuild(&[note("/a/", "Alpha", "alpha", false)]);
        assert!(!idx.is_empty());
        idx.rebuild(&[]);
        assert!(idx.is_empty());
    }
}
