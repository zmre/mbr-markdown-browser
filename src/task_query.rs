//! Pure filtering, grouping and counting for task queries.
//!
//! Everything here is a function of its arguments: no clock, no filesystem, no
//! shared state. [`run_query`] takes a snapshot of [`FileTasks`] (from
//! [`crate::task_index::TaskIndex::snapshot`]) plus the day to treat as "today"
//! and returns the whole response body. The axum handler is a thin wrapper, the
//! same split `SearchEngine::search` / `search_handler` uses.
//!
//! # Counting rules
//!
//! The two display modes count very differently, and the difference is the
//! whole reason grouping lives on the server:
//!
//! | Mode | Group | `total` counts | `done` counts |
//! |----------|--------------------|--------------------------------------------|-----------------|
//! | Category | one per file | **every** task in the file, filters ignored | its `Done` ones |
//! | Calendar | one per due bucket | tasks matching everything *except* status | its `Done` ones |
//!
//! Canceled tasks never count toward a total in either mode. In calendar mode
//! they are dropped outright — they cannot appear, be counted, or create a
//! bucket — because a canceled task has no meaningful due date.
//!
//! `limit` truncates the tasks that are *returned*; it never affects a count.
//! `total_matches` is the pre-truncation number of matching tasks.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

use crate::task_index::FileTasks;
use crate::tasks::{Task, TaskPriority, TaskStatus};

/// Default cap on returned tasks.
pub const DEFAULT_TASK_LIMIT: usize = 500;

/// A task query, as posted to `/.mbr/tasks`.
///
/// Every field is optional; `{}` is a valid request meaning "all incomplete
/// tasks in the repository, grouped by file".
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct TaskQuery {
    /// Filter text. Bare words match the display text; `#foo` matches tags.
    /// See [`parse_task_query`].
    pub q: String,
    /// Restrict to a folder **and all of its subfolders**, e.g. `/docs/`.
    pub folder: Option<String>,
    /// Statuses to show. **Empty means incomplete only** — the spec's default
    /// view — not "everything"; ask for everything with all three values.
    pub statuses: Vec<TaskStatus>,
    /// Priorities to show. Empty means all.
    pub priorities: Vec<TaskPriority>,
    /// Due-date filter.
    pub due: DueFilter,
    /// How results are grouped.
    pub mode: TaskMode,
    /// Maximum number of tasks returned across all groups.
    pub limit: usize,
}

impl Default for TaskQuery {
    fn default() -> Self {
        Self {
            q: String::new(),
            folder: None,
            statuses: Vec::new(),
            priorities: Vec::new(),
            due: DueFilter::Any,
            mode: TaskMode::Category,
            limit: DEFAULT_TASK_LIMIT,
        }
    }
}

/// Due-date filter. Mirrors [`DueBucket`] so the filter and the calendar
/// headings can never disagree about what "tomorrow" means.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DueFilter {
    /// No due-date restriction.
    #[default]
    Any,
    /// Due strictly before today.
    Overdue,
    /// Due today.
    Today,
    /// Due tomorrow.
    Tomorrow,
    /// Due after tomorrow.
    Upcoming,
    /// No due date at all.
    #[serde(rename = "none")]
    NoDue,
}

impl DueFilter {
    /// Whether a task in `bucket` passes this filter.
    pub fn matches(self, bucket: &DueBucket) -> bool {
        match self {
            Self::Any => true,
            Self::Overdue => matches!(bucket, DueBucket::Overdue),
            Self::Today => matches!(bucket, DueBucket::Today),
            Self::Tomorrow => matches!(bucket, DueBucket::Tomorrow),
            Self::Upcoming => matches!(bucket, DueBucket::Upcoming(_)),
            Self::NoDue => matches!(bucket, DueBucket::NoDue),
        }
    }
}

/// How results are grouped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskMode {
    /// One group per file, tasks in source order.
    #[default]
    Category,
    /// One group per due-date bucket.
    Calendar,
}

/// Which calendar heading a task falls under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DueBucket {
    /// Due on a day that has already passed.
    Overdue,
    /// Due today.
    Today,
    /// Due tomorrow.
    Tomorrow,
    /// Due on a specific later day (one group per date).
    Upcoming(NaiveDate),
    /// No due date. Always last.
    NoDue,
}

/// Places a due datetime in its calendar bucket relative to `today`.
///
/// Bucketing is by **date**, never by time of day: a task due today at 09:00 is
/// still "Today" at 17:00, and only becomes overdue once the day has ended.
/// That matches how `@due(2026-08-05)` with no time is stored (start of day) —
/// a time-sensitive rule would make an all-day task overdue the moment it was
/// created.
///
/// # Examples
///
/// ```
/// use chrono::NaiveDate;
/// use mbr::task_query::{DueBucket, due_bucket};
///
/// let today = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
/// let due = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap().and_hms_opt(9, 0, 0);
/// // New Year's Day is "tomorrow" from New Year's Eve.
/// assert_eq!(due_bucket(due, today), DueBucket::Tomorrow);
/// assert_eq!(due_bucket(None, today), DueBucket::NoDue);
/// ```
pub fn due_bucket(due: Option<NaiveDateTime>, today: NaiveDate) -> DueBucket {
    let Some(due) = due else {
        return DueBucket::NoDue;
    };
    let date = due.date();
    if date < today {
        return DueBucket::Overdue;
    }
    if date == today {
        return DueBucket::Today;
    }
    // `succ_opt` handles month and year rollover; it is `None` only at the end
    // of chrono's representable range, where "tomorrow" does not exist and
    // every future date is simply upcoming.
    if today.succ_opt() == Some(date) {
        return DueBucket::Tomorrow;
    }
    DueBucket::Upcoming(date)
}

/// The text filter, split into plain words and `#tag` tokens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryTerms {
    /// Lowercased bare words. Each must match the display text *or* a tag.
    pub words: Vec<String>,
    /// Lowercased `#tag` tokens (without the `#`). Each must match a tag.
    pub tags: Vec<String>,
}

