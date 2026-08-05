//! Task Parsing Module
//!
//! Pure, allocation-light parsing of markdown task lines and of whole markdown
//! sources. This module knows nothing about the filesystem, the index, or HTTP —
//! it turns text into [`Task`] values and back again.
//!
//! # Grammar
//!
//! A task line is a list item whose content starts with a checkbox:
//!
//! ```text
//! ^[ \t]*(?:[-*+]|\d+[.)])[ \t]+\[[ xX>-]\](?:[ \t]+text)?$
//! ```
//!
//! | Marker  | Status                            |
//! |---------|-----------------------------------|
//! | ` `     | [`TaskStatus::Open`]              |
//! | `x` `X` | [`TaskStatus::Done`]              |
//! | `-`     | [`TaskStatus::Canceled`]          |
//! | `>`     | [`TaskStatus::Canceled`] (moved)  |
//!
//! # Annotations
//!
//! Everything below is parsed out of the task text and stripped from
//! [`Task::text`], which is left whitespace-collapsed and trimmed.
//!
//! | Syntax                    | Meaning       | Rule |
//! |---------------------------|---------------|------|
//! | `@due(<dt>)`              | Due date      | `<dt>` = `YYYY-MM-DD`, optional ` HH:MM` (24h) or ` HH:MM AM/PM` |
//! | `@done(<dt>)`             | Completion    | same datetime grammar |
//! | `#tag`                    | Tag           | `[A-Za-z0-9_-]+`, after start-of-text or whitespace |
//! | `!!` / `!!!`              | High / Urgent | whitespace-delimited on both sides |
//! | `> YYYY-MM-DD` (trailing) | Moved-to date | recorded in [`Task::moved_to`], stripped |
//! | `< YYYY-MM-DD` (trailing) | Moved-from    | stripped and discarded |
//!
//! Dates are naive and local: no timezone conversion, no UTC round-trip. A
//! `@due(2026-08-05)` with no time is start-of-day.
//!
//! An annotation whose payload does not parse as a date is *not* an annotation.
//! `@due(next tuesday)` stays in the display text verbatim rather than being
//! silently swallowed — a user who mistypes a date should see their typo, not a
//! task that quietly lost its deadline.
//!
//! # Examples
//!
//! ```
//! use mbr::tasks::{TaskPriority, TaskStatus, parse_task_line};
//!
//! let task = parse_task_line("- [ ] ship it !!! #work @due(2026-08-05)", 12).unwrap();
//! assert_eq!(task.line, 12);
//! assert_eq!(task.status, TaskStatus::Open);
//! assert_eq!(task.priority, TaskPriority::Urgent);
//! assert_eq!(task.text, "ship it");
//! assert_eq!(task.tags, ["work"]);
//! assert!(task.due.is_some() && !task.due_has_time);
//! ```

use std::sync::LazyLock;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};

use crate::wikilink::{BlockScanner, LineKind, indent_width};

/// The task-line *prefix*: `- [ ] `, `1. [x]\t`, `* [-]`, `+ [>] `.
///
/// The grammar this implements is
/// `^[ \t]*(?:[-*+]|\d+[.)])[ \t]+\[[ xX>-]\](?:[ \t]+text)?$`, but the pattern
/// deliberately stops at the checkbox instead of capturing the indent and the
/// trailing text. Those two are recovered by slicing in [`match_task_line`],
/// which is exactly equivalent and about **ten times faster**: a trailing `.*`
/// capture forces the regex crate onto its slow capture-tracking engine and makes
/// it walk the whole line, which showed up as ~710ns per task line against ~70ns
/// here. On a repository with hundreds of thousands of tasks that is the
/// difference between a noticeable index build and an invisible one.
///
/// `(?:[ \t]+|$)` preserves the `$`-anchored original: whitespace must follow the
/// checkbox unless the line ends there, so `- [ ]x` is still not a task.
static TASK_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[ \t]*(?:[-*+]|\d+[.)])[ \t]+\[(?<marker>[ xX>-])\](?:[ \t]+|$)")
        .expect("literal task-line regex is valid and cannot fail to compile")
});

/// `@due(...)` / `@done(...)`. The payload is captured loosely and validated by
/// [`parse_datetime`], so an unparseable payload can be left in place verbatim.
static DATE_ANNOTATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"@(?<kind>due|done)\((?<value>[^)]*)\)")
        .expect("literal date-annotation regex is valid and cannot fail to compile")
});

/// `YYYY-MM-DD`, optionally ` HH:MM` (24-hour) or ` HH:MM AM/PM`. The AM/PM
/// suffix is case-insensitive and its leading space is optional.
static DATETIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?<y>\d{4})-(?<mo>\d{2})-(?<d>\d{2})(?:[ \t]+(?<h>\d{1,2}):(?<mi>\d{2})(?:[ \t]*(?<ap>[AaPp][Mm]))?)?$",
    )
    .expect("literal datetime regex is valid and cannot fail to compile")
});

/// `#tag`, anchored to start-of-text or whitespace so that the fragment in
/// `see page.md#anchor` is not mistaken for a tag.
static TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|\s)#(?<tag>[A-Za-z0-9_-]+)")
        .expect("literal tag regex is valid and cannot fail to compile")
});

/// A trailing `> YYYY-MM-DD` (moved to) or `< YYYY-MM-DD` (moved from). Anchored
/// to the end of the text and preceded by whitespace, so a `>` used in prose
/// mid-line is never eaten.
static TRAILING_MOVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[ \t])(?<dir>[<>])[ \t]*(?<date>\d{4}-\d{2}-\d{2})[ \t]*$")
        .expect("literal trailing-move regex is valid and cannot fail to compile")
});

/// Completion state of a task.
///
/// `[>]` (moved elsewhere) collapses to `Canceled`; the destination date is kept
/// separately in [`Task::moved_to`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    /// `[ ]` — not yet done.
    Open,
    /// `[x]` or `[X]` — completed.
    Done,
    /// `[-]` or `[>]` — will not be done here.
    Canceled,
}

impl TaskStatus {
    /// The marker byte [`set_marker`] writes for this status.
    ///
    /// `Canceled` always writes `-`: `>` carries a destination date this function
    /// has no way to supply, so a toggle to canceled deliberately downgrades to
    /// the plain form.
    const fn marker_char(self) -> char {
        match self {
            Self::Open => ' ',
            Self::Done => 'x',
            Self::Canceled => '-',
        }
    }
}

/// Task priority. There is no "low": `Normal` is the default and `!!` / `!!!`
/// escalate from there.
///
/// `Ord` is derived and meaningful — variants are declared lowest-first so that
/// "highest priority wins" is a `max`.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    /// No priority marker.
    #[default]
    Normal,
    /// `!!`
    High,
    /// `!!!`
    Urgent,
}

/// One parsed task line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    /// 1-based source line number.
    pub line: u32,
    /// Display indent level; see [`parse_task_line`] for how it is derived.
    pub depth: u8,
    /// Completion state.
    pub status: TaskStatus,
    /// Priority, `Normal` unless `!!` or `!!!` appeared.
    pub priority: TaskPriority,
    /// Display text: annotations stripped, whitespace collapsed, trimmed.
    pub text: String,
    /// Tags without the leading `#`, original case, de-duplicated.
    pub tags: Vec<String>,
    /// `@due(...)`, start-of-day when no time was given.
    pub due: Option<NaiveDateTime>,
    /// Whether `due` carried an explicit time.
    pub due_has_time: bool,
    /// `@done(...)`, start-of-day when no time was given.
    pub done: Option<NaiveDateTime>,
    /// Whether `done` carried an explicit time.
    pub done_has_time: bool,
    /// Trailing `> YYYY-MM-DD`: where this task was moved to.
    pub moved_to: Option<NaiveDate>,
}

