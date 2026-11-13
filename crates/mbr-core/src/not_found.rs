//! Where did a missing page go? The Rust port of
//! `components/src/not-found/match.ts`, the matcher behind the 404 page and the
//! page-errors panel's "Did you mean" links, for the link checker
//! (`mbr --check-links-only`).
//!
//! **Same semantics as the TypeScript, by test.** Both read
//! `tests/fixtures/not-found-match-cases.json` (this module's tests and
//! `components/src/not-found/match-cases.test.ts`), so a change to either side
//! that the other does not make fails. Add a case there when the rules change.
//!
//! The rules (see [`NotFoundIndex::find_matches`]): a target name is the last
//! URL segment, a markdown extension dropped, an index note's folder name
//! instead; tier 1 is the same normalized name (case, spaces, `-` and `_`
//! alike) by a note's file stem or URL name, or a non-markdown file's exact
//! name; tier 2 is a name extended or shortened at a word start, both sides at
//! least [`MIN_PARTIAL_LENGTH`] long, notes only; ranked by tier, then shared
//! leading folders, then URL; never the path itself.
//!
//! Unlike the TypeScript, which scans site.json per call, the index is built
//! once ([`NotFoundIndex::new`]) and each lookup touches only candidates: a
//! hash lookup for tier 1, a binary search over word-start suffixes and a
//! prefix walk of the target for tier 2. A repository of tens of thousands of
//! notes with thousands of broken links stays linear in the links.
//!
//! Every path is a **decoded** canonical site path (`/docs/My Note/`), the form
//! `url_path` uses.

use regex::Regex;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

/// At most this many matches on the 404 page.
pub const MAX_MATCHES: usize = 6;
/// At most this many "Did you mean" suggestions per broken link.
pub const MAX_DID_YOU_MEAN: usize = 3;
/// Shortest normalized name a partial (extended / shortened name) match may
/// involve, on either side, in UTF-16 code units (a JS string length): below
/// it, `faq` would match every `faq-…` note.
pub const MIN_PARTIAL_LENGTH: usize = 4;

/// Markdown extensions always stripped from a target name; the index adds
/// every extension its notes' `raw_path`s use.
const COMMON_MARKDOWN_EXTENSIONS: [&str; 6] = ["md", "markdown", "mdown", "mkd", "mkdn", "mdwn"];

/// One note, as the matcher reads a site.json `markdown_files` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteNote {
    /// Canonical, decoded site path (`/docs/guide/`).
    pub url_path: String,
    /// Repository-relative source path with `/` separators (`docs/guide.md`).
    pub raw_path: Option<String>,
    /// The frontmatter `title`, when it is a string.
    pub title: Option<String>,
}

/// Whether a suggestion is a note (a page) or another file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    Note,
    File,
}

/// One page or file offered as where the missing one went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestedPage {
    /// Canonical, decoded site path (`/archive/notes/`, `/files/report.pdf`).
    pub url: String,
    /// Frontmatter title, else the name in the URL.
    pub title: String,
    pub kind: SuggestionKind,
    /// Index into [`NotFoundIndex::notes`] or [`NotFoundIndex::files`],
    /// by `kind`.
    pub index: usize,
}

/// The fallback when nothing matches: the missing page's folder still exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderFallback {
    /// The folder's canonical site path (`/docs/`).
    pub url: String,
    /// The folder's index note title, else its name.
    pub title: String,
    /// Notes directly in the folder.
    pub notes: usize,
}

/// Lower-cased, with runs of whitespace, `-` and `_` read as one separator:
/// `My Note`, `my-note` and `my_note` are the same name.
pub fn normalize_name(name: &str) -> String {
    static SEPARATORS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[\s_-]+").expect("static separator regex is valid"));
    SEPARATORS
        .replace_all(&name.to_lowercase(), " ")
        .trim()
        .to_string()
}

