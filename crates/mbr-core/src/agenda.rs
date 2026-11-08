//! "Might this note have a day agenda?", answered from its markdown source.
//!
//! The agenda grammar itself (design/calendar.md §4.4) lives in **one** place,
//! `components/src/calendar/agenda-parse.ts`, which reads the *rendered* note.
//! Rust never parses agenda entries. What it needs is a cheap yes/no that
//! decides whether a QuickLook preview inlines the calendar script at all, so
//! an ordinary note pays nothing for it.
//!
//! [`may_have_agenda`] is that question, and it must be a **superset** of the
//! grammar: a note it rejects can never have an entry. It errs only towards
//! "yes"; the script then finds no entry and draws nothing.
//!
//! A line scan over unindented `- 9:00 …` lines is not a superset (a top-level
//! item may be indented up to three spaces, `- **09:00** Standup` renders a
//! time the source does not start with, a task's `!!` is stripped from its
//! display text), so the source is parsed with the renderer's own options and
//! each top-level list item's first line of text is tested, the way
//! `agenda-parse.ts`'s `leadingText` reads the DOM. Before any parse, two
//! cheap passes reject almost every note: one regex over the whole source
//! for anything time-like at all, then one over the **list-item lines** only
//! ([`ITEM_LINE`], [`item_line_may_have_time`]), so a time in prose, a
//! heading or a frontmatter timestamp no longer forces the parse.
//!
//! `tests/fixtures/agenda-prefilter-cases.json` is the shared table: mbr-ffi's
//! QuickLook tests render each case and hold this function to it, and
//! `components/src/calendar/agenda-prefilter-cases.test.ts` runs the TS grammar
//! over the same rendered HTML.

use crate::frontmatter;
use crate::markdown::markdown_options;
use crate::tasks;
use pulldown_cmark::{Event, Tag, TagEnd};
use regex::Regex;
use std::sync::LazyLock;

/// Anything time-like anywhere, or an HTML character reference (which could
/// spell one: `&#58;`, `&colon;`). A source without a match has no agenda and
/// is not parsed.
static TIME_ANYWHERE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\d[:.]\d{2}|\d\s*[ap]\.?m|noon|midnight|&(?:#|[a-z]+;)")
        .expect("time-anywhere regex is valid")
});

/// A line that can start a top-level list item: up to three spaces, a bullet
/// (`-`, `*`, `+`) or an ordered marker (`1.`, `1)`), then whitespace or the
/// end of the line. Group 1 is the rest of the line. A deeper indent is code
/// or a nested item, a `>` a quoted one; neither is an agenda. CRLF-aware.
static ITEM_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?mR)^ {0,3}(?:[-*+]|\d{1,9}[.)])(?:[ \t]+(.*))?$")
        .expect("item line regex is valid")
});

/// Link destinations and reference labels (`](x)`, `][ref]`): not text.
static LINK_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\]\([^)]*\)|\]\[[^\]]*\]").expect("link tail regex is valid"));

/// The main bundle's `AGENDA_PREFILTER` (`components/src/calendar/agenda-scope.ts`):
/// a cheap superset of the grammar, anchored at the start of an item's text.
/// No trailing `\b` and `\s*` before am/pm, as Phase 2 settled it, because
/// `9:00am` and `9  am` are entries.
static AGENDA_PREFILTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(?:(?:at|from)\s+)?(?:\d{1,2}[:.]\d{2}|\d{1,2}\s*[ap]\.?m|noon|midnight)")
        .expect("agenda prefilter regex is valid")
});

/// A task box the renderer turns into a checkbox but pulldown-cmark leaves as
/// text (`[-]`, `[>]`, …; `[ ]`/`[x]` arrive as `TaskListMarker`).
static TASK_BOX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\[[^\]]\]\s+").expect("task box regex is valid"));