impl QueryTerms {
    /// Whether this filter is satisfied by a task. All terms must match (AND).
    pub fn matches(&self, task: &Task) -> bool {
        if self.words.is_empty() && self.tags.is_empty() {
            return true;
        }
        let text = task.text.to_lowercase();
        self.tags.iter().all(|tag| has_tag_prefix(task, tag))
            && self
                .words
                .iter()
                .all(|word| text.contains(word) || has_tag_prefix(task, word))
    }
}

/// Whether any of the task's tags starts with `needle` (case-insensitive).
///
/// Prefix rather than substring: tags are short whole-word identifiers, and a
/// substring rule would make `#work` match `#homework`. Prefix still lets the
/// filter field narrow results while the user is still typing the tag.
fn has_tag_prefix(task: &Task, needle: &str) -> bool {
    task.tags.iter().any(|tag| {
        // `get` rather than a slice: `needle` is arbitrary user input and may
        // not land on a character boundary of `tag`.
        tag.get(..needle.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(needle))
    })
}

/// Splits a filter string into words and tags.
///
/// A token beginning with `#` and carrying at least one more character is a tag
/// token; everything else is a bare word. Matching is case-insensitive, so both
/// are lowercased here.
///
/// Bare words match the display text **or** a tag. Annotations are stripped out
/// of [`Task::text`], so a task rendered as `review  #work` has the literal text
/// `review` — a user who sees the `#work` pill and types `work` would otherwise
/// get nothing back.
///
/// # Examples
///
/// ```
/// use mbr::task_query::parse_task_query;
///
/// let terms = parse_task_query("Report  #Work");
/// assert_eq!(terms.words, ["report"]);
/// assert_eq!(terms.tags, ["work"]);
///
/// // A lone `#` is not a tag token.
/// assert_eq!(parse_task_query("#").words, ["#"]);
/// ```
pub fn parse_task_query(q: &str) -> QueryTerms {
    let mut terms = QueryTerms::default();
    for token in q.split_whitespace() {
        match token.strip_prefix('#') {
            Some(tag) if !tag.is_empty() => terms.tags.push(tag.to_lowercase()),
            _ => terms.words.push(token.to_lowercase()),
        }
    }
    terms
}

/// Normalizes a folder filter to a `/`-delimited prefix, or `None` for
/// "everywhere".
///
/// Both slashes are forced on so the prefix test cannot half-match a sibling:
/// `/doc` must not scope to `/docs/`. The repository root (`/` or an empty
/// string) is treated as no filter at all.
///
/// # Examples
///
/// ```
/// use mbr::task_query::normalize_folder;
///
/// assert_eq!(normalize_folder(Some("docs")).as_deref(), Some("/docs/"));
/// assert_eq!(normalize_folder(Some("/docs/")).as_deref(), Some("/docs/"));
/// assert_eq!(normalize_folder(Some("/")), None);
/// assert_eq!(normalize_folder(None), None);
/// ```
pub fn normalize_folder(folder: Option<&str>) -> Option<String> {
    let trimmed = folder?.trim().trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    Some(format!("/{trimmed}/"))
}

/// One task in a response: every [`Task`] field, plus where it lives.
///
/// Two locations, deliberately, because they answer different questions:
/// `url_path` is where a reader goes, `path` is what a writer patches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskHit {
    /// The task itself, flattened so `line`, `status`, `text` … are top-level.
    #[serde(flatten)]
    pub task: Task,
    /// URL of the page containing the task.
    pub url_path: String,
    /// Source path **relative to the repository root**, extension included and
    /// always `/`-separated — exactly what `POST /.mbr/task` wants as `path`.
    ///
    /// Sent rather than left to the client to reconstruct, because `url_path`
    /// does not determine it: an index file's URL *is* its folder
    /// (`docs/index.md` → `/docs/`), the static-folder overlay hides a whole
    /// directory level, and the extension is dropped. Any string surgery that
    /// recovered the file for one repository would silently patch the wrong
    /// one — or nothing — in another.
    pub path: String,
}

/// One heading in the results list, with its tasks and progress numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskGroup {
    /// Stable identifier, unique within a response. The file URL in category
    /// mode; `overdue` / `today` / `tomorrow` / `upcoming:<date>` / `none` in
    /// calendar mode. Suitable as a collapse-state key.
    pub key: String,
    /// Heading text: the note title, or the calendar bucket's name.
    pub label: String,
    /// Smaller secondary heading: the note's folder in category mode, empty in
    /// calendar mode.
    pub sublabel: String,
    /// Page to open when the heading is clicked; `None` for calendar buckets,
    /// which are not a place.
    pub url_path: Option<String>,
    /// The date this group covers, for `Today`/`Tomorrow`/`Upcoming` buckets.
    /// `None` in category mode and for `Overdue`/no-due-date.
    pub date: Option<NaiveDate>,
    /// Completed tasks counted by this group's rule (see the module docs).
    pub done: u32,
    /// Total tasks counted by this group's rule.
    pub total: u32,
    /// The matching tasks, already ordered and truncated.
    pub tasks: Vec<TaskHit>,
}

/// A folder and the number of matching tasks at or below it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderFacet {
    /// Folder path with leading and trailing slashes; `/` is the whole repo.
    pub path: String,
    /// Matching tasks in this folder **and its subfolders**.
    pub count: u32,
}

/// The `/.mbr/tasks` response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskQueryResponse {
    /// Result groups, in display order.
    pub groups: Vec<TaskGroup>,
    /// Folder facet counts for the folder pane, computed *ignoring* the folder
    /// filter so that selecting a folder does not empty out its siblings.
    pub folders: Vec<FolderFacet>,
    /// Matching tasks before `limit` was applied.
    pub total_matches: usize,
    /// Server-side query time.
    pub duration_ms: u64,
    /// True when the repository scan is still running, so the caller knows the
    /// results are partial. Set by the handler.
    pub scan_in_progress: bool,
}

/// Everything a task is tested against, resolved once per query.
struct Filters {
    terms: QueryTerms,
    statuses: Vec<TaskStatus>,
    priorities: Vec<TaskPriority>,
    due: DueFilter,
    today: NaiveDate,
    calendar: bool,
}

