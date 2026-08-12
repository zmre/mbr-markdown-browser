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
//! # Incomplete markers
//!
//! A line containing a configured marker word ([`MarkerRule`], from
//! `incomplete_markers`) is a second, read-only kind of entry:
//! [`parse_marker_line`] turns it into a [`TaskKind::Marker`] whose text is the
//! whole line and whose annotations are deliberately not parsed. Nothing here
//! can write one back — [`patch_task_line`] and [`set_marker`] both go through
//! the checkbox grammar — so "read-only" is a property of the code, not a
//! convention.
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

use std::sync::{Arc, LazyLock};

use papaya::HashMap as ConcurrentHashMap;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};

use crate::errors::TaskPatchError;
use crate::wikilink::{
    BlockScanner, LineKind, MarkupCursor, indent_width, is_reference_definition,
};

/// Format of the `@done(...)` timestamp [`set_status`] writes.
///
/// The 24-hour form on purpose: it is the shortest spelling
/// [`parse_datetime`] accepts, so a line mbr stamps parses straight back to the
/// instant it wrote.
const DONE_STAMP_FORMAT: &str = "%Y-%m-%d %H:%M";

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

/// What kind of source line a [`Task`] came from.
///
/// A checkbox is writable and carries the full annotation grammar; an incomplete
/// marker (`TK`, `TODO`, …) is a read-only pointer at a line somebody left
/// unfinished. Both are surfaced by the task browser, so the two have to travel
/// in one type — but almost every behaviour that differs between them keys off
/// this field, which is why it is not an `Option`-shaped afterthought.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    /// A `- [ ]` checkbox line.
    #[default]
    Task,
    /// A line containing an incomplete marker; see [`MarkerRule`].
    Marker,
}

/// One parsed task line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    /// 1-based source line number.
    pub line: u32,
    /// Display indent level; see [`parse_task_line`] for how it is derived.
    pub depth: u8,
    /// Whether this came from a checkbox or from an incomplete marker.
    pub kind: TaskKind,
    /// Completion state.
    pub status: TaskStatus,
    /// Priority, `Normal` unless `!!` or `!!!` appeared.
    pub priority: TaskPriority,
    /// Display text: annotations stripped, whitespace collapsed, trimmed.
    pub text: String,
    /// Where the marker word sits inside [`Task::text`], as **UTF-16 code unit**
    /// indices — which is what a JavaScript string index is, so the panel can
    /// `slice()` with them directly. `None` for a checkbox task.
    ///
    /// Sent rather than re-derived in the browser because the grammar is
    /// markup-aware and the boundary rules are per-alternative; a second
    /// implementation in TypeScript would drift from this one.
    pub marker_start: Option<u32>,
    /// End of the marker word; see [`Task::marker_start`].
    pub marker_end: Option<u32>,
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

/// The compiled `incomplete_markers` rule: one shared answer to "is this an
/// incomplete marker?", used by the renderer that highlights them and by the
/// scanner that indexes them.
///
/// It is a newtype rather than a bare `Regex` so that `regex` stays out of this
/// crate's public API, and so the two consumers cannot drift apart into two
/// notions of what a marker is.
///
/// # Word boundaries
///
/// Boundaries are decided **per alternative**, not once around the group. A
/// single `\b` on the whole alternation demands a word character on the far
/// side of every marker, whatever the marker ends with — so a marker configured
/// as `TODO:` would never match `TODO: foo`, because the byte after the colon is
/// a space. Symmetrically, a leading `\b` would stop `@todo` from matching at the
/// start of a line, and would demand a word character *before* the `@`. So a
/// `\b` is prefixed only when the marker's first character is word-ish and
/// suffixed only when its last one is, where word-ish means
/// `char::is_alphanumeric() || c == '_'` — the definition the regex crate's own
/// Unicode `\b` uses.
///
/// # Longest match
///
/// The regex crate's alternation is leftmost-*first*, not leftmost-longest: in
/// `TODO|TODOMAYBE` the shorter branch would win. Alternatives are therefore
/// sorted longest-first at construction, which makes the result independent of
/// the order the markers appear in the configuration file.
///
/// # No prefilter
///
/// There is deliberately no hand-rolled pre-check of the kind [`might_be_task`]
/// performs. The two situations are not the same. [`might_be_task`] exists to
/// keep [`TASK_LINE`] off the regex crate's *capture-tracking* engine, which is
/// the slow one — roughly 710ns against 70ns, as documented on [`TASK_LINE`]. A
/// [`MarkerRule`] search captures nothing, so it runs on the meta engine, which
/// already extracts the literal alternation and drives a SIMD prefilter over it.
/// A second prefilter in front of that would duplicate work the regex crate does
/// better, and would have to be kept in agreement with the boundary rules above.
#[derive(Debug, Clone)]
pub struct MarkerRule {
    pattern: Regex,
}

impl MarkerRule {
    /// Compiles a rule from the configured markers, or returns `None` when there
    /// is nothing to match.
    ///
    /// Empty entries are dropped, so `incomplete_markers = [""]` is the same off
    /// switch as `incomplete_markers = []` — and `None` is the signal to skip the
    /// pass entirely rather than to run a pattern that matches everywhere.
    ///
    /// Markers are `regex::escape`d, so a marker containing metacharacters
    /// (`FIXME(`) is matched literally instead of breaking compilation.
    pub fn new(markers: &[String]) -> Option<Self> {
        let mut parts: Vec<&str> = markers
            .iter()
            .map(String::as_str)
            .filter(|m| !m.is_empty())
            .collect();
        if parts.is_empty() {
            return None;
        }
        // Stable, so markers of equal length keep their configured order.
        parts.sort_by_key(|m| std::cmp::Reverse(m.len()));

        let alternation = parts
            .iter()
            .map(|m| bounded_alternative(m))
            .collect::<Vec<_>>()
            .join("|");
        // Escaping makes the pattern valid by construction; `ok()` only absorbs
        // the pathological case of a marker list large enough to blow the
        // regex crate's compiled-size limit.
        Regex::new(&format!("(?:{alternation})"))
            .ok()
            .map(|pattern| Self { pattern })
    }

    /// [`MarkerRule::new`], memoised on the marker list that produced it.
    ///
    /// **Use this on any path that runs per file or per request.** Compiling the
    /// pattern costs ~80µs — three to four times what rendering a small page
    /// costs in total — because the regex crate builds an NFA, a lazy DFA and a
    /// literal prefilter. The render pipeline needs a rule for every page it
    /// renders, and `mark_incomplete` is on by default in server and GUI mode,
    /// so compiling per render put that 80µs on the default path for every
    /// request. (Callers that build a rule once and keep it — [`TaskIndex`] does
    /// — should keep calling [`MarkerRule::new`] and own the result.)
    ///
    /// Keyed by the marker list rather than held in a plain `OnceLock` because a
    /// single process legitimately renders with more than one configuration: the
    /// test suite does it constantly, and nothing stops a GUI from opening two
    /// repositories whose `.mbr/config.toml` disagree. A `None` result is cached
    /// too, so a marker list that cannot compile is not retried per page.
    ///
    /// Entries are never evicted. The key space is the set of distinct
    /// `incomplete_markers` values a process ever sees — one in production, a
    /// handful under test — so there is nothing to bound.
    ///
    /// [`TaskIndex`]: crate::task_index::TaskIndex
    pub fn cached(markers: &[String]) -> Option<Arc<Self>> {
        /// Marker list -> the rule it compiles to, or `None` when it compiles to
        /// nothing. The `Option` is part of the value so a miss is cached too.
        type Compiled = ConcurrentHashMap<Box<[String]>, Option<Arc<MarkerRule>>>;

        static COMPILED: LazyLock<Compiled> = LazyLock::new(ConcurrentHashMap::new);

        let compiled = COMPILED.pin();
        if let Some(hit) = compiled.get(markers) {
            return hit.clone();
        }
        // A concurrent miss just compiles twice and one insert wins; the rules
        // are identical, so there is nothing to serialise on.
        let rule = Self::new(markers).map(Arc::new);
        compiled
            .get_or_insert_with(markers.into(), || rule.clone())
            .clone()
    }