/// True unless the note certainly has no agenda entry: no top-level list item
/// whose first line starts with something time-like.
pub fn may_have_agenda(source: &str) -> bool {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    if !TIME_ANYWHERE.is_match(source) {
        return false;
    }
    // A raw-HTML list (`<ul><li>09:00 …`) is a top-level list in the DOM but
    // `Html` events here. Rare enough to accept wholesale rather than parse.
    if contains_li_tag(source) {
        return true;
    }
    ITEM_LINE
        .captures_iter(source)
        .any(|line| item_line_may_have_time(line.get(1).map_or("", |rest| rest.as_str())))
        && top_level_item_lines(source).any(|line| starts_time_like(&line))
}

/// Whether the list-item line whose text after the marker is `rest` could
/// give its item a time-like first line: a superset, by the line alone, of
/// what [`top_level_item_lines`] can read there.
///
/// Yes when the line holds markup that hides text or spells it (`<` raw HTML
/// or an autolink, `&` a character reference, `!` an image or a `!!`
/// priority, `\` an escape, `[^` a footnote reference); when, with emphasis,
/// code and link punctuation and a task box removed, it holds a time token
/// anywhere (annotations stripped too, as a task displays); or when it holds
/// no text at all, since a blank first line goes on to the item's next line
/// (`-` alone, then `  09:00 Standup`).
fn item_line_may_have_time(rest: &str) -> bool {
    if rest.contains(['<', '&', '!', '\\']) || rest.contains("[^") {
        return true;
    }
    let unboxed = TASK_BOX.replace(rest, "");
    let text: String = LINK_TAIL
        .replace_all(&unboxed, "")
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '~' | '`' | '[' | ']'))
        .collect();
    !text.chars().any(char::is_alphanumeric)
        || TIME_ANYWHERE.is_match(&text)
        || TIME_ANYWHERE.is_match(&tasks::strip_annotations(&text).0)
}

/// Whether one item's first line passes the prefilter, as written or as the
/// renderer displays a task (box and annotations removed). [`TASK_BOX`] is
/// tried only on a line that starts with one.
fn starts_time_like(line: &str) -> bool {
    if AGENDA_PREFILTER.is_match(line) {
        return true;
    }
    let unboxed = if line.trim_start().starts_with('[') {
        TASK_BOX.replace(line, "")
    } else {
        std::borrow::Cow::Borrowed(line)
    };
    AGENDA_PREFILTER.is_match(&unboxed)
        || AGENDA_PREFILTER.is_match(&tasks::strip_annotations(&unboxed).0)
}

/// Whether `source` holds a real `<li` start tag: `<li`, ASCII
/// case-insensitive, followed by whitespace, `>` or `/` (the bytes that end a
/// tag name), so `<link>`, `<listing>` and the like do not force the full
/// parse. A `<li` with nothing after it is no tag at all.
fn contains_li_tag(source: &str) -> bool {
    source.as_bytes().windows(4).any(|w| {
        w[0] == b'<'
            && w[1].eq_ignore_ascii_case(&b'l')
            && w[2].eq_ignore_ascii_case(&b'i')
            && (w[3].is_ascii_whitespace() || w[3] == b'>' || w[3] == b'/')
    })
}

/// A block that can hold a list, as far as "top-level" is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    List,
    Item,
    /// Blockquote, footnote definition, definition list, table: a list inside
    /// one is never an agenda (`AGENDA_NESTING_SELECTOR`).
    Nesting,
}

fn container_of(tag: &Tag<'_>) -> Option<Container> {
    match tag {
        Tag::List(_) => Some(Container::List),
        Tag::Item => Some(Container::Item),
        Tag::BlockQuote(_) | Tag::FootnoteDefinition(_) | Tag::DefinitionList | Tag::Table(_) => {
            Some(Container::Nesting)
        }
        _ => None,
    }
}

fn container_of_end(tag: &TagEnd) -> Option<Container> {
    match tag {
        TagEnd::List(_) => Some(Container::List),
        TagEnd::Item => Some(Container::Item),
        TagEnd::BlockQuote(_)
        | TagEnd::FootnoteDefinition
        | TagEnd::DefinitionList
        | TagEnd::Table => Some(Container::Nesting),
        _ => None,
    }
}