impl Filters {
    fn new(query: &TaskQuery, today: NaiveDate) -> Self {
        Self {
            terms: parse_task_query(&query.q),
            // An empty selection is the spec's default view: incomplete only.
            statuses: if query.statuses.is_empty() {
                vec![TaskStatus::Open]
            } else {
                query.statuses.clone()
            },
            priorities: query.priorities.clone(),
            due: query.due,
            today,
            calendar: query.mode == TaskMode::Calendar,
        }
    }

    /// Calendar mode ignores canceled tasks entirely: they never show, never
    /// count, and never create a bucket.
    fn admissible(&self, task: &Task) -> bool {
        !(self.calendar && task.status == TaskStatus::Canceled)
    }

    /// Every filter *except* status. This is the predicate the calendar-mode
    /// progress counts use, so a bucket's `3/7` does not shrink when the user
    /// narrows the view to incomplete tasks.
    fn matches_except_status(&self, task: &Task) -> bool {
        (self.priorities.is_empty() || self.priorities.contains(&task.priority))
            && self.due.matches(&due_bucket(task.due, self.today))
            && self.terms.matches(task)
    }

    fn matches_status(&self, task: &Task) -> bool {
        self.statuses.contains(&task.status)
    }

    fn matches(&self, task: &Task) -> bool {
        self.matches_except_status(task) && self.matches_status(task)
    }
}

/// Runs a task query over an index snapshot.
///
/// `today` is a parameter rather than a `Local::now()` call so the bucketing is
/// testable without mocking the clock; the handler supplies the real one.
///
/// `scan_in_progress` is left `false`; the handler fills it in from the repo,
/// mirroring `search_handler`.
pub fn run_query(
    files: &[Arc<FileTasks>],
    query: &TaskQuery,
    today: NaiveDate,
) -> TaskQueryResponse {
    let start = std::time::Instant::now();
    let filters = Filters::new(query, today);
    let folder = normalize_folder(query.folder.as_deref());

    let folders = folder_facets(files, &filters);
    let scoped: Vec<&Arc<FileTasks>> = files
        .iter()
        .filter(|file| in_folder(file, folder.as_deref()))
        .collect();

    let mut groups = match query.mode {
        TaskMode::Category => group_by_file(&scoped, &filters),
        TaskMode::Calendar => group_by_due(&scoped, &filters),
    };

    let total_matches = groups.iter().map(|g| g.tasks.len()).sum();
    truncate_groups(&mut groups, query.limit);

    TaskQueryResponse {
        groups,
        folders,
        total_matches,
        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        scan_in_progress: false,
    }
}

/// Whether a file is inside the (already normalized) folder scope.
///
/// Prefix matching is what makes the scope include subfolders: `/docs/` matches
/// `/docs/`, `/docs/notes/`, and everything below.
fn in_folder(file: &FileTasks, folder: Option<&str>) -> bool {
    folder.is_none_or(|scope| file.folder().starts_with(scope))
}

/// Category mode: one group per file, tasks in source order.
///
/// `done`/`total` come straight off [`FileTasks`], so they describe the *file*,
/// not the view — a file showing one matching task can still read `3/7`.
fn group_by_file(files: &[&Arc<FileTasks>], filters: &Filters) -> Vec<TaskGroup> {
    let mut groups: Vec<TaskGroup> = files
        .iter()
        .filter_map(|file| {
            // Built once per file, not once per hit: `path_to_url` allocates.
            let path = crate::url_path::path_to_url(&file.raw_path);
            let mut tasks: Vec<TaskHit> = file
                .tasks
                .iter()
                .filter(|task| filters.admissible(task) && filters.matches(task))
                .map(|task| TaskHit {
                    task: task.clone(),
                    url_path: file.url_path.clone(),
                    path: path.clone(),
                })
                .collect();
            if tasks.is_empty() {
                return None;
            }
            tasks.sort_by_key(|hit| hit.task.line);

            Some(TaskGroup {
                key: file.url_path.clone(),
                label: file.display_title().to_string(),
                sublabel: file.folder().trim_matches('/').to_string(),
                url_path: Some(file.url_path.clone()),
                date: None,
                done: file.done,
                total: file.tracked(),
                tasks,
            })
        })
        .collect();

    // Sorted by URL so notes in the same folder stay together and the order is
    // stable across requests (the index is an unordered concurrent map).
    groups.sort_by(|a, b| a.key.cmp(&b.key));
    groups
}

/// Calendar mode: one group per due-date bucket.
///
/// Counts are filtered by everything except status, per the spec, so toggling
/// "show completed" moves tasks in and out of the list without moving the
/// progress bar.
fn group_by_due(files: &[&Arc<FileTasks>], filters: &Filters) -> Vec<TaskGroup> {
    // `BTreeMap` keyed by the bucket gives the display order for free:
    // `DueBucket`'s variants are declared Overdue → Today → Tomorrow →
    // Upcoming(date, ascending) → NoDue, and its derived `Ord` follows.
    let mut buckets: BTreeMap<DueBucket, BucketAccumulator> = BTreeMap::new();

    for file in files {
        // Hoisted out of the task loop for the same reason as in
        // `group_by_file`: one allocation per file rather than one per hit.
        let path = crate::url_path::path_to_url(&file.raw_path);
        for task in &file.tasks {
            if !filters.admissible(task) || !filters.matches_except_status(task) {
                continue;
            }
            let entry = buckets
                .entry(due_bucket(task.due, filters.today))
                .or_default();

            entry.total = entry.total.saturating_add(1);
            if task.status == TaskStatus::Done {
                entry.done = entry.done.saturating_add(1);
            }
            if filters.matches_status(task) {
                entry.tasks.push(TaskHit {
                    task: task.clone(),
                    url_path: file.url_path.clone(),
                    path: path.clone(),
                });
            }
        }
    }

    buckets
        .into_iter()
        .filter_map(|(bucket, mut acc)| {
            if acc.tasks.is_empty() {
                return None;
            }
            // Chronological within a bucket, then by page and line so the order
            // never depends on the index's iteration order.
            acc.tasks.sort_by(|a, b| {
                a.task
                    .due
                    .cmp(&b.task.due)
                    .then_with(|| a.url_path.cmp(&b.url_path))
                    .then_with(|| a.task.line.cmp(&b.task.line))
            });

            // Overdue deliberately carries no progress: a backlog of missed
            // deadlines has no meaningful denominator.
            let (done, total) = if bucket == DueBucket::Overdue {
                (0, 0)
            } else {
                (acc.done, acc.total)
            };

            Some(TaskGroup {
                key: bucket_key(&bucket),
                label: bucket_label(&bucket),
                sublabel: String::new(),
                url_path: None,
                date: bucket_date(&bucket, filters.today),
                done,
                total,
                tasks: acc.tasks,
            })
        })
        .collect()
}

