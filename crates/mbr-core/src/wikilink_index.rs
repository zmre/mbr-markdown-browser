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

use std::collections::{BTreeMap, HashMap};

use papaya::HashMap as ConcurrentHashMap;

use crate::relationships::{NoteRelInput, normalize_name};

/// A bare body wikilink whose name is shared by several notes.
///
/// Resolution is unchanged (first-wins, lexicographically-smallest URL); this
/// only records that the choice was arbitrary. Reported per page via
/// `errors.json`, because the page containing the `[[link]]` is the one whose
/// author has to disambiguate it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguousWikilink {
    /// The wikilink as authored, e.g. "[[John Doe]]". Reconstructed from the
    /// link target, so a piped `[[Target|Display]]` reports as `[[Target]]` —
    /// the target is the part that was ambiguous.
    pub raw: String,
    /// The note URL it resolved to.
    pub resolved_to: String,
    /// The other notes sharing that name.
    pub candidates: Vec<String>,
}

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
    /// Normalized name -> every note URL answering to it (sorted), for names
    /// owned by **more than one** note across the global title/alias/stem tiers.
    ///
    /// Unambiguous names are absent, so this is empty for most repositories and
    /// [`WikilinkIndex::ambiguity_for`] short-circuits on the first lookup.
    ambiguous_names: ConcurrentHashMap<String, Vec<String>>,
    /// (normalized folder, normalized stem) -> every note URL in that folder
    /// answering to the stem, when more than one does (`Japan.md` next to
    /// `japan.md`). Needed separately because a same-folder hit wins outright:
    /// without this, that unambiguous win would be judged against the whole-repo
    /// name table and falsely reported.
    ambiguous_dir_stems: ConcurrentHashMap<(String, String), Vec<String>>,
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
            ambiguous_names: ConcurrentHashMap::new(),
            ambiguous_dir_stems: ConcurrentHashMap::new(),
        }
    }

    /// Rebuilds the index from the given notes.
    ///
    /// Notes are visited in sorted-URL order so ambiguous names resolve
    /// deterministically to the lexicographically-smallest URL (first insertion
    /// wins), matching [`crate::relationships`]' name resolution.
    ///
    /// Also records which names *are* ambiguous, populating the two tables
    /// [`Self::ambiguity_for`] reads. Detection only: the resolution tables above
    /// are built exactly as before.
    ///
    /// **Deliberately silent** — nothing here logs. Reporting is
    /// [`Self::ambiguity_for`]'s job, surfaced as the per-page
    /// `ambiguous_wikilink` entry in `errors.json`, for two reasons this function
    /// cannot work around:
    ///
    /// - It is passed notes, never links, so it cannot tell a collision someone
    ///   wrote `[[Name]]` for from one nothing in the repository references. A
    ///   name two notes merely share is not a problem; only a *link* through it
    ///   is. Warning from here means a repository with no wikilinks at all
    ///   getting a screenful of warnings — on every scan, and again on every
    ///   watcher batch and editor save.
    /// - Ambiguity is decided per link site, not globally: a single same-folder
    ///   owner wins outright ([`Self::resolve_wikilink`] is current-folder-first),
    ///   so the same name is ambiguous from one page and unambiguous from its
    ///   neighbour, and the winner depends on where the link is. Only
    ///   [`Self::ambiguity_for`] knows the folder — a warning built from these
    ///   tables alone would name the lexicographically-smallest URL as the
    ///   winner and contradict the resolver.
    pub fn rebuild(&self, notes: &[NoteRelInput]) {
        let mut sorted: Vec<&NoteRelInput> = notes.iter().collect();
        sorted.sort_by(|a, b| a.url.cmp(&b.url));

        let mut by_title: HashMap<String, String> = HashMap::new();
        let mut by_alias: HashMap<String, String> = HashMap::new();
        let mut by_stem: HashMap<String, String> = HashMap::new();
        let mut by_dir_stem: HashMap<(String, String), String> = HashMap::new();
        // Owner lists for ambiguity detection. `BTreeMap` so a rebuild is
        // reproducible; each list stays in sorted-URL order because `sorted` is,
        // which is what lets `ambiguity_for` subtract the winner and report the
        // rest in a stable order.
        let mut name_owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut dir_stem_owners: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();

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
            let dir_stem_key = (page_folder(&note.url, note.is_index), stem_key);
            by_dir_stem
                .entry(dir_stem_key.clone())
                .or_insert_with(|| note.url.clone());

            // A note answering to the same name through two tiers (title ==
            // stem, the common case) is not ambiguous with itself, so dedupe by
            // URL.
            //
            // An index note's stem is a filename artifact, not a name anyone
            // means: every repository with more than one `index.md` would
            // otherwise warn that "index" is ambiguous on every single startup.
            // Its title still participates, and resolution is untouched —
            // `by_stem`/`by_dir_stem` above are built from every note.
            let stem_as_name = (!note.is_index).then_some(&note.stem);
            let names = std::iter::once(&note.title)
                .chain(&note.aliases)
                .chain(stem_as_name);
            for name in names {
                add_owner(
                    name_owners.entry(normalize_name(name)).or_default(),
                    &note.url,
                );
            }
            add_owner(dir_stem_owners.entry(dir_stem_key).or_default(), &note.url);
        }

        swap_in(&self.by_title, by_title);
        swap_in(&self.by_alias, by_alias);
        swap_in(&self.by_stem, by_stem);
        swap_in(&self.by_dir_stem, by_dir_stem);

        let ambiguous_names = retain_ambiguous(name_owners);
        let ambiguous_dir_stems = retain_ambiguous(dir_stem_owners);
        swap_in_values(&self.ambiguous_names, ambiguous_names);
        swap_in_values(&self.ambiguous_dir_stems, ambiguous_dir_stems);
    }

    /// Reports whether a bare body wikilink resolves through a name owned by
    /// more than one note, judged the way [`Self::resolve_wikilink`] actually
    /// resolves.
    ///
    /// `name` is the raw wikilink target, exactly as passed to
    /// [`Self::resolve_wikilink`]. Returns `None` when the name is unique, when
    /// nothing matches, or — importantly — when a **single** same-folder note
    /// owns the stem: that note wins outright, so however many namesakes exist
    /// elsewhere in the repository the link is not ambiguous.
    pub fn ambiguity_for(
        &self,
        name: &str,
        current_page_url: &str,
        current_is_index: bool,
    ) -> Option<AmbiguousWikilink> {
        // Almost every repository has no ambiguous names at all; keep the cost
        // for those at one empty-map check per wikilink.
        if self.ambiguous_names.pin().is_empty() && self.ambiguous_dir_stems.pin().is_empty() {
            return None;
        }

        let base = match name.split_once('#') {
            Some((base, _anchor)) => base.trim(),
            None => name.trim(),
        };
        if base.is_empty() {
            return None;
        }
        let key = normalize_name(base);

        // Tier 1 — current folder, mirroring `resolve_wikilink`.
        let dir_stem_key = (page_folder(current_page_url, current_is_index), key.clone());
        if let Some(winner) = self.by_dir_stem.pin().get(&dir_stem_key).cloned() {
            let candidates = self
                .ambiguous_dir_stems
                .pin()
                .get(&dir_stem_key)
                .map(|urls| other_candidates(urls, &winner))
                .unwrap_or_default();
            return Self::ambiguous(base, winner, candidates);
        }

        // Tier 2 — global fallback: title -> alias -> stem.
        let winner = {
            let title = self.by_title.pin();
            let alias = self.by_alias.pin();
            let stem = self.by_stem.pin();
            title
                .get(&key)
                .or_else(|| alias.get(&key))
                .or_else(|| stem.get(&key))
                .cloned()
        }?;
        let candidates = self
            .ambiguous_names
            .pin()
            .get(&key)
            .map(|urls| other_candidates(urls, &winner))
            .unwrap_or_default();
        Self::ambiguous(base, winner, candidates)
    }

    /// Builds the finding, or `None` when no other note shares the name.
    fn ambiguous(
        base: &str,
        resolved_to: String,
        candidates: Vec<String>,
    ) -> Option<AmbiguousWikilink> {
        (!candidates.is_empty()).then(|| AmbiguousWikilink {
            raw: format!("[[{base}]]"),
            resolved_to,
            candidates,
        })
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
        self.ambiguous_names.pin().clear();
        self.ambiguous_dir_stems.pin().clear();
    }

    /// Returns true when the index holds no entries.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.by_title.pin().is_empty()
            && self.by_alias.pin().is_empty()
            && self.by_stem.pin().is_empty()
            && self.by_dir_stem.pin().is_empty()
            && self.ambiguous_names.pin().is_empty()
            && self.ambiguous_dir_stems.pin().is_empty()
    }
}

