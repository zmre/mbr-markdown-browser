//! Flashcard review history: the pure half of `POST /.mbr/flashcard-review`.
//!
//! A note with `type: flashcard` turns its definition lists into a deck — each
//! term is a card's front, its definitions the back. Spaced-repetition results
//! are kept *in the note itself*, as one more definition of the term:
//!
//! ```markdown
//! What is the capital of France?
//! : Paris.
//! : ___Review History___
//!   * 2026-10-06 13:45 - Again
//!   * 2026-10-09 18:02 - Good
//! ```
//!
//! This module appends one entry to that list (creating the definition when the
//! term has none yet). Like [`crate::tasks::patch_task_line`] it knows nothing
//! about the filesystem or HTTP: the handler reads the file, passes the text and
//! the reviewer's time in, and writes back [`ReviewPatch::source`].
//!
//! # Whose clock
//!
//! The entry has no offset (the format is the user's, and it has to read well
//! as plain text), and the deck replays it as the *browser's* local time. So
//! the time must be the reviewer's wall clock, not the server's: stamped by a
//! server in another time zone, a review would land hours away from where the
//! deck expects it and FSRS's 1-minute/10-minute learning steps would come due
//! at the wrong moment or not at all. The client therefore sends its local time
//! and [`parse_review_time`] checks it — the exact entry format, and within
//! [`REVIEW_TIME_WINDOW_HOURS`] of the server's UTC clock.
//!
//! # Why a real parse, not a line scan
//!
//! Where a term's definitions end is a markdown question, not a textual one: a
//! `: not a definition` line inside a fenced code block, a lazy continuation, a
//! nested list in an answer and a loose list's blank lines all defeat a regex.
//! pulldown-cmark's offset iterator answers it with the same options the
//! renderer uses ([`markdown_options`]), so the writer and the page cannot
//! disagree about which `<dd>`s belong to which `<dt>`.
//!
//! # Scope: top-level definition lists only
//!
//! A definition list nested in a block quote or list item would need every
//! inserted line to repeat the container's `> ` / indentation prefix. The deck
//! (`components/src/flashcards/dom.ts`) collects top-level lists only, and this
//! module refuses anything else with [`FlashcardPatchError::NotATerm`], so the
//! two agree on what a card is.

use std::ops::Range;

use chrono::NaiveDateTime;
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

use crate::errors::{FlashcardPatchError, ReviewTimeError};
use crate::markdown::{LineIndex, markdown_options, strip_bom};
use crate::tasks::{line_span, split_line_terminator};

/// The label written when a term gets its first review.
///
/// `___x___` renders as `<em><strong>x</strong></em>`. Any emphasis wrapping the
/// words "Review History" (case-insensitive) is *recognised* — see
/// [`is_history_label`] — but this is the one spelling mbr writes.
pub const HISTORY_LABEL: &str = "___Review History___";

/// The label's words, compared case-insensitively once emphasis is stripped.
const HISTORY_TEXT: &str = "review history";

/// `YYYY-MM-DD HH:MM`, the reviewer's local wall-clock time without an offset
/// — the same shape as a task's `@done(...)` stamp.
const ENTRY_TIME_FORMAT: &str = "%Y-%m-%d %H:%M";

/// How far a reviewer's wall clock may sit from the server's UTC clock.
///
/// Real offsets run from UTC−12 to UTC+14, so every reviewer is within 14
/// hours; 26 leaves room for a badly-set clock on either side while still
/// refusing a stray epoch, a far-future date or a client that sent something
/// other than "now".
pub const REVIEW_TIME_WINDOW_HOURS: i64 = 26;

/// Validates the reviewer's local time as sent by the deck (`at`).
///
/// Strictly `YYYY-MM-DD HH:MM` — ASCII digits, zero-padded, no seconds, no
/// offset, nothing around it — because the string is written into the note
/// verbatim and has to read back as an entry. Chrono's own parser would accept
/// `2026-1-6 9:05` and a leading `+`, so the shape is checked byte by byte
/// first and chrono only judges the calendar (month 13, February 29th of a
/// common year, `24:00`, minute 60 — there is no minute-precision spelling of a
/// leap second).
///
/// `now_utc` is the server's clock as a naive UTC time, a parameter so the
/// window is testable without mocking it.
///
/// # Errors
///
/// [`ReviewTimeError::Malformed`] for anything that is not a real date and
/// time in the entry format; [`ReviewTimeError::OutOfRange`] for one more than
/// [`REVIEW_TIME_WINDOW_HOURS`] away from `now_utc`.
pub fn parse_review_time(
    at: &str,
    now_utc: NaiveDateTime,
) -> Result<NaiveDateTime, ReviewTimeError> {
    const SHAPE: &[u8; 16] = b"0000-00-00 00:00";
    let shaped = at.len() == SHAPE.len()
        && at.bytes().zip(SHAPE).all(|(byte, &want)| match want {
            b'0' => byte.is_ascii_digit(),
            separator => byte == separator,
        });
    if !shaped {
        return Err(ReviewTimeError::Malformed);
    }
    let time = NaiveDateTime::parse_from_str(at, ENTRY_TIME_FORMAT)
        .map_err(|_| ReviewTimeError::Malformed)?;
    let window = chrono::TimeDelta::hours(REVIEW_TIME_WINDOW_HOURS);
    if (time - now_utc).abs() > window {
        return Err(ReviewTimeError::OutOfRange);
    }
    Ok(time)
}

/// One self-rating, in Anki's four-button vocabulary.
///
/// Readers also accept `Fail` as a synonym for [`Rating::Again`] (that is the
/// frontend's job — it parses the history), but mbr only ever *writes* `Again`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Rating {
    Again,
    Hard,
    Good,
    Easy,
}

impl Rating {
    /// How the rating is spelled in a history entry.
    pub fn label(self) -> &'static str {
        match self {
            Self::Again => "Again",
            Self::Hard => "Hard",
            Self::Good => "Good",
            Self::Easy => "Easy",
        }
    }
}