/// Everything [`strip_annotations`] pulls out of a task's text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Annotations {
    /// Highest priority marker found.
    pub priority: TaskPriority,
    /// Tags without the leading `#`, original case, de-duplicated.
    pub tags: Vec<String>,
    /// `@due(...)`.
    pub due: Option<NaiveDateTime>,
    /// Whether `due` carried an explicit time.
    pub due_has_time: bool,
    /// `@done(...)`.
    pub done: Option<NaiveDateTime>,
    /// Whether `done` carried an explicit time.
    pub done_has_time: bool,
    /// Trailing `> YYYY-MM-DD`.
    pub moved_to: Option<NaiveDate>,
}

/// Parses a single line as a task, returning `None` if it is not one.
///
/// `line_number` is stored verbatim in [`Task::line`]; callers are responsible
/// for it being 1-based.
///
/// # Depth
///
/// Indentation is measured in columns with a tab counting as four, then halved:
/// `depth = columns / 2`. Markdown nesting is two-or-four spaces depending on the
/// author (and a tab depending on the editor), and there is no way to recover the
/// author's intent from a single line. All this value has to be is *monotonic*
/// for display indentation, so the cheap halving is enough — it maps both the
/// two-space and the four-space convention onto increasing levels, and it never
/// panics because it saturates at [`u8::MAX`].
///
/// # Examples
///
/// ```
/// use mbr::tasks::{TaskStatus, parse_task_line};
///
/// assert!(parse_task_line("not a task", 1).is_none());
/// assert!(parse_task_line("- [y] unknown marker", 1).is_none());
///
/// let task = parse_task_line("  * [x] done thing @done(2026-08-04 12:11 PM)", 3).unwrap();
/// assert_eq!(task.status, TaskStatus::Done);
/// assert_eq!(task.depth, 1);
/// assert_eq!(task.text, "done thing");
/// assert!(task.done_has_time);
/// ```
pub fn parse_task_line(line: &str, line_number: u32) -> Option<Task> {
    if !might_be_task(line) {
        return None;
    }
    let (marker_at, raw_text) = match_task_line(line)?;
    let status = match line.as_bytes().get(marker_at)? {
        b' ' => TaskStatus::Open,
        b'x' | b'X' => TaskStatus::Done,
        // `-` and `>`; the regex admits nothing else.
        _ => TaskStatus::Canceled,
    };
    // `indent_width` stops at the first non-space/tab byte, which is the bullet,
    // so measuring the whole line measures exactly the indent.
    let depth = u8::try_from(indent_width(line) / 2).unwrap_or(u8::MAX);
    let (text, annotations) = strip_annotations(raw_text);

    Some(Task {
        line: line_number,
        depth,
        status,
        priority: annotations.priority,
        text,
        tags: annotations.tags,
        due: annotations.due,
        due_has_time: annotations.due_has_time,
        done: annotations.done,
        done_has_time: annotations.done_has_time,
        moved_to: annotations.moved_to,
    })
}

/// Rewrites the marker of an existing task line, preserving every other byte.
///
/// Indentation, bullet style, internal spacing and annotations all survive
/// untouched, which is what makes this safe to use for an in-place line patch of
/// a file the user also edits by hand. Returns `None` when `line` is not a task.
///
/// # Examples
///
/// ```
/// use mbr::tasks::{TaskStatus, set_marker};
///
/// let line = "\t2)  [ ]   buy milk   @due(2026-08-05)";
/// assert_eq!(
///     set_marker(line, TaskStatus::Done).unwrap(),
///     "\t2)  [x]   buy milk   @due(2026-08-05)"
/// );
/// assert!(set_marker("# not a task", TaskStatus::Done).is_none());
/// ```
pub fn set_marker(line: &str, status: TaskStatus) -> Option<String> {
    let (at, _) = match_task_line(line)?;
    let mut out = String::with_capacity(line.len());
    out.push_str(&line[..at]);
    out.push(status.marker_char());
    // Every marker is one ASCII byte, so this split is always on a char boundary.
    out.push_str(&line[at + 1..]);
    Some(out)
}

/// Scans a whole markdown source for task lines.
///
/// Line numbers are 1-based and count every physical line, including the ones
/// skipped below. Lines are *not* treated as tasks when they fall inside:
///
/// - YAML frontmatter at the top of the file;
/// - a fenced code block (backtick or tilde, any fence length, info string or
///   not, including inside a blockquote);
/// - an indented code block.
///
/// # Examples
///
/// ```
/// use mbr::tasks::scan_source_tasks;
///
/// let source = "---\ntitle: Notes\n---\n\n- [ ] real task\n\n```\n- [ ] sample\n```\n";
/// let tasks = scan_source_tasks(source);
/// assert_eq!(tasks.len(), 1);
/// assert_eq!(tasks[0].line, 5);
/// assert_eq!(tasks[0].text, "real task");
/// ```
pub fn scan_source_tasks(source: &str) -> Vec<Task> {
    let skip = frontmatter_line_count(source);
    let mut blocks = BlockScanner::new();

    // NOTE (indented code): `BlockScanner` only opens an indented code block
    // outside a list, deciding "inside a list" from the last column-0 line. That
    // is deliberate and load-bearing here: a tab-indented `- [ ] subtask` is four
    // columns deep, so treating four columns as code unconditionally would lose
    // every nested subtask — the exact shape TASKS_SPEC.md asks us to support.
    // The price is one narrow false positive: a genuine indented code block
    // written *inside* a list item (item indent plus four more columns) has its
    // `- [ ]` lines read as tasks, because recognising it needs the full
    // list-item context this raw-text pass deliberately does not track. We take
    // that trade because a spurious task from a code sample is visible and
    // harmless, while a silently missing subtask is neither. Covered by
    // `indented_code_inside_a_list_item_is_a_known_false_positive`.
    source
        .lines()
        .enumerate()
        .skip(skip)
        .filter_map(|(index, line)| {
            // Every non-frontmatter line must be classified, even ones that
            // cannot be tasks, or the fence state machine loses track.
            if blocks.classify(line) == LineKind::Code {
                return None;
            }
            parse_task_line(line, u32::try_from(index + 1).unwrap_or(u32::MAX))
        })
        .collect()
}

/// Pulls every annotation out of a task's text.
///
/// Returns the display text — annotations removed, whitespace collapsed to
/// single spaces, trimmed — alongside what was found. Collapsing is
/// unconditional, so `a    b` becomes `a b` even in a task with no annotations
/// at all; the alternative is text whose spacing depends on whether an
/// annotation happened to be present.
///
/// Stripping runs in a fixed order: date annotations, then the trailing
/// move marker, then tags, then priority. That order is what "trailing" means for
/// `> YYYY-MM-DD` — at the very end once `@due(...)`/`@done(...)` are gone, but
/// still ahead of any trailing tag. `do the thing > 2026-08-04 #work` therefore
/// keeps its `>` in the display text, because the `#work` is what is trailing.
///
/// # Examples
///
/// ```
/// use mbr::tasks::strip_annotations;
///
/// let (text, ann) = strip_annotations("email #work #WORK bob @due(2026-08-05 09:30)");
/// assert_eq!(text, "email bob");
/// // De-duplication is case-insensitive and keeps the first spelling.
/// assert_eq!(ann.tags, ["work"]);
/// assert!(ann.due_has_time);
/// ```
pub fn strip_annotations(text: &str) -> (String, Annotations) {
    let mut annotations = Annotations::default();

    let without_dates = strip_date_annotations(text, &mut annotations);
    let without_move = strip_trailing_move(&without_dates, &mut annotations);
    let without_tags = strip_tags(&without_move, &mut annotations);
    let (display, priority) = collapse_taking_priority(&without_tags);
    annotations.priority = priority;

    (display, annotations)
}

