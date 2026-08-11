//! Lazy, in-memory index of the tasks contained in a repository's markdown.
//!
//! The index is **built on first use, never on startup**, and there is no
//! on-disk cache: mbr is pointed at live directories that change underneath it,
//! so the only trustworthy cache is one that a process restart discards.
//!
//! ```text
//! first POST /.mbr/tasks ──► ensure_built() ──► one sequential read pass
//!                                 │                over repo.markdown_files
//!                                 ▼
//!                        papaya map, keyed by absolute path,
//!                        holding only files that contain tasks
//!                                 ▲
//! watcher reconciliation ─────────┘  invalidate_file() re-scans one file
//! ```
//!
//! # Why sequential, not rayon
//!
//! [`TaskIndex::ensure_built`] reads every markdown file one after another on a
//! single `spawn_blocking` thread. That is deliberate and follows the precedent
//! set in `src/search.rs` (see the comments at `search.rs:362` and
//! `search.rs:658`): `par_iter` over file reads caused 30-second stalls from
//! rayon thread-pool contention when several requests ran at once. The repo
//! scanner owns the rayon pool; request-path work stays off it.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use papaya::HashMap;
use serde::Serialize;

use crate::errors::TaskIndexError;
use crate::repo::{MarkdownInfo, Repo};
use crate::tasks::{Task, TaskStatus, scan_source_tasks};
use crate::watcher::ChangeEventType;

/// Largest markdown file the task scanner will read, in bytes.
///
/// Task scanning needs the *whole* file (unlike frontmatter extraction, which
/// reads the first 8 KB), and the index build touches every markdown file in
/// the repository. A pathological multi-hundred-megabyte "markdown" file —
/// a generated log, a dumped dataset — would otherwise stall the build and
/// balloon memory for a file no human is tracking tasks in. Four megabytes is
/// roughly a million lines of prose; nothing hand-written comes close.
const MAX_SCAN_BYTES: u64 = 4 * 1024 * 1024;

/// The tasks found in one markdown file, plus the metadata a task list needs to
/// display and link to them.
///
/// Constructed only through [`FileTasks::new`] so that `open`/`done` can never
/// drift from `tasks`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileTasks {
    /// Canonical site URL of the page, e.g. `/docs/notes/weekly/`.
    pub url_path: String,
    /// Source path **relative to the repository root**, mirroring
    /// [`MarkdownInfo::raw_path`]. Join it with the root before any file I/O.
    pub raw_path: PathBuf,
    /// Frontmatter `title`, if the file declares one.
    ///
    /// `None` is not a display fallback — it means "this file has no declared
    /// title", and callers fall back to the file stem exactly as
    /// `markdown_file_to_json` does.
    pub title: Option<String>,
    /// Every task in the file, in source order.
    pub tasks: Vec<Task>,
    /// Count of [`TaskStatus::Open`] tasks.
    pub open: u32,
    /// Count of [`TaskStatus::Done`] tasks.
    ///
    /// Canceled tasks are excluded from **both** counts, so `open + done` is
    /// the denominator of the "3/7 done" progress indicator.
    pub done: u32,
    /// URL-shaped folder of the file, with leading and trailing slashes
    /// (`/docs/notes/`, or `/` at the repository root).
    ///
    /// Derived from `raw_path`, not from `url_path`: an index file's `url_path`
    /// *is* its folder (`docs/index.md` → `/docs/`), so trimming the last URL
    /// segment would file its tasks one level too high.
    #[serde(skip)]
    folder: String,
}

impl FileTasks {
    /// Builds a `FileTasks`, deriving the counts and the folder from the inputs.
    pub fn new(
        url_path: impl Into<String>,
        raw_path: impl Into<PathBuf>,
        title: Option<String>,
        tasks: Vec<Task>,
    ) -> Self {
        let raw_path = raw_path.into();
        let folder = folder_of(&raw_path);
        let open = count_with(&tasks, TaskStatus::Open);
        let done = count_with(&tasks, TaskStatus::Done);
        Self {
            url_path: url_path.into(),
            raw_path,
            title,
            tasks,
            open,
            done,
            folder,
        }
    }

    /// The file's folder as a URL path with leading and trailing slashes.
    pub fn folder(&self) -> &str {
        &self.folder
    }

    /// Display title: the frontmatter title, else the file stem.
    pub fn display_title(&self) -> &str {
        self.title.as_deref().unwrap_or_else(|| {
            self.raw_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
        })
    }

    /// Tasks that count toward the progress indicator (`open + done`).
    pub fn tracked(&self) -> u32 {
        self.open + self.done
    }
}

/// Counts tasks with the given status, saturating rather than wrapping.
fn count_with(tasks: &[Task], status: TaskStatus) -> u32 {
    u32::try_from(tasks.iter().filter(|t| t.status == status).count()).unwrap_or(u32::MAX)
}