/// The text of one history entry: `2026-10-06 13:45 - Good`.
pub fn format_entry(reviewed_at: NaiveDateTime, rating: Rating) -> String {
    format!(
        "{} - {}",
        reviewed_at.format(ENTRY_TIME_FORMAT),
        rating.label()
    )
}

/// A markdown source with one review appended by [`append_review`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewPatch {
    /// The whole file, ready to be written back.
    pub source: String,
    /// The entry text, as written (without the bullet).
    pub entry: String,
    /// 1-based line number of the first inserted line.
    pub inserted_at: u32,
    /// The inserted lines, without terminators: the bullet alone, or the
    /// history label followed by the bullet when the term had no history yet.
    ///
    /// Returned so a client holding the file's lines can splice them in rather
    /// than re-read the file, and shift every later line number it holds.
    pub inserted: Vec<String>,
}

impl ReviewPatch {
    /// 1-based line number of the new bullet (always the last inserted line).
    pub fn entry_line(&self) -> u32 {
        let extra = u32::try_from(self.inserted.len().saturating_sub(1)).unwrap_or(0);
        self.inserted_at.saturating_add(extra)
    }
}

/// Appends one review entry to the history of the term on `term_line`.
///
/// `reviewed_at` is the reviewer's local wall-clock time (see [`parse_review_time`]),
/// written as the entry's timestamp.
///
/// `term_line` is 1-based — the `data-mbr-line` the renderer puts on the
/// `<dt>`. `expected` is the line's text as the client last saw it, compared
/// modulo its terminator exactly like `POST /.mbr/task`'s, so an edit to the
/// term since the page was loaded is a conflict rather than a review recorded
/// against the wrong card.
///
/// Where the entry goes:
///
/// * the term has a history definition with a list — a new item right after
///   the list's last item, copying that item's indentation, marker and
///   spacing (so `-` lists stay `-` lists);
/// * a history definition with no list yet — a `* ` item at the definition's
///   content indentation, right after it;
/// * no history definition — `: ___Review History___` plus the item, right
///   after the term's last definition, copying that definition's `: ` prefix.
///
/// Every other byte is untouched: inserted lines use the terminator of the
/// line they follow (so CRLF files stay CRLF), and a file without a trailing
/// newline still has none.
///
/// # Errors
///
/// [`FlashcardPatchError`] — every variant means the client is looking at a
/// stale copy of the file.
///
/// # Examples
///
/// ```
/// use chrono::NaiveDate;
/// use mbr::flashcards::{Rating, append_review};
///
/// let reviewed_at = NaiveDate::from_ymd_opt(2026, 10, 6)
///     .and_then(|d| d.and_hms_opt(13, 45, 0))
///     .unwrap();
/// let source = "Capital of France?\n: Paris.\n";
/// let patch = append_review(source, 1, "Capital of France?", Rating::Good, reviewed_at).unwrap();
/// assert_eq!(
///     patch.source,
///     "Capital of France?\n: Paris.\n: ___Review History___\n  * 2026-10-06 13:45 - Good\n"
/// );
/// assert_eq!(patch.inserted_at, 3);
/// assert_eq!(patch.entry_line(), 4);
/// ```
pub fn append_review(
    source: &str,
    term_line: u32,
    expected: &str,
    rating: Rating,
    reviewed_at: NaiveDateTime,
) -> Result<ReviewPatch, FlashcardPatchError> {
    let span = line_span(source, term_line)
        .ok_or(FlashcardPatchError::LineOutOfRange { line: term_line })?;
    let (content, term_terminator) = split_line_terminator(&source[span]);
    if content != split_line_terminator(expected).0 {
        return Err(FlashcardPatchError::Mismatch { line: term_line });
    }

    let not_a_term = FlashcardPatchError::NotATerm { line: term_line };
    let term = scan_terms(source)
        .into_iter()
        .find(|term| term.line == term_line)
        .ok_or(not_a_term)?;

    let lines = LineIndex::build(source);
    let entry = format_entry(reviewed_at, rating);
    let (after_line, inserted) = match term.definitions.iter().find(|d| d.history) {
        Some(Definition {
            last_item: Some(item),
            ..
        }) => (
            last_line_of(source, &lines, item),
            vec![bullet_after(source, item.start, &entry)],
        ),
        Some(history) => {
            let indent = content_indent(definition_prefix(source, history.range.start));
            (
                last_line_of(source, &lines, &history.range),
                vec![format!("{indent}* {entry}")],
            )
        }
        None => {
            // A title is only ever emitted with at least one definition, so
            // this cannot fail on a list pulldown-cmark produced.
            let last = term.definitions.last().ok_or(not_a_term)?;
            let prefix = definition_prefix(source, last.range.start);
            (
                last_line_of(source, &lines, &last.range),
                vec![
                    format!("{prefix}{HISTORY_LABEL}"),
                    format!("{}* {entry}", content_indent(prefix)),
                ],
            )
        }
    };

    let fallback_terminator = if term_terminator.is_empty() {
        "\n"
    } else {
        term_terminator
    };
    let source = insert_lines_after(source, after_line, &inserted, fallback_terminator);
    Ok(ReviewPatch {
        source,
        entry,
        inserted_at: after_line.saturating_add(1),
        inserted,
    })
}

/// True when inline `text` — the words inside the emphasis that opens a
/// definition — names the review-history definition.
pub fn is_history_label(text: &str) -> bool {
    text.trim().eq_ignore_ascii_case(HISTORY_TEXT)
}

// ============================================================================
// Scanning
// ============================================================================

/// One top-level definition-list term and its definitions.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Term {
    /// 1-based line the term starts on.
    line: u32,
    definitions: Vec<Definition>,
}