/// Appends `url` to an owner list unless it is already there, so a note that
/// answers to one name through several tiers counts once.
fn add_owner(owners: &mut Vec<String>, url: &str) {
    if !owners.iter().any(|owned| owned == url) {
        owners.push(url.to_string());
    }
}

/// Drops every name owned by exactly one note, leaving only the ambiguous ones.
fn retain_ambiguous<K: Ord>(owners: BTreeMap<K, Vec<String>>) -> BTreeMap<K, Vec<String>> {
    owners
        .into_iter()
        .filter(|(_, urls)| urls.len() > 1)
        .collect()
}

/// The owners of a name other than the one resolution picked.
fn other_candidates(owners: &[String], winner: &str) -> Vec<String> {
    owners
        .iter()
        .filter(|url| *url != winner)
        .cloned()
        .collect()
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

/// [`swap_in`] for the owner-list tables, whose values are `Vec<String>`.
fn swap_in_values<K>(target: &ConcurrentHashMap<K, Vec<String>>, source: BTreeMap<K, Vec<String>>)
where
    K: Clone + Eq + Ord + std::hash::Hash + Send + Sync + 'static,
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

    // ----- ambiguity reporting (detection only; resolution is unchanged) -----

    #[test]
    fn ambiguity_for_reports_notes_sharing_a_title() {
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/people/john-jr/", "John Doe", "john-jr", false),
            note("/people/john-sr/", "John Doe", "john-sr", false),
        ]);

        let found = idx
            .ambiguity_for("John Doe", "/notes/family/", false)
            .expect("shared title should be reported");
        assert_eq!(found.raw, "[[John Doe]]");
        assert_eq!(found.resolved_to, "/people/john-jr/");
        assert_eq!(found.candidates, vec!["/people/john-sr/".to_string()]);

        // Resolution itself is untouched: the smallest URL still wins.
        assert_eq!(
            idx.resolve_wikilink("John Doe", "/notes/family/", false),
            Some("/people/john-jr/".to_string())
        );
    }

    #[test]
    fn ambiguity_for_unique_name_returns_none() {
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/people/john/", "John Doe", "john", false),
            note("/people/mary/", "Mary Doe", "mary", false),
        ]);
        assert_eq!(idx.ambiguity_for("John Doe", "/x/", false), None);
    }

    #[test]
    fn ambiguity_for_missing_name_returns_none() {
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/a/sam/", "Sam", "sam", false),
            note("/z/sam/", "Sam", "sam", false),
        ]);
        // "Nobody" matches nothing, so there was no arbitrary choice to report.
        assert_eq!(idx.ambiguity_for("Nobody", "/x/", false), None);
    }

    #[test]
    fn ambiguity_for_same_folder_win_is_not_ambiguous() {
        // The current-folder rule settles this outright, so however many
        // namesakes exist elsewhere the link is unambiguous. Judging ambiguity
        // any other way would flag links that resolve exactly as intended.
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/notes/family/", "Family", "family", false),
            note("/notes/sam/", "Sam", "sam", false),
            note("/other/sam/", "Sam", "sam", false),
        ]);

        // From /notes/family/, `[[sam]]` hits the sibling in /notes/.
        assert_eq!(idx.ambiguity_for("sam", "/notes/family/", false), None);
        // From elsewhere there is no folder-local winner, so it *is* ambiguous.
        let found = idx
            .ambiguity_for("sam", "/elsewhere/page/", false)
            .expect("no folder-local winner, so the global choice is arbitrary");
        assert_eq!(found.resolved_to, "/notes/sam/");
        assert_eq!(found.candidates, vec!["/other/sam/".to_string()]);
    }

    #[test]
    fn ambiguity_for_same_folder_case_variants_is_reported() {
        // `Japan.md` and `japan.md` in one folder collide on the (folder,
        // normalized stem) key, so even the folder-local win is arbitrary.
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/notes/Japan/", "Japan", "Japan", false),
            note("/notes/japan/", "japan", "japan", false),
            note("/notes/other/", "Other", "other", false),
        ]);

        let found = idx
            .ambiguity_for("japan", "/notes/other/", false)
            .expect("case-variant siblings are ambiguous");
        assert_eq!(found.resolved_to, "/notes/Japan/");
        assert_eq!(found.candidates, vec!["/notes/japan/".to_string()]);
    }

    #[test]
    fn ambiguity_for_ignores_a_note_matching_through_two_tiers() {
        // Title and stem both normalise to "alpha" on the *same* note: one
        // owner, so nothing is ambiguous.
        let idx = WikilinkIndex::new();
        idx.rebuild(&[note("/a/alpha/", "alpha", "alpha", false)]);
        assert_eq!(idx.ambiguity_for("alpha", "/x/", false), None);
    }

    #[test]
    fn ambiguity_for_strips_the_anchor() {
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/a/sam/", "Sam", "sam", false),
            note("/z/sam/", "Sam", "sam", false),
        ]);
        let found = idx
            .ambiguity_for("Sam#early-life", "/x/", false)
            .expect("anchor must not defeat the lookup");
        assert_eq!(found.raw, "[[Sam]]");
        assert_eq!(found.resolved_to, "/a/sam/");
    }

    #[test]
    fn index_note_stems_are_not_reported_as_ambiguous() {
        // Every folder's `index.md` shares the stem "index". Reporting that
        // would mean a spurious warning in almost every real repository, so the
        // stem of an index note is excluded from ambiguity detection — while
        // resolution still works exactly as before.
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/docs/", "Docs", "index", true),
            note("/guide/", "Guide", "index", true),
            note("/notes/", "Notes", "index", true),
        ]);

        assert_eq!(idx.ambiguity_for("index", "/x/y/", false), None);
        // Resolution is unchanged: "index" still resolves, first-wins.
        assert_eq!(
            idx.resolve_wikilink("index", "/x/y/", false),
            Some("/docs/".to_string())
        );
        // Non-index notes sharing a stem are still reported.
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/a/note/", "A", "note", false),
            note("/z/note/", "Z", "note", false),
        ]);
        assert!(idx.ambiguity_for("note", "/x/y/", false).is_some());
    }

    /// 2 * `count` notes, pairwise sharing a title, in distinct folders.
    fn colliding_notes(count: usize) -> Vec<NoteRelInput> {
        (0..count)
            .flat_map(|i| {
                [
                    note(
                        &format!("/a{i:02}/"),
                        &format!("Dup {i:02}"),
                        &format!("a{i:02}"),
                        false,
                    ),
                    note(
                        &format!("/b{i:02}/"),
                        &format!("Dup {i:02}"),
                        &format!("b{i:02}"),
                        false,
                    ),
                ]
            })
            .collect()
    }

    #[test]
    fn ambiguity_detection_is_complete_and_silent() {
        // `rebuild` detects every ambiguous name and logs none of them.
        //
        // Silence is the point, not an oversight. Detection is ungated and runs
        // for every repository on every scan, watcher batch and editor save,
        // while `rebuild` is handed *notes* and never links: it cannot tell a
        // collision someone actually wrote `[[Dup 07]]` for from one nothing
        // references, so a warning here fires for repositories containing no
        // wikilinks at all. And it could not state the outcome correctly even
        // when a link does exist — resolution is current-folder-first, so the
        // winner depends on the linking page (see
        // `resolution_winner_is_folder_first_not_the_smallest_url`), which is
        // known only at the link site. That is `ambiguity_for`'s job, and its
        // findings reach the author as the per-page `ambiguous_wikilink` entry
        // in `errors.json`.
        let notes = colliding_notes(25);
        let idx = WikilinkIndex::new();
        let (_built, logs) = crate::test_support::capture_tracing(|| idx.rebuild(&notes));

        assert!(
            !logs.to_lowercase().contains("ambiguous"),
            "rebuild must not log about ambiguity: {logs}"
        );
        // Stronger, and the invariant that actually keeps a quiet startup: an
        // index build is a bookkeeping pass and has nothing to say.
        assert!(logs.is_empty(), "rebuild must not log at all: {logs}");

        // Detection itself is untouched and complete — every collision, not just
        // the first 20 that the removed log would have named.
        for i in 0..25 {
            let name = format!("Dup {i:02}");
            let found = idx
                .ambiguity_for(&name, "/x/", false)
                .unwrap_or_else(|| panic!("`{name}` is owned by two notes"));
            assert_eq!(found.raw, format!("[[{name}]]"));
            assert_eq!(found.resolved_to, format!("/a{i:02}/"));
            assert_eq!(found.candidates, vec![format!("/b{i:02}/")]);
        }
    }

    #[test]
    fn rebuild_is_silent_without_any_wikilink_in_the_repository() {
        // The regression: notes sharing a name are not a problem, a *link*
        // through a shared name is. `rebuild`'s input carries no links at all,
        // so a repository that uses zero `[[ ]]` wikilinks must see nothing in
        // its log no matter how many titles collide.
        let idx = WikilinkIndex::new();
        let (_built, logs) = crate::test_support::capture_tracing(|| {
            idx.rebuild(&colliding_notes(3));
        });
        assert!(logs.is_empty(), "silent index build expected, got: {logs}");

        // Rebuilding over an existing index stays silent too — this runs again
        // on every watcher batch and every editor save.
        let (_rebuilt, logs) = crate::test_support::capture_tracing(|| {
            idx.rebuild(&colliding_notes(3));
        });
        assert!(logs.is_empty(), "silent rebuild expected, got: {logs}");
    }

    #[test]
    fn resolution_winner_is_folder_first_not_the_smallest_url() {
        // Why a warning built from the global owner table would have been wrong,
        // and not merely noisy: it can only name the lexicographically-smallest
        // URL, which is *not* who wins. `ambiguity_for_same_folder_win_is_not_
        // ambiguous` covers the reporting side of the folder-first rule; this
        // pins the winner itself disagreeing with the smallest URL.
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/a/sam/", "Sam", "sam", false),
            note("/z/sam/", "Sam", "sam", false),
            note("/z/other/", "Other", "other", false),
        ]);

        // From /z/other/, the sibling in /z/ wins — not /a/sam/, the smallest.
        // Spelled `SAM` so the answer is the URL itself rather than the "default
        // relative transform already reaches it" `None`.
        assert_eq!(
            idx.resolve_wikilink("SAM", "/z/other/", false),
            Some("/z/sam/".to_string())
        );
        assert_eq!(idx.ambiguity_for("SAM", "/z/other/", false), None);
    }

    #[test]
    fn rebuild_clears_stale_ambiguity() {
        let idx = WikilinkIndex::new();
        idx.rebuild(&[
            note("/a/sam/", "Sam", "sam", false),
            note("/z/sam/", "Sam", "sam", false),
        ]);
        assert!(idx.ambiguity_for("Sam", "/x/", false).is_some());
        // The duplicate is renamed; the finding must go away.
        idx.rebuild(&[
            note("/a/sam/", "Sam", "sam", false),
            note("/z/sam/", "Samuel", "samuel", false),
        ]);
        assert_eq!(idx.ambiguity_for("Sam", "/x/", false), None);
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