    /// Byte length of the marker `text` begins with, or `None` when it begins
    /// with something else.
    ///
    /// This is the renderer's block-initial test, and the direct replacement for
    /// the old `^(?:M1|M2)\b` pattern.
    pub fn block_initial_match(&self, text: &str) -> Option<usize> {
        // Matching is leftmost-first, so if anything matches at 0 this is it.
        // The prefix `\b` cannot spoil a match at offset 0: it is only ever
        // added when the marker starts with a word character, which is exactly
        // when start-of-haystack is a boundary.
        self.pattern
            .find(text)
            .filter(|m| m.start() == 0)
            .map(|m| m.end())
    }

    /// Every marker in `text`, as byte ranges, ascending and non-overlapping.
    pub fn find_iter<'t>(
        &'t self,
        text: &'t str,
    ) -> impl Iterator<Item = std::ops::Range<usize>> + 't {
        self.pattern.find_iter(text).map(|m| m.range())
    }

    /// Byte offset of the first marker in a raw source line that a reader would
    /// actually see, or `None` when the line has none.
    ///
    /// Unlike [`MarkerRule::find_iter`] this understands markdown: a marker
    /// inside a code span, a wikilink target, a link destination or title, an
    /// autolink or a reference definition does not count. See [`MarkupCursor`].
    pub fn find_in_line(&self, line: &str) -> Option<usize> {
        find_marker_outside_markup(line, self).map(|span| span.start)
    }
}

/// One alternation branch: the escaped marker, with a `\b` on each side that the
/// marker's own spelling asks for. See [`MarkerRule`] for why the boundaries are
/// conditional.
fn bounded_alternative(marker: &str) -> String {
    let word_ish = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(marker.len() + 4);

    if marker.chars().next().is_some_and(word_ish) {
        out.push_str(r"\b");
    }
    out.push_str(&regex::escape(marker));
    if marker.chars().next_back().is_some_and(word_ish) {
        out.push_str(r"\b");
    }
    out
}

/// First marker in `line` that falls outside markup, as a byte range.
///
/// The **range**, not just the start, because the panel highlights the marker
/// word inside the card the way the rendered page highlights it in the
/// document, and it cannot recover the end itself: the alternation is
/// longest-first over a configurable list, so which marker matched is only
/// known here.
///
/// **The order of the two passes is the performance design, not an accident.**
/// The regex runs first over the whole line and [`MarkupCursor`] is consulted
/// only about the offsets it returned, because well under one line in a hundred
/// contains a marker word at all — so on the overwhelming majority of lines the
/// markup scanner never runs. Segmenting the line first and matching within the
/// segments would invert that: every line in the repository would be parsed, and
/// the segments would have to be materialised somewhere.
///
/// Only a match's **start** is tested. A marker straddling the edge of a code
/// span is not a thing an author can write, and testing the whole range would
/// mean asking the forward-only cursor two questions per match.
fn find_marker_outside_markup(line: &str, rule: &MarkerRule) -> Option<std::ops::Range<usize>> {
    // A reference definition is a whole-line judgement rather than a span, so it
    // is settled before the cursor is built.
    if is_reference_definition(line) {
        return None;
    }
    let mut cursor = MarkupCursor::new(line);
    // `find_iter` yields ascending, non-overlapping ranges, which is exactly the
    // non-decreasing query order the cursor requires.
    rule.find_iter(line).find(|m| !cursor.excludes(m.start))
}

/// Byte offset within `s` re-expressed as a count of **UTF-16 code units**.
///
/// The panel slices [`Task::text`] with these, and a JavaScript string index
/// *is* a UTF-16 code unit index: a Rust byte offset would land mid-character
/// the moment the line contains anything outside ASCII, so `"café … TODO"`
/// would highlight the wrong span (or, past the end, none at all). Saturating
/// into `u32` costs nothing real — a source line long enough to overflow it is
/// not a line anybody is reading.
fn utf16_offset(s: &str, byte_offset: usize) -> u32 {
    u32::try_from(s[..byte_offset].encode_utf16().count()).unwrap_or(u32::MAX)
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
        kind: TaskKind::Task,
        status,
        priority: annotations.priority,
        text,
        // A checkbox task has no marker word to point at.
        marker_start: None,
        marker_end: None,
        tags: annotations.tags,
        due: annotations.due,
        due_has_time: annotations.due_has_time,
        done: annotations.done,
        done_has_time: annotations.done_has_time,
        moved_to: annotations.moved_to,
    })
}