/// Separator [`strip_annotations_across_runs`] splices between inline text runs.
///
/// U+0000 is the natural choice: CommonMark tells a parser to replace it with
/// U+FFFD, so it should never reach a renderer, and it is not whitespace, so it
/// cannot merge two runs into one token. pulldown-cmark 0.13 does *not* in fact
/// perform that replacement, so [`strip_annotations_across_runs`] does it
/// itself rather than assume — see [`RUN_BOUNDARY_REPLACEMENT`].
const RUN_BOUNDARY: char = '\0';

/// What a literal [`RUN_BOUNDARY`] in the source is rewritten to, which is what
/// CommonMark says should have happened to it upstream.
const RUN_BOUNDARY_REPLACEMENT: char = '\u{fffd}';

/// [`strip_annotations`] for a task whose text is split across inline
/// formatting boundaries.
///
/// A rendered task line does not arrive as one string: `fix **this** #bug`
/// reaches the renderer as the runs `["fix ", "this", " #bug"]` with
/// `<strong>` events in between. Stripping each run on its own is wrong twice
/// over — the per-run trim would delete the space before `<strong>`, and
/// `$`-anchored rules like the trailing `> YYYY-MM-DD` would fire at the end of
/// every run rather than at the end of the text.
///
/// So the runs are spliced into one string with [`RUN_BOUNDARY`] between them,
/// stripped by the single grammar in [`strip_annotations`], and split apart
/// again. The boundary is not whitespace, so it never merges two runs into one
/// token, and none of the stripping passes can consume or emit one; the
/// returned vector therefore has exactly one entry per input run. A literal
/// boundary character already in the text is rewritten first, so a document
/// containing one cannot desynchronise the split.
///
/// Splicing also makes the *renderer* agree with [`scan_source_tasks`] about
/// what is adjacent to what: `**a**#work` has no whitespace before the `#` in
/// the source, and the boundary preserves that, so neither reads it as a tag.
///
/// # Examples
///
/// ```
/// use mbr::tasks::strip_annotations_across_runs;
///
/// let (runs, ann) = strip_annotations_across_runs(&["fix ", "this", " #bug"]);
/// assert_eq!(runs, ["fix ", "this", ""]);
/// assert_eq!(ann.tags, ["bug"]);
/// ```
pub fn strip_annotations_across_runs(runs: &[&str]) -> (Vec<String>, Annotations) {
    match runs {
        [] => (Vec::new(), Annotations::default()),
        [only] => {
            let (text, annotations) = strip_annotations(only);
            (vec![text], annotations)
        }
        _ => {
            let mut joined = String::with_capacity(runs.iter().map(|r| r.len() + 1).sum());
            for (index, run) in runs.iter().enumerate() {
                if index > 0 {
                    joined.push(RUN_BOUNDARY);
                }
                if run.contains(RUN_BOUNDARY) {
                    joined.extend(run.chars().map(|c| {
                        if c == RUN_BOUNDARY {
                            RUN_BOUNDARY_REPLACEMENT
                        } else {
                            c
                        }
                    }));
                } else {
                    joined.push_str(run);
                }
            }

            let (stripped, annotations) = strip_annotations(&joined);
            let mut parts: Vec<String> = stripped.split(RUN_BOUNDARY).map(str::to_string).collect();

            // Unreachable given the invariant documented above, but a
            // misalignment here would silently drop text from the page, so
            // recover by keeping every character in the first run rather than
            // trusting the split.
            if parts.len() != runs.len() {
                debug_assert!(false, "run boundaries were not preserved: {stripped:?}");
                parts = std::iter::once(stripped.replace(RUN_BOUNDARY, " "))
                    .chain(std::iter::repeat_n(String::new(), runs.len() - 1))
                    .collect();
            }
            (parts, annotations)
        }
    }
}

/// Cheap conservative pre-filter: rejects lines that cannot possibly match
/// [`TASK_LINE`] without paying for the regex.
///
/// Most lines in a markdown file are prose, and this kills them on the first
/// non-whitespace byte. It must never reject a real task line.
fn might_be_task(line: &str) -> bool {
    let rest = line.trim_start_matches([' ', '\t']).as_bytes();
    if !matches!(rest.first(), Some(b'-' | b'*' | b'+' | b'0'..=b'9')) {
        return false;
    }
    rest.windows(3)
        .any(|w| w[0] == b'[' && matches!(w[1], b' ' | b'x' | b'X' | b'>' | b'-') && w[2] == b']')
}

/// Borrows a named capture group as a `&str`, without the panic of `caps[name]`.
fn group<'h>(caps: &Captures<'h>, name: &str) -> Option<&'h str> {
    caps.name(name).map(|m| m.as_str())
}

/// Matches the task-line prefix, returning the byte offset of the marker
/// character and the un-parsed text that follows the checkbox.
///
/// Shared by [`parse_task_line`] and [`set_marker`] so the two can never disagree
/// about what counts as a task — the round-trip property test depends on that.
fn match_task_line(line: &str) -> Option<(usize, &str)> {
    if !might_be_task(line) {
        return None;
    }
    let caps = TASK_LINE.captures(line)?;
    let marker_at = caps.name("marker")?.start();
    let text = &line[caps.get(0)?.end()..];

    // Both functions are documented to take a *single* line. The original
    // `$`-anchored pattern enforced that for free; now that the pattern stops at
    // the checkbox, say it explicitly. One trailing newline is fine (callers
    // reasonably pass a line with its terminator), but an interior one means the
    // caller handed us several lines, and silently welding them into one task's
    // text would be worse than refusing.
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.contains('\n') {
        return None;
    }
    Some((marker_at, text))
}

/// Replaces every *valid* `@due(...)` / `@done(...)` with a space, recording the
/// first of each kind. Invalid payloads are left exactly as written.
fn strip_date_annotations(text: &str, annotations: &mut Annotations) -> String {
    if !text.contains('@') {
        return text.to_string();
    }
    DATE_ANNOTATION
        .replace_all(text, |caps: &Captures| {
            let Some((parsed, has_time)) = group(caps, "value").and_then(parse_datetime) else {
                // Not a date, so not an annotation: keep the user's text.
                return caps[0].to_string();
            };
            match group(caps, "kind") {
                Some("due") if annotations.due.is_none() => {
                    annotations.due = Some(parsed);
                    annotations.due_has_time = has_time;
                }
                Some("done") if annotations.done.is_none() => {
                    annotations.done = Some(parsed);
                    annotations.done_has_time = has_time;
                }
                // A repeat of a kind we already have: still an annotation, so it
                // is still stripped; first one wins.
                _ => {}
            }
            " ".to_string()
        })
        .into_owned()
}

/// Strips trailing `> YYYY-MM-DD` / `< YYYY-MM-DD` markers, recording the `>`
/// destination. Loops so that `... > 2026-08-04 < 2026-08-01` yields both.
fn strip_trailing_move(text: &str, annotations: &mut Annotations) -> String {
    let mut rest = text;
    // The pattern ends `\d{4}-\d{2}-\d{2}[ \t]*$`, so it can only match when the
    // last non-blank byte is a digit. Testing that first keeps the regex crate's
    // capture-tracking engine — the slow one, by roughly a factor of ten, as
    // documented on [`TASK_LINE`] — off the overwhelming majority of task lines,
    // which carry no move marker at all. This is on the render path for every
    // task in a document and on the read path for every task in a repository.
    while ends_with_digit_ignoring_blanks(rest)
        && let Some(caps) = TRAILING_MOVE.captures(rest)
    {
        let Some(date) =
            group(&caps, "date").and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        else {
            // Digit-shaped but not a real date (`2026-13-45`): leave it alone.
            break;
        };
        if group(&caps, "dir") == Some(">") && annotations.moved_to.is_none() {
            annotations.moved_to = Some(date);
        }
        // A `<` source marker is recorded nowhere on purpose — it says where the
        // task came from, which nothing in this feature surfaces.
        let Some(whole) = caps.get(0) else { break };
        rest = &rest[..whole.start()];
    }
    rest.to_string()
}