/// Per-bucket working state while grouping.
#[derive(Default)]
struct BucketAccumulator {
    tasks: Vec<TaskHit>,
    done: u32,
    total: u32,
}

fn bucket_key(bucket: &DueBucket) -> String {
    match bucket {
        DueBucket::Overdue => "overdue".to_string(),
        DueBucket::Today => "today".to_string(),
        DueBucket::Tomorrow => "tomorrow".to_string(),
        DueBucket::Upcoming(date) => format!("upcoming:{date}"),
        DueBucket::NoDue => "none".to_string(),
    }
}

fn bucket_label(bucket: &DueBucket) -> String {
    match bucket {
        DueBucket::Overdue => "Overdue".to_string(),
        DueBucket::Today => "Today".to_string(),
        DueBucket::Tomorrow => "Tomorrow".to_string(),
        // ISO here, not a formatted date: the frontend renders it in the user's
        // locale, and a server-side format would hard-code English months.
        DueBucket::Upcoming(date) => date.to_string(),
        DueBucket::NoDue => "No due date".to_string(),
    }
}

fn bucket_date(bucket: &DueBucket, today: NaiveDate) -> Option<NaiveDate> {
    match bucket {
        DueBucket::Today => Some(today),
        DueBucket::Tomorrow => today.succ_opt(),
        DueBucket::Upcoming(date) => Some(*date),
        DueBucket::Overdue | DueBucket::NoDue => None,
    }
}

/// Counts matching tasks per folder, cumulatively up the tree.
///
/// Computed **without** the folder filter, because these numbers drive the
/// folder pane: scoping them to the current selection would zero out every
/// folder the user might want to switch to. Each task increments its own folder
/// and every ancestor, so `/` always carries the repository total and a parent's
/// count is what selecting it would show.
fn folder_facets(files: &[Arc<FileTasks>], filters: &Filters) -> Vec<FolderFacet> {
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();

    for file in files {
        let matched = u32::try_from(
            file.tasks
                .iter()
                .filter(|task| filters.admissible(task) && filters.matches(task))
                .count(),
        )
        .unwrap_or(u32::MAX);
        if matched == 0 {
            continue;
        }

        let folder = file.folder();
        // `/docs/notes/` contributes to `/`, `/docs/` and `/docs/notes/`. Slice
        // boundaries land on `/`, which is ASCII, so this cannot split a
        // multi-byte character.
        for (index, byte) in folder.bytes().enumerate() {
            if byte == b'/' {
                let ancestor = &folder[..=index];
                *counts.entry(ancestor).or_insert(0) += matched;
            }
        }
    }

    counts
        .into_iter()
        .map(|(path, count)| FolderFacet {
            path: path.to_string(),
            count,
        })
        .collect()
}