/// Parses a single line as an incomplete *marker* — a `TK`, `TODO`, `FIXME`
/// that a reader would actually see — returning `None` when it has none.
///
/// The result is a read-only pseudo-task: `status` is [`TaskStatus::Open`],
/// `priority` is [`TaskPriority::Normal`], and `tags`/`due`/`done`/`moved_to`
/// are all empty. Nothing here is inferred from the line, because there is no
/// checkbox to write back to — [`patch_task_line`] refuses a marker line, and
/// [`set_marker`] cannot match one — so an inferred field would be a value the
/// user can never correct.
///
/// # Text
///
/// [`Task::text`] is the **whole line**, whitespace-collapsed and otherwise
/// verbatim: the marker word, any `#tag`, `!!` or `@due(...)` all survive. A
/// marker is a pointer at a line somebody left unfinished, and the annotation
/// grammar is the checkbox grammar — parsing `#docs` out of
/// `TODO: cross-link #docs` would strip text the reader wrote as prose and file
/// the entry under a tag they never applied to a task.
///
/// [`Task::marker_start`] and [`Task::marker_end`] say where in that text the
/// marker word itself sits, so the panel can give it the same wash the rendered
/// page does without re-implementing the grammar.
///
/// # Matching
///
/// Whether a line carries a marker is [`MarkerRule::find_in_line`]'s judgement,
/// which is markup-aware: a marker inside a code span, a wikilink target, a
/// link destination or an autolink does not count. That is the same call the
/// renderer's highlighting makes, so a line that shows a highlight is a line
/// the panel lists, and only that.
///
/// # Depth
///
/// Indentation is measured exactly as [`parse_task_line`] measures it, so a
/// marker nested inside a list indents alongside the tasks around it.
///
/// # Examples
///
/// ```
/// use mbr::tasks::{MarkerRule, TaskKind, TaskStatus, parse_marker_line};
///
/// let rule = MarkerRule::new(&["TODO".to_string()]).expect("one marker compiles");
///
/// let marker = parse_marker_line("  the source is  TODO #docs", 7, &rule).unwrap();
/// assert_eq!(marker.line, 7);
/// assert_eq!(marker.kind, TaskKind::Marker);
/// assert_eq!(marker.status, TaskStatus::Open);
/// assert_eq!(marker.text, "the source is TODO #docs");
/// assert_eq!((marker.marker_start, marker.marker_end), (Some(14), Some(18)));
/// assert!(marker.tags.is_empty());
///
/// // A marker inside a code span is markup, not a marker.
/// assert!(parse_marker_line("call `TODO()` later", 1, &rule).is_none());
/// ```
pub fn parse_marker_line(line: &str, line_number: u32, rule: &MarkerRule) -> Option<Task> {
    // Gate on the RAW line, so what counts as a marker stays decided by the
    // bytes the author actually wrote.
    find_marker_outside_markup(line, rule)?;
    // Same measurement as `parse_task_line`, deliberately: see its `# Depth`.
    let depth = u8::try_from(indent_width(line) / 2).unwrap_or(u8::MAX);
    // Every token survives: markers get no annotation parsing at all.
    let text = collapse_whitespace(line, |_| true);
    // Located a second time, because the raw-line offset does not index `text`
    // once runs of whitespace have been collapsed. Collapsing whitespace cannot
    // change markup structure, so this finds the same occurrence. If it somehow
    // does not, the panel just renders the line without a highlight — a missing
    // wash is invisible, a wrong one is a lie.
    let span = find_marker_outside_markup(&text, rule);

    Some(Task {
        line: line_number,
        depth,
        kind: TaskKind::Marker,
        status: TaskStatus::Open,
        priority: TaskPriority::Normal,
        marker_start: span.as_ref().map(|s| utf16_offset(&text, s.start)),
        marker_end: span.as_ref().map(|s| utf16_offset(&text, s.end)),
        text,
        tags: Vec::new(),
        due: None,
        due_has_time: false,
        done: None,
        done_has_time: false,
        moved_to: None,
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

/// Rewrites the marker of a task line *and* maintains its `@done(...)` stamp.
///
/// `now` is the clock, passed in rather than read, so this stays a pure
/// function of its inputs. `None` disables stamping entirely (the
/// `tasks_stamp_done = false` case), making this exactly [`set_marker`]: an
/// existing `@done(...)` is then left as the author wrote it, because a user who
/// opted out has not asked mbr to curate their annotations.
///
/// With a clock:
///
/// - to [`TaskStatus::Done`], ` @done(<now>)` is appended — unless the line
///   already carries a recognised one, so toggling done twice does not stamp
///   twice;
/// - away from `Done`, every recognised `@done(...)` is removed together with
///   the whitespace in front of it.
///
/// A `@done(...)` whose payload is not a date is not an annotation (see
/// [`strip_annotations`]) and is therefore neither recognised nor removed.
///
/// Everything else survives byte for byte, including any line terminator — a
/// CRLF line stays CRLF.
///
/// # Examples
///
/// ```
/// use chrono::NaiveDate;
/// use mbr::tasks::{TaskStatus, set_status};
///
/// let now = NaiveDate::from_ymd_opt(2026, 8, 4)
///     .and_then(|d| d.and_hms_opt(14, 32, 0))
///     .unwrap();
///
/// let done = set_status("- [ ] write the report !!", TaskStatus::Done, Some(now)).unwrap();
/// assert_eq!(done, "- [x] write the report !! @done(2026-08-04 14:32)");
///
/// // Stamping is idempotent...
/// assert_eq!(set_status(&done, TaskStatus::Done, Some(now)).unwrap(), done);
/// // ...and reversible.
/// assert_eq!(
///     set_status(&done, TaskStatus::Open, Some(now)).unwrap(),
///     "- [ ] write the report !!"
/// );
/// ```
pub fn set_status(line: &str, status: TaskStatus, now: Option<NaiveDateTime>) -> Option<String> {
    let rewritten = set_marker(line, status)?;
    let Some(now) = now else {
        return Some(rewritten);
    };

    let (content, terminator) = split_line_terminator(&rewritten);
    let updated = match status {
        TaskStatus::Done => with_done_stamp(content, now),
        TaskStatus::Open | TaskStatus::Canceled => without_done_stamp(content),
    };
    Some(format!("{updated}{terminator}"))
}

/// A markdown source with exactly one line rewritten by [`patch_task_line`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchedSource {
    /// The whole file, ready to be written back.
    pub source: String,
    /// The new text of the patched line, without its terminator.
    pub text: String,
}

/// Rewrites one task line of a markdown source, checking first that the line is
/// still what the caller last saw.
///
/// This is the whole body of `POST /.mbr/task`, minus the I/O: the caller reads
/// the file, hands the text here, and writes back [`PatchedSource::source`].
/// Keeping it pure is what makes the awkward parts — line addressing, the
/// terminator, the `expected` comparison — testable without a filesystem.
///
/// `line_number` is 1-based. `expected` is compared to the line on disk
/// byte-for-byte *modulo the line terminator*, so a client that captured the
/// line with or without its `\n` (or `\r\n`) is treated the same.
///
/// Only the one line is touched: no other byte of `source` moves, the patched
/// line keeps its own terminator, and a file that did not end in a newline still
/// does not.
///
/// # Examples
///
/// ```
/// use chrono::NaiveDate;
/// use mbr::tasks::{TaskStatus, patch_task_line};
///
/// let now = NaiveDate::from_ymd_opt(2026, 8, 4)
///     .and_then(|d| d.and_hms_opt(14, 32, 0))
///     .unwrap();
/// let source = "# Notes\r\n- [ ] ship it\r\n- [ ] later\r\n";
///
/// let patched =
///     patch_task_line(source, 2, "- [ ] ship it", TaskStatus::Done, Some(now)).unwrap();
/// assert_eq!(patched.text, "- [x] ship it @done(2026-08-04 14:32)");
/// assert_eq!(
///     patched.source,
///     "# Notes\r\n- [x] ship it @done(2026-08-04 14:32)\r\n- [ ] later\r\n"
/// );
/// ```
pub fn patch_task_line(
    source: &str,
    line_number: u32,
    expected: &str,
    status: TaskStatus,
    now: Option<NaiveDateTime>,
) -> Result<PatchedSource, TaskPatchError> {
    let span = line_span(source, line_number)
        .ok_or(TaskPatchError::LineOutOfRange { line: line_number })?;
    let (content, terminator) = split_line_terminator(&source[span.clone()]);

    if content != split_line_terminator(expected).0 {
        return Err(TaskPatchError::Mismatch { line: line_number });
    }
    // `set_status` would refuse too, but checking here means a non-task line is
    // reported as one rather than as a generic failure to rewrite.
    if parse_task_line(content, line_number).is_none() {
        return Err(TaskPatchError::NotATask { line: line_number });
    }
    let text =
        set_status(content, status, now).ok_or(TaskPatchError::NotATask { line: line_number })?;

    let mut patched = String::with_capacity(source.len() + text.len());
    patched.push_str(&source[..span.start]);
    patched.push_str(&text);
    patched.push_str(terminator);
    patched.push_str(&source[span.end..]);

    Ok(PatchedSource {
        source: patched,
        text,
    })
}

/// Byte range of the 1-based `line_number`th line of `source`, terminator
/// included, or `None` when the source has no such line.
///
/// A trailing newline terminates the last line rather than starting an empty
/// one, so `"a\n"` has exactly one line — which is what every editor, and
/// [`str::lines`], also says.
fn line_span(source: &str, line_number: u32) -> Option<std::ops::Range<usize>> {
    let target = usize::try_from(line_number).ok()?.checked_sub(1)?;
    let mut start = 0;
    for (index, line) in source.split_inclusive('\n').enumerate() {
        if index == target {
            return Some(start..start + line.len());
        }
        start += line.len();
    }
    None
}

/// Splits a line into its content and its terminator (`""`, `"\n"`, `"\r\n"`,
/// or a lone `"\r"`).
///
/// The terminator is carried through verbatim rather than normalized: a CRLF
/// file that mbr patches one line of must stay a CRLF file.
fn split_line_terminator(line: &str) -> (&str, &str) {
    let content = line.trim_end_matches(['\r', '\n']);
    (content, &line[content.len()..])
}

/// Appends `@done(now)`, unless a recognised one is already there.
fn with_done_stamp(content: &str, now: NaiveDateTime) -> String {
    if done_annotations(content).next().is_some() {
        return content.to_string();
    }
    // Trailing blanks would otherwise strand a widening gap in front of the
    // stamp every time a task is re-completed.
    let trimmed = content.trim_end_matches([' ', '\t']);
    format!("{trimmed} @done({})", now.format(DONE_STAMP_FORMAT))
}

/// Removes every recognised `@done(...)`, taking the whitespace in front of it
/// so that un-completing a task does not leave a trailing space behind.
fn without_done_stamp(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut copied = 0;

    for range in done_annotations(content) {
        let mut start = range.start;
        while start > copied && matches!(content.as_bytes()[start - 1], b' ' | b'\t') {
            start -= 1;
        }
        out.push_str(&content[copied..start]);
        copied = range.end;
    }
    out.push_str(&content[copied..]);
    out
}

/// Byte ranges of every `@done(...)` in `text` whose payload really is a
/// datetime.
///
/// Shared by the add and remove paths so they cannot disagree about what a
/// stamp is — the idempotence of [`set_status`] rests on exactly that.
fn done_annotations(text: &str) -> impl Iterator<Item = std::ops::Range<usize>> + '_ {
    DATE_ANNOTATION.captures_iter(text).filter_map(|caps| {
        if group(&caps, "kind") != Some("done") {
            return None;
        }
        group(&caps, "value").and_then(parse_datetime)?;
        Some(caps.get(0)?.range())
    })
}

/// Scans a whole markdown source for task lines.
///
/// Checkboxes only; [`scan_source_tasks_with_markers`] is the same pass with
/// incomplete markers turned on.
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
    scan_source_tasks_with_markers(source, None)
}