/// URL-shaped folder for a repo-relative source path.
///
/// `docs/notes/weekly.md` → `/docs/notes/`, `readme.md` → `/`.
fn folder_of(raw_path: &Path) -> String {
    match raw_path.parent() {
        Some(parent) if parent.as_os_str().is_empty() => "/".to_string(),
        // `path_to_url` keeps the value `/`-separated on Windows, where
        // `raw_path` uses backslashes.
        Some(parent) => format!("/{}/", crate::url_path::path_to_url(parent)),
        None => "/".to_string(),
    }
}

/// Lazy, thread-safe index of every markdown file that contains tasks.
///
/// Files without tasks are deliberately **absent** rather than stored empty:
/// on a repository of tens of thousands of notes, only a fraction carry tasks,
/// and skipping the rest keeps both memory and every query's iteration small.
///
/// Files matching `tasks_ignore_globs` are absent too. Excluding them *here*,
/// at the one place the index is filled, is what makes the exclusion complete:
/// the panel, the folder facets, `total_matches` and every count all derive
/// from [`TaskIndex::snapshot`].
pub struct TaskIndex {
    /// Keyed by absolute path, matching [`Repo::markdown_files`] so the
    /// watcher's canonical paths address the same entries.
    files: HashMap<PathBuf, Arc<FileTasks>>,
    /// Single-flight guard for the one-time build.
    ///
    /// `tokio::sync::OnceCell::get_or_try_init` leaves the cell *uninitialized*
    /// when the initializer returns an error, so a failed build is retried by
    /// the next caller instead of poisoning the index for the life of the
    /// process.
    built: tokio::sync::OnceCell<()>,
    /// `tasks_ignore_globs`, compiled once at construction. See
    /// [`TaskIndex::is_ignored`].
    ignore_globs: Vec<glob::Pattern>,
}

impl Default for TaskIndex {
    fn default() -> Self {
        Self::new(&[])
    }
}

impl TaskIndex {
    /// Creates an empty, unbuilt index that excludes files matching
    /// `ignore_globs` (the `tasks_ignore_globs` config option).
    ///
    /// The patterns are compiled **here, once**, mirroring `Repo::init`'s
    /// `compiled_ignore_globs`: the build walks every markdown file in the
    /// repository, so re-parsing the patterns per file would cost far more than
    /// the matching itself.
    ///
    /// A pattern that does not compile is dropped with a warning rather than
    /// refused. [`Config::validate`](crate::config::Config::validate) already
    /// rejects one at startup, so the only way to reach here with a bad pattern
    /// is to bypass config loading entirely.
    pub fn new(ignore_globs: &[String]) -> Self {
        Self {
            files: HashMap::new(),
            built: tokio::sync::OnceCell::new(),
            ignore_globs: compile_ignore_globs(ignore_globs),
        }
    }

    /// Whether `tasks_ignore_globs` excludes this repository-relative path.
    ///
    /// Matching is against the `/`-separated relative path
    /// (`docs/templates/onboarding.md`), not the absolute one: a pattern must
    /// mean the same thing wherever the repository happens to sit on disk, and
    /// on every platform — [`MarkdownInfo::raw_path`] uses `\` on Windows, which
    /// `glob::Pattern` would never match against a `/`-shaped pattern.
    ///
    /// Returns before allocating when nothing is configured, which is the
    /// default and therefore the hot path.
    fn is_ignored(&self, raw_path: &Path) -> bool {
        if self.ignore_globs.is_empty() {
            return false;
        }
        let relative = crate::url_path::path_to_url(raw_path);
        self.ignore_globs
            .iter()
            .any(|pattern| pattern.matches(&relative))
    }

    /// Whether the one-time build has completed.
    pub fn is_built(&self) -> bool {
        self.built.initialized()
    }

    /// Number of indexed files (only files that contain tasks).
    pub fn len(&self) -> usize {
        self.files.pin().len()
    }

    /// Whether the index currently holds no files.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The tasks recorded for one absolute path, if any.
    pub fn get(&self, abs_path: &Path) -> Option<Arc<FileTasks>> {
        self.files.pin().get(abs_path).cloned()
    }

    /// A cheap snapshot of every indexed file, for querying.
    ///
    /// Clones `Arc`s, not data. Taking a snapshot rather than querying through
    /// a pin guard keeps the guard off the (potentially slow) grouping pass and
    /// gives the query a stable view even if the watcher fires mid-query.
    pub fn snapshot(&self) -> Vec<Arc<FileTasks>> {
        self.files.pin().values().cloned().collect()
    }

    /// Builds the index if it has not been built yet.
    ///
    /// Idempotent and single-flight: concurrent callers await one build. The
    /// work runs on `spawn_blocking` because it is a sequential pass of
    /// synchronous file reads.
    ///
    /// Individual unreadable files (permissions, invalid UTF-8, oversized) are
    /// skipped with a debug log; only a panic in the build task is an error.
    pub async fn ensure_built(
        self: &Arc<Self>,
        repo: &Arc<Repo>,
        root_dir: &Path,
    ) -> Result<(), TaskIndexError> {
        let index = Arc::clone(self);
        let repo = Arc::clone(repo);
        let root_dir = root_dir.to_path_buf();

        self.built
            .get_or_try_init(|| async move {
                tokio::task::spawn_blocking(move || index.build_blocking(&repo, &root_dir))
                    .await
                    .map_err(|e| TaskIndexError::BuildFailed {
                        reason: e.to_string(),
                    })
            })
            .await
            .map(|_| ())
    }