/// One `<dd>`, as byte ranges into the source.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Definition {
    range: Range<usize>,
    /// Whether this is the review-history definition.
    history: bool,
    /// The last item of the definition's first directly-nested list.
    last_item: Option<Range<usize>>,
}

/// Whether a definition's leading content names it the history definition.
#[derive(Debug)]
enum Label {
    /// Nothing but block wrappers seen yet.
    Pending,
    /// Inside the leading emphasis opened at `depth`, collecting its text.
    Capturing {
        depth: usize,
        text: String,
    },
    Decided(bool),
}

/// Depth of a definition's own content: `DefinitionList` (0) →
/// `DefinitionListDefinition` (1) → content (2).
const CONTENT_DEPTH: usize = 2;

/// A definition being walked.
#[derive(Debug)]
struct DefinitionScan {
    range: Range<usize>,
    label: Label,
    /// Depth of the first list that is a direct child of the definition.
    list_depth: Option<usize>,
    list_closed: bool,
    last_item: Option<Range<usize>>,
}

impl DefinitionScan {
    fn new(range: Range<usize>) -> Self {
        Self {
            range,
            label: Label::Pending,
            list_depth: None,
            list_closed: false,
            last_item: None,
        }
    }

    /// A `Start` at `depth` (the depth *outside* the tag being opened).
    fn start(&mut self, tag: &Tag<'_>, depth: usize, range: &Range<usize>) {
        if matches!(self.label, Label::Pending) {
            self.label = match tag {
                // A loose definition wraps its content in a paragraph.
                Tag::Paragraph if depth == CONTENT_DEPTH => Label::Pending,
                Tag::Emphasis | Tag::Strong => Label::Capturing {
                    depth,
                    text: String::new(),
                },
                _ => Label::Decided(false),
            };
        }
        match tag {
            Tag::List(_) if depth == CONTENT_DEPTH && self.list_depth.is_none() => {
                self.list_depth = Some(depth);
            }
            Tag::Item if !self.list_closed && self.list_depth.is_some_and(|d| d + 1 == depth) => {
                self.last_item = Some(range.clone());
            }
            _ => {}
        }
    }

    /// An `End` that closes a tag opened at `depth`.
    fn end(&mut self, tag: &TagEnd, depth: usize) {
        if let Label::Capturing {
            depth: opened,
            text,
        } = &self.label
            && *opened == depth
        {
            self.label = Label::Decided(is_history_label(text));
        }
        if matches!(tag, TagEnd::List(_)) && self.list_depth == Some(depth) {
            self.list_closed = true;
        }
    }

    /// Any non-tag event inside the definition.
    fn inline(&mut self, event: &Event<'_>) {
        match &mut self.label {
            Label::Capturing { text, .. } => {
                if let Event::Text(t) | Event::Code(t) = event {
                    text.push_str(t);
                }
            }
            // Leading text before any emphasis: an ordinary answer.
            Label::Pending => self.label = Label::Decided(false),
            Label::Decided(_) => {}
        }
    }

    fn finish(self) -> Definition {
        Definition {
            range: self.range,
            history: matches!(self.label, Label::Decided(true)),
            last_item: self.last_item,
        }
    }
}

/// Every top-level definition-list term in `source`, in document order.
fn scan_terms(source: &str) -> Vec<Term> {
    let body = strip_bom(source);
    let shift = source.len() - body.len();
    let lines = LineIndex::build(source);

    let mut terms = Vec::new();
    let mut depth = 0usize;
    let mut in_top_list = false;
    let mut term: Option<Term> = None;
    let mut definition: Option<DefinitionScan> = None;

    for (event, range) in Parser::new_ext(body, markdown_options()).into_offset_iter() {
        let range = range.start + shift..range.end + shift;
        match event {
            Event::Start(tag) => {
                match (&tag, depth) {
                    (Tag::DefinitionList, 0) => in_top_list = true,
                    (Tag::DefinitionListTitle, 1) if in_top_list => {
                        let next = Term {
                            line: lines.line_of(range.start),
                            definitions: Vec::new(),
                        };
                        terms.extend(term.replace(next));
                    }
                    (Tag::DefinitionListDefinition, 1) if in_top_list => {
                        definition = Some(DefinitionScan::new(range));
                    }
                    _ => {
                        if let Some(scan) = definition.as_mut() {
                            scan.start(&tag, depth, &range);
                        }
                    }
                }
                depth += 1;
            }
            Event::End(tag) => {
                depth = depth.saturating_sub(1);
                match (&tag, depth) {
                    (TagEnd::DefinitionListDefinition, 1) if in_top_list => {
                        if let (Some(scan), Some(open)) = (definition.take(), term.as_mut()) {
                            open.definitions.push(scan.finish());
                        }
                    }
                    (TagEnd::DefinitionList, 0) if in_top_list => {
                        in_top_list = false;
                        terms.extend(term.take());
                    }
                    _ => {
                        if let Some(scan) = definition.as_mut() {
                            scan.end(&tag, depth);
                        }
                    }
                }
            }
            other => {
                if let Some(scan) = definition.as_mut() {
                    scan.inline(&other);
                }
            }
        }
    }
    terms.extend(term);
    terms
}

// ============================================================================
// Source surgery
// ============================================================================

/// Byte offset of the start of the line containing `offset`.
fn line_start(source: &str, offset: usize) -> usize {
    source[..offset]
        .rfind('\n')
        .map_or(0, |newline| newline + 1)
}

/// The text of the line containing `offset`, without its terminator.
fn line_containing(source: &str, offset: usize) -> &str {
    let start = line_start(source, offset);
    let end = source[start..]
        .find('\n')
        .map_or(source.len(), |newline| start + newline);
    source[start..end].trim_end_matches('\r')
}