/// A JavaScript string's `length`.
fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// JavaScript's `<` on strings: UTF-16 code unit order.
fn js_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn segments_of(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

/// Canonical form for comparing two site paths: a leading and trailing `/`.
fn page_path(path: &str) -> String {
    let segs = segments_of(path);
    if segs.is_empty() {
        "/".to_string()
    } else {
        format!("/{}/", segs.join("/"))
    }
}

static EXTENSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.([A-Za-z0-9]{1,10})$").expect("static extension regex"));

fn extension_of(name: &str) -> Option<String> {
    EXTENSION
        .captures(name)
        .and_then(|caps| caps.get(1))
        .map(|ext| ext.as_str().to_ascii_lowercase())
}

fn strip_extension(name: &str) -> &str {
    EXTENSION.find(name).map_or(name, |m| &name[..m.start()])
}

fn file_name_of(path: &str) -> &str {
    segments_of(path).last().copied().unwrap_or("")
}

/// `needle` starts a word of `haystack` (`meeting` in `weekly meetings`).
fn starts_a_word(haystack: &str, needle: &str) -> bool {
    haystack.starts_with(needle) || haystack.contains(&format!(" {needle}"))
}

/// 1 = same name, 2 = extended or shortened name, 0 = no match.
fn tier_for(target: &str, names: &[String]) -> u8 {
    if names.iter().any(|name| name == target) {
        return 1;
    }
    if js_len(target) < MIN_PARTIAL_LENGTH {
        return 0;
    }
    let partial = names.iter().any(|name| {
        js_len(name) >= MIN_PARTIAL_LENGTH
            && (starts_a_word(name, target) || starts_a_word(target, name))
    });
    if partial { 2 } else { 0 }
}

/// Leading folder segments `url` shares with `folder`.
fn shared_folders(url: &str, folder: &[String]) -> usize {
    segments_of(url)
        .iter()
        .zip(folder)
        .take_while(|(seg, wanted)| **seg == wanted.as_str())
        .count()
}

/// Byte offsets where a word of a normalized name starts: 0 and after each
/// space.
fn word_starts(name: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(
        name.char_indices()
            .filter(|(_, c)| *c == ' ')
            .map(|(at, _)| at + 1)
            .filter(move |at| *at < name.len()),
    )
}

/// What a missing URL names.
struct Target {
    /// Normalized name to look for.
    name: String,
    /// The folders the URL put it in.
    folder: Vec<String>,
    /// The literal file name when the URL names a non-markdown file.
    file_name: Option<String>,
}

struct NoteEntry {
    note: SiteNote,
    page_path: String,
    title: String,
    names: Vec<String>,
}

struct FileEntry {
    url: String,
    name: String,
}

#[derive(Default)]
struct FolderStats {
    /// Title of the first note served at the folder's own URL.
    index_title: Option<String>,
    /// Distinct note URLs directly in the folder.
    direct: HashSet<String>,
}

/// The matcher's view of a site, built once.
pub struct NotFoundIndex {
    notes: Vec<NoteEntry>,
    files: Vec<FileEntry>,
    index_stem: String,
    markdown_extensions: HashSet<String>,
    /// Normalized note name → notes answering to it, in note order.
    by_name: HashMap<String, Vec<usize>>,
    /// `(word-start suffix of a name, note)`, sorted, for names at least
    /// [`MIN_PARTIAL_LENGTH`] long: "the target starts a word of the name"
    /// is a prefix range of this list.
    word_suffixes: Vec<(String, usize)>,
    /// Normalized file name → files, in file order.
    files_by_name: HashMap<String, Vec<usize>>,
    /// Page path → the first note served there.
    page_paths: HashMap<String, usize>,
    /// Every folder that holds a note, at any depth (`/docs/`).
    folders: HashMap<String, FolderStats>,
}

/// One candidate in a ranking.
struct Ranked {
    page: SuggestedPage,
    tier: u8,
    shared: usize,
}

impl NotFoundIndex {
    /// Builds the index. `notes` and `other_files` (site URLs of non-markdown
    /// files) are taken in the order given; where two notes share a URL the
    /// first wins, as in the TypeScript. `index_file` is the configured index
    /// file name (`index.md`).
    pub fn new(
        notes: impl IntoIterator<Item = SiteNote>,
        other_files: impl IntoIterator<Item = String>,
        index_file: &str,
    ) -> Self {
        let index_stem = strip_extension(if index_file.is_empty() {
            "index.md"
        } else {
            index_file
        })
        .to_lowercase();

        let notes: Vec<NoteEntry> = notes
            .into_iter()
            .map(|note| {
                let names = names_of(&note, &index_stem);
                NoteEntry {
                    page_path: page_path(&note.url_path),
                    title: title_of(&note),
                    names,
                    note,
                }
            })
            .collect();

        let markdown_extensions = COMMON_MARKDOWN_EXTENSIONS
            .iter()
            .map(|ext| (*ext).to_string())
            .chain(notes.iter().filter_map(|entry| {
                entry
                    .note
                    .raw_path
                    .as_deref()
                    .and_then(|raw| extension_of(file_name_of(raw)))
            }))
            .collect();

        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        let mut word_suffixes = Vec::new();
        for (idx, entry) in notes.iter().enumerate() {
            for name in &entry.names {
                by_name.entry(name.clone()).or_default().push(idx);
                if js_len(name) >= MIN_PARTIAL_LENGTH {
                    word_suffixes.extend(word_starts(name).map(|at| (name[at..].to_string(), idx)));
                }
            }
        }
        word_suffixes.sort();
        word_suffixes.dedup();

        let files: Vec<FileEntry> = other_files
            .into_iter()
            .map(|url| FileEntry {
                name: file_name_of(&url).to_string(),
                url,
            })
            .collect();
        let mut files_by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, file) in files.iter().enumerate() {
            files_by_name
                .entry(normalize_name(&file.name))
                .or_default()
                .push(idx);
        }

        let mut page_paths = HashMap::new();
        for (idx, entry) in notes.iter().enumerate() {
            page_paths.entry(entry.page_path.clone()).or_insert(idx);
        }

        let mut folders: HashMap<String, FolderStats> = HashMap::new();
        for entry in &notes {
            let segs = segments_of(&entry.page_path);
            // The note's own URL is a folder too (`/docs/` for `docs/index.md`,
            // and for `docs.md`): its first note there names it.
            for depth in 1..=segs.len() {
                let folder = format!("/{}/", segs[..depth].join("/"));
                let stats = folders.entry(folder).or_default();
                if depth == segs.len() {
                    stats.index_title.get_or_insert_with(|| entry.title.clone());
                } else if depth + 1 == segs.len() {
                    stats.direct.insert(entry.note.url_path.clone());
                }
            }
        }

        Self {
            notes,
            files,
            index_stem,
            markdown_extensions,
            by_name,
            word_suffixes,
            files_by_name,
            page_paths,
            folders,
        }
    }

    /// The note behind a [`SuggestedPage`] of kind [`SuggestionKind::Note`].
    pub fn note(&self, index: usize) -> Option<&SiteNote> {
        self.notes.get(index).map(|entry| &entry.note)
    }

    /// How many notes answer to `name` (normalized) by file stem or URL name.
    pub fn notes_named(&self, name: &str) -> usize {
        self.by_name
            .get(&normalize_name(name))
            .map_or(0, |notes| notes.len())
    }

    /// Reads the name out of a missing path: the last segment, a markdown
    /// extension dropped, and for an index note (`/a/b/index.md`) the folder's
    /// name instead. `None` for the site root.
    fn target_of(&self, path: &str) -> Option<Target> {
        let segs = segments_of(path);
        let last = *segs.last()?;
        let ext = extension_of(last);
        let is_markdown = ext
            .as_ref()
            .is_some_and(|ext| self.markdown_extensions.contains(ext));
        let stem = if is_markdown {
            strip_extension(last)
        } else {
            last
        };
        if stem.to_lowercase() == self.index_stem {
            if segs.len() < 2 {
                return None;
            }
            return Some(Target {
                name: normalize_name(segs[segs.len() - 2]),
                folder: segs[..segs.len() - 2]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                file_name: None,
            });
        }
        let name = normalize_name(stem);
        if name.is_empty() {
            return None;
        }
        Some(Target {
            name,
            folder: segs[..segs.len() - 1]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            file_name: (!is_markdown && ext.is_some()).then(|| last.to_string()),
        })
    }

    /// Notes that may match `target` at tier 1 or 2: a superset, judged
    /// exactly by [`tier_for`] afterwards.
    fn note_candidates(&self, target: &str) -> BTreeSet<usize> {
        let mut found: BTreeSet<usize> = self
            .by_name
            .get(target)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        if js_len(target) < MIN_PARTIAL_LENGTH {
            return found;
        }
        // The target starts a word of a name.
        let start = self
            .word_suffixes
            .partition_point(|(suffix, _)| suffix.as_str() < target);
        found.extend(
            self.word_suffixes[start..]
                .iter()
                .take_while(|(suffix, _)| suffix.starts_with(target))
                .map(|(_, idx)| *idx),
        );
        // A name starts a word of the target: every prefix of every word-start
        // suffix of the target is looked up.
        for at in word_starts(target) {
            let rest = &target[at..];
            for (end, c) in rest.char_indices() {
                let prefix = &rest[..end + c.len_utf8()];
                if let Some(notes) = self.by_name.get(prefix)
                    && js_len(prefix) >= MIN_PARTIAL_LENGTH
                {
                    found.extend(notes.iter().copied());
                }
            }
        }
        found
    }

    /// Notes (and, with `include_files`, other files) that are probably what
    /// `path` used to name, best first, at most `limit`:
    ///
    /// 1. the same name elsewhere (a note's file stem or URL name; a file's
    ///    exact file name), compared case-insensitively with spaces, `-` and
    ///    `_` alike;
    /// 2. a name that was extended or shortened (`meeting` ↔ `meeting-notes`),
    ///    at a word start, both sides at least [`MIN_PARTIAL_LENGTH`] long —
    ///    for notes, and not when the URL names a non-markdown file.
    ///
    /// Within a tier, the more leading folders a candidate shares with `path`
    /// the better, then by URL. `path` itself is never suggested.
    pub fn find_matches(
        &self,
        path: &str,
        include_files: bool,
        limit: usize,
    ) -> Vec<SuggestedPage> {
        let Some(target) = self.target_of(path) else {
            return Vec::new();
        };
        let self_path = page_path(path);
        let mut ranked: Vec<Ranked> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();

        for idx in self.note_candidates(&target.name) {
            let entry = &self.notes[idx];
            let url = entry.note.url_path.as_str();
            if entry.page_path == self_path || seen.contains(url) {
                continue;
            }
            let tier = tier_for(&target.name, &entry.names);
            // A file's name with its extension (`report.pdf`) is not a renamed
            // note's: only an exact match counts there.
            if tier == 0 || (target.file_name.is_some() && tier != 1) {
                continue;
            }
            seen.insert(url);
            ranked.push(Ranked {
                page: SuggestedPage {
                    url: url.to_string(),
                    title: entry.title.clone(),
                    kind: SuggestionKind::Note,
                    index: idx,
                },
                tier,
                shared: shared_folders(url, &target.folder),
            });
        }

        if include_files && let Some(file_name) = &target.file_name {
            let wanted = normalize_name(file_name);
            for &idx in self.files_by_name.get(&wanted).into_iter().flatten() {
                let file = &self.files[idx];
                if page_path(&file.url) == self_path || seen.contains(file.url.as_str()) {
                    continue;
                }
                seen.insert(file.url.as_str());
                ranked.push(Ranked {
                    page: SuggestedPage {
                        url: file.url.clone(),
                        title: file.name.clone(),
                        kind: SuggestionKind::File,
                        index: idx,
                    },
                    tier: 1,
                    shared: shared_folders(&file.url, &target.folder),
                });
            }
        }

        ranked.sort_by(|a, b| {
            a.tier
                .cmp(&b.tier)
                .then(b.shared.cmp(&a.shared))
                .then_with(|| js_cmp(&a.page.url, &b.page.url))
        });
        ranked
            .into_iter()
            .take(limit)
            .map(|ranked| ranked.page)
            .collect()
    }

    /// Whether a page or folder index is published at `path`.
    pub fn page_exists(&self, path: &str) -> bool {
        self.page_paths.contains_key(&page_path(path))
    }

    /// The note served at `path` (the first, when several are), as a
    /// suggestion.
    pub fn note_at(&self, path: &str) -> Option<SuggestedPage> {
        let idx = *self.page_paths.get(&page_path(path))?;
        let entry = &self.notes[idx];
        Some(SuggestedPage {
            url: entry.note.url_path.clone(),
            title: entry.title.clone(),
            kind: SuggestionKind::Note,
            index: idx,
        })
    }

    /// "Did you mean" for a broken link: matches only, at most `limit`, and
    /// nothing when the target exists after all. `include_files` also offers
    /// non-markdown files when the link names one (the 404 page's rule; the
    /// browser's page-errors panel passes no files).
    pub fn did_you_mean(
        &self,
        path: &str,
        include_files: bool,
        limit: usize,
    ) -> Vec<SuggestedPage> {
        if self.page_exists(path) {
            return Vec::new();
        }
        self.find_matches(path, include_files, limit)
    }

    /// The missing page's own folder, when it still exists (some note lives
    /// in or under it). `None` for a page at the root, or when the folder is
    /// gone too; this never walks further up.
    pub fn folder_fallback(&self, path: &str) -> Option<FolderFallback> {
        let target = self.target_of(path)?;
        if target.folder.is_empty() {
            return None;
        }
        let url = format!("/{}/", target.folder.join("/"));
        let stats = self.folders.get(&url)?;
        Some(FolderFallback {
            title: stats
                .index_title
                .clone()
                .unwrap_or_else(|| target.folder.last().cloned().unwrap_or_default()),
            notes: stats.direct.len(),
            url,
        })
    }
}