    /// Re-scans (Created/Modified) or drops (Deleted) one file.
    ///
    /// **No-op when the index has never been built.** A change arriving before
    /// anybody asked for tasks must not trigger the very build the laziness
    /// exists to avoid.
    ///
    /// NOTE (known window): a file changed *while* the initial build is running
    /// is skipped here, because the build has not published itself yet, and the
    /// build may already have read that file's previous contents. The entry
    /// then stays stale until the file changes again or the process restarts.
    /// Closing it would mean either serializing the watcher against the build
    /// (blocking edits behind a full-repo read pass) or queueing deferred
    /// invalidations forever for the common case where tasks are never opened.
    /// Neither is worth it for a window that is bounded by one build.
    ///
    /// Call this *after* [`Repo::invalidate_file`] for the same event: the
    /// url/title of a created or modified file are read back out of the repo.
    pub fn invalidate_file(
        &self,
        abs_path: &Path,
        event: &ChangeEventType,
        repo: &Repo,
        root_dir: &Path,
    ) {
        if !self.is_built() {
            return;
        }

        match event {
            ChangeEventType::Deleted => {
                self.files.pin().remove(abs_path);
            }
            ChangeEventType::Created | ChangeEventType::Modified => {
                // Absent from the repo means "not a markdown file the repo
                // tracks" — the watcher forwards create/delete events for
                // assets too — or a file deleted between the two calls.
                let Some(target) = repo
                    .markdown_files
                    .pin()
                    .get(abs_path)
                    .map(|info| ScanTarget::from_info(abs_path, info))
                else {
                    self.files.pin().remove(abs_path);
                    return;
                };

                // The same guard `scan_all` applies, repeated because this is a
                // separate path into the map: without it, editing a file in an
                // ignored folder would quietly index the very file the build
                // skipped.
                if self.is_ignored(&target.raw_path) {
                    self.files.pin().remove(abs_path);
                    return;
                }

                let mut buffer = String::new();
                match target.scan(root_dir, &mut buffer) {
                    // A file that lost its last task must leave the index, or
                    // it lingers as an empty group forever.
                    Some(file_tasks) => {
                        self.files
                            .pin()
                            .insert(abs_path.to_path_buf(), Arc::new(file_tasks));
                    }
                    None => {
                        self.files.pin().remove(abs_path);
                    }
                }
            }
        }
    }

    /// Re-scans the whole repository, but only if the index was already built.
    ///
    /// The watcher calls this on its full-rescan path (a change batch too large
    /// to patch file by file), where per-file invalidation would cost more than
    /// one read pass. Like [`Self::invalidate_file`] it does nothing when the
    /// index has never been built, so an unused task index stays free.
    ///
    /// Entries are replaced and stale ones pruned in a single pass at the end,
    /// so a concurrent query never observes a half-empty index — only the old
    /// contents or the new ones.
    pub fn rebuild_if_built(&self, repo: &Repo, root_dir: &Path) {
        if !self.is_built() {
            return;
        }

        let fresh = self.scan_all(repo, root_dir);
        let files = self.files.pin();
        // Set membership, not a nested scan: a linear search per key would be
        // quadratic in the number of task-bearing files.
        let stale: Vec<PathBuf> = {
            let fresh_paths: std::collections::HashSet<&PathBuf> =
                fresh.iter().map(|(path, _)| path).collect();
            files
                .keys()
                .filter(|key| !fresh_paths.contains(*key))
                .cloned()
                .collect()
        };

        for (path, file_tasks) in fresh {
            files.insert(path, file_tasks);
        }
        for path in stale {
            files.remove(&path);
        }
    }

    /// Fills an empty index. See the module docs for why this is not parallel.
    fn build_blocking(&self, repo: &Repo, root_dir: &Path) {
        let files = self.files.pin();
        for (path, file_tasks) in self.scan_all(repo, root_dir) {
            files.insert(path, file_tasks);
        }
    }

    /// The one sequential read pass: every markdown file the repo knows about,
    /// read once, keeping only those that contain tasks.
    fn scan_all(&self, repo: &Repo, root_dir: &Path) -> Vec<(PathBuf, Arc<FileTasks>)> {
        let start = std::time::Instant::now();

        // Snapshot the file list before touching the filesystem: holding the
        // pin guard across thousands of blocking reads would pin the map's
        // epoch for the whole pass. `search.rs` collects first for the same
        // reason.
        let targets: Vec<ScanTarget> = repo
            .markdown_files
            .pin()
            .iter()
            // Filtered before the read, not after: an excluded file is never
            // opened, so a large template folder costs nothing to skip.
            .filter(|(_, info)| !self.is_ignored(&info.raw_path))
            .map(|(abs_path, info)| ScanTarget::from_info(abs_path, info))
            .collect();

        let total = targets.len();
        // One buffer reused across every file; `scan` clears it per read.
        let mut buffer = String::new();
        let mut found = Vec::new();

        for target in targets {
            // Bound before the `if let` so the borrow of `target` has ended by
            // the time its `abs_path` is moved out.
            let scanned = target.scan(root_dir, &mut buffer);
            if let Some(file_tasks) = scanned {
                found.push((target.abs_path, Arc::new(file_tasks)));
            }
        }

        tracing::debug!(
            "Task scan: {}/{total} files contain tasks ({:?})",
            found.len(),
            start.elapsed()
        );
        found
    }
}