/// The last line holding anything but whitespace within `range`. A loose
/// block's range can run on over trailing blank lines, and inserting after
/// those would detach the new line from the block it belongs to.
fn last_line_of(source: &str, lines: &LineIndex, range: &Range<usize>) -> u32 {
    let text = &source[range.clone()];
    let content = text.trim_end();
    if content.is_empty() {
        lines.line_of(range.start)
    } else {
        lines.line_of(range.start + content.len() - 1)
    }
}

/// The `: ` prefix (leading blanks, colon, following blanks) of the definition
/// starting at `offset`, or the canonical `": "` if the line has another shape.
fn definition_prefix(source: &str, offset: usize) -> &str {
    let line = line_containing(source, offset);
    let lead = line.len() - line.trim_start_matches([' ', '\t']).len();
    match line[lead..].strip_prefix(':') {
        Some(rest) => {
            let gap = rest.len() - rest.trim_start_matches([' ', '\t']).len();
            if gap == 0 {
                ": "
            } else {
                &line[..lead + 1 + gap]
            }
        }
        None => ": ",
    }
}

/// Indentation that puts a line inside a definition whose first line starts
/// with `prefix` — the prefix with its colon blanked out.
fn content_indent(prefix: &str) -> String {
    prefix.replace(':', " ")
}

/// A list item for `entry` shaped like the item starting at `item_start`: same
/// indentation, marker and gap.
fn bullet_after(source: &str, item_start: usize, entry: &str) -> String {
    let start = line_start(source, item_start);
    // Only whitespace is expected in front of a top-level definition's list
    // item; blank anything else so the column survives regardless.
    let indent: String = source[start..item_start]
        .chars()
        .map(|c| if c == '\t' { '\t' } else { ' ' })
        .collect();
    let rest = line_containing(source, item_start);
    let rest = &rest[item_start - start..];
    let marker_len = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
    let marker = &rest[..marker_len];
    let after = &rest[marker_len..];
    let gap = &after[..after.len() - after.trim_start_matches([' ', '\t']).len()];
    let (marker, gap) = if marker.is_empty() {
        ("*", " ")
    } else if gap.is_empty() {
        (marker, " ")
    } else {
        (marker, gap)
    };
    format!("{indent}{marker}{gap}{entry}")
}

