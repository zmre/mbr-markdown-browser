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
//! | Grid     | one per placed day | `open`: open tasks placed there, everything *except* status | its `Done` ones |
//!
//! Grid mode ([`run_grid_query`], design/calendar.md §8.1) answers with a
//! different body, [`TaskGridResponse`]: days instead of groups, placed by
//! [`grid_placement`] (done date → due → scheduled), plus the undated tasks of
//! each day note (markers only) and the overdue count before the range. It
//! drops canceled tasks and markers exactly as calendar mode does.
//!
//! Canceled tasks never count toward a total in either mode. In calendar mode
//! they are dropped outright — they cannot appear, be counted, or create a
//! bucket — because a canceled task has no meaningful due date.
//!
//! Incomplete markers ([`crate::tasks::TaskKind::Marker`]) are category-mode
//! only, dropped in calendar mode by the same rule and for a stronger reason:
//! they can never carry a due date. They are real matches everywhere else —
//! `total_matches` and the folder facets count them — but never move a note's
//! `done`/`total`, which [`FileTasks`] derives from checkboxes alone.
//!
//! `limit` truncates the tasks that are *returned*; it never affects a count.
//! `total_matches` is the pre-truncation number of matching tasks.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};

use crate::task_index::FileTasks;
use crate::tasks::{Task, TaskKind, TaskPriority, TaskStatus};

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
    /// Which kinds of entry to show: checkboxes, incomplete markers, or both.
    pub include: IncludeFilter,
    /// How results are grouped.
    pub mode: TaskMode,
    /// Maximum number of tasks returned across all groups.
    pub limit: usize,
    /// First day of a [`TaskMode::Grid`] range, inclusive. Ignored by the
    /// other modes; see [`grid_range`] for the defaults and the clamp.
    pub from: Option<NaiveDate>,
    /// Last day of a [`TaskMode::Grid`] range, inclusive.
    pub to: Option<NaiveDate>,
}