/// One file the scanner has to read, decoupled from the repo's pin guard.
struct ScanTarget {
    abs_path: PathBuf,
    raw_path: PathBuf,
    url_path: String,
    title: Option<String>,
}

impl ScanTarget {
    fn from_info(abs_path: &Path, info: &MarkdownInfo) -> Self {
        Self {
            abs_path: abs_path.to_path_buf(),
            raw_path: info.raw_path.clone(),
            url_path: info.url_path.clone(),
            title: info
                .frontmatter
                .as_ref()
                .and_then(|fm| fm.get("title"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }
    }

    /// Reads and scans the file, returning `None` when it cannot be read or
    /// contains no tasks.
    fn scan(&self, root_dir: &Path, buffer: &mut String) -> Option<FileTasks> {
        // `raw_path` is repo-relative, so rejoin the root before touching the
        // filesystem — the same rejoin `SearchEngine::search_file_content`
        // makes. The map key would also work, but going through `raw_path`
        // keeps this identical to the rest of the codebase's file access.
        let path = root_dir.join(&self.raw_path);

        if let Err(e) = read_capped(&path, buffer) {
            tracing::debug!("Task scan skipped {}: {e}", path.display());
            return None;
        }

        let tasks = scan_source_tasks(buffer);
        if tasks.is_empty() {
            return None;
        }
        Some(FileTasks::new(
            self.url_path.clone(),
            self.raw_path.clone(),
            self.title.clone(),
            tasks,
        ))
    }
}

/// Compiles `tasks_ignore_globs`, skipping any pattern that does not parse.
///
/// Lenient by design, and safe to be: [`Config::validate`](crate::config::Config::validate)
/// refuses a malformed pattern before a server ever starts, so this only
/// forgives callers that construct a `TaskIndex` without loading config.
fn compile_ignore_globs(patterns: &[String]) -> Vec<glob::Pattern> {
    patterns
        .iter()
        .filter_map(|pattern| {
            glob::Pattern::new(pattern)
                .map_err(|e| tracing::warn!("Invalid tasks_ignore_globs pattern '{pattern}': {e}"))
                .ok()
        })
        .collect()
}

/// Reads a whole file into `buffer`, refusing anything over [`MAX_SCAN_BYTES`].
///
/// The size check uses the already-open handle's metadata, so it costs no extra
/// path lookup. Invalid UTF-8 is an error rather than a lossy decode: a file
/// that is not text has no tasks in it worth guessing at.
fn read_capped(path: &Path, buffer: &mut String) -> std::io::Result<()> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if len > MAX_SCAN_BYTES {
        return Err(std::io::Error::other(format!(
            "file is {len} bytes, over the {MAX_SCAN_BYTES}-byte task-scan limit"
        )));
    }

    buffer.clear();
    // `read_to_string` appends, and the buffer was just cleared, so this reads
    // the whole file exactly once with at most one growth.
    buffer.reserve(usize::try_from(len).unwrap_or(0));
    file.read_to_string(buffer)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::parse_task_line;

    /// A scanned repository over `(relative path, contents)` pairs, plus the
    /// root to address it by.
    struct TestRepo {
        repo: Arc<Repo>,
        /// The **canonical** root. `Repo` canonicalizes the root it is given and
        /// keys `markdown_files` by canonical absolute paths, and on macOS a
        /// temp dir differs from its canonical form (`/var` vs `/private/var`).
        /// Addressing the index by the raw `TempDir` path would miss every
        /// entry — the same trap `server.rs` documents for the watcher.
        root: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl TestRepo {
        fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }

        fn write(&self, relative: &str, contents: &str) -> PathBuf {
            let path = self.path(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent dir");
            }
            std::fs::write(&path, contents).expect("write fixture file");
            path
        }
    }

    fn repo_over(files: &[(&str, &str)]) -> TestRepo {
        let dir = tempfile::tempdir().expect("create temp dir");
        let root = dir.path().canonicalize().expect("canonicalize temp dir");
        for (rel, contents) in files {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent dir");
            }
            std::fs::write(&path, contents).expect("write fixture file");
        }

        let repo = Repo::init(
            root.clone(),
            "static",
            &["md".to_string()],
            &[],
            &[],
            "index.md",
            &[],
            &[],
        );
        repo.scan_all().expect("scan repo");
        TestRepo {
            repo: Arc::new(repo),
            root,
            _dir: dir,
        }
    }

    fn task(line: &str) -> Task {
        parse_task_line(line, 1).expect("fixture line is a task")
    }

    fn file_tasks(url: &str, raw: &str, lines: &[&str]) -> FileTasks {
        FileTasks::new(
            url,
            PathBuf::from(raw),
            None,
            lines.iter().copied().map(task).collect(),
        )
    }