/// `source` with `inserted` placed after its 1-based `after_line`.
///
/// Each inserted line takes the terminator of the line it follows. When that
/// line is the last one and has none, the file gains a terminator *before* the
/// insert (`fallback`), and the insert itself ends unterminated, so the file
/// still does not end in a newline.
fn insert_lines_after(
    source: &str,
    after_line: u32,
    inserted: &[String],
    fallback: &str,
) -> String {
    let added: usize = inserted.iter().map(|line| line.len() + 2).sum();
    let mut out = String::with_capacity(source.len() + added);
    let Some(span) = line_span(source, after_line) else {
        // Unreachable for a line found by scanning `source`; append instead of
        // losing the entry.
        out.push_str(source);
        for line in inserted {
            out.push_str(fallback);
            out.push_str(line);
        }
        return out;
    };
    let (_, terminator) = split_line_terminator(&source[span.clone()]);
    if terminator.is_empty() {
        out.push_str(source);
        for line in inserted {
            out.push_str(fallback);
            out.push_str(line);
        }
    } else {
        out.push_str(&source[..span.end]);
        for line in inserted {
            out.push_str(line);
            out.push_str(terminator);
        }
        out.push_str(&source[span.end..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use proptest::prelude::*;

    fn at(h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 6)
            .and_then(|d| d.and_hms_opt(h, m, 0))
            .expect("valid test time")
    }

    /// Appends a `Good` at 13:45 to the term on `line`, reading `expected`
    /// straight from the source.
    fn append(source: &str, line: u32) -> Result<ReviewPatch, FlashcardPatchError> {
        let expected = source
            .split('\n')
            .nth(line as usize - 1)
            .unwrap_or_default()
            .trim_end_matches('\r');
        append_review(source, line, expected, Rating::Good, at(13, 45))
    }

    fn appended(source: &str, line: u32) -> String {
        append(source, line).expect("append succeeds").source
    }

    // ---- format -------------------------------------------------------------

    #[test]
    fn entry_format_is_minutes_and_capitalised_rating() {
        assert_eq!(
            format_entry(at(9, 5), Rating::Again),
            "2026-10-06 09:05 - Again"
        );
        assert_eq!(
            format_entry(at(23, 59), Rating::Easy),
            "2026-10-06 23:59 - Easy"
        );
    }

    // ---- the reviewer's clock ----------------------------------------------

    /// The server's UTC "now" for the window tests.
    fn utc_now() -> NaiveDateTime {
        at(12, 0)
    }

    #[test]
    fn review_time_accepts_any_real_time_zone() {
        // UTC-12 (Baker Island) to UTC+14 (Kiribati), and UTC itself.
        for at in ["2026-10-06 00:00", "2026-10-06 12:00", "2026-10-07 02:00"] {
            assert_eq!(
                parse_review_time(at, utc_now()).map(|t| format_entry(t, Rating::Good)),
                Ok(format!("{at} - Good")),
                "{at}"
            );
        }
        // The window's edges are inclusive.
        assert!(parse_review_time("2026-10-05 10:00", utc_now()).is_ok());
        assert!(parse_review_time("2026-10-07 14:00", utc_now()).is_ok());
    }

    #[test]
    fn review_time_refuses_times_no_time_zone_can_explain() {
        for at in [
            "2026-10-05 09:59", // 26h01m behind
            "2026-10-07 14:01", // 26h01m ahead
            "1970-01-01 00:00",
            "9999-12-31 23:59",
        ] {
            assert_eq!(
                parse_review_time(at, utc_now()),
                Err(ReviewTimeError::OutOfRange),
                "{at}"
            );
        }
    }

    #[test]
    fn review_time_is_strictly_the_entry_format() {
        for at in [
            "",
            "now",
            "2026-10-06",
            "2026-10-06 12:00:00", // seconds are not stored
            "2026-10-06T12:00",
            "2026-10-06 12:00Z",
            "2026-10-06 12:00 ",
            " 2026-10-06 12:00",
            "2026-10-6 12:00",
            "2026-10-06 2:00",
            "+2026-10-06 12:00",
            "2026-1０-06 12:00", // a full-width digit
            "2026-10-06 12:00 - Good",
        ] {
            assert_eq!(
                parse_review_time(at, utc_now()),
                Err(ReviewTimeError::Malformed),
                "{at:?}"
            );
        }
    }

    #[test]
    fn review_time_refuses_impossible_calendar_values() {
        for at in [
            "2026-13-01 12:00",
            "2026-00-10 12:00",
            "2026-02-29 12:00", // not a leap year
            "2026-10-32 12:00",
            "2026-10-06 24:00",
            "2026-10-06 12:60",
            // A leap second has no minute-precision spelling: 23:59:60 is
            // malformed (no seconds) and 23:60 is not a minute.
            "2016-12-31 23:60",
            "2016-12-31 23:59:60",
        ] {
            assert_eq!(
                parse_review_time(at, utc_now()),
                Err(ReviewTimeError::Malformed),
                "{at}"
            );
        }
    }

    proptest! {
        /// Never panics, and anything it accepts round-trips through the entry
        /// format unchanged — which is what the server writes.
        #[test]
        fn review_time_never_panics_and_round_trips(at in "\\PC{0,24}") {
            if let Ok(time) = parse_review_time(&at, utc_now()) {
                prop_assert_eq!(time.format(ENTRY_TIME_FORMAT).to_string(), at);
            }
        }
    }

    #[test]
    fn rating_deserialises_lowercase_only() {
        let good: Rating = serde_json::from_str("\"good\"").unwrap();
        assert_eq!(good, Rating::Good);
        assert!(serde_json::from_str::<Rating>("\"Good\"").is_err());
        assert!(serde_json::from_str::<Rating>("\"fail\"").is_err());
    }

    #[test]
    fn history_label_matching() {
        assert!(is_history_label("Review History"));
        assert!(is_history_label("  review history "));
        assert!(is_history_label("REVIEW HISTORY"));
        assert!(!is_history_label("Review"));
        assert!(!is_history_label("Review History of Rome"));
    }

    // ---- creating a history -------------------------------------------------

    #[test]
    fn first_review_creates_the_history_definition() {
        let source = "Q?\n: A.\n\nNext?\n: B.\n";
        let patch = append(source, 1).unwrap();
        assert_eq!(
            patch.source,
            "Q?\n: A.\n: ___Review History___\n  * 2026-10-06 13:45 - Good\n\nNext?\n: B.\n"
        );
        assert_eq!(patch.entry, "2026-10-06 13:45 - Good");
        assert_eq!(patch.inserted_at, 3);
        assert_eq!(
            patch.inserted,
            vec![": ___Review History___", "  * 2026-10-06 13:45 - Good"]
        );
        assert_eq!(patch.entry_line(), 4);
    }

    #[test]
    fn history_goes_after_the_last_of_several_answers() {
        let source = "Q?\n: one\n: two\n";
        assert_eq!(
            appended(source, 1),
            "Q?\n: one\n: two\n: ___Review History___\n  * 2026-10-06 13:45 - Good\n"
        );
    }

    #[test]
    fn history_copies_a_wider_definition_prefix() {
        let source = "Q?\n:   A wide answer\n";
        assert_eq!(
            appended(source, 1),
            "Q?\n:   A wide answer\n:   ___Review History___\n    * 2026-10-06 13:45 - Good\n"
        );
    }

    #[test]
    fn loose_definitions_get_the_history_after_the_last_content_line() {
        let source = "Q?\n\n: A.\n\n: B.\n\nNext?\n\n: C.\n";
        assert_eq!(
            appended(source, 1),
            "Q?\n\n: A.\n\n: B.\n: ___Review History___\n  * 2026-10-06 13:45 - Good\n\nNext?\n\n: C.\n"
        );
    }

    #[test]
    fn multi_line_answer_with_nested_list_and_code() {
        let source = concat!(
            "Q?\n",
            ": Steps:\n",
            "  * first\n",
            "  * second\n",
            "\n",
            "  ```\n",
            "  : not a definition\n",
            "  ```\n",
            "\n",
            "Next?\n",
            ": B.\n",
        );
        let patched = appended(source, 1);
        assert_eq!(
            patched,
            concat!(
                "Q?\n",
                ": Steps:\n",
                "  * first\n",
                "  * second\n",
                "\n",
                "  ```\n",
                "  : not a definition\n",
                "  ```\n",
                ": ___Review History___\n",
                "  * 2026-10-06 13:45 - Good\n",
                "\n",
                "Next?\n",
                ": B.\n",
            )
        );
        // And the nested list did not get mistaken for the history.
        let terms = scan_terms(&patched);
        assert_eq!(terms[0].definitions.len(), 2);
        assert!(!terms[0].definitions[0].history);
        assert!(terms[0].definitions[1].history);
    }

    // ---- appending to a history ---------------------------------------------

    #[test]
    fn appends_after_the_last_history_bullet() {
        let source = concat!(
            "Q?\n",
            ": A.\n",
            ": ___Review History___\n",
            "  * 2026-10-01 08:00 - Again\n",
            "  * 2026-10-02 09:00 - Good\n",
            "\n",
            "Next?\n",
            ": B.\n",
        );
        let patch = append(source, 1).unwrap();
        assert_eq!(patch.inserted, vec!["  * 2026-10-06 13:45 - Good"]);
        assert_eq!(patch.inserted_at, 6);
        assert_eq!(patch.entry_line(), 6);
        assert_eq!(
            patch.source,
            concat!(
                "Q?\n",
                ": A.\n",
                ": ___Review History___\n",
                "  * 2026-10-01 08:00 - Again\n",
                "  * 2026-10-02 09:00 - Good\n",
                "  * 2026-10-06 13:45 - Good\n",
                "\n",
                "Next?\n",
                ": B.\n",
            )
        );
    }

    #[test]
    fn keeps_dash_markers_and_wide_gaps() {
        let source = "Q?\n: A.\n: **Review History**\n  -   2026-10-01 08:00 - Hard\n";
        assert_eq!(
            appended(source, 1),
            "Q?\n: A.\n: **Review History**\n  -   2026-10-01 08:00 - Hard\n  -   2026-10-06 13:45 - Good\n"
        );
    }

    #[test]
    fn recognises_single_emphasis_and_any_case() {
        let source = "Q?\n: A.\n: *review history*\n  * 2026-10-01 08:00 - Fail\n";
        let patch = append(source, 1).unwrap();
        assert_eq!(patch.inserted, vec!["  * 2026-10-06 13:45 - Good"]);
    }

    #[test]
    fn plain_text_review_history_is_an_answer_not_a_history() {
        // Without emphasis it is just an answer that happens to say that.
        let source = "Q?\n: Review History\n";
        let patch = append(source, 1).unwrap();
        assert_eq!(patch.inserted.len(), 2, "a real history is created");
    }

    #[test]
    fn label_only_history_gets_its_first_bullet() {
        let source = "Q?\n: A.\n: ___Review History___\n\nNext?\n: B.\n";
        assert_eq!(
            appended(source, 1),
            "Q?\n: A.\n: ___Review History___\n  * 2026-10-06 13:45 - Good\n\nNext?\n: B.\n"
        );
    }

    #[test]
    fn loose_history_list_appends_after_last_item() {
        let source = concat!(
            "Q?\n\n",
            ": A.\n\n",
            ": ___Review History___\n\n",
            "  * 2026-10-01 08:00 - Good\n\n",
            "  * 2026-10-02 08:00 - Easy\n\n",
            "Next?\n\n",
            ": B.\n",
        );
        let patched = appended(source, 1);
        assert!(
            patched.contains("  * 2026-10-02 08:00 - Easy\n  * 2026-10-06 13:45 - Good\n\nNext?"),
            "{patched}"
        );
    }

    #[test]
    fn unparseable_history_lines_are_left_alone() {
        let source = "Q?\n: A.\n: ___Review History___\n  * yesterday, sort of\n";
        assert_eq!(
            appended(source, 1),
            "Q?\n: A.\n: ___Review History___\n  * yesterday, sort of\n  * 2026-10-06 13:45 - Good\n"
        );
    }

    // ---- addressing -------------------------------------------------------

    #[test]
    fn second_term_in_the_same_list_and_terms_in_other_lists() {
        let source = concat!(
            "# Deck\n",       // 1
            "\n",             // 2
            "One?\n",         // 3
            ": 1\n",          // 4
            "\n",             // 5
            "Two?\n",         // 6
            ": 2\n",          // 7
            "\n",             // 8
            "A paragraph.\n", // 9
            "\n",             // 10
            "Three?\n",       // 11
            ": 3\n",          // 12
        );
        assert!(appended(source, 6).contains("Two?\n: 2\n: ___Review History___\n  * "));
        assert!(
            appended(source, 11)
                .ends_with("Three?\n: 3\n: ___Review History___\n  * 2026-10-06 13:45 - Good\n")
        );
        let one = appended(source, 3);
        assert!(
            one.starts_with("# Deck\n\nOne?\n: 1\n: ___Review History___\n"),
            "{one}"
        );
    }

    #[test]
    fn frontmatter_counts_toward_line_numbers() {
        let source = "---\ntype: flashcard\n---\n\nQ?\n: A.\n";
        assert!(
            appended(source, 5)
                .ends_with(": A.\n: ___Review History___\n  * 2026-10-06 13:45 - Good\n")
        );
        // The frontmatter's `type: flashcard` is not a term.
        assert_eq!(
            append(source, 2),
            Err(FlashcardPatchError::NotATerm { line: 2 })
        );
    }

    #[test]
    fn bom_does_not_shift_anything() {
        let source = "\u{feff}---\ntype: flashcard\n---\nQ?\n: A.\n";
        let patched = appended(source, 4);
        assert!(patched.starts_with('\u{feff}'));
        assert!(patched.ends_with(": A.\n: ___Review History___\n  * 2026-10-06 13:45 - Good\n"));
    }

    #[test]
    fn colon_lines_in_code_fences_are_not_terms() {
        let source = "```\nFake?\n: not a definition\n```\n\nReal?\n: yes\n";
        assert_eq!(
            append(source, 2),
            Err(FlashcardPatchError::NotATerm { line: 2 })
        );
        assert!(
            appended(source, 6)
                .ends_with(": yes\n: ___Review History___\n  * 2026-10-06 13:45 - Good\n")
        );
    }

    /// A term inside a ```` ```chat ```` body renders as a real `<dt>` with a
    /// `data-mbr-line`, but in the source it is code-fence text. The writer must
    /// refuse it, which is why the deck treats `.mbr-chat` as nesting.
    #[test]
    fn terms_inside_chat_blocks_are_not_terms() {
        let source = "```chat\n{{Alice|Chat term\n: chat answer\n|}}\n```\n\nReal?\n: yes\n";
        assert_eq!(
            append(source, 2),
            Err(FlashcardPatchError::NotATerm { line: 2 })
        );
        assert!(
            appended(source, 7)
                .ends_with(": yes\n: ___Review History___\n  * 2026-10-06 13:45 - Good\n")
        );
    }

    #[test]
    fn definition_lines_and_prose_are_not_terms() {
        let source = "Prose.\n\nQ?\n: A.\n";
        assert_eq!(
            append(source, 1),
            Err(FlashcardPatchError::NotATerm { line: 1 })
        );
        assert_eq!(
            append(source, 4),
            Err(FlashcardPatchError::NotATerm { line: 4 })
        );
    }

    #[test]
    fn nested_definition_lists_are_refused() {
        let source = "> Q?\n> : A.\n\n- Q2?\n  : A2.\n";
        assert_eq!(
            append(source, 1),
            Err(FlashcardPatchError::NotATerm { line: 1 })
        );
        assert_eq!(
            append(source, 4),
            Err(FlashcardPatchError::NotATerm { line: 4 })
        );
    }

    #[test]
    fn mismatch_and_out_of_range() {
        let source = "Q?\n: A.\n";
        assert_eq!(
            append_review(source, 1, "Q? (edited)", Rating::Good, at(1, 2)),
            Err(FlashcardPatchError::Mismatch { line: 1 })
        );
        assert_eq!(
            append_review(source, 9, "Q?", Rating::Good, at(1, 2)),
            Err(FlashcardPatchError::LineOutOfRange { line: 9 })
        );
        assert_eq!(
            append_review(source, 0, "Q?", Rating::Good, at(1, 2)),
            Err(FlashcardPatchError::LineOutOfRange { line: 0 })
        );
    }

    #[test]
    fn expected_is_compared_modulo_terminator() {
        let source = "Q?\r\n: A.\r\n";
        assert!(append_review(source, 1, "Q?\r\n", Rating::Hard, at(1, 2)).is_ok());
        assert!(append_review(source, 1, "Q?", Rating::Hard, at(1, 2)).is_ok());
    }

    // ---- terminators --------------------------------------------------------

    #[test]
    fn crlf_is_preserved() {
        let source = "Q?\r\n: A.\r\n: ___Review History___\r\n  * 2026-10-01 08:00 - Good\r\n";
        assert_eq!(
            appended(source, 1),
            "Q?\r\n: A.\r\n: ___Review History___\r\n  * 2026-10-01 08:00 - Good\r\n  * 2026-10-06 13:45 - Good\r\n"
        );
        let fresh = "Q?\r\n: A.\r\n";
        assert_eq!(
            appended(fresh, 1),
            "Q?\r\n: A.\r\n: ___Review History___\r\n  * 2026-10-06 13:45 - Good\r\n"
        );
    }

    #[test]
    fn missing_trailing_newline_stays_missing() {
        assert_eq!(
            appended("Q?\n: A.", 1),
            "Q?\n: A.\n: ___Review History___\n  * 2026-10-06 13:45 - Good"
        );
        assert_eq!(
            appended("Q?\r\n: A.", 1),
            "Q?\r\n: A.\r\n: ___Review History___\r\n  * 2026-10-06 13:45 - Good"
        );
        assert_eq!(
            appended(
                "Q?\n: A.\n: ___Review History___\n  * 2026-10-01 08:00 - Good",
                1
            ),
            "Q?\n: A.\n: ___Review History___\n  * 2026-10-01 08:00 - Good\n  * 2026-10-06 13:45 - Good"
        );
    }

    #[test]
    fn repeated_reviews_accumulate() {
        let mut source = "Q?\n: A.\n\nNext?\n: B.\n".to_string();
        for (i, rating) in [Rating::Again, Rating::Hard, Rating::Good, Rating::Easy]
            .into_iter()
            .enumerate()
        {
            source = append_review(&source, 1, "Q?", rating, at(10, i as u32))
                .unwrap()
                .source;
        }
        assert_eq!(
            source,
            concat!(
                "Q?\n",
                ": A.\n",
                ": ___Review History___\n",
                "  * 2026-10-06 10:00 - Again\n",
                "  * 2026-10-06 10:01 - Hard\n",
                "  * 2026-10-06 10:02 - Good\n",
                "  * 2026-10-06 10:03 - Easy\n",
                "\n",
                "Next?\n",
                ": B.\n",
            )
        );
    }

    // ---- properties ---------------------------------------------------------

    /// One generated card: its term, its answers and an optional history.
    #[derive(Debug, Clone)]
    struct GenCard {
        loose: bool,
        answers: Vec<u8>,
        history: Option<(char, Vec<u8>)>,
    }

    /// Answer shapes, by index — single line, multi-line continuation, nested
    /// list, code fence holding a fake definition, and an inline-code answer.
    fn answer_text(kind: u8, n: usize) -> String {
        match kind % 5 {
            0 => format!(": answer {n}"),
            1 => format!(": answer {n} line one\n  and line two"),
            2 => format!(": list {n}:\n  * a{n}\n  * b{n}"),
            3 => format!(": code {n}\n\n  ```\n  : fake {n}\n  Fake term?\n  ```"),
            _ => format!(": `code {n}` answer"),
        }
    }

    fn history_text(marker: char, entries: &[u8]) -> String {
        let mut text = String::from(": ___Review History___");
        for (i, kind) in entries.iter().enumerate() {
            let rating = ["Again", "Fail", "Hard", "Good", "Easy", "garbage"][*kind as usize % 6];
            text.push_str(&format!(
                "\n  {marker} 2026-09-{:02} 10:00 - {rating}",
                i + 1
            ));
        }
        text
    }

    fn gen_card() -> impl Strategy<Value = GenCard> {
        (
            any::<bool>(),
            prop::collection::vec(0u8..5, 1..4),
            prop::option::of((
                prop::sample::select(vec!['*', '-']),
                prop::collection::vec(0u8..6, 0..4),
            )),
        )
            .prop_map(|(loose, answers, history)| GenCard {
                loose,
                answers,
                history,
            })
    }

    /// A document: optional frontmatter, then deflists (separated by a
    /// paragraph or a fenced block of fake cards), with chosen line endings.
    fn gen_document() -> impl Strategy<Value = (String, Vec<u32>)> {
        (
            any::<bool>(),
            prop::collection::vec(prop::collection::vec(gen_card(), 1..4), 1..4),
            any::<bool>(),
            any::<bool>(),
        )
            .prop_map(|(frontmatter, lists, crlf, trailing_newline)| {
                let mut blocks: Vec<String> = Vec::new();
                if frontmatter {
                    blocks.push("---\ntype: flashcard\n---".to_string());
                }
                let mut term_index = 0usize;
                for (list_index, cards) in lists.iter().enumerate() {
                    if list_index > 0 {
                        blocks.push(if list_index % 2 == 0 {
                            "Some prose between lists.".to_string()
                        } else {
                            "```\nFake?\n: fake\n```".to_string()
                        });
                    }
                    for card in cards {
                        term_index += 1;
                        let separator = if card.loose { "\n\n" } else { "\n" };
                        let mut definitions: Vec<String> = card
                            .answers
                            .iter()
                            .map(|kind| answer_text(*kind, term_index))
                            .collect();
                        if let Some((marker, entries)) = &card.history {
                            definitions.push(history_text(*marker, entries));
                        }
                        blocks.push(format!(
                            "Term {term_index}?{separator}{}",
                            definitions.join(separator)
                        ));
                    }
                }
                let mut text = blocks.join("\n\n");
                if trailing_newline {
                    text.push('\n');
                }
                let terms: Vec<u32> = text
                    .split('\n')
                    .enumerate()
                    .filter(|(_, line)| line.starts_with("Term "))
                    .map(|(i, _)| i as u32 + 1)
                    .collect();
                if crlf {
                    text = text.replace('\n', "\r\n");
                }
                (text, terms)
            })
    }

    /// What a card means, independent of where its bytes are: the term line's
    /// text, each answer's trimmed source, and the history item count.
    fn outline(source: &str) -> Vec<(String, Vec<String>, Option<usize>)> {
        scan_terms(source)
            .iter()
            .map(|term| {
                let term_text =
                    line_containing(source, line_span(source, term.line).unwrap().start);
                let answers = term
                    .definitions
                    .iter()
                    .filter(|d| !d.history)
                    .map(|d| source[d.range.clone()].trim_end().to_string())
                    .collect();
                let history = term.definitions.iter().find(|d| d.history).map(|d| {
                    let text = &source[d.range.clone()];
                    text.lines()
                        .filter(|line| line.trim_start().starts_with(['*', '-']))
                        .count()
                });
                (term_text.to_string(), answers, history)
            })
            .collect()
    }

    proptest! {
        /// Appending to one card changes nothing but that card's history,
        /// which gains exactly one entry — and the new entry is findable.
        #[test]
        fn append_adds_exactly_one_history_entry(
            (source, terms) in gen_document(),
            pick in any::<prop::sample::Index>(),
            rating in prop::sample::select(vec![Rating::Again, Rating::Hard, Rating::Good, Rating::Easy]),
        ) {
            let before = outline(&source);
            prop_assert_eq!(before.len(), terms.len(), "generator and scanner agree: {:?}", source);

            let target = pick.index(terms.len());
            let line = terms[target];
            let expected = line_containing(&source, line_span(&source, line).unwrap().start).to_string();
            let patch = append_review(&source, line, &expected, rating, at(13, 45)).unwrap();
            let after = outline(&patch.source);

            prop_assert_eq!(after.len(), before.len());
            for (i, (b, a)) in before.iter().zip(&after).enumerate() {
                prop_assert_eq!(&b.0, &a.0, "term text");
                prop_assert_eq!(&b.1, &a.1, "answers of term {}", i);
                if i == target {
                    prop_assert_eq!(a.2, Some(b.2.unwrap_or(0) + 1), "{}", patch.source);
                } else {
                    prop_assert_eq!(b.2, a.2);
                }
            }

            // The inserted lines are where the patch says they are.
            let lines: Vec<&str> = patch.source.split('\n').map(|l| l.trim_end_matches('\r')).collect();
            for (offset, inserted) in patch.inserted.iter().enumerate() {
                prop_assert_eq!(lines[patch.inserted_at as usize - 1 + offset], inserted.as_str());
            }
            prop_assert!(lines[patch.entry_line() as usize - 1].ends_with(&patch.entry));

            // Terminator style and trailing-newline presence are preserved.
            let crlf = source.contains("\r\n");
            prop_assert_eq!(crlf, patch.source.contains("\r\n"));
            if crlf {
                prop_assert_eq!(
                    patch.source.matches('\n').count(),
                    patch.source.matches("\r\n").count(),
                    "no bare LF in a CRLF file"
                );
            }
            prop_assert_eq!(source.ends_with('\n'), patch.source.ends_with('\n'));
            prop_assert_eq!(
                patch.source.split('\n').count(),
                source.split('\n').count() + patch.inserted.len()
            );
        }

        /// Arbitrary input never panics.
        #[test]
        fn append_never_panics(source in "(?s).{0,200}", line in 0u32..20, expected in ".{0,20}") {
            let _ = append_review(&source, line, &expected, Rating::Good, at(1, 2));
        }

        /// Arbitrary input made of markdown-ish pieces never panics either, and
        /// any success re-parses with the term still in place.
        #[test]
        fn append_on_markdownish_input_never_panics(
            pieces in prop::collection::vec(
                prop::sample::select(vec![
                    "Q?", ": a", ": ___Review History___", "  * x", "  - 2026-01-01 00:00 - Good",
                    "", "```", "> q", "- item", "  ```", ":", "  : nested", "\t* tab", "**Review History**",
                ]),
                0..24,
            ),
            line in 1u32..24,
        ) {
            let source = pieces.join("\n");
            let expected = source.split('\n').nth(line as usize - 1).unwrap_or_default().to_string();
            if let Ok(patch) = append_review(&source, line, &expected, Rating::Easy, at(1, 2)) {
                prop_assert!(scan_terms(&patch.source).iter().any(|t| t.line == line));
            }
        }
    }
}