/// [`scan_source_tasks`], additionally recording incomplete markers.
///
/// `None` scans checkboxes only and is exactly [`scan_source_tasks`]; `Some`
/// also records a [`TaskKind::Marker`] entry for every line carrying a marker
/// the rule recognises. The two skip rules are shared, so a `TODO` inside
/// frontmatter or a fenced code block is no more a marker than a `- [ ]` there
/// is a task.
///
/// # One entry per source line
///
/// The composition below **is** the precedence rule, structurally rather than
/// by inspection:
///
/// ```text
/// parse_task_line(line, n).or_else(|| markers.and_then(|r| parse_marker_line(line, n, r)))
/// ```
///
/// `- [ ] TODO: ship it` is a task and never also a marker, because
/// [`parse_task_line`] answering first short-circuits the `or_else`. And since
/// the whole thing sits inside a `filter_map`, a line can yield at most one
/// entry — which is what makes "one entry per source line" hold by
/// construction, and with it the uniqueness of the `#mbr-marker-{line}` anchor
/// the panel deep-links to.
///
/// # Examples
///
/// ```
/// use mbr::tasks::{MarkerRule, TaskKind, scan_source_tasks_with_markers};
///
/// let rule = MarkerRule::new(&["TODO".to_string()]).expect("one marker compiles");
/// let source = "- [ ] TODO: ship it\n\nthe rest is TODO\n";
/// let found = scan_source_tasks_with_markers(source, Some(&rule));
///
/// assert_eq!(found.len(), 2, "the checkbox line is one entry, not two");
/// assert_eq!(found[0].kind, TaskKind::Task);
/// assert_eq!(found[1].kind, TaskKind::Marker);
/// assert_eq!(found[1].line, 3);
/// ```
pub fn scan_source_tasks_with_markers(source: &str, markers: Option<&MarkerRule>) -> Vec<Task> {
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
            let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
            // See `# One entry per source line`: the `or_else` is the checkbox
            // precedence rule, and the enclosing `filter_map` is the
            // one-entry-per-line rule.
            parse_task_line(line, number)
                .or_else(|| markers.and_then(|rule| parse_marker_line(line, number, rule)))
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

/// Collapses runs of whitespace to single spaces, keeping only the tokens
/// `keep` accepts.
///
/// This is the one definition of what [`Task::text`] is made of, and it is
/// shared on purpose: a checkbox drops its `!!` here while a marker keeps every
/// token, but "whitespace-collapsed and trimmed" has to mean the same thing for
/// both kinds or the panel would render two subtly different sorts of text.
///
/// `keep` is `FnMut` because the priority pass consumes the tokens it rejects.
fn collapse_whitespace(text: &str, mut keep: impl FnMut(&str) -> bool) -> String {
    let mut display = String::with_capacity(text.len());

    for token in text.split_whitespace() {
        if !keep(token) {
            continue;
        }
        if !display.is_empty() {
            display.push(' ');
        }
        display.push_str(token);
    }
    display
}

/// Collapses whitespace to single spaces and lifts out `!!` / `!!!` priority
/// markers in the same pass.
///
/// Splitting on whitespace *is* the "whitespace-delimited on both sides" rule:
/// `wow!!` and `a!!b` are single tokens that are not equal to `!!`, so they never
/// register as priorities.
fn collapse_taking_priority(text: &str) -> (String, TaskPriority) {
    let mut priority = TaskPriority::Normal;
    let display = collapse_whitespace(text, |token| match token {
        "!!!" => {
            priority = priority.max(TaskPriority::Urgent);
            false
        }
        "!!" => {
            priority = priority.max(TaskPriority::High);
            false
        }
        _ => true,
    });
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

    /// The shipped `incomplete_markers` default, which is what almost every
    /// marker test wants.
    const DEFAULT_MARKERS: &[&str] = &["TK", "TODO", "FIXME", "XXX"];

    /// An `&str` marker list as the `Vec<String>` the config carries, so tests
    /// are not littered with `to_string()`.
    fn owned(markers: &[&str]) -> Vec<String> {
        markers.iter().map(|m| (*m).to_string()).collect()
    }

    /// Compiles a [`MarkerRule`] from an `&str` list.
    fn rule(markers: &[&str]) -> MarkerRule {
        MarkerRule::new(&owned(markers)).expect("a non-empty marker list compiles")
    }

    /// The default rule's markup-aware search, which most Stage-2 tests drive.
    fn find(line: &str) -> Option<usize> {
        rule(DEFAULT_MARKERS).find_in_line(line)
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
                kind: TaskKind::Task,
                status: TaskStatus::Open,
                priority: TaskPriority::Normal,
                text: "this is a task due tomorrow".to_string(),
                marker_start: None,
                marker_end: None,
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
                kind: TaskKind::Task,
                status: TaskStatus::Done,
                priority: TaskPriority::Normal,
                text: "this task was done yesterday".to_string(),
                marker_start: None,
                marker_end: None,
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
                kind: TaskKind::Task,
                status: TaskStatus::Open,
                priority: TaskPriority::Urgent,
                text: "this task is urgent".to_string(),
                marker_start: None,
                marker_end: None,
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

    // ---- set_status (the @done stamp) ----------------------------------------

    /// The clock every stamping test is handed, so the expected strings can be
    /// written out literally.
    fn stamp() -> NaiveDateTime {
        dt(2026, 8, 4, 14, 32)
    }

    fn stamped(line: &str, status: TaskStatus) -> String {
        set_status(line, status, Some(stamp())).unwrap_or_else(|| panic!("task line: {line:?}"))
    }

    #[test]
    fn completing_a_task_appends_a_done_stamp() {
        assert_eq!(
            stamped("- [ ] write the report !!", TaskStatus::Done),
            "- [x] write the report !! @done(2026-08-04 14:32)"
        );
    }

    #[test]
    fn a_stamp_round_trips_through_the_parser() {
        // The stamp is only worth writing if the reader agrees it is one.
        let task = parse(&stamped("- [ ] ship it", TaskStatus::Done));
        assert_eq!(task.status, TaskStatus::Done);
        assert_eq!(task.done, Some(stamp()));
        assert!(task.done_has_time);
        assert_eq!(task.text, "ship it");
    }

    #[test]
    fn stamping_is_idempotent() {
        let once = stamped("- [ ] ship it", TaskStatus::Done);
        let twice = stamped(&once, TaskStatus::Done);
        assert_eq!(twice, once, "a second completion must not stamp again");
        assert_eq!(twice.matches("@done(").count(), 1);
    }

    #[test]
    fn an_existing_stamp_is_left_at_the_time_the_author_wrote() {
        // Re-completing an already-done task must not silently re-date it.
        let line = "- [x] ship it @done(2020-01-02 03:04)";
        assert_eq!(stamped(line, TaskStatus::Done), line);
    }

    #[test]
    fn reopening_a_task_removes_the_stamp() {
        let done = stamped("- [ ] ship it", TaskStatus::Done);
        assert_eq!(stamped(&done, TaskStatus::Open), "- [ ] ship it");
        assert_eq!(stamped(&done, TaskStatus::Canceled), "- [-] ship it");
    }

    #[test]
    fn removing_a_stamp_takes_its_leading_whitespace_but_nothing_else() {
        assert_eq!(
            stamped("- [x] a @done(2026-08-04 14:32) b", TaskStatus::Open),
            "- [ ] a b"
        );
        assert_eq!(
            stamped("- [x] a  \t@done(2026-08-04 14:32)", TaskStatus::Open),
            "- [ ] a"
        );
        // A stamp is the only thing that goes; the rest of the line is intact.
        assert_eq!(
            stamped(
                "\t2)  [x]   do  it !!! #work @due(2026-08-05) @done(2026-08-04 14:32)",
                TaskStatus::Open
            ),
            "\t2)  [ ]   do  it !!! #work @due(2026-08-05)"
        );
    }

    #[test]
    fn a_line_that_already_ends_in_annotations_keeps_them_all() {
        let line = "- [ ] file taxes !!! #work #irs @due(2026-08-05 09:00)";
        let done = stamped(line, TaskStatus::Done);
        assert_eq!(
            done,
            "- [x] file taxes !!! #work #irs @due(2026-08-05 09:00) @done(2026-08-04 14:32)"
        );

        // And every one of them still parses out of the stamped line.
        let task = parse(&done);
        assert_eq!(task.text, "file taxes");
        assert_eq!(task.priority, TaskPriority::Urgent);
        assert_eq!(task.tags, ["work", "irs"]);
        assert_eq!(task.due, Some(dt(2026, 8, 5, 9, 0)));
        assert_eq!(task.done, Some(stamp()));

        // ...and reopening puts the line back exactly as it was.
        assert_eq!(stamped(&done, TaskStatus::Open), line);
    }

    #[test]
    fn a_trailing_move_marker_survives_a_stamp() {
        // `> YYYY-MM-DD` is "trailing" only after date annotations are stripped,
        // which is exactly why appending the stamp after it is safe.
        let done = stamped("- [ ] shuffled > 2026-08-04", TaskStatus::Done);
        assert_eq!(done, "- [x] shuffled > 2026-08-04 @done(2026-08-04 14:32)");
        let task = parse(&done);
        assert_eq!(task.text, "shuffled");
        assert_eq!(task.moved_to, NaiveDate::from_ymd_opt(2026, 8, 4));
    }

    #[test]
    fn stamping_does_not_pile_up_trailing_whitespace() {
        assert_eq!(
            stamped("- [ ] ship it   \t ", TaskStatus::Done),
            "- [x] ship it @done(2026-08-04 14:32)"
        );
    }

    #[test]
    fn an_unparseable_done_payload_is_prose_not_a_stamp() {
        // `strip_annotations` leaves it verbatim, so neither may this: the user
        // gets one real stamp added next to their typo, and reopening the task
        // leaves the typo alone.
        let done = stamped("- [ ] a @done(sometime)", TaskStatus::Done);
        assert_eq!(done, "- [x] a @done(sometime) @done(2026-08-04 14:32)");
        assert_eq!(
            stamped(&done, TaskStatus::Open),
            "- [ ] a @done(sometime)",
            "only the recognised stamp is removed"
        );
    }

    #[test]
    fn a_line_terminator_is_preserved_through_a_stamp() {
        for terminator in ["\n", "\r\n", ""] {
            let line = format!("- [ ] ship it{terminator}");
            assert_eq!(
                stamped(&line, TaskStatus::Done),
                format!("- [x] ship it @done(2026-08-04 14:32){terminator}"),
                "terminator {terminator:?}"
            );
            let done = format!("- [x] ship it @done(2026-08-04 14:32){terminator}");
            assert_eq!(
                stamped(&done, TaskStatus::Open),
                format!("- [ ] ship it{terminator}"),
                "terminator {terminator:?}"
            );
        }
    }

    #[test]
    fn without_a_clock_set_status_is_exactly_set_marker() {
        for line in [
            "- [ ] ship it",
            "- [x] ship it @done(2026-08-04 14:32)",
            "- [ ] a @done(2020-01-01 00:00) b\r\n",
        ] {
            for status in [TaskStatus::Open, TaskStatus::Done, TaskStatus::Canceled] {
                assert_eq!(
                    set_status(line, status, None),
                    set_marker(line, status),
                    "{line:?} -> {status:?}"
                );
            }
        }
    }

    #[test]
    fn set_status_returns_none_for_non_tasks() {
        for line in ["# heading", "- not a task", "", "[ ] bare"] {
            assert!(
                set_status(line, TaskStatus::Done, Some(stamp())).is_none(),
                "{line:?}"
            );
        }
    }

    // ---- patch_task_line -----------------------------------------------------

    fn patch(
        source: &str,
        line: u32,
        expected: &str,
        status: TaskStatus,
    ) -> Result<PatchedSource, TaskPatchError> {
        patch_task_line(source, line, expected, status, Some(stamp()))
    }

    #[test]
    fn patch_rewrites_only_the_addressed_line() {
        let source = "# Notes\n\n- [ ] first\n- [ ] second\n";
        let patched = patch(source, 4, "- [ ] second", TaskStatus::Done).expect("patched");
        assert_eq!(patched.text, "- [x] second @done(2026-08-04 14:32)");
        assert_eq!(
            patched.source,
            "# Notes\n\n- [ ] first\n- [x] second @done(2026-08-04 14:32)\n"
        );
    }

    #[test]
    fn patch_preserves_crlf_terminators() {
        let source = "- [ ] first\r\n- [ ] second\r\n";
        let patched = patch(source, 1, "- [ ] first", TaskStatus::Done).expect("patched");
        assert_eq!(
            patched.source,
            "- [x] first @done(2026-08-04 14:32)\r\n- [ ] second\r\n"
        );
        // The response text is the line, not the line plus its terminator.
        assert_eq!(patched.text, "- [x] first @done(2026-08-04 14:32)");
    }

    #[test]
    fn patch_neither_adds_nor_removes_a_trailing_newline() {
        let source = "- [ ] last line has no newline";
        let patched = patch(source, 1, source, TaskStatus::Canceled).expect("patched");
        assert_eq!(patched.source, "- [-] last line has no newline");
        assert!(!patched.source.ends_with('\n'));

        let terminated = "- [ ] terminated\n";
        let patched =
            patch(terminated, 1, "- [ ] terminated", TaskStatus::Canceled).expect("patched");
        assert_eq!(patched.source, "- [-] terminated\n");
    }

    #[test]
    fn patch_accepts_expected_with_or_without_its_terminator() {
        let source = "- [ ] ship it\r\n";
        for expected in ["- [ ] ship it", "- [ ] ship it\n", "- [ ] ship it\r\n"] {
            assert!(
                patch(source, 1, expected, TaskStatus::Done).is_ok(),
                "{expected:?}"
            );
        }
    }

    #[test]
    fn patch_rejects_a_line_that_changed_underneath_the_client() {
        let source = "- [ ] the text changed\n";
        assert_eq!(
            patch(source, 1, "- [ ] what the client saw", TaskStatus::Done),
            Err(TaskPatchError::Mismatch { line: 1 })
        );
        // Whitespace is part of the line: a difference there is still a change.
        assert_eq!(
            patch(source, 1, "- [ ]  the text changed", TaskStatus::Done),
            Err(TaskPatchError::Mismatch { line: 1 })
        );
    }

    #[test]
    fn patch_rejects_a_line_number_the_file_does_not_have() {
        let source = "- [ ] only line\n";
        // A trailing newline terminates line 1; it does not start a line 2.
        assert_eq!(
            patch(source, 2, "", TaskStatus::Done),
            Err(TaskPatchError::LineOutOfRange { line: 2 })
        );
        assert_eq!(
            patch(source, 0, "- [ ] only line", TaskStatus::Done),
            Err(TaskPatchError::LineOutOfRange { line: 0 })
        );
        assert_eq!(
            patch("", 1, "", TaskStatus::Done),
            Err(TaskPatchError::LineOutOfRange { line: 1 })
        );
    }

    #[test]
    fn patch_rejects_a_line_that_is_not_a_task() {
        let source = "# Notes\n- [ ] a task\n";
        assert_eq!(
            patch(source, 1, "# Notes", TaskStatus::Done),
            Err(TaskPatchError::NotATask { line: 1 })
        );
    }

    #[test]
    fn patch_addresses_the_same_lines_the_scanner_reports() {
        // The endpoint's line numbers come from the index, which comes from
        // `scan_source_tasks`; if the two ever disagreed, a toggle would patch
        // the wrong line.
        let source = concat!(
            "---\ntitle: T\n---\n\n",   // 1-4
            "- [ ] first\n",            // 5
            "\n```\n- [ ] fake\n```\n", // 6-9
            "- [ ] second\n",           // 10
        );
        for task in scan_source_tasks(source) {
            let line = source
                .lines()
                .nth(usize::try_from(task.line).expect("fits") - 1)
                .expect("line exists");
            let patched = patch(source, task.line, line, TaskStatus::Done).expect("patched");
            assert!(patched.source.contains("- [ ] fake"), "code stayed code");
            assert_eq!(
                patched.source.matches("@done(").count(),
                1,
                "exactly one line was touched"
            );
        }
    }

    #[test]
    fn line_span_covers_every_line_including_its_terminator() {
        let source = "a\r\nbb\nccc";
        assert_eq!(line_span(source, 1).map(|r| &source[r]), Some("a\r\n"));
        assert_eq!(line_span(source, 2).map(|r| &source[r]), Some("bb\n"));
        assert_eq!(line_span(source, 3).map(|r| &source[r]), Some("ccc"));
        assert_eq!(line_span(source, 4), None);
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

    // ---- MarkerRule ----------------------------------------------------------

    #[test]
    fn the_default_markers_match_at_the_start_of_a_block() {
        let rule = rule(DEFAULT_MARKERS);
        assert_eq!(rule.block_initial_match("TK"), Some(2));
        assert_eq!(rule.block_initial_match("TK rewrite this"), Some(2));
        assert_eq!(rule.block_initial_match("TODO foo"), Some(4));
        assert_eq!(rule.block_initial_match("FIXME(name)"), Some(5));
        assert_eq!(rule.block_initial_match("XXX:"), Some(3));

        // Word boundaries rule out TKTK / TODOs / Tk / lowercase.
        assert_eq!(rule.block_initial_match("TKTK"), None);
        assert_eq!(rule.block_initial_match("TODOs"), None);
        assert_eq!(rule.block_initial_match("Tk"), None);
        assert_eq!(rule.block_initial_match("todo"), None);
        assert_eq!(rule.block_initial_match("Tomato"), None);
    }

    #[test]
    fn a_block_initial_match_must_be_at_the_very_start() {
        let rule = rule(DEFAULT_MARKERS);
        assert_eq!(rule.block_initial_match("see TODO here"), None);
    }

    #[test]
    fn an_empty_marker_list_compiles_to_no_rule() {
        // The documented `incomplete_markers = []` off switch: `None` tells the
        // caller to skip the pass, rather than handing back a rule.
        assert!(MarkerRule::new(&[]).is_none());
        // An entry that is only an empty string is the same off switch.
        assert!(MarkerRule::new(&["".to_string()]).is_none());
    }

    #[test]
    fn caching_hands_back_the_same_rule_for_the_same_markers() {
        // The point of `cached` is that the render pipeline stops paying ~80µs
        // of regex compilation per page, so what matters is that the second call
        // returns the *same allocation*, not merely an equal one.
        let markers = owned(DEFAULT_MARKERS);
        let first = MarkerRule::cached(&markers).expect("the default markers compile");
        let second = MarkerRule::cached(&markers).expect("the default markers compile");
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn caching_keys_on_the_marker_list_not_the_process() {
        // A plain `OnceLock` would be wrong here: the test suite alone renders
        // with several marker sets, and nothing stops a GUI from opening two
        // repositories whose `.mbr/config.toml` disagree.
        let custom = MarkerRule::cached(&owned(&["NOTE"])).expect("one marker compiles");
        let default = MarkerRule::cached(&owned(DEFAULT_MARKERS)).expect("defaults compile");

        assert!(custom.find_in_line("NOTE check this").is_some());
        assert!(custom.find_in_line("TODO check this").is_none());
        assert!(default.find_in_line("TODO check this").is_some());
    }

    #[test]
    fn caching_remembers_a_marker_list_that_compiles_to_nothing() {
        // `None` is cached as a value, so the off switch does not re-attempt
        // compilation on every page.
        assert!(MarkerRule::cached(&[]).is_none());
        assert!(MarkerRule::cached(&[]).is_none());
    }

    #[test]
    fn regex_metacharacters_in_a_marker_are_matched_literally() {
        // Without `regex::escape` the unbalanced `(` would fail to compile and
        // take its sibling markers down with it.
        let rule = rule(&["FOO(", "BAR"]);
        assert_eq!(rule.block_initial_match("BAR foo"), Some(3));
        assert_eq!(rule.block_initial_match("FOO(bar)"), Some(4));
        assert_eq!(rule.block_initial_match("Tomato"), None);
    }

    #[test]
    fn a_marker_ending_in_punctuation_does_not_require_a_word_after_it() {
        // The bug the per-alternative boundaries fix: one `\b` on the whole
        // group demanded a word character after the colon, so this never
        // matched at all.
        let rule = rule(&["TODO:"]);
        let line = "see TODO: here";
        assert_eq!(rule.find_in_line(line), line.find("TODO:"));
        assert_eq!(rule.block_initial_match("TODO: here"), Some(5));
    }

    #[test]
    fn a_marker_starting_with_punctuation_does_not_require_a_word_before_it() {
        let rule = rule(&["@todo"]);
        // A leading `\b` would demand a word character in front of the `@`...
        assert_eq!(rule.find_in_line(" @todo "), Some(1));
        // ...and would equally reject the case where one is present.
        assert_eq!(rule.find_in_line("x@todo"), Some(1));
    }

    #[test]
    fn the_longest_marker_wins_regardless_of_configuration_order() {
        // Alternation is leftmost-first, so without the longest-first sort the
        // answer would depend on how the user happened to order their config.
        for order in [["TODO", "TODOMAYBE"], ["TODOMAYBE", "TODO"]] {
            let rule = rule(&order);
            assert_eq!(
                rule.block_initial_match("TODOMAYBE later"),
                Some(9),
                "for {order:?}"
            );
        }
    }

    // ---- MarkupCursor exclusions ---------------------------------------------

    #[test]
    fn a_marker_embedded_in_prose_is_found() {
        // The motivating case: not block-initial, and invisible to the old rule.
        let line = "The market fell 10% (source: TK).";
        assert_eq!(find(line), line.find("TK"));
    }

    #[test]
    fn a_marker_inside_an_inline_code_span_is_not_found() {
        assert_eq!(find("run `TODO` later"), None);
    }

    #[test]
    fn a_marker_after_an_inline_code_span_is_found() {
        let line = "run `x` then TODO";
        assert_eq!(find(line), line.find("TODO"));
    }

    #[test]
    fn a_longer_backtick_span_hides_a_shorter_run_inside_it() {
        // Only a run of exactly two backticks closes this span, so the single
        // ones are content — which is why the scan needs `code_span_end` rather
        // than a pattern.
        assert_eq!(find("``a ` TODO ` b``"), None);
    }

    #[test]
    fn an_unmatched_backtick_does_not_swallow_the_rest_of_the_line() {
        let line = "a ` b TODO";
        assert_eq!(find(line), line.find("TODO"));
    }

    #[test]
    fn a_marker_inside_a_wikilink_target_is_not_found() {
        assert_eq!(find("see [[TODO list]] here"), None);
    }

    #[test]
    fn a_marker_in_a_wikilink_alias_is_found() {
        // pulldown-cmark emits the alias as `Event::Text`, so a reader sees it.
        let line = "[[page|TODO fix]]";
        assert_eq!(find(line), line.find("TODO"));
    }

    #[test]
    fn an_unclosed_double_bracket_does_not_hide_a_later_marker() {
        let line = "[[page and TODO";
        assert_eq!(find(line), line.find("TODO"));
    }

    #[test]
    fn a_marker_in_a_link_destination_is_not_found() {
        assert_eq!(find("[link](/TODO/page)"), None);
    }

    #[test]
    fn a_marker_in_a_link_title_is_not_found() {
        assert_eq!(find("[link](/page \"TODO\")"), None);
    }

    #[test]
    fn a_marker_in_link_text_is_found() {
        let line = "[TODO fix](/page)";
        assert_eq!(find(line), line.find("TODO"));
    }

    #[test]
    fn a_marker_in_image_alt_text_is_found_but_not_in_its_destination() {
        // No rule of its own: the `](` case fires on the `]` that ends the alt
        // text, so the alt stays included and the destination is excluded.
        let line = "![TODO](/img/shot.png)";
        assert_eq!(find(line), line.find("TODO"));
        assert_eq!(find("![alt](/img/TODO.png)"), None);
    }

    #[test]
    fn a_link_destination_with_balanced_parentheses_is_skipped_whole() {
        let line = "[x](/a(TODO)b) TODO";
        assert_eq!(find(line), line.rfind("TODO"));
    }

    #[test]
    fn an_unclosed_link_destination_does_not_hide_a_later_marker() {
        let line = "[x](/a and TODO";
        assert_eq!(find(line), line.find("TODO"));
    }

    #[test]
    fn a_marker_inside_an_autolink_is_not_found() {
        assert_eq!(find("<https://example.com/TODO>"), None);
    }

    #[test]
    fn a_marker_inside_an_inline_html_tag_is_not_found() {
        assert_eq!(find("a <!--TODO--> b"), None);
        // The delimiters are markup; what sits between two of them is not.
        let line = "<kbd>TODO</kbd>";
        assert_eq!(find(line), line.find("TODO"));
    }

    #[test]
    fn a_less_than_sign_in_prose_is_not_mistaken_for_a_tag() {
        // Whitespace inside the brackets is what tells these apart, and it is
        // also what pulldown-cmark uses to decide this is text.
        let line = "a < b TODO > c";
        assert_eq!(find(line), line.find("TODO"));
    }

    #[test]
    fn a_marker_in_a_bare_url_is_found_the_way_the_renderer_sees_it() {
        // A bare URL is `Event::Text`, so the page highlights it; agreeing with
        // the renderer beats being clever about what the author meant.
        let line = "see https://example.com/TODO now";
        assert_eq!(find(line), line.find("TODO"));
    }

    #[test]
    fn a_reference_link_definition_never_yields_a_marker() {
        assert_eq!(find("[TODO]: /some/path"), None);
        assert_eq!(find("[label]: /TODO/path \"TODO\""), None);
        // Up to three columns of indent is still a definition.
        assert_eq!(find("   [label]: /TODO/path"), None);
    }

    #[test]
    fn a_footnote_definition_is_not_a_reference_definition() {
        // `[^1]:` is a footnote, and its body really is `Event::Text`.
        let line = "[^1]: TODO fix this";
        assert_eq!(find(line), line.find("TODO"));
    }

    #[test]
    fn the_first_qualifying_marker_wins_when_an_excluded_one_comes_first() {
        let line = "`TODO` then FIXME";
        assert_eq!(find(line), line.find("FIXME"));
    }

    // ---- parse_marker_line ---------------------------------------------------

    /// Parses a marker line with the default rule, panicking if it is not one.
    fn marker(line: &str) -> Task {
        parse_marker_line(line, 1, &rule(DEFAULT_MARKERS))
            .unwrap_or_else(|| panic!("expected a marker from {line:?}"))
    }

    #[test]
    fn a_marker_line_becomes_an_open_normal_priority_pseudo_task() {
        let found = marker("The market fell 10% (source: TK).");
        assert_eq!(found.kind, TaskKind::Marker);
        assert_eq!(found.status, TaskStatus::Open);
        assert_eq!(found.priority, TaskPriority::Normal);
        assert_eq!(found.line, 1);
        assert!(found.tags.is_empty());
        assert_eq!(found.due, None);
        assert_eq!(found.done, None);
        assert_eq!(found.moved_to, None);
        assert!(!found.due_has_time && !found.done_has_time);

        // A line with no marker is not one, and neither is a marker hidden in
        // markup — `find_in_line` is the whole test.
        let rule = rule(DEFAULT_MARKERS);
        assert!(parse_marker_line("ordinary prose", 1, &rule).is_none());
        assert!(parse_marker_line("call `TODO()` later", 1, &rule).is_none());
    }

    #[test]
    fn marker_text_is_the_whole_line_whitespace_collapsed() {
        // The marker word itself stays in the text: it is the reason the entry
        // exists, and stripping it would leave a card the reader cannot place.
        assert_eq!(
            marker("  TK   check   this\tagain  ").text,
            "TK check this again"
        );
        // Prose around a mid-line marker is kept on both sides.
        assert_eq!(
            marker("The market fell 10% (source: TK).").text,
            "The market fell 10% (source: TK)."
        );
    }

    #[test]
    fn marker_text_keeps_tags_priority_and_date_annotations_verbatim() {
        // A marker gets no annotation parsing at all: the checkbox grammar is
        // the checkbox's, and applying it here would strip prose the author
        // wrote and file the entry under a tag they never applied to a task.
        let found = marker("TODO cross-link #docs !! before @due(2026-08-05)");
        assert_eq!(
            found.text,
            "TODO cross-link #docs !! before @due(2026-08-05)"
        );
        assert!(found.tags.is_empty());
        assert_eq!(found.priority, TaskPriority::Normal);
        assert_eq!(found.due, None);
    }

    /// `text[start..end]` for a marker, sliced the way the panel slices it —
    /// by UTF-16 code unit, not by byte.
    fn marker_word(found: &Task) -> String {
        let (start, end) = (
            found.marker_start.expect("a marker carries a span") as usize,
            found.marker_end.expect("a marker carries a span") as usize,
        );
        let units: Vec<u16> = found.text.encode_utf16().collect();
        String::from_utf16(&units[start..end]).expect("the span is on a boundary")
    }

    #[test]
    fn a_marker_span_covers_the_marker_word_in_the_display_text() {
        let found = marker("The market fell 10% (source: TK).");
        assert_eq!((found.marker_start, found.marker_end), (Some(29), Some(31)));
        assert_eq!(marker_word(&found), "TK");
    }

    #[test]
    fn a_marker_span_is_in_utf16_units_so_non_ascii_before_it_does_not_shift_it() {
        // The test that fails if these are ever byte offsets: `é` is two bytes
        // and `€`/`—` are three, so the byte offset of `TODO` here is nine past
        // its UTF-16 index — a JavaScript `slice()` with the former would cut
        // the wrong nine characters.
        let found = marker("café costs 5€ — TODO check");
        assert_eq!(marker_word(&found), "TODO");
        assert_eq!((found.marker_start, found.marker_end), (Some(16), Some(20)));
        assert_ne!(
            found.marker_start.unwrap() as usize,
            found.text.find("TODO").unwrap()
        );
    }

    #[test]
    fn a_checkbox_task_carries_no_marker_span() {
        // Not "a span covering nothing": there is no marker word, and a client
        // that highlights on presence must see absence.
        let task = parse_task_line("- [ ] TODO looks like a marker but is a task", 1).unwrap();
        assert_eq!(task.marker_start, None);
        assert_eq!(task.marker_end, None);
    }

    #[test]
    fn a_marker_span_survives_whitespace_collapsing() {
        // The raw-line offset of `TODO` is 7; the collapsed text puts it at 4.
        // Locating twice is what keeps the span an index into `text`.
        let found = marker("foo    TODO   bar");
        assert_eq!(found.text, "foo TODO bar");
        assert_eq!((found.marker_start, found.marker_end), (Some(4), Some(8)));
        assert_eq!(marker_word(&found), "TODO");
    }

    #[test]
    fn a_marker_span_points_at_the_occurrence_the_rule_accepted() {
        // The first `TODO` is inside a code span, so it is not the marker — and
        // a client re-deriving the position with a naive `indexOf` would wash
        // the wrong word. This is the case that makes sending the span the
        // point of the exercise.
        let found = marker("Set `TODO` in config and TK fix it");
        assert_eq!(marker_word(&found), "TK");
        assert_eq!((found.marker_start, found.marker_end), (Some(25), Some(27)));
    }

    #[test]
    fn marker_depth_comes_from_indentation_like_a_task() {
        // Same measurement `parse_task_line` makes, so a marker inside a list
        // indents alongside the tasks around it.
        assert_eq!(marker("TK top level").depth, 0);
        assert_eq!(marker("  TK two spaces").depth, 1);
        assert_eq!(marker("    TK four spaces").depth, 2);
        assert_eq!(marker("\tTK one tab").depth, 2);
        assert_eq!(marker("- TK inside a bullet").depth, 0);
    }

    #[test]
    fn a_marker_line_cannot_be_patched() {
        // The read-only guarantee, pinned at the API rather than left to the
        // frontend: `patch_task_line` goes through `parse_task_line`, which
        // never produces a marker, so the endpoint answers 400.
        let source = "notes\nTODO: write this up\n";
        let error = patch_task_line(
            source,
            2,
            "TODO: write this up",
            TaskStatus::Done,
            Some(dt(2026, 8, 4, 14, 32)),
        )
        .expect_err("a marker line is not patchable");
        assert_eq!(error, TaskPatchError::NotATask { line: 2 });

        // ...and the lower-level rewrite refuses it too, so there is no way in.
        assert!(set_marker("TODO: write this up", TaskStatus::Done).is_none());
        assert!(set_status("TODO: write this up", TaskStatus::Done, None).is_none());
    }

    // ---- scan_source_tasks_with_markers --------------------------------------

    /// Scans with the default marker rule.
    fn scan_with_markers(source: &str) -> Vec<Task> {
        scan_source_tasks_with_markers(source, Some(&rule(DEFAULT_MARKERS)))
    }

    #[test]
    fn a_checkbox_line_that_also_says_todo_is_one_task_not_two() {
        let found = scan_with_markers("- [ ] TODO: ship it\n");
        assert_eq!(found.len(), 1, "checkbox wins, and yields one entry");
        assert_eq!(found[0].kind, TaskKind::Task);
        // Parsed as a task, annotations and all.
        assert_eq!(found[0].text, "TODO: ship it");
        assert_eq!(found[0].status, TaskStatus::Open);
    }

    #[test]
    fn scanning_without_a_marker_rule_matches_the_old_behaviour() {
        let source = concat!(
            "# TODO list\n",
            "\n",
            "- [ ] real task\n",
            "The market fell 10% (source: TK).\n",
            "- [x] FIXME later\n",
        );
        assert_eq!(
            scan_source_tasks(source),
            scan_source_tasks_with_markers(source, None)
        );
        // ...and the rule really would have found something, so the equality
        // above is not vacuous.
        assert!(scan_with_markers(source).len() > scan_source_tasks(source).len());
    }

    #[test]
    fn markers_and_tasks_come_back_in_source_order() {
        let source = concat!(
            "TK intro needs a source\n", // 1
            "\n",                        // 2
            "- [ ] real task\n",         // 3
            "prose with a FIXME here\n", // 4
            "- [x] done thing\n",        // 5
        );
        let found = scan_with_markers(source);
        assert_eq!(
            found.iter().map(|t| (t.line, t.kind)).collect::<Vec<_>>(),
            vec![
                (1, TaskKind::Marker),
                (3, TaskKind::Task),
                (4, TaskKind::Marker),
                (5, TaskKind::Task),
            ]
        );
    }

    #[test]
    fn markers_are_skipped_inside_fenced_code() {
        // The fence state machine is shared with the checkbox scan, so a code
        // sample full of `// TODO:` stays invisible.
        let source =
            "TK real\n\n```rust\n// TODO: fake\nlet x = 1; // FIXME fake\n```\n\nXXX also real\n";
        let found = scan_with_markers(source);
        assert_eq!(found.iter().map(|t| t.line).collect::<Vec<_>>(), vec![1, 8]);

        // An indented code block is skipped too.
        let indented = "Some prose.\n\n    TODO: fake\n\nTK real\n";
        assert_eq!(
            scan_with_markers(indented)
                .iter()
                .map(|t| t.line)
                .collect::<Vec<_>>(),
            vec![5]
        );
    }

    #[test]
    fn markers_are_skipped_inside_yaml_frontmatter() {
        let source = "---\ntitle: TODO pick a title\nstatus: TK\n---\n\nTK real\n";
        let found = scan_with_markers(source);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 6);
        assert_eq!(found[0].text, "TK real");
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

    /// Lines for the marker properties: arbitrary text, plus text drawn from an
    /// alphabet that can actually spell markup and markers.
    ///
    /// The union is deliberate. `.*` gets the multi-byte characters that stress
    /// the byte-wise scan's char-boundary handling, but almost never happens to
    /// spell `](` or a marker word, so on its own it would exercise
    /// [`MarkupCursor`] barely at all.
    fn markerish_line() -> impl Strategy<Value = String> {
        prop_oneof![
            ".*",
            proptest::collection::vec(
                prop::sample::select(vec![
                    "a", " ", "`", "[", "]", "(", ")", "<", ">", "|", "\"", "\\", ":", "!", "TK",
                    "TODO", "FIXME", "XXX", "todo", "é",
                ]),
                0..40,
            )
            .prop_map(|parts| parts.concat()),
        ]
    }

    proptest! {
        /// Every offset the scanner reports lands on a character boundary and on
        /// a configured marker.
        ///
        /// The offset is handed straight to a slice by the renderer, so a stale
        /// or mid-character one is a panic in production.
        #[test]
        fn a_returned_marker_offset_always_lands_on_a_marker(line in markerish_line()) {
            if let Some(at) = rule(DEFAULT_MARKERS).find_in_line(&line) {
                prop_assert!(
                    line.is_char_boundary(at),
                    "offset {} splits a character in {:?}", at, line
                );
                prop_assert!(
                    DEFAULT_MARKERS.iter().any(|m| line[at..].starts_with(m)),
                    "offset {} in {:?} is not a marker", at, line
                );
            }
        }

        /// A marker inside a code span is never reported, whatever surrounds it.
        ///
        /// The alphabet contains neither a backtick nor a marker, so the span in
        /// the middle is the only candidate the generated line can contain.
        #[test]
        fn a_marker_wrapped_in_a_code_span_is_never_found(
            prefix in "[a-z ]{0,20}",
            suffix in "[a-z ]{0,20}",
        ) {
            let line = format!("{prefix}`x TODO y`{suffix}");
            prop_assert_eq!(rule(DEFAULT_MARKERS).find_in_line(&line), None);
        }

        /// The scanner is fed whatever is on disk, so it must not panic on any
        /// of it — including bytes that are not markdown at all.
        #[test]
        fn finding_a_marker_never_panics_on_arbitrary_input(line in markerish_line()) {
            let rule = rule(DEFAULT_MARKERS);
            let _ = rule.find_in_line(&line);
            let _ = rule.block_initial_match(&line);
            let _ = rule.find_iter(&line).count();
        }

        /// The marker scan is fed whatever is on disk too, and it runs over
        /// every line of every file rather than only over candidate ones.
        #[test]
        fn scanning_with_markers_never_panics_on_arbitrary_input(s in ".*") {
            let rule = rule(DEFAULT_MARKERS);
            let _ = scan_source_tasks_with_markers(&s, Some(&rule));
            let _ = parse_marker_line(&s, 1, &rule);
        }

        /// A source line yields at most one entry, so line numbers come back
        /// strictly increasing.
        ///
        /// This is the invariant the `#mbr-marker-{line}` anchor rests on: two
        /// entries for one line would be two cards pointing at one id, and one
        /// of them could never be reached.
        #[test]
        fn every_source_line_yields_at_most_one_entry(source in markerish_line()) {
            let with_newlines = source.replace(' ', "\n");
            let found = scan_source_tasks_with_markers(
                &with_newlines,
                Some(&rule(DEFAULT_MARKERS)),
            );
            for pair in found.windows(2) {
                prop_assert!(
                    pair[0].line < pair[1].line,
                    "line {} repeated or out of order in {:?}",
                    pair[1].line,
                    with_newlines
                );
            }
        }

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

        /// Completing a task and then reopening it restores the line exactly,
        /// as long as it carried no stamp to begin with.
        ///
        /// This is the round trip the UI performs every time somebody mis-clicks
        /// a checkbox, so anything it corrupts, it corrupts in the user's file.
        #[test]
        fn stamping_and_unstamping_restores_the_original_line(
            indent in "[ \t]{0,4}",
            bullet in prop::sample::select(vec!["-", "*", "+", "1.", "7)"]),
            body in "[a-zA-Z0-9#@!>< -]{0,40}",
            terminator in prop::sample::select(vec!["", "\n", "\r\n"]),
            reopen_as in prop::sample::select(vec![TaskStatus::Open, TaskStatus::Canceled]),
        ) {
            // Trailing blanks are deliberately not preserved (the stamp would
            // strand them), so keep them out of the generated line.
            let content = format!("{indent}{bullet} [ ] {body}");
            let line = format!("{}{terminator}", content.trim_end());
            prop_assume!(parse_task_line(&line, 1).is_some());
            prop_assume!(done_annotations(&line).next().is_none());

            let now = dt(2026, 8, 4, 14, 32);
            let done = set_status(&line, TaskStatus::Done, Some(now)).expect("task line");
            prop_assert_eq!(
                parse_task_line(&done, 1).and_then(|t| t.done),
                Some(now),
                "the stamp must be readable back out of {:?}", done
            );

            let reopened = set_status(&done, reopen_as, Some(now)).expect("task line");
            let expected = set_marker(&line, reopen_as).expect("task line");
            prop_assert_eq!(reopened, expected);
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