    // ---- FileTasks -----------------------------------------------------------

    #[test]
    fn counts_exclude_canceled_from_both() {
        let file = file_tasks(
            "/notes/",
            "notes.md",
            &[
                "- [ ] open one",
                "- [ ] open two",
                "- [x] done one",
                "- [-] canceled",
                "- [>] moved > 2026-08-04",
            ],
        );
        assert_eq!(file.open, 2);
        assert_eq!(file.done, 1);
        // The progress denominator is open + done: the two canceled tasks are
        // invisible to it.
        assert_eq!(file.tracked(), 3);
        assert_eq!(file.tasks.len(), 5);
    }

    #[test]
    fn folder_comes_from_the_source_path_not_the_url() {
        // A regular note: folder is its parent directory.
        let note = file_tasks("/docs/notes/weekly/", "docs/notes/weekly.md", &["- [ ] a"]);
        assert_eq!(note.folder(), "/docs/notes/");

        // An index file's url_path *is* the folder, so deriving the folder from
        // the URL would file its tasks under `/` instead of `/docs/`.
        let index = file_tasks("/docs/", "docs/index.md", &["- [ ] a"]);
        assert_eq!(index.folder(), "/docs/");

        // Root-level file.
        let root = file_tasks("/readme/", "readme.md", &["- [ ] a"]);
        assert_eq!(root.folder(), "/");
    }

    #[test]
    fn display_title_falls_back_to_the_file_stem() {
        let untitled = file_tasks("/docs/guide/", "docs/guide.md", &["- [ ] a"]);
        assert_eq!(untitled.display_title(), "guide");

        let titled = FileTasks::new(
            "/docs/guide/",
            PathBuf::from("docs/guide.md"),
            Some("The Guide".to_string()),
            vec![task("- [ ] a")],
        );
        assert_eq!(titled.display_title(), "The Guide");
    }

    // ---- read_capped ---------------------------------------------------------

    #[test]
    fn read_capped_reuses_the_buffer_without_appending() {
        let dir = tempfile::tempdir().expect("temp dir");
        let first = dir.path().join("a.md");
        let second = dir.path().join("b.md");
        std::fs::write(&first, "aaaa").expect("write");
        std::fs::write(&second, "bb").expect("write");

        let mut buffer = String::new();
        read_capped(&first, &mut buffer).expect("read a");
        assert_eq!(buffer, "aaaa");
        read_capped(&second, &mut buffer).expect("read b");
        assert_eq!(buffer, "bb", "buffer must be cleared between files");
    }

    #[test]
    fn read_capped_refuses_oversized_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("huge.md");
        let oversized = vec![b'x'; usize::try_from(MAX_SCAN_BYTES).unwrap_or(0) + 1];
        std::fs::write(&path, &oversized).expect("write");