impl Default for TaskQuery {
    fn default() -> Self {
        Self {
            q: String::new(),
            folder: None,
            statuses: Vec::new(),
            priorities: Vec::new(),
            due: DueFilter::Any,
            include: IncludeFilter::All,
            mode: TaskMode::Category,
            limit: DEFAULT_TASK_LIMIT,
            from: None,
            to: None,
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

/// Which kinds of entry a query wants back.
///
/// **Single-select, like [`DueFilter`], rather than an array like
/// `statuses`.** The three options are mutually exclusive and exhaust the
/// space, so an array would admit two states that mean nothing — the empty
/// array, and both values at once — and the client would need exactly the
/// "refuses to clear the last one" guard `statuses` already carries. One enum
/// with a default makes both unrepresentable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IncludeFilter {
    /// Checkbox tasks and incomplete markers.
    #[default]
    All,
    /// Checkbox tasks only.
    Tasks,
    /// Incomplete markers only.
    Markers,
}

impl IncludeFilter {
    /// Whether an entry of this kind passes the filter.
    pub fn matches(self, kind: TaskKind) -> bool {
        match self {
            Self::All => true,
            Self::Tasks => kind == TaskKind::Task,
            Self::Markers => kind == TaskKind::Marker,
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
    /// Tasks placed on the days of a date range, for the month and week grids
    /// and the companion's day markers. Answered by [`run_grid_query`] with a
    /// [`TaskGridResponse`]; [`run_query`] treats it as [`Self::Calendar`].
    Grid,
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
/// use mbr_core::task_query::{DueBucket, due_bucket};
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
/// use mbr_core::task_query::parse_task_query;
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
/// use mbr_core::task_query::normalize_folder;
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
    include: IncludeFilter,
    today: NaiveDate,
    calendar: bool,
}

impl Filters {
    fn new(query: &TaskQuery, today: NaiveDate) -> Self {
        let grid = query.mode == TaskMode::Grid;
        Self {
            terms: parse_task_query(&query.q),
            // An empty selection is the spec's default view: incomplete only.
            statuses: if query.statuses.is_empty() {
                vec![TaskStatus::Open]
            } else {
                query.statuses.clone()
            },
            priorities: query.priorities.clone(),
            // The grid places tasks by date itself, so a due filter could only
            // hide days; and `include` is pinned as calendar mode pins it.
            due: if grid { DueFilter::Any } else { query.due },
            include: if grid {
                IncludeFilter::Tasks
            } else {
                query.include
            },
            today,
            calendar: query.mode != TaskMode::Category,
        }
    }

    /// Whether an entry may take part in this query *at all* — before any
    /// question about which group it lands in or whether it matches the text.
    ///
    /// Two rules live here, and they live here rather than in
    /// [`Self::matches_except_status`] because [`group_by_due`] consults this
    /// one **before** incrementing a bucket's `total`, so anything rejected
    /// cannot leak into a count or conjure a bucket that has no visible tasks:
    ///
    /// - the `include` filter, which is a property of the entry and not of the
    ///   view;
    /// - calendar mode, which drops canceled tasks (no meaningful due date) and
    ///   incomplete markers (no due date at all, and nothing to complete). A
    ///   marker would otherwise pile into the "No due date" bucket, which is
    ///   the one calendar heading that is a place for tasks whose deadline is
    ///   still to be set.
    ///
    /// Everything else about a marker falls out of the ordinary filters without
    /// a special case: its bucket is `NoDue`, its priority is `Normal` and its
    /// status is `Open`.
    fn admissible(&self, task: &Task) -> bool {
        if !self.include.matches(task.kind) {
            return false;
        }
        if self.calendar {
            return task.kind == TaskKind::Task && task.status != TaskStatus::Canceled;
        }
        true
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

    let folders = folder_facets(files, folder_counter(&filters));
    let scoped: Vec<&Arc<FileTasks>> = files
        .iter()
        .filter(|file| in_folder(file, folder.as_deref()))
        .collect();

    let mut groups = match query.mode {
        TaskMode::Category => group_by_file(&scoped, &filters),
        TaskMode::Calendar | TaskMode::Grid => group_by_due(&scoped, &filters),
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
fn folder_facets(
    files: &[Arc<FileTasks>],
    counts_task: impl Fn(&Task) -> bool,
) -> Vec<FolderFacet> {
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();

    for file in files {
        let matched = u32::try_from(file.tasks.iter().filter(|task| counts_task(task)).count())
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

/// The list modes' folder-facet predicate: what a group would show.
fn folder_counter(filters: &Filters) -> impl Fn(&Task) -> bool + '_ {
    |task| filters.admissible(task) && filters.matches(task)
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

// ============================================================================
// Grid mode (design/calendar.md §8.1)
// ============================================================================

/// The widest range one grid request may cover, in days: a six-week month
/// grid is 42, and 62 leaves room for two calendar months.
pub const MAX_GRID_DAYS: u32 = 62;

/// Days a grid request covers when it names no `to`: one week.
const DEFAULT_GRID_DAYS: u64 = 7;

/// The inclusive range a grid request covers.
///
/// `from` defaults to `today` and `to` to a week after `from`; reversed ends
/// are swapped (as the search date grammar swaps them), and a range wider than
/// [`MAX_GRID_DAYS`] is cut to that many days from `from`. Clamped rather than
/// refused: a client asking for too much still gets a usable answer, and the
/// response echoes the range it actually covers.
///
/// # Examples
///
/// ```
/// use chrono::NaiveDate;
/// use mbr_core::task_query::grid_range;
///
/// let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
/// assert_eq!(grid_range(Some(d(10, 31)), Some(d(10, 1)), d(1, 1)), (d(10, 1), d(10, 31)));
/// assert_eq!(grid_range(Some(d(1, 1)), Some(d(12, 31)), d(1, 1)), (d(1, 1), d(3, 3)));
/// assert_eq!(grid_range(None, None, d(5, 4)), (d(5, 4), d(5, 10)));
/// ```
pub fn grid_range(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    today: NaiveDate,
) -> (NaiveDate, NaiveDate) {
    let start = from.unwrap_or(today);
    let end = to.unwrap_or_else(|| {
        start
            .checked_add_days(chrono::Days::new(DEFAULT_GRID_DAYS - 1))
            .unwrap_or(start)
    });
    let (start, end) = if end < start {
        (end, start)
    } else {
        (start, end)
    };
    let last = start
        .checked_add_days(chrono::Days::new(u64::from(MAX_GRID_DAYS - 1)))
        .unwrap_or(NaiveDate::MAX);
    (start, end.min(last))
}

/// Which of a task's dates put it on a grid day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Placement {
    /// A completed task on its `@done` / `✅` / `[completion::]` date.
    Done,
    /// Its due date.
    Due,
    /// Its scheduled date, when it has no due date.
    Scheduled,
}

/// The day a task is drawn on in the grids, and why — `None` for a task that
/// is not drawn at all.
///
/// In order: a **completed** task with a done date goes on that date; else
/// the due date; else the scheduled date. A canceled task is never placed,
/// and neither is a task with none of the three dates (D9: undated tasks in a
/// day note count toward that day's markers only, see
/// [`TaskGridResponse::undated_in_day_notes`]).
///
/// The done date counts only for a task whose checkbox says done: an open
/// task carrying a stale stamp (the box unticked by hand) is still pending,
/// and placing it on a past day would draw it as finished work.
pub fn grid_placement(task: &Task) -> Option<(NaiveDateTime, bool, Placement)> {
    match task.status {
        TaskStatus::Canceled => None,
        TaskStatus::Done if task.done.is_some() => task
            .done
            .map(|at| (at, task.done_has_time, Placement::Done)),
        TaskStatus::Open | TaskStatus::Done => task
            .due
            .map(|at| (at, task.due_has_time, Placement::Due))
            .or_else(|| {
                task.scheduled
                    .map(|at| (at, task.scheduled_has_time, Placement::Scheduled))
            }),
    }
}

/// A time of day at the very start of a task's text (`09:30 standup`,
/// `9:30am call`, `9.30 pm dinner`), in minutes after midnight.
///
/// A sort key and label for the grid's day lists, nothing more: the agenda
/// grammar proper lives in `components/src/calendar/agenda-parse.ts`. This
/// reads `H:MM` / `HH:MM` with an optional am/pm, and the `.` separator only
/// with am/pm, so a leading version number (`1.25 release`) is not a time.
fn leading_minutes(text: &str) -> Option<u32> {
    let bytes = text.as_bytes();
    let digits = bytes
        .iter()
        .take(2)
        .take_while(|b| b.is_ascii_digit())
        .count();
    let separator = *bytes.get(digits)?;
    let minutes = bytes.get(digits + 1..digits + 3)?;
    if digits == 0
        || !matches!(separator, b':' | b'.')
        || !minutes.iter().all(u8::is_ascii_digit)
        || bytes.get(digits + 3).is_some_and(u8::is_ascii_digit)
    {
        return None;
    }
    let hour: u32 = text[..digits].parse().ok()?;
    let minute: u32 = text[digits + 1..digits + 3].parse().ok()?;
    let rest = text[digits + 3..].trim_start().to_ascii_lowercase();
    let meridiem = ["am", "a.m"]
        .iter()
        .any(|m| rest.starts_with(m))
        .then_some(false)
        .or_else(|| {
            ["pm", "p.m"]
                .iter()
                .any(|m| rest.starts_with(m))
                .then_some(true)
        });
    let hour = match meridiem {
        None if separator == b'.' => return None,
        None => hour,
        Some(_) if hour == 0 || hour > 12 => return None,
        Some(pm) => hour % 12 + if pm { 12 } else { 0 },
    };
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}

/// One task on a grid day: the [`TaskHit`] plus how the grid reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GridTaskHit {
    /// Every [`TaskHit`] field, top-level, exactly as the list modes send it.
    #[serde(flatten)]
    pub hit: TaskHit,
    /// The date that put the task on this day.
    pub placed_by: Placement,
    /// `HH:MM` when the placing date carried a time or the text starts with
    /// one; the day list's time order, and the label the grid draws.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    /// The note's display title, for the grid's hover text.
    pub note: String,
}

/// One day of a [`TaskGridResponse`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GridDay {
    /// The day's tasks that pass every filter, ordered: open before done, then
    /// by time of day (timed first), then by priority (urgent first), then by
    /// page and line. Truncated by `limit`; the counts are not.
    pub tasks: Vec<GridTaskHit>,
    /// Open tasks placed on the day, by every filter except status.
    pub open: u32,
    /// Done tasks placed on the day, by every filter except status.
    pub done: u32,
    /// The day is before today and still has open tasks.
    pub overdue: bool,
}

/// The `/.mbr/tasks` body for `mode: "grid"`.
///
/// ```json
/// { "from": "2026-10-27", "to": "2026-11-02",
///   "days": { "2026-10-31": { "tasks": [ … ], "open": 3, "done": 2, "overdue": true } },
///   "undated_in_day_notes": { "2026-10-30": 4 },
///   "overdue_before_range": 12,
///   "folders": [ … ], "total_matches": 41, "duration_ms": 3, "scan_in_progress": false }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskGridResponse {
    /// The range covered, after [`grid_range`]'s defaults and clamp.
    pub from: NaiveDate,
    /// Last day covered, inclusive.
    pub to: NaiveDate,
    /// Days with at least one placed task (open or done), keyed `YYYY-MM-DD`.
    pub days: BTreeMap<String, GridDay>,
    /// Open tasks with **no** date at all in each day note of the range
    /// (D9: day markers only — they are never placed in `days`).
    pub undated_in_day_notes: BTreeMap<String, u32>,
    /// Open tasks due before the range and before today, for the
    /// "Overdue (N)" chip that switches the panel to the overdue list.
    pub overdue_before_range: u32,
    /// Folder facets counting the tasks the grid would show.
    pub folders: Vec<FolderFacet>,
    /// Placed tasks matching every filter, before `limit`.
    pub total_matches: usize,
    /// Server-side query time.
    pub duration_ms: u64,
    /// Set by the handler, as for [`TaskQueryResponse`].
    pub scan_in_progress: bool,
}

/// Grid placement of `task` when it falls inside `from..=to`.
fn placed_in(
    task: &Task,
    from: NaiveDate,
    to: NaiveDate,
) -> Option<(NaiveDateTime, bool, Placement)> {
    grid_placement(task).filter(|(at, _, _)| (from..=to).contains(&at.date()))
}

/// Runs a [`TaskMode::Grid`] query.
///
/// `day_notes` maps the `url_path` of each **day note** in the range to its
/// date (the server reads it off the periodic index); it is consulted only for
/// [`TaskGridResponse::undated_in_day_notes`]. Pure like [`run_query`]: `today`
/// is a parameter and `scan_in_progress` is left for the handler.
///
/// Filters: `q`, `folder`, `statuses` and `priorities` apply; `include` is
/// pinned to tasks and `due` is ignored (see [`Filters::new`]). As in calendar
/// mode, `open`/`done` count every filter except status, so a day's marker
/// does not vanish when the view narrows to completed tasks.
pub fn run_grid_query(
    files: &[Arc<FileTasks>],
    query: &TaskQuery,
    today: NaiveDate,
    day_notes: &HashMap<String, NaiveDate>,
) -> TaskGridResponse {
    let start = std::time::Instant::now();
    let filters = Filters::new(query, today);
    let (from, to) = grid_range(query.from, query.to, today);
    let folder = normalize_folder(query.folder.as_deref());
    // Overdue before the range is "overdue" only up to today: a range in the
    // future must not call next week's deadlines late.
    let overdue_cutoff = from.min(today);

    let folders = folder_facets(files, |task| {
        filters.admissible(task) && filters.matches(task) && placed_in(task, from, to).is_some()
    });

    let mut days: BTreeMap<NaiveDate, GridDay> = BTreeMap::new();
    let mut undated: BTreeMap<NaiveDate, u32> = BTreeMap::new();
    let mut overdue_before_range = 0u32;

    for file in files
        .iter()
        .filter(|file| in_folder(file, folder.as_deref()))
    {
        let path = crate::url_path::path_to_url(&file.raw_path);
        let day_note = day_notes.get(&file.url_path).copied();
        for task in &file.tasks {
            if !filters.admissible(task) || !filters.matches_except_status(task) {
                continue;
            }
            let open = task.status == TaskStatus::Open;
            if open && task.due.is_some_and(|due| due.date() < overdue_cutoff) {
                overdue_before_range = overdue_before_range.saturating_add(1);
            }
            let Some((at, has_time, placed_by)) = placed_in(task, from, to) else {
                if let Some(date) = day_note.filter(|_| {
                    open && task.due.is_none() && task.scheduled.is_none() && task.done.is_none()
                }) {
                    *undated.entry(date).or_insert(0) += 1;
                }
                continue;
            };
            let day = days.entry(at.date()).or_insert_with(|| GridDay {
                tasks: Vec::new(),
                open: 0,
                done: 0,
                overdue: false,
            });
            if open {
                day.open = day.open.saturating_add(1);
            } else {
                day.done = day.done.saturating_add(1);
            }
            if filters.matches_status(task) {
                let minutes = if has_time {
                    Some(at.hour() * 60 + at.minute())
                } else {
                    leading_minutes(&task.text)
                };
                day.tasks.push(GridTaskHit {
                    hit: TaskHit {
                        task: task.clone(),
                        url_path: file.url_path.clone(),
                        path: path.clone(),
                    },
                    placed_by,
                    time: minutes.map(|m| format!("{:02}:{:02}", m / 60, m % 60)),
                    note: file.display_title().to_string(),
                });
            }
        }
    }

    let mut remaining = query.limit;
    let mut total_matches = 0usize;
    let days = days
        .into_iter()
        .map(|(date, mut day)| {
            day.overdue = date < today && day.open > 0;
            day.tasks.sort_by(grid_order);
            total_matches += day.tasks.len();
            day.tasks.truncate(remaining);
            remaining -= day.tasks.len();
            (date.to_string(), day)
        })
        .collect();

    TaskGridResponse {
        from,
        to,
        days,
        undated_in_day_notes: undated
            .into_iter()
            .map(|(date, count)| (date.to_string(), count))
            .collect(),
        overdue_before_range,
        folders,
        total_matches,
        duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        scan_in_progress: false,
    }
}

/// A grid day's display order; see [`GridDay::tasks`].
fn grid_order(a: &GridTaskHit, b: &GridTaskHit) -> std::cmp::Ordering {
    let rank = |hit: &GridTaskHit| u8::from(hit.hit.task.status != TaskStatus::Open);
    // `None` sorts after every time: untimed tasks follow the timed ones.
    let time = |hit: &GridTaskHit| (hit.time.is_none(), hit.time.clone());
    rank(a)
        .cmp(&rank(b))
        .then_with(|| time(a).cmp(&time(b)))
        .then_with(|| b.hit.task.priority.cmp(&a.hit.task.priority))
        .then_with(|| a.hit.url_path.cmp(&b.hit.url_path))
        .then_with(|| a.hit.task.line.cmp(&b.hit.task.line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::{MarkerRule, scan_source_tasks, scan_source_tasks_with_markers};

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

    /// [`file`], with the default `incomplete_markers` turned on, so the entries
    /// come back merged and in source order exactly as the index stores them.
    fn marked_file(url: &str, raw: &str, body: &str) -> Arc<FileTasks> {
        let markers: Vec<String> = ["TK", "TODO", "FIXME", "XXX"]
            .iter()
            .map(|m| (*m).to_string())
            .collect();
        let rule = MarkerRule::new(&markers).expect("the default markers compile");
        Arc::new(FileTasks::new(
            url,
            std::path::PathBuf::from(raw),
            None,
            scan_source_tasks_with_markers(body, Some(&rule)),
        ))
    }

    /// Every returned entry's kind, in response order.
    fn kinds(response: &TaskQueryResponse) -> Vec<TaskKind> {
        response
            .groups
            .iter()
            .flat_map(|g| g.tasks.iter().map(|t| t.task.kind))
            .collect()
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

    /// Due dates written in another tool's notation bucket exactly like
    /// `@due(...)`, and a scheduled date never moves a task between buckets.
    #[test]
    fn due_filter_reads_every_due_notation_and_ignores_scheduled() {
        let files = vec![file(
            "/notes/",
            "notes.md",
            None,
            concat!(
                "- [ ] emoji late 📅 2026-08-01\n",
                "- [ ] dataview today [due:: 2026-08-04]\n",
                "- [ ] org tomorrow\n",
                "  DEADLINE: <2026-08-05 Wed>\n",
                "- [ ] scheduled only ⏳ 2026-08-01\n",
                "- [ ] org scheduled only\n",
                "  SCHEDULED: <2026-08-04 Tue>\n",
            ),
        )];

        for (filter, expected) in [
            (DueFilter::Overdue, vec!["emoji late"]),
            (DueFilter::Today, vec!["dataview today"]),
            (DueFilter::Tomorrow, vec!["org tomorrow"]),
            (
                DueFilter::NoDue,
                vec!["scheduled only", "org scheduled only"],
            ),
        ] {
            let q = TaskQuery {
                due: filter,
                ..query()
            };
            let response = run_query(&files, &q, today());
            assert_eq!(texts(&response), expected, "for {filter:?}");
        }

        let calendar = run_query(&files, &calendar_query(), today());
        let none = calendar
            .groups
            .iter()
            .find(|g| g.key == "none")
            .expect("a no-due bucket");
        assert_eq!(
            none.tasks
                .iter()
                .map(|t| t.task.text.as_str())
                .collect::<Vec<_>>(),
            ["scheduled only", "org scheduled only"]
        );
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
        assert_eq!(hit["scheduled"], serde_json::Value::Null);
        assert_eq!(hit["scheduled_has_time"], false);
    }

    #[test]
    fn a_task_hit_serializes_its_scheduled_date() {
        let files = vec![file(
            "/notes/",
            "notes.md",
            None,
            "- [ ] plan it [scheduled:: 2026-08-06T09:30]\n",
        )];
        let response = run_query(&files, &query(), today());
        let json = serde_json::to_value(&response).expect("response serializes");
        let hit = &json["groups"][0]["tasks"][0];

        assert_eq!(hit["text"], "plan it");
        assert_eq!(hit["scheduled"], "2026-08-06T09:30:00");
        assert_eq!(hit["scheduled_has_time"], true);
        assert_eq!(hit["due"], serde_json::Value::Null);
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
        // The container-level `#[serde(default)]` reads this off the manual
        // `Default` impl, so a client that predates the field still gets both
        // kinds rather than an empty panel.
        assert_eq!(parsed.include, IncludeFilter::All);
    }

    #[test]
    fn a_query_deserializes_every_field() {
        let parsed: TaskQuery = serde_json::from_str(
            r#"{"q":"report #work","folder":"/docs/","statuses":["open","done"],
                "priorities":["urgent"],"due":"overdue","include":"markers",
                "mode":"calendar","limit":10}"#,
        )
        .expect("full object is a query");

        assert_eq!(parsed.q, "report #work");
        assert_eq!(parsed.folder.as_deref(), Some("/docs/"));
        assert_eq!(parsed.statuses, [TaskStatus::Open, TaskStatus::Done]);
        assert_eq!(parsed.priorities, [TaskPriority::Urgent]);
        assert_eq!(parsed.due, DueFilter::Overdue);
        assert_eq!(parsed.include, IncludeFilter::Markers);
        assert_eq!(parsed.mode, TaskMode::Calendar);
        assert_eq!(parsed.limit, 10);

        // Single-select, lowercase on the wire, like `due` and `mode`.
        let tasks: TaskQuery =
            serde_json::from_str(r#"{"include":"tasks"}"#).expect("include is a query field");
        assert_eq!(tasks.include, IncludeFilter::Tasks);
    }

    #[test]
    fn the_no_due_filter_is_spelled_none_on_the_wire() {
        let parsed: TaskQuery = serde_json::from_str(r#"{"due":"none"}"#).expect("query");
        assert_eq!(parsed.due, DueFilter::NoDue);
    }

    // ---- incomplete markers --------------------------------------------------

    /// One file carrying both kinds, interleaved, so source order is testable.
    fn mixed_kind_file() -> Vec<Arc<FileTasks>> {
        vec![marked_file(
            "/notes/",
            "notes.md",
            concat!(
                "TK verify this figure\n",   // 1, marker
                "\n",                        // 2
                "- [ ] write the report\n",  // 3, task
                "prose with a FIXME here\n"  // 4, marker
            ),
        )]
    }

    #[test]
    fn markers_appear_in_category_mode_in_source_order() {
        let response = run_query(&mixed_kind_file(), &query(), today());
        assert_eq!(
            texts(&response),
            [
                "TK verify this figure",
                "write the report",
                "prose with a FIXME here"
            ]
        );
        assert_eq!(
            kinds(&response),
            [TaskKind::Marker, TaskKind::Task, TaskKind::Marker]
        );
        assert_eq!(response.total_matches, 3, "markers are real matches");
    }

    #[test]
    fn markers_never_appear_in_calendar_mode() {
        let q = TaskQuery {
            mode: TaskMode::Calendar,
            ..query()
        };
        let response = run_query(&mixed_kind_file(), &q, today());

        assert_eq!(texts(&response), ["write the report"]);
        // The one bucket a marker could have landed in is the undated one, and
        // `admissible` runs before the bucket total is incremented — so the
        // markers neither create it nor inflate it.
        let none = response
            .groups
            .iter()
            .find(|g| g.key == "none")
            .expect("the undated task still makes a bucket");
        assert_eq!((none.done, none.total), (0, 1));
        assert_eq!(response.total_matches, 1);

        // And with nothing but markers there is no bucket at all.
        let markers_only = vec![marked_file("/prose/", "prose.md", "the source is TK\n")];
        let response = run_query(&markers_only, &q, today());
        assert!(
            response.groups.is_empty(),
            "a marker must not conjure a calendar bucket: {:?}",
            response.groups
        );
        assert_eq!(response.total_matches, 0);
    }

    #[test]
    fn include_filter_selects_tasks_markers_or_both() {
        let files = mixed_kind_file();
        for (include, expected) in [
            (
                IncludeFilter::All,
                vec![TaskKind::Marker, TaskKind::Task, TaskKind::Marker],
            ),
            (IncludeFilter::Tasks, vec![TaskKind::Task]),
            (IncludeFilter::Markers, vec![TaskKind::Marker; 2]),
        ] {
            let q = TaskQuery { include, ..query() };
            let response = run_query(&files, &q, today());
            assert_eq!(kinds(&response), expected, "for {include:?}");
            assert_eq!(response.total_matches, expected.len(), "for {include:?}");
        }
    }

    #[test]
    fn markers_pass_the_default_status_and_priority_filters() {
        // Nothing about a marker is special-cased in `matches_except_status` or
        // `matches_status`: it is `Open` and `Normal`, so the default view shows
        // it and an explicit filter for those values still does.
        let files = mixed_kind_file();
        for q in [
            TaskQuery {
                statuses: vec![TaskStatus::Open],
                ..query()
            },
            TaskQuery {
                priorities: vec![TaskPriority::Normal],
                ..query()
            },
        ] {
            assert_eq!(run_query(&files, &q, today()).total_matches, 3);
        }

        // ...and a filter they cannot satisfy drops them, with no extra rule.
        let done_only = TaskQuery {
            statuses: vec![TaskStatus::Done],
            ..query()
        };
        assert_eq!(run_query(&files, &done_only, today()).total_matches, 0);
    }

    #[test]
    fn markers_are_dropped_by_a_dated_due_filter_but_kept_by_none() {
        let files = mixed_kind_file();

        // A marker has no due date, so `due_bucket(None)` is `NoDue` and every
        // dated filter excludes it without a kind check.
        for due in [DueFilter::Overdue, DueFilter::Today, DueFilter::Upcoming] {
            let q = TaskQuery { due, ..query() };
            assert_eq!(
                run_query(&files, &q, today()).total_matches,
                0,
                "for {due:?}"
            );
        }

        let undated = TaskQuery {
            due: DueFilter::NoDue,
            ..query()
        };
        assert_eq!(run_query(&files, &undated, today()).total_matches, 3);
    }

    #[test]
    fn a_word_query_matches_marker_text_but_a_tag_query_never_does() {
        // A marker's text is the whole line, so a bare word searches it the way
        // it searches a task.
        let files = vec![marked_file(
            "/notes/",
            "notes.md",
            "TODO cross-link this #docs\n",
        )];
        let word = TaskQuery {
            q: "cross-link".to_string(),
            ..query()
        };
        assert_eq!(run_query(&files, &word, today()).total_matches, 1);

        // But a `#tag` token matches *tags*, and a marker has none — even when
        // the line literally contains `#docs`. That is intentional: markers get
        // no annotation parsing, so `#docs` here is prose the author wrote, not
        // a tag they applied, and letting it match would file the entry under a
        // label that exists nowhere else.
        let tag = TaskQuery {
            q: "#docs".to_string(),
            ..query()
        };
        assert_eq!(run_query(&files, &tag, today()).total_matches, 0);
    }

    #[test]
    fn folder_facets_count_markers() {
        // Markers are real matches, so the folder pane counts them and its
        // numbers keep agreeing with what selecting a folder would show.
        let files = vec![
            marked_file("/docs/notes/", "docs/notes.md", "the source is TK\n"),
            marked_file("/inbox/", "inbox.md", "- [ ] write the report\n"),
        ];
        let response = run_query(&files, &query(), today());
        let counts: Vec<(&str, u32)> = response
            .folders
            .iter()
            .map(|f| (f.path.as_str(), f.count))
            .collect();
        assert_eq!(counts, vec![("/", 2), ("/docs/", 1)]);

        // Narrowing to tasks only takes the marker back out of the facet.
        let tasks_only = TaskQuery {
            include: IncludeFilter::Tasks,
            ..query()
        };
        let response = run_query(&files, &tasks_only, today());
        assert_eq!(
            response
                .folders
                .iter()
                .map(|f| (f.path.as_str(), f.count))
                .collect::<Vec<_>>(),
            vec![("/", 1)]
        );
    }

    #[test]
    fn a_group_of_only_markers_reports_zero_of_zero() {
        // The progress bar describes checkboxes, and this file has none — so
        // the heading reads 0/0 rather than a total nobody can ever complete.
        let files = vec![marked_file(
            "/prose/",
            "prose.md",
            "The market fell 10% (source: TK).\nand FIXME that too\n",
        )];
        let response = run_query(&files, &query(), today());
        let group = &response.groups[0];
        assert_eq!((group.done, group.total), (0, 0));
        assert_eq!(group.tasks.len(), 2);
    }

    // ---- grid mode ------------------------------------------------------------

    fn grid_query(from: NaiveDate, to: NaiveDate) -> TaskQuery {
        TaskQuery {
            mode: TaskMode::Grid,
            from: Some(from),
            to: Some(to),
            statuses: vec![TaskStatus::Open, TaskStatus::Done],
            ..query()
        }
    }

    /// The week of 2026-08-03 (Mon) .. 2026-08-09; today is the 4th.
    fn grid_week() -> TaskQuery {
        grid_query(day(2026, 8, 3), day(2026, 8, 9))
    }

    fn no_day_notes() -> HashMap<String, NaiveDate> {
        HashMap::new()
    }

    /// `(date, text)` of every placed task, in response order.
    fn placed(response: &TaskGridResponse) -> Vec<(String, String)> {
        response
            .days
            .iter()
            .flat_map(|(date, d)| {
                d.tasks
                    .iter()
                    .map(move |t| (date.clone(), t.hit.task.text.clone()))
            })
            .collect()
    }

    #[test]
    fn grid_range_defaults_swaps_and_clamps() {
        let today = today();
        assert_eq!(grid_range(None, None, today), (today, day(2026, 8, 10)));
        let from = day(2026, 9, 1);
        assert_eq!(grid_range(Some(from), None, today), (from, day(2026, 9, 7)));
        assert_eq!(
            grid_range(Some(day(2026, 8, 31)), Some(day(2026, 8, 1)), today),
            (day(2026, 8, 1), day(2026, 8, 31)),
            "reversed ends are swapped"
        );
        let (start, end) = grid_range(Some(day(2026, 1, 1)), Some(day(2026, 12, 31)), today);
        assert_eq!(start, day(2026, 1, 1));
        assert_eq!(
            (end - start).num_days() + 1,
            i64::from(MAX_GRID_DAYS),
            "at most 62 days"
        );
        // A 42-day month grid is untouched.
        let six_weeks = day(2026, 9, 28) + chrono::Days::new(41);
        assert_eq!(
            grid_range(Some(day(2026, 9, 28)), Some(six_weeks), today),
            (day(2026, 9, 28), six_weeks)
        );
        // No overflow at the end of chrono's range.
        assert_eq!(
            grid_range(Some(NaiveDate::MAX), None, today),
            (NaiveDate::MAX, NaiveDate::MAX)
        );
    }

    #[test]
    fn grid_echoes_the_clamped_range() {
        let q = grid_query(day(2026, 12, 31), day(2026, 1, 1));
        let response = run_grid_query(&[], &q, today(), &no_day_notes());
        assert_eq!(response.from, day(2026, 1, 1));
        assert_eq!(response.to, day(2026, 3, 3));
    }

    #[test]
    fn grid_places_done_over_due_over_scheduled() {
        let files = vec![file(
            "/n/",
            "n.md",
            None,
            "- [x] finished @due(2026-08-03) @done(2026-08-05)\n\
             - [x] done undated stamp @due(2026-08-06)\n\
             - [ ] due wins @due(2026-08-07) ⏳ 2026-08-08\n\
             - [ ] only scheduled ⏳ 2026-08-08\n\
             - [ ] stale stamp @due(2026-08-09) @done(2026-08-04)\n\
             - [-] canceled @due(2026-08-05)\n\
             - [ ] no dates at all\n",
        )];
        let response = run_grid_query(&files, &grid_week(), today(), &no_day_notes());
        assert_eq!(
            placed(&response),
            [
                ("2026-08-05".into(), "finished".into()),
                ("2026-08-06".into(), "done undated stamp".into()),
                ("2026-08-07".into(), "due wins".into()),
                ("2026-08-08".into(), "only scheduled".into()),
                ("2026-08-09".into(), "stale stamp".into()),
            ]
        );
        let by = |date: &str| response.days[date].tasks[0].placed_by;
        assert_eq!(by("2026-08-05"), Placement::Done);
        assert_eq!(by("2026-08-06"), Placement::Due);
        assert_eq!(by("2026-08-08"), Placement::Scheduled);
        assert_eq!(
            by("2026-08-09"),
            Placement::Due,
            "an open task's stale stamp is ignored"
        );
        assert_eq!(response.total_matches, 5);
    }

    #[test]
    fn grid_reads_every_date_notation() {
        let files = vec![file(
            "/n/",
            "n.md",
            None,
            "- [ ] emoji 📅 2026-08-03\n\
             - [ ] dataview [due:: 2026-08-04]\n\
             - [x] completion [completion:: 2026-08-05]\n\
             - [ ] org\n  SCHEDULED: <2026-08-06 Thu>\n",
        )];
        let response = run_grid_query(&files, &grid_week(), today(), &no_day_notes());
        let dates: Vec<String> = placed(&response).into_iter().map(|(d, _)| d).collect();
        assert_eq!(
            dates,
            ["2026-08-03", "2026-08-04", "2026-08-05", "2026-08-06"]
        );
    }

    #[test]
    fn grid_leaves_out_tasks_outside_the_range() {
        let files = vec![file(
            "/n/",
            "n.md",
            None,
            "- [ ] before @due(2026-08-02)\n- [ ] inside @due(2026-08-09)\n- [ ] after @due(2026-08-10)\n",
        )];
        let response = run_grid_query(&files, &grid_week(), today(), &no_day_notes());
        assert_eq!(placed(&response), [("2026-08-09".into(), "inside".into())]);
    }

    #[test]
    fn undated_tasks_count_only_in_day_notes_and_are_never_placed() {
        let files = vec![
            file(
                "/daily/2026-08-05/",
                "daily/2026-08-05.md",
                None,
                "- [ ] undated one\n- [ ] undated two\n- [x] undated done\n\
                 - [-] undated canceled\n- [ ] dated @due(2026-08-20)\n",
            ),
            file("/notes/", "notes.md", None, "- [ ] undated elsewhere\n"),
        ];
        let day_notes = HashMap::from([("/daily/2026-08-05/".to_string(), day(2026, 8, 5))]);
        let response = run_grid_query(&files, &grid_week(), today(), &day_notes);
        assert!(
            response.days.is_empty(),
            "D9: undated tasks are not plotted"
        );
        assert_eq!(
            response.undated_in_day_notes,
            BTreeMap::from([("2026-08-05".to_string(), 2)]),
            "open, undated, in the day note only"
        );
        assert_eq!(response.total_matches, 0);
    }

    #[test]
    fn undated_counts_honor_the_text_and_folder_filters() {
        let files = vec![file(
            "/daily/2026-08-05/",
            "daily/2026-08-05.md",
            None,
            "- [ ] call mum #home\n- [ ] write report\n",
        )];
        let day_notes = HashMap::from([("/daily/2026-08-05/".to_string(), day(2026, 8, 5))]);
        let q = TaskQuery {
            q: "#home".into(),
            ..grid_week()
        };
        let response = run_grid_query(&files, &q, today(), &day_notes);
        assert_eq!(response.undated_in_day_notes["2026-08-05"], 1);
        let q = TaskQuery {
            folder: Some("/elsewhere/".into()),
            ..grid_week()
        };
        assert!(
            run_grid_query(&files, &q, today(), &day_notes)
                .undated_in_day_notes
                .is_empty()
        );
    }

    #[test]
    fn grid_orders_open_before_done_then_time_then_priority() {
        let files = vec![file(
            "/n/",
            "n.md",
            None,
            "- [x] done first in source @due(2026-08-05)\n\
             - [ ] plain normal @due(2026-08-05)\n\
             - [ ] urgent !!! @due(2026-08-05)\n\
             - [ ] 14:00 afternoon @due(2026-08-05)\n\
             - [ ] morning @due(2026-08-05 09:30)\n\
             - [ ] high !! @due(2026-08-05)\n",
        )];
        let response = run_grid_query(&files, &grid_week(), today(), &no_day_notes());
        let day = &response.days["2026-08-05"];
        let texts: Vec<&str> = day.tasks.iter().map(|t| t.hit.task.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "morning",
                "14:00 afternoon",
                "urgent",
                "high",
                "plain normal",
                "done first in source"
            ]
        );
        assert_eq!(day.tasks[0].time.as_deref(), Some("09:30"));
        assert_eq!(day.tasks[1].time.as_deref(), Some("14:00"));
        assert_eq!(day.tasks[2].time, None);
    }

    #[test]
    fn grid_day_counts_ignore_status_but_the_list_honors_it() {
        let files = vec![file(
            "/n/",
            "n.md",
            None,
            "- [ ] open a @due(2026-08-05)\n- [ ] open b @due(2026-08-05)\n- [x] done @due(2026-08-05)\n\
             - [x] only done @due(2026-08-06)\n",
        )];
        let q = TaskQuery {
            statuses: vec![TaskStatus::Open],
            ..grid_week()
        };
        let response = run_grid_query(&files, &q, today(), &no_day_notes());
        let fifth = &response.days["2026-08-05"];
        assert_eq!((fifth.open, fifth.done, fifth.tasks.len()), (2, 1, 2));
        // A day with only filtered-out tasks still reports its counts.
        let sixth = &response.days["2026-08-06"];
        assert_eq!((sixth.open, sixth.done, sixth.tasks.len()), (0, 1, 0));
        assert_eq!(response.total_matches, 2);
    }

    #[test]
    fn grid_day_is_overdue_only_in_the_past_with_open_tasks() {
        let files = vec![file(
            "/n/",
            "n.md",
            None,
            "- [ ] late @due(2026-08-03)\n- [x] done late @due(2026-08-03)\n\
             - [x] all done @due(2026-08-02)\n- [ ] today @due(2026-08-04)\n",
        )];
        let q = grid_query(day(2026, 8, 1), day(2026, 8, 7));
        let response = run_grid_query(&files, &q, today(), &no_day_notes());
        assert!(response.days["2026-08-03"].overdue);
        assert!(!response.days["2026-08-02"].overdue, "nothing open");
        assert!(!response.days["2026-08-04"].overdue, "today is not overdue");
    }

    #[test]
    fn overdue_before_range_counts_open_tasks_due_before_the_range_and_today() {
        let files = vec![file(
            "/n/",
            "n.md",
            None,
            "- [ ] long late @due(2026-07-01)\n- [ ] late !! @due(2026-08-02)\n\
             - [x] done late @due(2026-07-01)\n- [-] canceled late @due(2026-07-01)\n\
             - [ ] scheduled only ⏳ 2026-07-01\n- [ ] in range @due(2026-08-03)\n",
        )];
        let response = run_grid_query(&files, &grid_week(), today(), &no_day_notes());
        assert_eq!(response.overdue_before_range, 2);
        let q = TaskQuery {
            priorities: vec![TaskPriority::High],
            ..grid_week()
        };
        assert_eq!(
            run_grid_query(&files, &q, today(), &no_day_notes()).overdue_before_range,
            1,
            "the other filters apply"
        );
        // A range in the future counts what is late *today* (the 3rd's task
        // now too), never a task due between today and the range.
        let files = vec![file(
            "/n/",
            "n.md",
            None,
            "- [ ] late @due(2026-08-03)\n- [ ] not yet @due(2026-08-20)\n",
        )];
        let next_month = grid_query(day(2026, 9, 1), day(2026, 9, 30));
        let response = run_grid_query(&files, &next_month, today(), &no_day_notes());
        assert_eq!(
            response.overdue_before_range, 1,
            "due before today, not before `from`"
        );
    }

    #[test]
    fn grid_ignores_include_and_due_and_drops_markers() {
        let files = vec![marked_file(
            "/n/",
            "n.md",
            "TODO: a marker\n\n- [ ] task @due(2026-08-05)\n",
        )];
        let q = TaskQuery {
            include: IncludeFilter::Markers,
            due: DueFilter::NoDue,
            ..grid_week()
        };
        let response = run_grid_query(&files, &q, today(), &no_day_notes());
        assert_eq!(placed(&response), [("2026-08-05".into(), "task".into())]);
    }

    #[test]
    fn grid_limit_truncates_tasks_but_not_counts() {
        let files = vec![file(
            "/n/",
            "n.md",
            None,
            "- [ ] a @due(2026-08-03)\n- [ ] b @due(2026-08-03)\n- [ ] c @due(2026-08-05)\n",
        )];
        let q = TaskQuery {
            limit: 1,
            ..grid_week()
        };
        let response = run_grid_query(&files, &q, today(), &no_day_notes());
        assert_eq!(response.total_matches, 3);
        assert_eq!(response.days["2026-08-03"].tasks.len(), 1);
        assert_eq!(response.days["2026-08-03"].open, 2);
        assert!(response.days["2026-08-05"].tasks.is_empty());
        assert_eq!(response.days["2026-08-05"].open, 1);
    }

    #[test]
    fn grid_folder_scope_and_facets() {
        let files = vec![
            file("/a/x/", "a/x.md", None, "- [ ] in a @due(2026-08-05)\n"),
            file(
                "/b/y/",
                "b/y.md",
                None,
                "- [ ] in b @due(2026-08-05)\n- [ ] b late @due(2026-01-01)\n",
            ),
        ];
        let q = TaskQuery {
            folder: Some("/a/".into()),
            ..grid_week()
        };
        let response = run_grid_query(&files, &q, today(), &no_day_notes());
        assert_eq!(placed(&response), [("2026-08-05".into(), "in a".into())]);
        assert_eq!(
            response.overdue_before_range, 0,
            "scoped like everything else"
        );
        let facets: Vec<(&str, u32)> = response
            .folders
            .iter()
            .map(|f| (f.path.as_str(), f.count))
            .collect();
        // Facets ignore the folder filter and count what the grid would show.
        assert_eq!(facets, [("/", 2), ("/a/", 1), ("/b/", 1)]);
    }

    #[test]
    fn grid_hits_carry_the_note_title_and_source_path() {
        let files = vec![file(
            "/docs/",
            "docs/index.md",
            Some("Docs Home"),
            "- [ ] t @due(2026-08-05)\n",
        )];
        let response = run_grid_query(&files, &grid_week(), today(), &no_day_notes());
        let hit = &response.days["2026-08-05"].tasks[0];
        assert_eq!(hit.note, "Docs Home");
        assert_eq!(hit.hit.path, "docs/index.md");
    }

    #[test]
    fn grid_response_wire_format() {
        let files = vec![file(
            "/daily/2026-08-05/",
            "daily/2026-08-05.md",
            None,
            "- [ ] 09:00 call @due(2026-08-05)\n- [ ] undated\n",
        )];
        let day_notes = HashMap::from([("/daily/2026-08-05/".to_string(), day(2026, 8, 5))]);
        let response = run_grid_query(&files, &grid_week(), today(), &day_notes);
        let mut json = serde_json::to_value(&response).expect("serializes");
        json["duration_ms"] = serde_json::json!(0);
        let hit = &json["days"]["2026-08-05"]["tasks"][0];
        assert_eq!(hit["placed_by"], "due");
        assert_eq!(hit["time"], "09:00");
        assert_eq!(hit["note"], "2026-08-05");
        assert_eq!(hit["path"], "daily/2026-08-05.md");
        assert_eq!(hit["url_path"], "/daily/2026-08-05/");
        assert_eq!(hit["line"], 1, "TaskHit fields stay top-level");
        assert_eq!(hit["status"], "open");
        let mut day = json["days"]["2026-08-05"].clone();
        day["tasks"] = serde_json::json!([]);
        assert_eq!(
            day,
            serde_json::json!({"tasks": [], "open": 1, "done": 0, "overdue": false})
        );
        assert_eq!(json["from"], "2026-08-03");
        assert_eq!(json["to"], "2026-08-09");
        assert_eq!(
            json["undated_in_day_notes"],
            serde_json::json!({"2026-08-05": 1})
        );
        assert_eq!(json["overdue_before_range"], 0);
        assert_eq!(json["total_matches"], 1);
        assert_eq!(json["scan_in_progress"], false);
        assert!(json.get("groups").is_none(), "the grid replaces `groups`");
        assert!(json["folders"].is_array());
    }

    #[test]
    fn a_grid_query_deserializes() {
        let parsed: TaskQuery =
            serde_json::from_str(r#"{"mode":"grid","from":"2026-10-01","to":"2026-10-31"}"#)
                .expect("grid query");
        assert_eq!(parsed.mode, TaskMode::Grid);
        assert_eq!(parsed.from, Some(day(2026, 10, 1)));
        assert_eq!(parsed.to, Some(day(2026, 10, 31)));
        assert!(serde_json::from_str::<TaskQuery>(r#"{"from":"2026-13-01"}"#).is_err());
    }

    #[test]
    fn the_list_modes_ignore_the_grid_range() {
        let files = calendar_files();
        let plain = run_query(&files, &calendar_query(), today());
        let ranged = TaskQuery {
            from: Some(day(2030, 1, 1)),
            to: Some(day(2030, 1, 2)),
            ..calendar_query()
        };
        let with_range = run_query(&files, &ranged, today());
        assert_eq!(plain.groups, with_range.groups);
    }

    #[test]
    fn leading_minutes_reads_a_leading_clock_time() {
        let cases: &[(&str, Option<u32>)] = &[
            ("09:30 standup", Some(570)),
            ("9:30 standup", Some(570)),
            ("9:30am call", Some(570)),
            ("12:15 am late", Some(15)),
            ("12:15pm lunch", Some(735)),
            ("1:05 p.m. walk", Some(785)),
            ("9.30pm dinner", Some(1290)),
            ("23:59", Some(1439)),
            ("24:00 nope", None),
            ("9:60 nope", None),
            ("13:00pm nope", None),
            ("1.25 release", None),
            ("123:45", None),
            ("9:305", None),
            ("call at 9:30", None),
            ("", None),
            ("9", None),
        ];
        for (text, minutes) in cases {
            assert_eq!(leading_minutes(text), *minutes, "for {text:?}");
        }
    }

    mod grid_props {
        use super::*;
        use proptest::prelude::*;

        /// A task line with a random status and a random mix of the three dates,
        /// all within a few weeks of `today()`.
        fn task_line() -> impl Strategy<Value = String> {
            let date = (0u64..40)
                .prop_map(|offset| (day(2026, 7, 20) + chrono::Days::new(offset)).to_string());
            (
                prop_oneof![Just(' '), Just('x'), Just('-')],
                proptest::option::of(date.clone()),
                proptest::option::of(date.clone()),
                proptest::option::of(date),
                "[a-z]{1,8}",
            )
                .prop_map(|(mark, due, scheduled, done, word)| {
                    let mut line = format!("- [{mark}] {word}");
                    if let Some(d) = due {
                        line.push_str(&format!(" @due({d})"));
                    }
                    if let Some(d) = scheduled {
                        line.push_str(&format!(" ⏳ {d}"));
                    }
                    if let Some(d) = done {
                        line.push_str(&format!(" @done({d})"));
                    }
                    line
                })
        }

        proptest! {
            #[test]
            fn every_placed_task_sits_on_its_placement_date_inside_the_range(
                lines in proptest::collection::vec(task_line(), 0..30),
                start in 0u64..30,
                len in 0u64..80,
            ) {
                let body = lines.join("\n");
                let files = vec![file("/n/", "n.md", None, &body)];
                let from = day(2026, 7, 20) + chrono::Days::new(start);
                let q = grid_query(from, from + chrono::Days::new(len));
                let response = run_grid_query(&files, &q, today(), &no_day_notes());
                let (from, to) = (response.from, response.to);
                prop_assert!((to - from).num_days() < i64::from(MAX_GRID_DAYS));

                let mut expected = 0usize;
                for task in &files[0].tasks {
                    let placement = grid_placement(task)
                        .filter(|(at, _, _)| (from..=to).contains(&at.date()));
                    if let Some((at, _, _)) = placement {
                        expected += 1;
                        let day = &response.days[&at.date().to_string()];
                        prop_assert!(day.tasks.iter().any(|t| t.hit.task.line == task.line));
                    }
                }
                prop_assert_eq!(response.total_matches, expected);
                let counted: u32 = response.days.values().map(|d| d.open + d.done).sum();
                prop_assert_eq!(counted as usize, expected);
                for (date, d) in &response.days {
                    let date: NaiveDate = date.parse().expect("ISO key");
                    prop_assert!((from..=to).contains(&date));
                    prop_assert!(d.tasks.iter().all(|t| t.hit.task.status != TaskStatus::Canceled));
                    prop_assert!(d.tasks.windows(2).all(|w| grid_order(&w[0], &w[1]) != std::cmp::Ordering::Greater));
                }
            }
        }
    }
}