/// Applies `limit` across the ordered groups, dropping groups left with nothing.
///
/// Counts are already computed, so truncation never changes a `done`/`total`
/// pair — only which tasks came back.
fn truncate_groups(groups: &mut Vec<TaskGroup>, limit: usize) {
    let mut remaining = limit;
    for group in groups.iter_mut() {
        if group.tasks.len() > remaining {
            group.tasks.truncate(remaining);
        }
        remaining -= group.tasks.len();
    }
    groups.retain(|group| !group.tasks.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::scan_source_tasks;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid test date")
    }

    /// Today for every test below, unless a test says otherwise.
    fn today() -> NaiveDate {
        day(2026, 8, 4)
    }

    /// Builds a `FileTasks` by scanning a markdown body, so tests exercise the
    /// real parser rather than hand-built `Task` values.
    fn file(url: &str, raw: &str, title: Option<&str>, body: &str) -> Arc<FileTasks> {
        Arc::new(FileTasks::new(
            url,
            std::path::PathBuf::from(raw),
            title.map(str::to_string),
            scan_source_tasks(body),
        ))
    }

    fn query() -> TaskQuery {
        TaskQuery::default()
    }

    /// Every returned task's display text, in response order.
    fn texts(response: &TaskQueryResponse) -> Vec<&str> {
        response
            .groups
            .iter()
            .flat_map(|g| g.tasks.iter().map(|t| t.task.text.as_str()))
            .collect()
    }

    // ---- parse_task_query ----------------------------------------------------

    #[test]
    fn query_parsing_splits_words_and_tags() {
        let terms = parse_task_query("report #work #Ops draft");
        assert_eq!(terms.words, ["report", "draft"]);
        assert_eq!(terms.tags, ["work", "ops"]);
    }

    #[test]
    fn query_parsing_lowercases_and_ignores_extra_whitespace() {
        let terms = parse_task_query("  REPORT \t #WORK  ");
        assert_eq!(terms.words, ["report"]);
        assert_eq!(terms.tags, ["work"]);
    }

    #[test]
    fn query_parsing_treats_a_bare_hash_as_a_word() {
        assert_eq!(parse_task_query("#").words, ["#"]);
        assert!(parse_task_query("#").tags.is_empty());
    }

    #[test]
    fn empty_query_matches_everything() {
        let terms = parse_task_query("   ");
        assert!(terms.words.is_empty() && terms.tags.is_empty());
        let task = crate::tasks::parse_task_line("- [ ] anything", 1).expect("task");
        assert!(terms.matches(&task));
    }

    #[test]
    fn words_and_multiple_terms_are_anded() {
        let task =
            crate::tasks::parse_task_line("- [ ] write the quarterly report", 1).expect("task");
        assert!(parse_task_query("write report").matches(&task));
        assert!(!parse_task_query("write missing").matches(&task));
    }

    #[test]
    fn word_matching_is_a_case_insensitive_substring() {
        let task = crate::tasks::parse_task_line("- [ ] Write The Report", 1).expect("task");
        assert!(parse_task_query("write").matches(&task));
        assert!(
            parse_task_query("REPO").matches(&task),
            "substring, any case"
        );
    }

    #[test]
    fn tag_tokens_match_by_prefix_not_substring() {
        let task = crate::tasks::parse_task_line("- [ ] a #homework", 1).expect("task");
        assert!(parse_task_query("#home").matches(&task), "prefix matches");
        assert!(
            !parse_task_query("#work").matches(&task),
            "substring must not match, or #work would hit #homework"
        );
    }

    #[test]
    fn a_bare_word_also_matches_a_tag() {
        // The tag is stripped out of the display text, so a user who sees the
        // `#work` pill and types `work` must still get the task.
        let task = crate::tasks::parse_task_line("- [ ] review #work", 1).expect("task");
        assert_eq!(task.text, "review");
        assert!(parse_task_query("work").matches(&task));
    }

    #[test]
    fn a_tag_token_does_not_match_the_display_text() {
        let task = crate::tasks::parse_task_line("- [ ] do the work", 1).expect("task");
        assert!(!parse_task_query("#work").matches(&task));
    }

    // ---- normalize_folder ----------------------------------------------------

    #[test]
    fn folder_normalization_forces_both_slashes() {
        for input in ["docs", "/docs", "docs/", "/docs/", "  /docs/  "] {
            assert_eq!(
                normalize_folder(Some(input)).as_deref(),
                Some("/docs/"),
                "for {input:?}"
            );
        }
    }

    #[test]
    fn folder_normalization_treats_the_root_as_no_filter() {
        for input in ["", "/", "   ", "//"] {
            assert_eq!(normalize_folder(Some(input)), None, "for {input:?}");
        }
        assert_eq!(normalize_folder(None), None);
    }

    // ---- due_bucket ----------------------------------------------------------

    fn bucket_for(date: NaiveDate, today: NaiveDate) -> DueBucket {
        due_bucket(date.and_hms_opt(9, 0, 0), today)
    }

    #[test]
    fn due_bucketing_covers_every_bucket() {
        let today = today();
        assert_eq!(bucket_for(day(2026, 8, 3), today), DueBucket::Overdue);
        assert_eq!(bucket_for(today, today), DueBucket::Today);
        assert_eq!(bucket_for(day(2026, 8, 5), today), DueBucket::Tomorrow);
        assert_eq!(
            bucket_for(day(2026, 8, 9), today),
            DueBucket::Upcoming(day(2026, 8, 9))
        );
        assert_eq!(due_bucket(None, today), DueBucket::NoDue);
    }

    #[test]
    fn a_task_due_later_today_is_not_yet_overdue() {
        // Bucketing is by date: 09:00 today is still "Today" all day long.
        let today = today();
        let early = today.and_hms_opt(0, 1, 0);
        assert_eq!(due_bucket(early, today), DueBucket::Today);
        let late = today.and_hms_opt(23, 59, 0);
        assert_eq!(due_bucket(late, today), DueBucket::Today);
    }

    #[test]
    fn due_bucketing_crosses_a_month_boundary() {
        let today = day(2026, 8, 31);
        assert_eq!(bucket_for(day(2026, 8, 30), today), DueBucket::Overdue);
        assert_eq!(bucket_for(day(2026, 9, 1), today), DueBucket::Tomorrow);
        assert_eq!(
            bucket_for(day(2026, 9, 2), today),
            DueBucket::Upcoming(day(2026, 9, 2))
        );
    }

    #[test]
    fn due_bucketing_crosses_a_year_boundary() {
        let today = day(2026, 12, 31);
        assert_eq!(bucket_for(day(2026, 12, 30), today), DueBucket::Overdue);
        assert_eq!(bucket_for(day(2027, 1, 1), today), DueBucket::Tomorrow);
        assert_eq!(
            bucket_for(day(2027, 1, 2), today),
            DueBucket::Upcoming(day(2027, 1, 2))
        );
    }

    #[test]
    fn due_bucketing_crosses_a_leap_day() {
        let today = day(2028, 2, 28);
        assert_eq!(bucket_for(day(2028, 2, 29), today), DueBucket::Tomorrow);
        let leap_day = day(2028, 2, 29);
        assert_eq!(bucket_for(day(2028, 3, 1), leap_day), DueBucket::Tomorrow);
    }

    #[test]
    fn bucket_ordering_is_the_display_order() {
        let mut buckets = vec![
            DueBucket::NoDue,
            DueBucket::Upcoming(day(2026, 9, 1)),
            DueBucket::Tomorrow,
            DueBucket::Upcoming(day(2026, 8, 20)),
            DueBucket::Today,
            DueBucket::Overdue,
        ];
        buckets.sort();
        assert_eq!(
            buckets,
            vec![
                DueBucket::Overdue,
                DueBucket::Today,
                DueBucket::Tomorrow,
                DueBucket::Upcoming(day(2026, 8, 20)),
                DueBucket::Upcoming(day(2026, 9, 1)),
                DueBucket::NoDue,
            ]
        );
    }

    // ---- status filtering ----------------------------------------------------

    fn mixed_status_file() -> Vec<Arc<FileTasks>> {
        vec![file(
            "/notes/",
            "notes.md",
            None,
            "- [ ] open one\n- [x] done one\n- [-] canceled one\n",
        )]
    }

    #[test]
    fn default_status_filter_is_incomplete_only() {
        let response = run_query(&mixed_status_file(), &query(), today());
        assert_eq!(texts(&response), ["open one"]);
        assert_eq!(response.total_matches, 1);
    }

    #[test]
    fn status_filter_is_a_multi_select() {
        let q = TaskQuery {
            statuses: vec![TaskStatus::Open, TaskStatus::Done],
            ..query()
        };
        let response = run_query(&mixed_status_file(), &q, today());
        assert_eq!(texts(&response), ["open one", "done one"]);

        let q = TaskQuery {
            statuses: vec![TaskStatus::Canceled],
            ..query()
        };
        let response = run_query(&mixed_status_file(), &q, today());
        assert_eq!(texts(&response), ["canceled one"]);
    }

    #[test]
    fn priority_filter_is_a_multi_select() {
        let files = vec![file(
            "/notes/",
            "notes.md",
            None,
            "- [ ] plain\n- [ ] high !!\n- [ ] urgent !!!\n",
        )];

        let all = run_query(&files, &query(), today());
        assert_eq!(texts(&all), ["plain", "high", "urgent"]);

        let q = TaskQuery {
            priorities: vec![TaskPriority::High, TaskPriority::Urgent],
            ..query()
        };
        let filtered = run_query(&files, &q, today());
        assert_eq!(texts(&filtered), ["high", "urgent"]);
    }

    #[test]
    fn due_filter_selects_one_bucket() {
        let files = vec![file(
            "/notes/",
            "notes.md",
            None,
            concat!(
                "- [ ] late @due(2026-08-01)\n",
                "- [ ] now @due(2026-08-04)\n",
                "- [ ] soon @due(2026-08-05)\n",
                "- [ ] later @due(2026-08-20)\n",
                "- [ ] someday\n",
            ),
        )];

        for (filter, expected) in [
            (DueFilter::Overdue, vec!["late"]),
            (DueFilter::Today, vec!["now"]),
            (DueFilter::Tomorrow, vec!["soon"]),
            (DueFilter::Upcoming, vec!["later"]),
            (DueFilter::NoDue, vec!["someday"]),
        ] {
            let q = TaskQuery {
                due: filter,
                ..query()
            };
            let response = run_query(&files, &q, today());
            assert_eq!(texts(&response), expected, "for {filter:?}");
        }
    }

    // ---- folder scoping ------------------------------------------------------

    fn folder_tree() -> Vec<Arc<FileTasks>> {
        vec![
            file("/top/", "top.md", None, "- [ ] at root\n"),
            file("/docs/guide/", "docs/guide.md", None, "- [ ] in docs\n"),
            file(
                "/docs/notes/weekly/",
                "docs/notes/weekly.md",
                None,
                "- [ ] in notes\n",
            ),
            file("/other/thing/", "other/thing.md", None, "- [ ] elsewhere\n"),
        ]
    }

    #[test]
    fn folder_scope_includes_subfolders() {
        let q = TaskQuery {
            folder: Some("/docs/".to_string()),
            ..query()
        };
        let response = run_query(&folder_tree(), &q, today());
        assert_eq!(texts(&response), ["in docs", "in notes"]);
    }

    #[test]
    fn folder_scope_does_not_half_match_a_sibling() {
        let files = vec![
            file("/docs/a/", "docs/a.md", None, "- [ ] real\n"),
            file("/docsy/b/", "docsy/b.md", None, "- [ ] decoy\n"),
        ];
        let q = TaskQuery {
            folder: Some("docs".to_string()),
            ..query()
        };
        assert_eq!(texts(&run_query(&files, &q, today())), ["real"]);
    }

    #[test]
    fn folder_facets_are_cumulative_and_ignore_the_folder_filter() {
        let q = TaskQuery {
            folder: Some("/docs/notes/".to_string()),
            ..query()
        };
        let response = run_query(&folder_tree(), &q, today());

        // The view is scoped...
        assert_eq!(texts(&response), ["in notes"]);
        // ...but the facets still describe the whole repo, so the folder pane
        // can offer somewhere else to go.
        let facets: Vec<(&str, u32)> = response
            .folders
            .iter()
            .map(|f| (f.path.as_str(), f.count))
            .collect();
        assert_eq!(
            facets,
            vec![("/", 4), ("/docs/", 2), ("/docs/notes/", 1), ("/other/", 1),]
        );
    }

    #[test]
    fn folder_facets_respect_the_other_filters() {
        let files = vec![file(
            "/docs/a/",
            "docs/a.md",
            None,
            "- [ ] open\n- [x] done\n",
        )];
        let response = run_query(&files, &query(), today());
        assert_eq!(
            response
                .folders
                .iter()
                .map(|f| (f.path.as_str(), f.count))
                .collect::<Vec<_>>(),
            vec![("/", 1), ("/docs/", 1)],
            "the completed task is filtered out of the facet counts too"
        );
    }

    // ---- category mode -------------------------------------------------------

    #[test]
    fn category_groups_are_per_file_and_ordered_by_line() {
        let files = vec![file(
            "/notes/",
            "notes.md",
            Some("Weekly notes"),
            "# Notes\n\n- [ ] second\n\nprose\n\n- [ ] first\n",
        )];
        let response = run_query(&files, &query(), today());

        assert_eq!(response.groups.len(), 1);
        let group = &response.groups[0];
        assert_eq!(group.label, "Weekly notes");
        assert_eq!(group.key, "/notes/");
        assert_eq!(group.url_path.as_deref(), Some("/notes/"));
        assert_eq!(group.date, None);
        // Source order, which is line order — not the order they were written
        // about above.
        assert_eq!(
            group.tasks.iter().map(|t| t.task.line).collect::<Vec<_>>(),
            vec![3, 7]
        );
    }

    #[test]
    fn category_group_labels_fall_back_to_the_file_stem() {
        let files = vec![file(
            "/docs/untitled-note/",
            "docs/untitled-note.md",
            None,
            "- [ ] a\n",
        )];
        let response = run_query(&files, &query(), today());
        assert_eq!(response.groups[0].label, "untitled-note");
        assert_eq!(response.groups[0].sublabel, "docs");
    }

    #[test]
    fn category_totals_include_tasks_filtered_out_of_the_view() {
        // Seven tracked tasks (three done), plus a canceled one that counts for
        // nothing, and only one of them matches the text filter.
        let files = vec![file(
            "/notes/",
            "notes.md",
            None,
            concat!(
                "- [x] alpha\n",
                "- [x] beta\n",
                "- [x] gamma\n",
                "- [ ] delta\n",
                "- [ ] epsilon\n",
                "- [ ] zeta\n",
                "- [ ] findme\n",
                "- [-] canceled\n",
            ),
        )];
        let q = TaskQuery {
            q: "findme".to_string(),
            ..query()
        };
        let response = run_query(&files, &q, today());

        let group = &response.groups[0];
        assert_eq!(group.tasks.len(), 1, "only the matching task is returned");
        assert_eq!(
            (group.done, group.total),
            (3, 7),
            "counts describe the file, not the view, and exclude the canceled task"
        );
        assert_eq!(response.total_matches, 1);
    }

    #[test]
    fn category_mode_can_show_canceled_tasks_without_counting_them() {
        let files = vec![file(
            "/notes/",
            "notes.md",
            None,
            "- [ ] open\n- [x] done\n- [-] canceled\n",
        )];
        let q = TaskQuery {
            statuses: vec![TaskStatus::Canceled],
            ..query()
        };
        let response = run_query(&files, &q, today());

        assert_eq!(texts(&response), ["canceled"]);
        assert_eq!((response.groups[0].done, response.groups[0].total), (1, 2));
    }

    #[test]
    fn category_groups_are_sorted_by_url_and_empty_ones_dropped() {
        let files = vec![
            file("/zeta/", "zeta.md", None, "- [ ] z\n"),
            file("/alpha/", "alpha.md", None, "- [ ] a\n"),
            file("/nothing/", "nothing.md", None, "- [x] all done here\n"),
        ];
        let response = run_query(&files, &query(), today());
        assert_eq!(
            response
                .groups
                .iter()
                .map(|g| g.key.as_str())
                .collect::<Vec<_>>(),
            vec!["/alpha/", "/zeta/"]
        );
    }

    // ---- calendar mode -------------------------------------------------------

    fn calendar_query() -> TaskQuery {
        TaskQuery {
            mode: TaskMode::Calendar,
            ..TaskQuery::default()
        }
    }

    fn calendar_files() -> Vec<Arc<FileTasks>> {
        vec![file(
            "/notes/",
            "notes.md",
            None,
            concat!(
                "- [ ] late @due(2026-08-01)\n",
                "- [ ] now @due(2026-08-04)\n",
                "- [ ] soon @due(2026-08-05)\n",
                "- [ ] far @due(2026-08-20)\n",
                "- [ ] farther @due(2026-08-21)\n",
                "- [ ] whenever\n",
            ),
        )]
    }

    #[test]
    fn calendar_buckets_are_in_display_order_with_one_group_per_upcoming_date() {
        let response = run_query(&calendar_files(), &calendar_query(), today());
        assert_eq!(
            response
                .groups
                .iter()
                .map(|g| (g.key.as_str(), g.label.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("overdue", "Overdue"),
                ("today", "Today"),
                ("tomorrow", "Tomorrow"),
                ("upcoming:2026-08-20", "2026-08-20"),
                ("upcoming:2026-08-21", "2026-08-21"),
                ("none", "No due date"),
            ]
        );
    }

    #[test]
    fn calendar_groups_carry_their_date_and_no_page_url() {
        let response = run_query(&calendar_files(), &calendar_query(), today());
        let by_key = |key: &str| {
            response
                .groups
                .iter()
                .find(|g| g.key == key)
                .unwrap_or_else(|| panic!("expected a {key} group"))
        };
        assert_eq!(by_key("today").date, Some(day(2026, 8, 4)));
        assert_eq!(by_key("tomorrow").date, Some(day(2026, 8, 5)));
        assert_eq!(by_key("upcoming:2026-08-20").date, Some(day(2026, 8, 20)));
        assert_eq!(by_key("overdue").date, None);
        assert_eq!(by_key("none").date, None);
        assert!(response.groups.iter().all(|g| g.url_path.is_none()));
    }

    #[test]
    fn calendar_overdue_has_no_totals() {
        let response = run_query(&calendar_files(), &calendar_query(), today());
        let overdue = &response.groups[0];
        assert_eq!(overdue.key, "overdue");
        assert_eq!(overdue.tasks.len(), 1);
        assert_eq!((overdue.done, overdue.total), (0, 0));
    }

    #[test]
    fn calendar_totals_ignore_the_status_filter_but_honor_the_rest() {
        let files = vec![file(
            "/notes/",
            "notes.md",
            None,
            concat!(
                "- [ ] open one @due(2026-08-04)\n",
                "- [x] done one @due(2026-08-04)\n",
                "- [x] done two @due(2026-08-04)\n",
                // Filtered out by the text query, so it is not counted at all.
                "- [ ] unrelated @due(2026-08-04)\n",
            ),
        )];
        let q = TaskQuery {
            q: "one".to_string(),
            ..calendar_query()
        };
        let response = run_query(&files, &q, today());

        let today_group = &response.groups[0];
        assert_eq!(today_group.key, "today");
        // The view shows only the open task (default status filter)...
        assert_eq!(
            today_group
                .tasks
                .iter()
                .map(|t| t.task.text.as_str())
                .collect::<Vec<_>>(),
            vec!["open one"]
        );
        // ...but the progress covers every task matching the text filter.
        assert_eq!((today_group.done, today_group.total), (2, 3));
    }

    #[test]
    fn calendar_ignores_canceled_tasks_entirely() {
        let files = vec![file(
            "/notes/",
            "notes.md",
            None,
            concat!(
                "- [ ] open @due(2026-08-04)\n",
                "- [-] canceled @due(2026-08-04)\n",
                "- [>] moved @due(2026-08-04) > 2026-08-10\n",
            ),
        )];

        let response = run_query(&files, &calendar_query(), today());
        assert_eq!(texts(&response), ["open"]);
        // Neither canceled task is in the denominator.
        assert_eq!((response.groups[0].done, response.groups[0].total), (0, 1));

        // Even asking for canceled tasks explicitly shows none in this mode.
        let q = TaskQuery {
            statuses: vec![TaskStatus::Canceled],
            ..calendar_query()
        };
        assert!(run_query(&files, &q, today()).groups.is_empty());
    }

    #[test]
    fn calendar_tasks_are_chronological_within_a_bucket() {
        let files = vec![
            file(
                "/b/",
                "b.md",
                None,
                "- [ ] afternoon @due(2026-08-04 15:00)\n",
            ),
            file(
                "/a/",
                "a.md",
                None,
                "- [ ] morning @due(2026-08-04 09:00)\n",
            ),
        ];
        let response = run_query(&files, &calendar_query(), today());
        assert_eq!(texts(&response), ["morning", "afternoon"]);
    }

    // ---- limits --------------------------------------------------------------

    #[test]
    fn limit_truncates_tasks_without_changing_counts_or_total_matches() {
        let files = vec![
            file("/a/", "a.md", None, "- [ ] a1\n- [ ] a2\n- [ ] a3\n"),
            file("/b/", "b.md", None, "- [ ] b1\n- [ ] b2\n"),
        ];
        let q = TaskQuery {
            limit: 4,
            ..query()
        };
        let response = run_query(&files, &q, today());

        assert_eq!(texts(&response), ["a1", "a2", "a3", "b1"]);
        assert_eq!(response.total_matches, 5, "counted before truncation");
        // The first group's own progress is untouched by the cut.
        assert_eq!((response.groups[0].done, response.groups[0].total), (0, 3));
        assert_eq!((response.groups[1].done, response.groups[1].total), (0, 2));
    }

    #[test]
    fn limit_drops_groups_that_end_up_empty() {
        let files = vec![
            file("/a/", "a.md", None, "- [ ] a1\n- [ ] a2\n"),
            file("/b/", "b.md", None, "- [ ] b1\n"),
        ];
        let q = TaskQuery {
            limit: 2,
            ..query()
        };
        let response = run_query(&files, &q, today());

        assert_eq!(response.groups.len(), 1, "the empty /b/ group is dropped");
        assert_eq!(response.total_matches, 3);
    }

    #[test]
    fn a_zero_limit_returns_no_tasks_but_still_counts_them() {
        let files = vec![file("/a/", "a.md", None, "- [ ] a1\n- [ ] a2\n")];
        let q = TaskQuery {
            limit: 0,
            ..query()
        };
        let response = run_query(&files, &q, today());

        assert!(response.groups.is_empty());
        assert_eq!(response.total_matches, 2);
        assert_eq!(response.folders[0].count, 2, "facets are never truncated");
    }

    // ---- shape ---------------------------------------------------------------

    #[test]
    fn an_empty_index_yields_an_empty_response() {
        let response = run_query(&[], &query(), today());
        assert!(response.groups.is_empty());
        assert!(response.folders.is_empty());
        assert_eq!(response.total_matches, 0);
        assert!(!response.scan_in_progress);
    }

    #[test]
    fn a_task_hit_serializes_its_task_fields_alongside_the_page_url() {
        let files = vec![file(
            "/notes/",
            "notes.md",
            None,
            "- [ ] ship it !!! #work @due(2026-08-05)\n",
        )];
        let response = run_query(&files, &query(), today());
        let json = serde_json::to_value(&response).expect("response serializes");
        let hit = &json["groups"][0]["tasks"][0];

        assert_eq!(hit["url_path"], "/notes/");
        assert_eq!(hit["path"], "notes.md");
        assert_eq!(hit["line"], 1);
        assert_eq!(hit["status"], "open");
        assert_eq!(hit["priority"], "urgent");
        assert_eq!(hit["text"], "ship it");
        assert_eq!(hit["tags"][0], "work");
        assert_eq!(hit["due"], "2026-08-05T00:00:00");
    }

    #[test]
    fn a_hit_carries_the_source_path_that_the_page_url_cannot_reproduce() {
        // Two files whose URLs are indistinguishable from a folder: only
        // `path` says which file `POST /.mbr/task` must open.
        let files = vec![
            file("/docs/", "docs/index.md", None, "- [ ] from the index\n"),
            file(
                "/docs/guide/",
                "docs/guide.md",
                None,
                "- [ ] from the guide\n",
            ),
        ];
        let response = run_query(&files, &query(), today());

        let located: Vec<(&str, &str)> = response
            .groups
            .iter()
            .flat_map(|g| {
                g.tasks
                    .iter()
                    .map(|t| (t.url_path.as_str(), t.path.as_str()))
            })
            .collect();
        assert_eq!(
            located,
            vec![
                ("/docs/", "docs/index.md"),
                ("/docs/guide/", "docs/guide.md"),
            ]
        );
    }

    #[test]
    fn calendar_hits_carry_the_source_path_too() {
        let files = vec![file(
            "/docs/",
            "docs/index.md",
            None,
            "- [ ] due today @due(2026-08-04)\n",
        )];
        let q = TaskQuery {
            mode: TaskMode::Calendar,
            ..query()
        };
        let response = run_query(&files, &q, today());

        assert_eq!(response.groups[0].tasks[0].path, "docs/index.md");
    }

    #[test]
    fn a_query_deserializes_from_an_empty_object() {
        let parsed: TaskQuery = serde_json::from_str("{}").expect("empty object is a query");
        assert_eq!(parsed, TaskQuery::default());
        assert_eq!(parsed.limit, DEFAULT_TASK_LIMIT);
    }

    #[test]
    fn a_query_deserializes_every_field() {
        let parsed: TaskQuery = serde_json::from_str(
            r#"{"q":"report #work","folder":"/docs/","statuses":["open","done"],
                "priorities":["urgent"],"due":"overdue","mode":"calendar","limit":10}"#,
        )
        .expect("full object is a query");

        assert_eq!(parsed.q, "report #work");
        assert_eq!(parsed.folder.as_deref(), Some("/docs/"));
        assert_eq!(parsed.statuses, [TaskStatus::Open, TaskStatus::Done]);
        assert_eq!(parsed.priorities, [TaskPriority::Urgent]);
        assert_eq!(parsed.due, DueFilter::Overdue);
        assert_eq!(parsed.mode, TaskMode::Calendar);
        assert_eq!(parsed.limit, 10);
    }

    #[test]
    fn the_no_due_filter_is_spelled_none_on_the_wire() {
        let parsed: TaskQuery = serde_json::from_str(r#"{"due":"none"}"#).expect("query");
        assert_eq!(parsed.due, DueFilter::NoDue);
    }
}