/// A block start that ends an item's first line (`agenda-parse.ts`'s
/// `BLOCK_TAGS`). A paragraph is entered when nothing has been read yet.
fn is_block_start(tag: &Tag<'_>) -> bool {
    !matches!(
        tag,
        Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

/// The first line of text of every top-level list item, in document order.
fn top_level_item_lines(source: &str) -> impl Iterator<Item = String> + '_ {
    let mut stack: Vec<Container> = Vec::new();
    // The line being read: `Some` from a top-level `<li>` until its line ends.
    let mut line: Option<String> = None;
    let mut image_depth = 0usize;

    frontmatter::parse(source, markdown_options()).filter_map(move |event| {
        let mut finished = None;
        if let Some(text) = line.as_mut() {
            let blank = text.trim().is_empty();
            let ends = match &event {
                Event::Start(Tag::Paragraph) => !blank,
                Event::Start(tag) => is_block_start(tag),
                Event::End(TagEnd::Paragraph | TagEnd::Item) | Event::Rule => true,
                Event::SoftBreak | Event::HardBreak => !blank,
                Event::Text(t) | Event::Code(t) | Event::InlineMath(t) if image_depth == 0 => {
                    text.push_str(t);
                    false
                }
                _ => false,
            };
            if ends {
                finished = line.take();
            }
        }

        match &event {
            Event::Start(Tag::Image { .. }) => image_depth += 1,
            Event::End(TagEnd::Image) => image_depth = image_depth.saturating_sub(1),
            Event::Start(tag) => {
                if let Some(container) = container_of(tag) {
                    if container == Container::Item && stack == [Container::List] {
                        line = Some(String::new());
                    }
                    stack.push(container);
                }
            }
            Event::End(tag) if container_of_end(tag).is_some() => {
                stack.pop();
            }
            _ => {}
        }
        finished
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(source: &str) -> Vec<String> {
        top_level_item_lines(source).collect()
    }

    #[test]
    fn reads_each_top_level_items_first_line() {
        let source = "- 09:00 Standup\n  - Bob\n- Notes\n  second line 10:00\n\n1. 13:00 Lunch\n";
        assert_eq!(lines(source), ["09:00 Standup", "Notes", "13:00 Lunch"]);
    }

    #[test]
    fn enters_a_loose_items_first_paragraph_only() {
        let source = "- 9am Breakfast\n\n  17:00 not first\n\n- Notes\n";
        assert_eq!(lines(source), ["9am Breakfast", "Notes"]);
    }

    #[test]
    fn reads_through_inline_markup_but_not_image_alt_text() {
        assert_eq!(
            lines("- **09:00** - 10:00 *Deep* work\n"),
            ["09:00 - 10:00 Deep work"]
        );
        assert_eq!(lines("- ![09:00](x.png) Photo\n"), [" Photo"]);
    }

    #[test]
    fn skips_nested_quoted_fenced_and_frontmatter_lists() {
        let source = "---\ntimes:\n- 09:00 yaml\n---\n\
                      - Projects\n  - 09:00 nested\n\n\
                      > - 09:00 quoted\n\n\
                      ```\n- 09:00 fenced\n```\n\n\
                      [^1]: - 09:00 footnote\n";
        assert!(!may_have_agenda(source), "{:?}", lines(source));
    }

    #[test]
    fn accepts_the_owners_calendar_section() {
        let source = "## Calendar\n\n- 09:00 - 09:15 Standup\n  - Employees\n";
        assert!(may_have_agenda(source));
    }

    #[test]
    fn accepts_task_items_with_any_box_and_leading_annotations() {
        assert!(may_have_agenda("- [ ] 10:00 - 10:30 Wake up\n"));
        assert!(may_have_agenda("- [x] 10:00 Done\n"));
        assert!(may_have_agenda("- [-] 10:00 Dropped\n"));
        assert!(may_have_agenda("- [ ] !! 10:00 Urgent\n"));
    }

    #[test]
    fn accepts_one_to_three_spaces_of_indentation() {
        assert!(may_have_agenda("   - 09:00 Indented but top-level\n"));
        assert!(may_have_agenda("  3) 09:00 Ordered with a paren\n"));
        assert!(may_have_agenda("+\t09:00 Tab after the marker\n"));
    }

    #[test]
    fn accepts_raw_html_lists_wholesale() {
        assert!(may_have_agenda("<ul><li>09:00 Raw</li></ul>\n"));
        assert!(may_have_agenda("<UL><LI>09:00 Upper</LI></UL>\n"));
        assert!(may_have_agenda(
            "<ul><li class=\"x\">09:00 Classy</li></ul>\n"
        ));
        assert!(may_have_agenda("<ul>\n<li\n>09:00 Broken tag</li></ul>\n"));
    }

    /// `<link>`, `<listing>` and a bare `<li` are not list items, so a note
    /// with one and only prose times is rejected without the full parse.
    #[test]
    fn a_tag_merely_starting_with_li_is_not_a_list_item() {
        assert!(!contains_li_tag("<link rel=\"stylesheet\" href=\"x.css\">"));
        assert!(!contains_li_tag("<LISTING>x</LISTING>"));
        assert!(!contains_li_tag("ends with <li"));
        assert!(contains_li_tag("<li/>"));
        assert!(!may_have_agenda(
            "<link rel=\"stylesheet\" href=\"x.css\">\n\nWe met at 09:00.\n"
        ));
    }

    #[test]
    fn rejects_notes_without_time_like_items() {
        for source in [
            "",
            "# Title\n\nJust prose at 09:00.\n",
            "- 2026 goals\n- 10 reasons\n- 1:1 with Bob\n",
            "- Standup at 09:00\n",
            "    - 09:00 indented code\n",
        ] {
            assert!(!may_have_agenda(source), "{source:?}");
        }
    }

    /// A time outside list items — prose, a heading, a frontmatter
    /// timestamp — no longer costs a parse: the item-line pass rejects it.
    #[test]
    fn times_outside_item_lines_skip_the_parse() {
        for rest in [
            "Groceries",
            "Call Bob",
            "[ ] Pay rent",
            "**Plan** the [trip](x.md)",
        ] {
            assert!(!item_line_may_have_time(rest), "{rest:?}");
        }
        assert!(!may_have_agenda(
            "---\ncreated: 2026-01-01T09:00:00\n---\n\n# 09:00 sync\n\n- Groceries\n- Call Bob\n\nMet at 09:00.\n"
        ));
    }

    /// Every way an item's displayed first line can come to start with a
    /// time passes the item-line pass (the parse then decides).
    #[test]
    fn item_lines_that_may_display_a_time_pass() {
        for rest in [
            "09:00 Standup",
            "**09:00** Standup",
            "`09:00` Standup",
            "[09](x):00 linked",
            "[ ] 10:00 task",
            "[-] 10:00 task",
            "Standup at 09:00",
            "9am",
            "noon Lunch",
            "",
            "[ ]",
            "**",
            "![](x.png)",
            "![photo][p]",
            "<b>09</b>:00",
            "9&colon;00",
            "&#48;9:00",
            "[^1] 09:00",
            "!! 10:00",
            "\\",
        ] {
            assert!(item_line_may_have_time(rest), "{rest:?}");
        }
    }

    #[test]
    fn reads_a_time_on_the_line_after_an_empty_marker() {
        assert!(may_have_agenda("-\n  09:00 Late start\n"));
        assert!(may_have_agenda("- ![](x.png)\n  09:00 After a photo\n"));
        assert!(may_have_agenda("-\r\n  09:00 Windows line ends\r\n"));
    }

    #[test]
    fn strips_a_byte_order_mark() {
        assert!(may_have_agenda("\u{feff}- 09:00 Standup\n"));
    }
}