        let mut buffer = String::new();
        assert!(read_capped(&path, &mut buffer).is_err());
    }

    #[test]
    fn read_capped_rejects_invalid_utf8() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("binary.md");
        std::fs::write(&path, [0xff, 0xfe, 0x00]).expect("write");

        let mut buffer = String::new();
        assert!(read_capped(&path, &mut buffer).is_err());
    }

    // ---- build ---------------------------------------------------------------

    #[tokio::test]
    async fn index_is_not_built_until_first_use() {
        let fixture = repo_over(&[("notes.md", "- [ ] a task\n")]);
        let index = Arc::new(TaskIndex::new(&[]));

        // Constructing the index reads nothing.
        assert!(!index.is_built());
        assert!(index.is_empty());

        // Neither does a file change: invalidation must not become a build.
        index.invalidate_file(
            &fixture.path("notes.md"),
            &ChangeEventType::Modified,
            &fixture.repo,
            &fixture.root,
        );
        assert!(!index.is_built());
        assert!(index.is_empty());

        // Nor does a full-rescan notification.
        index.rebuild_if_built(&fixture.repo, &fixture.root);
        assert!(!index.is_built());
        assert!(index.is_empty());

        index
            .ensure_built(&fixture.repo, &fixture.root)
            .await
            .expect("build succeeds");
        assert!(index.is_built());
        assert_eq!(index.len(), 1);
    }

    #[tokio::test]
    async fn build_stores_only_files_that_contain_tasks() {
        let fixture = repo_over(&[
            ("with.md", "# Notes\n\n- [ ] a task\n"),
            ("without.md", "# Notes\n\nJust prose.\n"),
            ("fenced.md", "```\n- [ ] not a task\n```\n"),
        ]);
        let index = Arc::new(TaskIndex::new(&[]));
        index
            .ensure_built(&fixture.repo, &fixture.root)
            .await
            .expect("build succeeds");

        assert_eq!(index.len(), 1);
        let stored = index.get(&fixture.path("with.md")).expect("indexed");
        assert_eq!(stored.tasks.len(), 1);
        assert_eq!(stored.tasks[0].text, "a task");
        assert_eq!(stored.url_path, "/with/");
        assert_eq!(stored.raw_path, PathBuf::from("with.md"));
    }

    #[tokio::test]
    async fn build_records_frontmatter_titles_and_counts() {
        let fixture = repo_over(&[(
            "docs/plan.md",
            "---\ntitle: The Plan\n---\n\n- [ ] one\n- [x] two\n- [-] three\n",
        )]);
        let index = Arc::new(TaskIndex::new(&[]));
        index
            .ensure_built(&fixture.repo, &fixture.root)
            .await
            .expect("build succeeds");

        let stored = index.get(&fixture.path("docs/plan.md")).expect("indexed");
        assert_eq!(stored.title.as_deref(), Some("The Plan"));
        assert_eq!(stored.display_title(), "The Plan");
        assert_eq!((stored.open, stored.done), (1, 1));
        assert_eq!(stored.folder(), "/docs/");
        // Line numbers are 1-based over the whole file, frontmatter included.
        assert_eq!(stored.tasks[0].line, 5);
    }

    #[tokio::test]
    async fn ensure_built_is_idempotent_and_single_flight() {
        let fixture = repo_over(&[("notes.md", "- [ ] a task\n")]);
        let index = Arc::new(TaskIndex::new(&[]));

        // Concurrent callers must converge on one build, not N.
        let calls = (0..8).map(|_| {
            let index = Arc::clone(&index);
            let repo = Arc::clone(&fixture.repo);
            let root = fixture.root.clone();
            async move { index.ensure_built(&repo, &root).await }
        });
        for result in futures::future::join_all(calls).await {
            result.expect("build succeeds");
        }

        assert_eq!(index.len(), 1);
        let stored = index.get(&fixture.path("notes.md")).expect("indexed");
        assert_eq!(
            stored.tasks.len(),
            1,
            "a re-run would have duplicated tasks"
        );
    }

    // ---- invalidation --------------------------------------------------------

    #[tokio::test]
    async fn invalidate_rescans_a_modified_file() {
        let fixture = repo_over(&[("notes.md", "- [ ] before\n")]);
        let index = Arc::new(TaskIndex::new(&[]));
        index
            .ensure_built(&fixture.repo, &fixture.root)
            .await
            .expect("build succeeds");

        let path = fixture.write("notes.md", "- [x] after\n- [ ] and more\n");
        index.invalidate_file(
            &path,
            &ChangeEventType::Modified,
            &fixture.repo,
            &fixture.root,
        );

        let stored = index.get(&path).expect("still indexed");
        assert_eq!(stored.tasks.len(), 2);
        assert_eq!(stored.tasks[0].text, "after");
        assert_eq!((stored.open, stored.done), (1, 1));
    }

    #[tokio::test]
    async fn invalidate_drops_a_file_that_lost_its_last_task() {
        let fixture = repo_over(&[("notes.md", "- [ ] a task\n")]);
        let index = Arc::new(TaskIndex::new(&[]));
        index
            .ensure_built(&fixture.repo, &fixture.root)
            .await
            .expect("build succeeds");
        assert_eq!(index.len(), 1);

        let path = fixture.write("notes.md", "just prose now\n");
        index.invalidate_file(
            &path,
            &ChangeEventType::Modified,
            &fixture.repo,
            &fixture.root,
        );

        assert!(
            index.get(&path).is_none(),
            "empty file must leave the index"
        );
        assert!(index.is_empty());
    }

    #[tokio::test]
    async fn invalidate_removes_a_deleted_file() {
        let fixture = repo_over(&[("notes.md", "- [ ] a task\n")]);
        let index = Arc::new(TaskIndex::new(&[]));
        index
            .ensure_built(&fixture.repo, &fixture.root)
            .await
            .expect("build succeeds");

        let path = fixture.path("notes.md");
        std::fs::remove_file(&path).expect("delete");
        fixture
            .repo
            .invalidate_file(&path, &ChangeEventType::Deleted);
        index.invalidate_file(
            &path,
            &ChangeEventType::Deleted,
            &fixture.repo,
            &fixture.root,
        );

        assert!(index.is_empty());
    }

    #[tokio::test]
    async fn invalidate_indexes_a_newly_created_file() {
        let fixture = repo_over(&[("notes.md", "- [ ] a task\n")]);
        let index = Arc::new(TaskIndex::new(&[]));
        index
            .ensure_built(&fixture.repo, &fixture.root)
            .await
            .expect("build succeeds");

        let path = fixture.write("fresh.md", "---\ntitle: Fresh\n---\n\n- [ ] brand new\n");
        // Mirrors the watcher: the repo learns about the file first.
        fixture
            .repo
            .invalidate_file(&path, &ChangeEventType::Created);
        index.invalidate_file(
            &path,
            &ChangeEventType::Created,
            &fixture.repo,
            &fixture.root,
        );

        let stored = index.get(&path).expect("indexed");
        assert_eq!(stored.tasks[0].text, "brand new");
        assert_eq!(stored.title.as_deref(), Some("Fresh"));
    }

    #[tokio::test]
    async fn invalidate_ignores_files_the_repo_does_not_track() {
        let fixture = repo_over(&[("notes.md", "- [ ] a task\n")]);
        let index = Arc::new(TaskIndex::new(&[]));
        index
            .ensure_built(&fixture.repo, &fixture.root)
            .await
            .expect("build succeeds");

        // The watcher forwards create/delete for assets too; those must not
        // land in a *task* index.
        let asset = fixture.write("photo.png", "not markdown");
        index.invalidate_file(
            &asset,
            &ChangeEventType::Created,
            &fixture.repo,
            &fixture.root,
        );

        assert_eq!(index.len(), 1);
        assert!(index.get(&asset).is_none());
    }

    // ---- full rescan ---------------------------------------------------------

    #[tokio::test]
    async fn rebuild_refreshes_adds_and_prunes_in_one_pass() {
        let fixture = repo_over(&[
            ("keep.md", "- [ ] unchanged\n"),
            ("change.md", "- [ ] before\n"),
            ("gone.md", "- [ ] doomed\n"),
        ]);
        let index = Arc::new(TaskIndex::new(&[]));
        index
            .ensure_built(&fixture.repo, &fixture.root)
            .await
            .expect("build succeeds");
        assert_eq!(index.len(), 3);

        // A change batch too large for surgical invalidation: edit, delete and
        // add all at once, then rescan wholesale.
        fixture.write("change.md", "- [x] after\n");
        std::fs::remove_file(fixture.path("gone.md")).expect("delete");
        fixture.write("added.md", "- [ ] newcomer\n");
        fixture.repo.full_rescan();
        index.rebuild_if_built(&fixture.repo, &fixture.root);

        assert_eq!(index.len(), 3);
        assert!(index.get(&fixture.path("gone.md")).is_none(), "pruned");
        assert_eq!(
            index
                .get(&fixture.path("change.md"))
                .expect("indexed")
                .tasks[0]
                .text,
            "after",
            "refreshed"
        );
        assert_eq!(
            index.get(&fixture.path("added.md")).expect("indexed").tasks[0].text,
            "newcomer",
            "added"
        );
        assert!(index.get(&fixture.path("keep.md")).is_some(), "kept");
    }

    // ---- tasks_ignore_globs --------------------------------------------------

    /// A `templates/` folder at the root, a second one nested under `docs/`,
    /// and one file that is nobody's template.
    fn templates_repo() -> TestRepo {
        repo_over(&[
            ("templates/checklist.md", "- [ ] template step\n"),
            ("templates/nested/deep.md", "- [ ] nested template step\n"),
            ("docs/templates/local.md", "- [ ] docs template step\n"),
            ("docs/plan.md", "- [ ] real work\n"),
        ])
    }

    /// Every indexed file's repo-relative path, sorted.
    fn indexed_paths(index: &TaskIndex) -> Vec<String> {
        let mut paths: Vec<String> = index
            .snapshot()
            .iter()
            .map(|f| crate::url_path::path_to_url(&f.raw_path))
            .collect();
        paths.sort();
        paths
    }

    async fn built_index(fixture: &TestRepo, globs: &[&str]) -> Arc<TaskIndex> {
        let globs: Vec<String> = globs.iter().map(|g| (*g).to_string()).collect();
        let index = Arc::new(TaskIndex::new(&globs));
        index
            .ensure_built(&fixture.repo, &fixture.root)
            .await
            .expect("build succeeds");
        index
    }

    /// The pattern table in `docs/reference/configuration.md#task-settings`,
    /// pinned so the documentation cannot drift from the matcher.
    #[test]
    fn documented_pattern_semantics() {
        let matches = |pattern: &str, path: &str| {
            TaskIndex::new(&[pattern.to_string()]).is_ignored(Path::new(path))
        };

        // `templates/**` — anchored at the root, reaching all the way down.
        assert!(matches("templates/**", "templates/a.md"));
        assert!(matches("templates/**", "templates/deep/b.md"));
        assert!(!matches("templates/**", "docs/templates/a.md"));

        // `**/templates/**` — any depth, including the root: `**/` matches zero
        // leading components.
        assert!(matches("**/templates/**", "templates/a.md"));
        assert!(matches("**/templates/**", "docs/templates/a.md"));
        assert!(matches("**/templates/**", "a/b/templates/c.md"));
        assert!(!matches("**/templates/**", "templates.md"));

        // A literal path excludes exactly one folder.
        assert!(matches("docs/templates/**", "docs/templates/a.md"));
        assert!(!matches("docs/templates/**", "templates/a.md"));

        // Suffix patterns.
        assert!(matches(
            "**/*.checklist.md",
            "templates/onboarding.checklist.md"
        ));
        assert!(!matches("**/*.checklist.md", "templates/onboarding.md"));

        // Patterns name files, so a bare folder name excludes nothing.
        assert!(!matches("templates", "templates/a.md"));

        // A single `*` is not stopped by `/` — mbr's globs behave this way
        // everywhere, and the docs say so.
        assert!(matches("templates/*.md", "templates/deep/a.md"));
    }

    /// `templates/**` is anchored at the repository root and reaches all the way
    /// down, so the nested file goes too — but a same-named folder elsewhere
    /// stays.
    #[tokio::test]
    async fn ignore_globs_exclude_a_folder_and_everything_under_it() {
        let fixture = templates_repo();
        let index = built_index(&fixture, &["templates/**"]).await;

        assert_eq!(
            indexed_paths(&index),
            vec!["docs/plan.md", "docs/templates/local.md"]
        );
        assert!(
            index
                .get(&fixture.path("templates/nested/deep.md"))
                .is_none(),
            "`**` must reach past the folder's direct children"
        );
        assert!(
            index.get(&fixture.path("docs/plan.md")).is_some(),
            "files outside the pattern must be untouched"
        );
    }

    /// The other documented spelling: `**/templates/**` matches a `templates`
    /// folder at *any* depth, including the repository root — `**/` matches zero
    /// leading components.
    #[tokio::test]
    async fn leading_double_star_matches_at_the_root_too() {
        let fixture = templates_repo();
        let index = built_index(&fixture, &["**/templates/**"]).await;

        assert_eq!(indexed_paths(&index), vec!["docs/plan.md"]);
    }

    /// Documented semantics, pinned: patterns match a *file* path, so a bare
    /// folder name matches nothing. `templates/**` is the spelling that works.
    #[tokio::test]
    async fn a_bare_folder_name_matches_nothing() {
        let fixture = templates_repo();
        let index = built_index(&fixture, &["templates"]).await;

        assert_eq!(index.len(), 4, "a bare folder name excludes nothing");
    }

    /// The default: no patterns, nothing excluded.
    #[tokio::test]
    async fn no_ignore_globs_excludes_nothing() {
        let fixture = templates_repo();
        let index = built_index(&fixture, &[]).await;

        assert_eq!(index.len(), 4);
    }

    /// A malformed pattern is dropped rather than treated as a literal, so it
    /// cannot accidentally exclude something. `Config::validate` rejects it
    /// before a real server ever gets here.
    #[tokio::test]
    async fn a_malformed_pattern_is_dropped_not_applied() {
        let fixture = templates_repo();
        let index = built_index(&fixture, &["templates/**bad", "templates/**"]).await;

        assert_eq!(
            indexed_paths(&index),
            vec!["docs/plan.md", "docs/templates/local.md"],
            "the valid pattern still applies"
        );
    }

    /// The guard `invalidate_file` needs of its own: it is a second way into the
    /// map, so without it, editing a file in an ignored folder silently indexes
    /// exactly the file the build skipped.
    #[tokio::test]
    async fn invalidate_does_not_readd_an_ignored_file() {
        let fixture = templates_repo();
        let index = built_index(&fixture, &["templates/**"]).await;
        assert_eq!(index.len(), 2);

        let path = fixture.write("templates/checklist.md", "- [ ] edited template step\n");
        // Mirrors the watcher: the repo is invalidated first.
        fixture
            .repo
            .invalidate_file(&path, &ChangeEventType::Modified);
        index.invalidate_file(
            &path,
            &ChangeEventType::Modified,
            &fixture.repo,
            &fixture.root,
        );

        assert!(
            index.get(&path).is_none(),
            "editing an ignored file must not index it"
        );
        assert_eq!(index.len(), 2);
    }

    /// Creating a file inside an ignored folder is the same story through the
    /// `Created` arm.
    #[tokio::test]
    async fn invalidate_does_not_add_a_newly_created_ignored_file() {
        let fixture = templates_repo();
        let index = built_index(&fixture, &["templates/**"]).await;

        let path = fixture.write("templates/fresh.md", "- [ ] brand new template step\n");
        fixture
            .repo
            .invalidate_file(&path, &ChangeEventType::Created);
        index.invalidate_file(
            &path,
            &ChangeEventType::Created,
            &fixture.repo,
            &fixture.root,
        );

        assert!(index.get(&path).is_none());
        assert_eq!(index.len(), 2);
    }

    /// The full-rescan path shares `scan_all`, so it inherits the filter.
    #[tokio::test]
    async fn rebuild_keeps_ignored_files_out() {
        let fixture = templates_repo();
        let index = built_index(&fixture, &["templates/**"]).await;

        fixture.write("templates/added.md", "- [ ] another template step\n");
        fixture.repo.full_rescan();
        index.rebuild_if_built(&fixture.repo, &fixture.root);

        assert_eq!(
            indexed_paths(&index),
            vec!["docs/plan.md", "docs/templates/local.md"]
        );
    }

    #[tokio::test]
    async fn snapshot_returns_every_indexed_file() {
        let fixture = repo_over(&[
            ("a.md", "- [ ] one\n"),
            ("docs/b.md", "- [ ] two\n"),
            ("docs/c.md", "no tasks\n"),
        ]);
        let index = Arc::new(TaskIndex::new(&[]));
        index
            .ensure_built(&fixture.repo, &fixture.root)
            .await
            .expect("build succeeds");

        let mut urls: Vec<String> = index
            .snapshot()
            .iter()
            .map(|f| f.url_path.clone())
            .collect();
        urls.sort();
        assert_eq!(urls, vec!["/a/", "/docs/b/"]);
    }
}