/// Whether the last byte of `text`, ignoring trailing spaces and tabs, is an
/// ASCII digit. A necessary condition for [`TRAILING_MOVE`] to match, and far
/// cheaper to test.
fn ends_with_digit_ignoring_blanks(text: &str) -> bool {
    text.as_bytes()
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t'))
        .is_some_and(|at| text.as_bytes()[at].is_ascii_digit())
}

/// Removes `#tag` occurrences, collecting them de-duplicated case-insensitively
/// with the first spelling kept.
fn strip_tags(text: &str, annotations: &mut Annotations) -> String {
    if !text.contains('#') {
        return text.to_string();
    }
    TAG.replace_all(text, |caps: &Captures| {
        if let Some(tag) = group(caps, "tag")
            && !annotations
                .tags
                .iter()
                .any(|seen| seen.eq_ignore_ascii_case(tag))
        {
            annotations.tags.push(tag.to_string());
        }
        // The leading whitespace the pattern consumed is restored as a single
        // space; the collapse pass tidies up whatever that leaves behind.
        " ".to_string()
    })
    .into_owned()
}

/// Collapses whitespace to single spaces and lifts out `!!` / `!!!` priority
/// markers in the same pass.
///
/// Splitting on whitespace *is* the "whitespace-delimited on both sides" rule:
/// `wow!!` and `a!!b` are single tokens that are not equal to `!!`, so they never
/// register as priorities.
fn collapse_taking_priority(text: &str) -> (String, TaskPriority) {
    let mut priority = TaskPriority::Normal;
    let mut display = String::with_capacity(text.len());

    for token in text.split_whitespace() {
        match token {
            "!!!" => priority = priority.max(TaskPriority::Urgent),
            "!!" => priority = priority.max(TaskPriority::High),
            _ => {
                if !display.is_empty() {
                    display.push(' ');
                }
                display.push_str(token);
            }
        }
    }
    (display, priority)
}

/// Parses `YYYY-MM-DD`, `YYYY-MM-DD HH:MM` or `YYYY-MM-DD HH:MM AM/PM`.
///
/// Returns the datetime and whether an explicit time was present. Times are
/// naive and local; a bare date is start-of-day.
fn parse_datetime(value: &str) -> Option<(NaiveDateTime, bool)> {
    let caps = DATETIME.captures(value.trim())?;
    let date = NaiveDate::from_ymd_opt(
        group(&caps, "y")?.parse().ok()?,
        group(&caps, "mo")?.parse().ok()?,
        group(&caps, "d")?.parse().ok()?,
    )?;

    let Some(hour_text) = group(&caps, "h") else {
        return Some((date.and_time(NaiveTime::MIN), false));
    };
    let hour: u32 = hour_text.parse().ok()?;
    let minute: u32 = group(&caps, "mi")?.parse().ok()?;

    // `| 0x20` lowercases the ASCII letter; the regex guarantees one is there.
    let hour24 = match group(&caps, "ap")
        .and_then(|ap| ap.as_bytes().first())
        .map(|b| b | 0x20)
    {
        None => hour,
        // 12-hour clock: 12 AM is 00:00 and 12 PM is 12:00, hence the `% 12`.
        Some(b'a') if (1..=12).contains(&hour) => hour % 12,
        Some(_) if (1..=12).contains(&hour) => hour % 12 + 12,
        // `14:00 PM` is not a time anybody meant.
        Some(_) => return None,
    };
    Some((
        date.and_time(NaiveTime::from_hms_opt(hour24, minute, 0)?),
        true,
    ))
}