fn title_of(note: &SiteNote) -> String {
    note.title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            segments_of(&note.url_path)
                .last()
                .map_or_else(|| "/".to_string(), |s| s.to_string())
        })
}

/// The names a note can be linked by: its URL's last segment and its file stem.
fn names_of(note: &SiteNote, index_stem: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut push = |name: String| {
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    };
    let url_name = file_name_of(&note.url_path);
    if !url_name.is_empty() {
        push(normalize_name(url_name));
    }
    if let Some(raw) = &note.raw_path {
        let stem = strip_extension(file_name_of(raw));
        if !stem.is_empty() && stem.to_lowercase() != index_stem {
            push(normalize_name(stem));
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    fn note(raw: &str) -> SiteNote {
        let stem = raw.trim_end_matches(".md");
        let url = if stem == "index" {
            "/".to_string()
        } else {
            format!("/{}/", stem.trim_end_matches("/index"))
        };
        SiteNote {
            url_path: url,
            raw_path: Some(raw.to_string()),
            title: None,
        }
    }

    fn index(raws: &[&str], files: &[&str]) -> NotFoundIndex {
        NotFoundIndex::new(
            raws.iter().map(|raw| note(raw)),
            files.iter().map(|f| f.to_string()),
            "index.md",
        )
    }

    fn urls(pages: &[SuggestedPage]) -> Vec<&str> {
        pages.iter().map(|p| p.url.as_str()).collect()
    }

    #[test]
    fn normalize_name_treats_case_spaces_dashes_and_underscores_alike() {
        assert_eq!(normalize_name("My Note"), "my note");
        assert_eq!(normalize_name("my-note"), "my note");
        assert_eq!(normalize_name("MY__note"), "my note");
        assert_eq!(normalize_name(" a - b "), "a b");
    }

    #[test]
    fn finds_a_note_moved_to_another_folder() {
        let site = index(
            &["index.md", "archive/2024/project-plan.md", "work/other.md"],
            &[],
        );
        assert_eq!(
            urls(&site.find_matches("/work/project-plan/", false, MAX_MATCHES)),
            ["/archive/2024/project-plan/"]
        );
    }

    #[test]
    fn same_name_ranks_before_partial_and_shared_folders_break_ties() {
        let site = index(
            &[
                "a/meeting-notes.md",
                "b/meeting.md",
                "work/x/meeting.md",
                "work/meeting-agenda.md",
            ],
            &[],
        );
        assert_eq!(
            urls(&site.find_matches("/work/x/old/meeting/", false, MAX_MATCHES)),
            [
                "/work/x/meeting/",
                "/b/meeting/",
                "/work/meeting-agenda/",
                "/a/meeting-notes/"
            ]
        );
    }

    #[test]
    fn partial_match_works_in_both_directions() {
        let site = index(&["a/weekly-meetings.md", "b/plan.md", "c/roadmap.md"], &[]);
        // Target starts a word of the name.
        assert_eq!(
            urls(&site.find_matches("/x/meetings/", false, MAX_MATCHES)),
            ["/a/weekly-meetings/"]
        );
        // The name starts a word of the target.
        assert_eq!(
            urls(&site.find_matches("/x/roadmap-2025/", false, MAX_MATCHES)),
            ["/c/roadmap/"]
        );
        // Too short for a partial match.
        assert!(site.find_matches("/x/pla/", false, MAX_MATCHES).is_empty());
    }

    #[test]
    fn files_match_only_by_exact_name_and_only_when_asked() {
        let site = index(&["docs/report.md"], &["/assets/report.pdf"]);
        assert_eq!(
            urls(&site.find_matches("/docs/report.pdf", true, MAX_MATCHES)),
            ["/assets/report.pdf"]
        );
        assert!(
            site.find_matches("/docs/report.pdf", false, MAX_MATCHES)
                .is_empty()
        );
    }

    #[test]
    fn did_you_mean_is_silent_when_the_page_exists() {
        let site = index(&["docs/guide.md", "old/guide.md"], &[]);
        assert!(site.did_you_mean("/docs/guide", false, 3).is_empty());
        assert_eq!(
            urls(&site.did_you_mean("/notes/guide/", false, 3)),
            ["/docs/guide/", "/old/guide/"]
        );
    }

    #[test]
    fn folder_fallback_names_a_surviving_folder() {
        let site = index(&["docs/index.md", "docs/a.md", "docs/b.md"], &[]);
        let fallback = site.folder_fallback("/docs/gone/").expect("folder exists");
        assert_eq!(fallback.url, "/docs/");
        assert_eq!(fallback.title, "docs");
        assert_eq!(fallback.notes, 2);
        assert!(site.folder_fallback("/gone/").is_none());
        assert!(site.folder_fallback("/nope/gone/").is_none());
    }

    // ---- The shared fixture: the TypeScript matcher reads the same table. ----

    #[derive(Deserialize)]
    struct Fixture {
        sites: HashMap<String, FixtureSite>,
        cases: Vec<FixtureCase>,
    }

    #[derive(Deserialize)]
    struct FixtureSite {
        #[serde(default)]
        index_file: Option<String>,
        markdown_files: Vec<FixtureNote>,
        #[serde(default)]
        other_files: Vec<FixtureFile>,
    }

    #[derive(Deserialize)]
    struct FixtureNote {
        url_path: String,
        #[serde(default)]
        raw_path: Option<String>,
        #[serde(default)]
        frontmatter: Option<serde_json::Value>,
    }

    #[derive(Deserialize)]
    struct FixtureFile {
        url_path: String,
    }

    #[derive(Deserialize)]
    struct FixtureCase {
        name: String,
        site: String,
        path: String,
        /// `find_matches(path, with other files, MAX_MATCHES)`.
        matches: Vec<String>,
        /// `did_you_mean(path, no files, MAX_DID_YOU_MEAN)`.
        did_you_mean: Vec<String>,
        /// `folder_fallback(path)`: `[url, title]` or null.
        folder: Option<(String, String)>,
    }

    fn fixture() -> Fixture {
        let text = include_str!("../../../tests/fixtures/not-found-match-cases.json");
        serde_json::from_str(text).expect("fixture parses")
    }

    fn fixture_index(site: &FixtureSite) -> NotFoundIndex {
        NotFoundIndex::new(
            site.markdown_files.iter().map(|f| SiteNote {
                url_path: f.url_path.clone(),
                raw_path: f.raw_path.clone(),
                title: f
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| fm.get("title"))
                    .and_then(|t| t.as_str())
                    .map(str::to_string),
            }),
            site.other_files.iter().map(|f| f.url_path.clone()),
            site.index_file.as_deref().unwrap_or("index.md"),
        )
    }

    #[test]
    fn shared_fixture_cases_match_the_typescript_matcher() {
        let fixture = fixture();
        let indexes: HashMap<&str, NotFoundIndex> = fixture
            .sites
            .iter()
            .map(|(name, site)| (name.as_str(), fixture_index(site)))
            .collect();
        assert!(!fixture.cases.is_empty());
        for case in &fixture.cases {
            let site = &indexes[case.site.as_str()];
            assert_eq!(
                urls(&site.find_matches(&case.path, true, MAX_MATCHES)),
                case.matches,
                "matches: {}",
                case.name
            );
            assert_eq!(
                urls(&site.did_you_mean(&case.path, false, MAX_DID_YOU_MEAN)),
                case.did_you_mean,
                "did_you_mean: {}",
                case.name
            );
            assert_eq!(
                site.folder_fallback(&case.path).map(|f| (f.url, f.title)),
                case.folder,
                "folder: {}",
                case.name
            );
        }
    }
}