/// Number of leading lines occupied by YAML frontmatter, or `0` when there is
/// none.
///
/// Requires an opening `---` on line 1 at column 0 and a closing `---` or `...`.
/// An *unclosed* opener is not treated as frontmatter: a lone `---` is a thematic
/// break far more often than it is the start of a metadata block someone forgot
/// to finish, and swallowing the rest of the file would hide every task in it.
fn frontmatter_line_count(source: &str) -> usize {
    let mut lines = source.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return 0;
    }
    lines
        .position(|line| matches!(line.trim_end(), "---" | "..."))
        // `position` is 0-based and relative to line 2, so +2 covers the opening
        // delimiter and the closing one.
        .map_or(0, |index| index + 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Builds the `NaiveDateTime` a test expects, without `unwrap` noise.
    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .and_then(|date| date.and_hms_opt(h, min, 0))
            .expect("test datetime is valid")
    }

    fn parse(line: &str) -> Task {
        parse_task_line(line, 1).unwrap_or_else(|| panic!("expected a task from {line:?}"))
    }

    // ---- markers and bullets -------------------------------------------------

    #[test]
    fn open_marker_parses() {
        let task = parse("- [ ] open thing");
        assert_eq!(task.status, TaskStatus::Open);
        assert_eq!(task.text, "open thing");
        assert_eq!(task.depth, 0);
        assert_eq!(task.line, 1);
    }

    #[test]
    fn lowercase_x_is_done() {
        assert_eq!(parse("- [x] finished").status, TaskStatus::Done);
    }

    #[test]
    fn uppercase_x_is_done() {
        assert_eq!(parse("- [X] finished").status, TaskStatus::Done);
    }

    #[test]
    fn dash_marker_is_canceled() {
        let task = parse("* [-] this task was canceled");
        assert_eq!(task.status, TaskStatus::Canceled);
        assert_eq!(task.moved_to, None);
    }

    #[test]
    fn angle_marker_is_canceled_with_moved_to() {
        let task = parse("* [>] This task was moved to a specific date > 2026-08-04");
        assert_eq!(task.status, TaskStatus::Canceled);
        assert_eq!(task.text, "This task was moved to a specific date");
        assert_eq!(task.moved_to, NaiveDate::from_ymd_opt(2026, 8, 4));
    }

    #[test]
    fn all_bullet_styles_parse() {
        for line in [
            "- [ ] a",
            "* [ ] a",
            "+ [ ] a",
            "1. [ ] a",
            "1) [ ] a",
            "42. [ ] a",
        ] {
            assert_eq!(parse(line).text, "a", "bullet style {line:?}");
        }
    }

    #[test]
    fn tab_indent_sets_depth() {
        // A tab is four columns, and depth is columns / 2.
        assert_eq!(parse("\t- [ ] subtask").depth, 2);
        assert_eq!(parse("\t\t- [ ] deeper").depth, 4);
        assert_eq!(parse("  - [ ] two spaces").depth, 1);
        assert_eq!(parse("    - [ ] four spaces").depth, 2);
    }

    #[test]
    fn depth_saturates_instead_of_overflowing() {
        let line = format!("{}- [ ] very deep", " ".repeat(1000));
        assert_eq!(parse(&line).depth, u8::MAX);
    }

    #[test]
    fn empty_task_text_is_still_a_task() {
        let task = parse("- [ ]");
        assert_eq!(task.text, "");
        assert_eq!(task.status, TaskStatus::Open);
        // Trailing whitespace after the marker is equally fine.
        assert_eq!(parse("- [x]   ").text, "");
    }

    #[test]
    fn tab_after_marker_is_accepted() {
        assert_eq!(parse("-\t[ ]\tbuy milk").text, "buy milk");
    }

    // ---- non-tasks -----------------------------------------------------------

    #[test]
    fn non_tasks_are_rejected() {
        for line in [
            "- [] x",           // no marker character
            "- [ x] x",         // two characters in the box
            "-[ ] x",           // no space after the bullet
            "- [y] x",          // unknown marker
            "- [ ]x",           // no space after the box
            "just some prose",  // prose
            "# A heading",      // heading
            "[ ]",              // bare checkbox, no list item
            "[ ] do a thing",   // still no list item
            "> [ ] blockquote", // blockquote, not a list item
            "",                 // empty line
            "  ",               // whitespace only
        ] {
            assert!(
                parse_task_line(line, 1).is_none(),
                "expected {line:?} to be rejected"
            );
        }
    }

    #[test]
    fn a_single_trailing_newline_is_tolerated() {
        // Callers reading a line from a file reasonably keep its terminator.
        assert_eq!(parse("- [ ] buy milk\n").text, "buy milk");
        assert_eq!(parse("- [ ] buy milk\r\n").text, "buy milk");
        assert_eq!(
            set_marker("- [ ] buy milk\n", TaskStatus::Done).as_deref(),
            Some("- [x] buy milk\n")
        );
    }

    #[test]
    fn multi_line_input_is_refused_rather_than_welded_together() {
        // Both entry points must agree, or `set_marker` could patch a line that
        // `parse_task_line` never accepted.
        for input in ["- [ ] first\n- [ ] second", "- [ ] first\nprose\n"] {
            assert!(parse_task_line(input, 1).is_none(), "{input:?}");
            assert!(set_marker(input, TaskStatus::Done).is_none(), "{input:?}");
        }
    }

    #[test]
    fn might_be_task_never_rejects_a_real_task() {
        for line in ["- [ ] a", "\t1) [X] b", "   * [-] c", "99. [>] d"] {
            assert!(might_be_task(line), "{line:?}");
            assert!(parse_task_line(line, 1).is_some(), "{line:?}");
        }
    }

    // ---- annotations in isolation -------------------------------------------

    #[test]
    fn due_date_only() {
        let task = parse("- [ ] pay rent @due(2026-08-05)");
        assert_eq!(task.text, "pay rent");
        assert_eq!(task.due, Some(dt(2026, 8, 5, 0, 0)));
        assert!(!task.due_has_time);
    }

    #[test]
    fn due_with_24_hour_time() {
        let task = parse("- [ ] standup @due(2026-08-05 14:30)");
        assert_eq!(task.due, Some(dt(2026, 8, 5, 14, 30)));
        assert!(task.due_has_time);
    }

    #[test]
    fn due_with_am_pm_time() {
        assert_eq!(
            parse("- [ ] a @due(2026-08-05 03:00 PM)").due,
            Some(dt(2026, 8, 5, 15, 0))
        );
        assert_eq!(
            parse("- [ ] a @due(2026-08-05 3:00pm)").due,
            Some(dt(2026, 8, 5, 15, 0))
        );
        assert_eq!(
            parse("- [ ] a @due(2026-08-05 09:15 am)").due,
            Some(dt(2026, 8, 5, 9, 15))
        );
        // Midnight and noon are the two the 12-hour clock always gets wrong.
        assert_eq!(
            parse("- [ ] a @due(2026-08-05 12:00 AM)").due,
            Some(dt(2026, 8, 5, 0, 0))
        );
        assert_eq!(
            parse("- [ ] a @due(2026-08-05 12:00 PM)").due,
            Some(dt(2026, 8, 5, 12, 0))
        );
    }

    #[test]
    fn done_annotation() {
        let task = parse("- [x] shipped @done(2026-08-04 12:11 PM)");
        assert_eq!(task.done, Some(dt(2026, 8, 4, 12, 11)));
        assert!(task.done_has_time);
        assert_eq!(task.due, None);
    }

    #[test]
    fn invalid_dates_are_left_verbatim() {
        for (line, expected) in [
            ("- [ ] a @due(next tuesday)", "a @due(next tuesday)"),
            ("- [ ] a @due(2026-13-45)", "a @due(2026-13-45)"),
            ("- [ ] a @due(2026-02-30)", "a @due(2026-02-30)"),
            ("- [ ] a @due()", "a @due()"),
            ("- [ ] a @due(2026-08-05 25:00)", "a @due(2026-08-05 25:00)"),
            // 14 o'clock cannot also be PM.
            (
                "- [ ] a @due(2026-08-05 14:00 PM)",
                "a @due(2026-08-05 14:00 PM)",
            ),
            ("- [x] a @done(garbage)", "a @done(garbage)"),
        ] {
            let task = parse(line);
            assert_eq!(task.text, expected, "for {line:?}");
            assert_eq!(task.due, None, "for {line:?}");
            assert_eq!(task.done, None, "for {line:?}");
        }
    }

    #[test]
    fn first_annotation_of_a_kind_wins_and_all_are_stripped() {
        let task = parse("- [ ] a @due(2026-08-05) b @due(2026-09-09) c");
        assert_eq!(task.due, Some(dt(2026, 8, 5, 0, 0)));
        assert_eq!(task.text, "a b c");
    }

    #[test]
    fn tags_are_extracted_and_stripped() {
        let task = parse("- [ ] review #work #urgent-ish #v2_final");
        assert_eq!(task.text, "review");
        assert_eq!(task.tags, ["work", "urgent-ish", "v2_final"]);
    }

    #[test]
    fn tags_dedupe_case_insensitively_keeping_first_spelling() {
        let task = parse("- [ ] a #Work #work #WORK #wOrK b");
        assert_eq!(task.tags, ["Work"]);
        assert_eq!(task.text, "a b");
    }

    #[test]
    fn url_fragment_is_not_a_tag() {
        let task = parse("- [ ] see page.md#anchor for details");
        assert!(task.tags.is_empty());
        assert_eq!(task.text, "see page.md#anchor for details");
    }

    #[test]
    fn adjacent_hashes_only_yield_the_first_tag() {
        // The second `#` is not preceded by whitespace, so it is not a tag.
        let task = parse("- [ ] a #one#two b");
        assert_eq!(task.tags, ["one"]);
        assert_eq!(task.text, "a #two b");
    }

    #[test]
    fn tag_stops_at_punctuation() {
        let task = parse("- [ ] ping #bob, then go");
        assert_eq!(task.tags, ["bob"]);
        assert_eq!(task.text, "ping , then go");
    }

    #[test]
    fn priorities_are_whitespace_delimited() {
        assert_eq!(parse("- [ ] a !! b").priority, TaskPriority::High);
        assert_eq!(parse("- [ ] a !!! b").priority, TaskPriority::Urgent);
        assert_eq!(parse("- [ ] !! leading").priority, TaskPriority::High);
        assert_eq!(parse("- [ ] trailing !!!").priority, TaskPriority::Urgent);
    }

    #[test]
    fn non_delimited_bangs_are_not_priorities() {
        for line in ["- [ ] wow!!", "- [ ] a!!b", "- [ ] wow!!!", "- [ ] !!!!"] {
            let task = parse(line);
            assert_eq!(task.priority, TaskPriority::Normal, "for {line:?}");
            // ...and they stay in the text.
            assert!(task.text.contains("!!"), "for {line:?}");
        }
    }

    #[test]
    fn highest_priority_wins() {
        assert_eq!(parse("- [ ] a !! b !!! c").priority, TaskPriority::Urgent);
        assert_eq!(parse("- [ ] a !!! b !! c").priority, TaskPriority::Urgent);
        assert_eq!(parse("- [ ] a !! b !! c").priority, TaskPriority::High);
    }

    #[test]
    fn priority_markers_are_stripped_from_text() {
        assert_eq!(parse("- [ ] a !!! b").text, "a b");
    }

    #[test]
    fn trailing_moved_from_marker_is_discarded() {
        let task = parse("- [ ] carried over < 2026-08-01");
        assert_eq!(task.text, "carried over");
        assert_eq!(task.moved_to, None);
    }

    #[test]
    fn both_trailing_move_markers_on_one_line() {
        let task = parse("- [>] shuffled > 2026-08-04 < 2026-08-01");
        assert_eq!(task.text, "shuffled");
        assert_eq!(task.moved_to, NaiveDate::from_ymd_opt(2026, 8, 4));
    }

    #[test]
    fn mid_line_angle_bracket_is_left_alone() {
        for line in [
            "- [ ] migrate 1.0 > 2.0 today",
            "- [ ] a > b > c",
            "- [ ] compare x>y",
        ] {
            let task = parse(line);
            assert_eq!(task.moved_to, None, "for {line:?}");
            assert!(task.text.contains('>'), "for {line:?}");
        }
    }

    /// The cheap pre-filter in front of [`TRAILING_MOVE`] must never reject a
    /// line the regex would have matched.
    #[test]
    fn trailing_move_prefilter_never_rejects_a_match() {
        for text in [
            "moved > 2026-08-04",
            "moved > 2026-08-04   ",
            "moved >2026-08-04\t",
            "moved < 2026-08-01",
            "> 2026-08-04",
            "nothing here",
            "ends in a digit 42",
            "trailing space ",
            "",
            "   ",
            "a > b",
        ] {
            assert!(
                !TRAILING_MOVE.is_match(text) || ends_with_digit_ignoring_blanks(text),
                "prefilter rejected a line the regex matches: {text:?}"
            );
        }
        // It really does reject: otherwise it would be buying nothing.
        assert!(!ends_with_digit_ignoring_blanks("nothing here"));
        assert!(ends_with_digit_ignoring_blanks("moved > 2026-08-04  "));
    }

    #[test]
    fn trailing_move_needs_a_real_date() {
        let task = parse("- [ ] nope > 2026-13-45");
        assert_eq!(task.moved_to, None);
        assert_eq!(task.text, "nope > 2026-13-45");
    }

    #[test]
    fn trailing_means_after_date_annotations_but_before_tags() {
        // Documented ordering: `@due(..)` is removed first, so the `>` is trailing.
        let with_due = parse("- [ ] moved @due(2026-08-05) > 2026-08-04");
        assert_eq!(with_due.moved_to, NaiveDate::from_ymd_opt(2026, 8, 4));
        assert_eq!(with_due.text, "moved");

        // ...but a trailing tag means the `>` is no longer at the very end.
        let with_tag = parse("- [ ] moved > 2026-08-04 #work");
        assert_eq!(with_tag.moved_to, None);
        assert_eq!(with_tag.text, "moved > 2026-08-04");
        assert_eq!(with_tag.tags, ["work"]);
    }

    // ---- combinations --------------------------------------------------------

    #[test]
    fn every_annotation_on_one_line_in_several_orders() {
        let expected_due = Some(dt(2026, 8, 5, 15, 0));
        let expected_done = Some(dt(2026, 8, 4, 12, 11));
        for line in [
            "- [x] ship it !!! #work #ops @due(2026-08-05 03:00 PM) @done(2026-08-04 12:11 PM)",
            "- [x] @due(2026-08-05 03:00 PM) ship it #work @done(2026-08-04 12:11 PM) !!! #ops",
            "- [x] #work ship it @done(2026-08-04 12:11 PM) !!! #ops @due(2026-08-05 15:00)",
            "- [x] !!! #work #ops @done(2026-08-04 12:11 PM) @due(2026-08-05 15:00) ship it",
        ] {
            let task = parse(line);
            assert_eq!(task.status, TaskStatus::Done, "for {line:?}");
            assert_eq!(task.priority, TaskPriority::Urgent, "for {line:?}");
            assert_eq!(task.tags, ["work", "ops"], "for {line:?}");
            assert_eq!(task.due, expected_due, "for {line:?}");
            assert_eq!(task.done, expected_done, "for {line:?}");
            assert!(task.due_has_time && task.done_has_time, "for {line:?}");
            assert_eq!(task.text, "ship it", "for {line:?}");
        }
    }

    #[test]
    fn whitespace_is_collapsed_in_display_text() {
        assert_eq!(parse("- [ ] a    b\tc").text, "a b c");
    }

    /// The three literal examples from `TASKS_SPEC.md` lines 20-23.
    #[test]
    fn spec_examples_parse_exactly() {
        let first = parse("* [ ] this is a task due tomorrow @due(2026-08-05)");
        assert_eq!(
            first,
            Task {
                line: 1,
                depth: 0,
                status: TaskStatus::Open,
                priority: TaskPriority::Normal,
                text: "this is a task due tomorrow".to_string(),
                tags: vec![],
                due: Some(dt(2026, 8, 5, 0, 0)),
                due_has_time: false,
                done: None,
                done_has_time: false,
                moved_to: None,
            }
        );

        let second = parse("* [x] this task was done yesterday @done(2026-08-04 12:11 PM)");
        assert_eq!(
            second,
            Task {
                line: 1,
                depth: 0,
                status: TaskStatus::Done,
                priority: TaskPriority::Normal,
                text: "this task was done yesterday".to_string(),
                tags: vec![],
                due: None,
                due_has_time: false,
                done: Some(dt(2026, 8, 4, 12, 11)),
                done_has_time: true,
                moved_to: None,
            }
        );

        let third = parse("* [ ] this task is urgent !!! #hotlist #work @due(2026-08-04 03:00 PM)");
        assert_eq!(
            third,
            Task {
                line: 1,
                depth: 0,
                status: TaskStatus::Open,
                priority: TaskPriority::Urgent,
                text: "this task is urgent".to_string(),
                tags: vec!["hotlist".to_string(), "work".to_string()],
                due: Some(dt(2026, 8, 4, 15, 0)),
                due_has_time: true,
                done: None,
                done_has_time: false,
                moved_to: None,
            }
        );
    }

    /// The nested-subtask shape from `TASKS_SPEC.md` lines 41-45.
    #[test]
    fn spec_nested_subtasks_are_independent_tasks() {
        let source =
            "* [ ] parent task\n\t* [ ] broken down subtask 1\n\t* [ ] broken down subtask 2\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].depth, 0);
        assert_eq!(tasks[1].depth, 2);
        assert_eq!(tasks[2].depth, 2);
        assert_eq!(tasks[1].text, "broken down subtask 1");
    }

    // ---- set_marker ----------------------------------------------------------

    #[test]
    fn set_marker_rewrites_only_the_marker() {
        assert_eq!(
            set_marker("- [ ] buy milk", TaskStatus::Done).as_deref(),
            Some("- [x] buy milk")
        );
        assert_eq!(
            set_marker("- [x] buy milk", TaskStatus::Open).as_deref(),
            Some("- [ ] buy milk")
        );
        assert_eq!(
            set_marker("- [ ] buy milk", TaskStatus::Canceled).as_deref(),
            Some("- [-] buy milk")
        );
    }

    #[test]
    fn set_marker_preserves_exotic_spacing_and_annotations() {
        let line =
            "\t\t 12)   [>]    do   the  thing !!! #work @due(2026-08-05 03:00 PM) > 2026-08-04";
        let toggled = set_marker(line, TaskStatus::Done).expect("task line");
        assert_eq!(
            toggled,
            "\t\t 12)   [x]    do   the  thing !!! #work @due(2026-08-05 03:00 PM) > 2026-08-04"
        );
        assert_eq!(toggled.len(), line.len());
    }

    #[test]
    fn set_marker_round_trips() {
        let line = "  * [ ] a task @due(2026-08-05)";
        let done = set_marker(line, TaskStatus::Done).expect("task line");
        let back = set_marker(&done, TaskStatus::Open).expect("task line");
        assert_eq!(back, line);
    }

    #[test]
    fn set_marker_preserves_a_trailing_carriage_return() {
        assert_eq!(
            set_marker("- [ ] windows line\r", TaskStatus::Done).as_deref(),
            Some("- [x] windows line\r")
        );
    }

    #[test]
    fn set_marker_returns_none_for_non_tasks() {
        for line in ["# heading", "- not a task", "- [] x", "", "[ ] bare"] {
            assert!(set_marker(line, TaskStatus::Done).is_none(), "{line:?}");
        }
    }

    // ---- scan_source_tasks ---------------------------------------------------

    #[test]
    fn scan_reports_one_based_line_numbers() {
        let source = "# Title\n\n- [ ] first\n- [x] second\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].line, 3);
        assert_eq!(tasks[1].line, 4);
    }

    #[test]
    fn scan_skips_backtick_fenced_code() {
        let source = "- [ ] real\n\n```\n- [ ] fake\n```\n\n- [x] also real\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.iter().map(|t| t.line).collect::<Vec<_>>(), vec![1, 7]);
    }

    #[test]
    fn scan_skips_tilde_fenced_code_with_info_string() {
        let source = "~~~markdown\n- [ ] fake\n~~~\n- [ ] real\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].line, 4);
    }

    #[test]
    fn scan_respects_longer_closing_fence_rule() {
        // A three-backtick run does not close a four-backtick fence.
        let source = "````\n- [ ] fake\n```\n- [ ] still fake\n````\n- [ ] real\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].text, "real");
    }

    #[test]
    fn scan_skips_fenced_code_with_an_info_string() {
        let source = "```rust\n// - [ ] fake\n- [ ] fake too\n```\n- [ ] real\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].line, 5);
    }

    #[test]
    fn scan_treats_an_unclosed_fence_as_running_to_end_of_file() {
        let source = "```\n- [ ] fake\n- [ ] also fake\n";
        assert!(scan_source_tasks(source).is_empty());
    }

    #[test]
    fn scan_skips_indented_code_blocks() {
        let source = "Some prose.\n\n    - [ ] fake\n\n- [ ] real\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].line, 5);
    }

    #[test]
    fn scan_skips_tab_indented_code_blocks() {
        let source = "Some prose.\n\n\t- [ ] fake\n\n- [ ] real\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].text, "real");
    }

    /// The one place the raw-text scan knowingly gets indented code wrong.
    ///
    /// Recognising an indented code block *inside* a list item needs the list's
    /// content-column, which this pass does not track — and pretending every
    /// four-column line is code would delete the nested-subtask feature. See the
    /// NOTE in [`scan_source_tasks`].
    #[test]
    fn indented_code_inside_a_list_item_is_a_known_false_positive() {
        let source = "- item\n\n      - [ ] this really is code\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 1, "known limitation: code inside a list item");
        assert_eq!(tasks[0].text, "this really is code");
    }

    #[test]
    fn scan_skips_yaml_frontmatter() {
        let source = "---\ntitle: Notes\nchecklist: \"- [ ] fake\"\n---\n\n- [ ] real\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].line, 6);
        assert_eq!(tasks[0].text, "real");
    }

    #[test]
    fn scan_skips_frontmatter_closed_with_dots() {
        let source = "---\nfake: \"- [ ] no\"\n...\n- [ ] real\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].line, 4);
    }

    #[test]
    fn unclosed_frontmatter_opener_is_treated_as_a_thematic_break() {
        // Otherwise a document that opens with a horizontal rule would lose every
        // task in it.
        let source = "---\n\n- [ ] real\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].line, 3);
    }

    #[test]
    fn a_later_triple_dash_is_not_frontmatter() {
        let source = "# Title\n\n---\n\n- [ ] real\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].line, 5);
    }

    #[test]
    fn scan_line_numbers_survive_multi_line_blocks() {
        let source = concat!(
            "---\n",            // 1
            "title: T\n",       // 2
            "---\n",            // 3
            "\n",               // 4
            "- [ ] first\n",    // 5
            "\n",               // 6
            "```js\n",          // 7
            "// - [ ] fake\n",  // 8
            "\n",               // 9
            "```\n",            // 10
            "\n",               // 11
            "    indented\n",   // 12
            "    - [ ] fake\n", // 13
            "\n",               // 14
            "- [x] second\n",   // 15
        );
        let tasks = scan_source_tasks(source);
        assert_eq!(
            tasks.iter().map(|t| t.line).collect::<Vec<_>>(),
            vec![5, 15]
        );
        assert_eq!(tasks[1].status, TaskStatus::Done);
    }

    #[test]
    fn scan_handles_crlf_line_endings() {
        let source = "- [ ] first\r\n```\r\n- [ ] fake\r\n```\r\n- [x] second\r\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].text, "first");
        assert_eq!(tasks[1].text, "second");
        assert_eq!(tasks[1].line, 5);
    }

    #[test]
    fn scan_of_empty_source_is_empty() {
        assert!(scan_source_tasks("").is_empty());
        assert!(scan_source_tasks("\n\n\n").is_empty());
    }

    #[test]
    fn scan_finds_tasks_inside_a_blockquote_fence_correctly() {
        // A fence written inside a blockquote still hides its contents.
        let source = "> ```\n> - [ ] fake\n> ```\n- [ ] real\n";
        let tasks = scan_source_tasks(source);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].text, "real");
    }

    // ---- strip_annotations_across_runs ---------------------------------------

    /// Convenience wrapper so the tests read like the renderer's call site.
    fn across(runs: &[&str]) -> (Vec<String>, Annotations) {
        strip_annotations_across_runs(runs)
    }

    #[test]
    fn runs_keep_the_whitespace_that_separates_inline_formatting() {
        // `fix **this** #bug`: stripping each run alone would trim the trailing
        // space off "fix " and weld the words together.
        let (runs, ann) = across(&["fix ", "this", " #bug"]);
        assert_eq!(runs, ["fix ", "this", ""]);
        assert_eq!(ann.tags, ["bug"]);
    }

    #[test]
    fn a_single_run_matches_strip_annotations_exactly() {
        let (runs, ann) = across(&["ship it !!! #work @due(2026-08-05)"]);
        let (text, expected) = strip_annotations("ship it !!! #work @due(2026-08-05)");
        assert_eq!(runs, [text]);
        assert_eq!(ann, expected);
    }

    #[test]
    fn an_empty_run_list_yields_nothing() {
        let (runs, ann) = across(&[]);
        assert!(runs.is_empty());
        assert_eq!(ann, Annotations::default());
    }

    #[test]
    fn every_annotation_kind_is_found_across_run_boundaries() {
        let (runs, ann) = across(&[
            "do ",
            "the",
            " thing !! #work @due(2026-08-05) > 2026-08-04",
        ]);
        // Only the annotations go; the words around them stay where they were.
        assert_eq!(runs, ["do ", "the", " thing"]);
        assert_eq!(ann.priority, TaskPriority::High);
        assert_eq!(ann.tags, ["work"]);
        assert_eq!(ann.due, Some(dt(2026, 8, 5, 0, 0)));
        assert_eq!(ann.moved_to, NaiveDate::from_ymd_opt(2026, 8, 4));
    }

    #[test]
    fn a_run_boundary_is_not_whitespace_so_it_cannot_invent_a_tag() {
        // `**a**#work` has no space before the `#` in the source, and
        // `strip_annotations` would not call it a tag either.
        let (runs, ann) = across(&["", "a", "#work"]);
        assert!(ann.tags.is_empty());
        assert_eq!(runs.concat(), "a#work");
    }

    #[test]
    fn a_literal_boundary_character_cannot_desynchronise_the_split() {
        // pulldown-cmark 0.13 does not perform CommonMark's U+0000 → U+FFFD
        // replacement, so a NUL really can reach the renderer.
        let (runs, ann) = across(&["a\u{0}b ", "c", " #t"]);
        assert_eq!(runs.len(), 3, "run count must be preserved");
        assert_eq!(runs, ["a\u{fffd}b ", "c", ""]);
        assert_eq!(ann.tags, ["t"]);
    }

    // ---- serde ---------------------------------------------------------------

    #[test]
    fn status_and_priority_serialize_lowercase() {
        let task = parse("- [x] a !!! @due(2026-08-05 15:00)");
        let json = serde_json::to_value(&task).expect("task serializes");
        assert_eq!(json["status"], "done");
        assert_eq!(json["priority"], "urgent");
        assert_eq!(json["due"], "2026-08-05T15:00:00");
        assert_eq!(json["moved_to"], serde_json::Value::Null);
    }

    #[test]
    fn status_and_priority_deserialize_lowercase() {
        let status: TaskStatus = serde_json::from_str("\"canceled\"").expect("status");
        assert_eq!(status, TaskStatus::Canceled);
        let priority: TaskPriority = serde_json::from_str("\"high\"").expect("priority");
        assert_eq!(priority, TaskPriority::High);
    }

    // ---- property tests ------------------------------------------------------

    /// A small alphabet that can actually spell annotations, so the invariants
    /// below are exercised rather than passing vacuously on random Unicode.
    fn taskish_text() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop::sample::select(vec![
                "a", "b", " ", "\t", "#", "@", "!", "(", ")", "[", "]", "-", "*", "+", ">", "<",
                "x", "X", "due", "done", "2026", "08", "05", "12", ":", ".", "0", "1", "PM", "am",
            ]),
            0..40,
        )
        .prop_map(|parts| parts.concat())
    }

    proptest! {
        /// The parser is fed whatever is on disk, so it must not panic on any of it.
        #[test]
        fn parse_task_line_never_panics_on_arbitrary_input(s in ".*") {
            let _ = parse_task_line(&s, 1);
            let _ = set_marker(&s, TaskStatus::Done);
            let _ = scan_source_tasks(&s);
        }

        #[test]
        fn parse_task_line_never_panics_on_taskish_input(s in taskish_text()) {
            let _ = parse_task_line(&s, 1);
            let _ = set_marker(&s, TaskStatus::Done);
        }

        /// No *recognised* annotation survives into the display text.
        ///
        /// The invariant has to be phrased in terms of recognition rather than
        /// the literal `@due(`, because an unparseable payload is deliberately
        /// left verbatim — see [`strip_annotations`]. So the check is: whatever
        /// `@due(...)`-shaped text remains must not parse as a datetime.
        #[test]
        fn display_text_keeps_no_recognised_annotation(s in taskish_text()) {
            let line = format!("- [ ] {s}");
            if let Some(task) = parse_task_line(&line, 1) {
                for caps in DATE_ANNOTATION.captures_iter(&task.text) {
                    let value = group(&caps, "value").unwrap_or("");
                    prop_assert!(
                        parse_datetime(value).is_none(),
                        "recognised annotation survived in {:?}",
                        task.text
                    );
                }
                prop_assert!(
                    !task.text.split_whitespace().any(|t| t == "!!" || t == "!!!"),
                    "standalone priority marker survived in {:?}",
                    task.text
                );
                prop_assert_eq!(task.text.trim(), &task.text, "text is not trimmed");
            }
        }

        /// Annotations built from valid components are always recognised and removed.
        #[test]
        fn constructed_annotations_are_always_stripped(
            body in "[a-z ]{0,24}",
            year in 2000i32..2100,
            month in 1u32..=12,
            day in 1u32..=28,
            hour in 0u32..24,
            minute in 0u32..60,
            urgent in any::<bool>(),
            tag in "[A-Za-z][A-Za-z0-9_-]{0,10}",
        ) {
            let bangs = if urgent { "!!!" } else { "!!" };
            let line = format!(
                "- [ ] {body} @due({year:04}-{month:02}-{day:02} {hour:02}:{minute:02}) {bangs} #{tag}"
            );
            let task = parse_task_line(&line, 1).expect("constructed line is a task");

            prop_assert!(!task.text.contains("@due("));
            prop_assert!(!task.text.contains('#'));
            prop_assert!(!task.text.split_whitespace().any(|t| t == bangs));
            prop_assert_eq!(task.tags, vec![tag]);
            prop_assert_eq!(
                task.priority,
                if urgent { TaskPriority::Urgent } else { TaskPriority::High }
            );
            prop_assert_eq!(task.due, Some(dt(year, month, day, hour, minute)));
            prop_assert!(task.due_has_time);
        }

        /// `set_marker` is a single-byte edit, and the result parses back to the
        /// status that was asked for.
        #[test]
        fn set_marker_is_a_one_byte_edit_that_round_trips(
            indent in "[ \t]{0,6}",
            bullet in prop::sample::select(vec!["-", "*", "+", "1.", "7)"]),
            gap in "[ \t]{1,3}",
            marker in prop::sample::select(vec![' ', 'x', 'X', '-', '>']),
            body in "[a-zA-Z0-9 #@!()-]{0,40}",
            status in prop::sample::select(vec![
                TaskStatus::Open,
                TaskStatus::Done,
                TaskStatus::Canceled,
            ]),
        ) {
            let line = format!("{indent}{bullet}{gap}[{marker}]{gap}{body}");
            let original = parse_task_line(&line, 1).expect("constructed line is a task");
            let rewritten = set_marker(&line, status).expect("constructed line is a task");

            prop_assert_eq!(rewritten.len(), line.len(), "marker edit changed the length");
            prop_assert_eq!(
                rewritten.bytes().zip(line.bytes()).filter(|(a, b)| a != b).count() <= 1,
                true,
                "more than the marker byte changed"
            );

            let reparsed = parse_task_line(&rewritten, 1).expect("rewritten line is still a task");
            prop_assert_eq!(reparsed.status, status);
            // Only the marker moved; everything derived from the text is untouched.
            prop_assert_eq!(&reparsed.text, &original.text);
            prop_assert_eq!(&reparsed.tags, &original.tags);
            prop_assert_eq!(reparsed.depth, original.depth);
            prop_assert_eq!(reparsed.priority, original.priority);
            prop_assert_eq!(reparsed.due, original.due);
        }

        /// The pre-filter guarding [`TRAILING_MOVE`] is only ever allowed to
        /// reject text the regex would also have rejected.
        #[test]
        fn trailing_move_prefilter_is_sound(s in taskish_text()) {
            prop_assert!(
                !TRAILING_MOVE.is_match(&s) || ends_with_digit_ignoring_blanks(&s),
                "prefilter rejected a match in {s:?}"
            );
        }

        /// Splicing runs together never loses or gains one, whatever the text.
        ///
        /// This is the invariant the renderer's inline-formatting support rests
        /// on: one output run per input run, so each `Event::Text` can be
        /// rewritten in place without disturbing the `<strong>`/`<em>`/`<a>`
        /// events between them.
        #[test]
        fn splicing_runs_preserves_their_count(
            runs in proptest::collection::vec(taskish_text(), 0..6)
        ) {
            let borrowed: Vec<&str> = runs.iter().map(String::as_str).collect();
            let (out, _) = strip_annotations_across_runs(&borrowed);
            prop_assert_eq!(out.len(), runs.len());
        }

        /// Whatever the run split, the concatenated display text is the same as
        /// stripping the equivalent single string — so a task's text does not
        /// depend on where the author happened to put a `**`.
        #[test]
        fn splicing_agrees_with_single_string_stripping_on_plain_text(
            words in proptest::collection::vec("[a-z]{1,6}", 1..6),
            split_at in 0usize..6,
        ) {
            let joined = words.join(" ");
            let at = split_at.min(words.len());
            let (head, tail) = words.split_at(at);
            let runs = [
                format!("{} ", head.join(" ")),
                tail.join(" "),
            ];
            let borrowed: Vec<&str> = runs.iter().map(String::as_str).collect();
            let (out, _) = strip_annotations_across_runs(&borrowed);

            let collapsed: String = out.concat().split_whitespace().collect::<Vec<_>>().join(" ");
            let (single, _) = strip_annotations(&joined);
            prop_assert_eq!(collapsed, single);
        }

        /// Every task the scanner returns points at a line that really is that task.
        #[test]
        fn scanned_line_numbers_address_the_right_line(source in taskish_text()) {
            let with_newlines = source.replace('.', "\n");
            let lines: Vec<&str> = with_newlines.lines().collect();
            for task in scan_source_tasks(&with_newlines) {
                let index = usize::try_from(task.line).unwrap_or(usize::MAX);
                let Some(line) = index.checked_sub(1).and_then(|i| lines.get(i)) else {
                    return Err(TestCaseError::fail("line number out of range"));
                };
                let reparsed = parse_task_line(line, task.line);
                prop_assert_eq!(reparsed.as_ref(), Some(&task));
            }
        }
    }
}
