//! ```` ```chat ```` code blocks: a conversation rendered as speech bubbles.
//!
//! Compatible with the `chat` block of the Obsidian
//! [Chat View](https://github.com/adifyr/obsidian-chat-view) plugin (v2.0,
//! `src/processors/chat.ts`), so notes written for it render the same here.
//! Other renderers show the raw code block, which still reads as a transcript.
//!
//! This module is **pure**: [`parse`] turns the block's text into
//! [`ChatItem`]s and the `*_html` helpers produce the wrapper markup. It knows
//! nothing about markdown — bodies are handed back as text, and
//! `markdown::collect_events_and_headings` parses them with the page's own
//! pipeline so links, wikilinks and media inside a bubble resolve exactly as
//! they would anywhere else on the page.
//!
//! # Grammar
//!
//! Every construct starts in column 0 of a line, as in the plugin:
//!
//! | Line | Meaning |
//! |------|---------|
//! | `{{header\|body\|subtext}}` | A message. May span lines; ends at the first line ending in `\|subtext}}` |
//! | `...` | A centred divider |
//! | `# text` | A centred note outside the bubbles |
//! | `> Name, Name` | These speakers are drawn on the right |
//! | `^ Name, Name` | These speakers are drawn in the centre |
//! | `[Name=color]` / `{key=value}` | Chat View 1.x configuration — silently ignored |
//! | anything else | Markdown, rendered between the bubbles (an mbr extension) |
//!
//! Two deliberate departures from the plugin:
//!
//! * The plugin collects `>`/`^` alignment lines with a whole-block regex, so a
//!   blockquote written *inside* a message body also re-aligns speakers. Here
//!   only lines outside messages count.
//! * The plugin drops lines that match nothing. They are rendered as markdown
//!   here, because a transcript interleaved with commentary is the point.
//!
//! # Matching the plugin's message regex
//!
//! The plugin's pattern is
//! `^\{\{([^|\n]*)\|([\s\S]*?)\|([^|\n]*)\}\}\s*$` (multiline). The header is
//! the text up to the first `|` on the opening line; the body is lazy, so it
//! ends at the **first** `|` that is followed by pipe-free text, `}}` and the
//! end of a line. Only the last `|` on a line can be followed by pipe-free text,
//! so each line has at most one candidate closer, and [`parse`] precomputes
//! them once — an unclosed `{{` then costs O(1) to reject instead of a rescan of
//! the rest of the block, which would make a block of a thousand broken
//! messages quadratic.

use std::collections::HashSet;

/// Number of speaker colour slots in `theme.css` (`.mbr-chat-c0` …).
pub const SPEAKER_COLORS: u8 = 8;

/// Which side of the log a message is drawn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Center,
}

impl Side {
    fn class(self) -> &'static str {
        match self {
            Side::Left => "mbr-chat-left",
            Side::Right => "mbr-chat-right",
            Side::Center => "mbr-chat-center",
        }
    }
}

/// One `{{header|body|subtext}}` message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub side: Side,
    /// The speaker. For a message with an empty header this is the previous
    /// speaker's name (empty if there was none), so the bubble keeps their
    /// colour.
    pub speaker: String,
    /// True when the speaker label is not repeated: an empty header, or the
    /// same name as the message immediately before.
    pub continuation: bool,
    /// Markdown source of the body, trimmed, with `\|` unescaped.
    pub body: String,
    /// Byte offset of `body`'s first character in the block text. Lets the
    /// caller map the body's own lines back to source lines; `\|` unescaping
    /// never removes a newline, so line arithmetic survives it.
    pub body_offset: usize,
    pub subtext: String,
}

/// One element of a chat block, in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatItem {
    Message(ChatMessage),
    /// `...`
    Delimiter,
    /// `# text` — rendered as inline-ish markdown in a muted, centred note.
    Comment {
        text: String,
        offset: usize,
    },
    /// Lines outside any message that are none of the above, kept together
    /// until the next construct. Blank lines inside stay, so paragraphs work.
    Markdown {
        text: String,
        offset: usize,
    },
}

/// Whether a fenced code block's info string makes it a chat block.
///
/// Only the first word counts, so ```` ```chat title ```` still qualifies, and
/// `chat-old`, `chat-webvtt` & co. (other Chat View formats mbr does not
/// support) do not.
pub fn is_chat_info(info: &str) -> bool {
    info.split_whitespace().next() == Some("chat")
}

/// Trims and unescapes `\|`, as the plugin's `fmt` does.
fn fmt(text: &str) -> String {
    text.trim().replace("\\|", "|")
}

/// A line with its byte span in the block (newline excluded).
#[derive(Debug, Clone, Copy)]
struct Line {
    start: usize,
    end: usize,
}

fn split_lines(src: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (newline, _) in src.match_indices('\n') {
        lines.push(Line {
            start,
            end: newline,
        });
        start = newline + 1;
    }
    if start < src.len() {
        lines.push(Line {
            start,
            end: src.len(),
        });
    }
    lines
}

/// Absolute offset of the `|` that could close a message on this line: the
/// line, right-trimmed, must end in `}}`, and the closer is the last `|` before
/// that. `None` when the line cannot close a message.
fn closer(src: &str, line: Line) -> Option<usize> {
    let text = src[line.start..line.end].trim_end();
    let inner = text.strip_suffix("}}")?;
    inner.rfind('|').map(|pipe| line.start + pipe)
}

/// Chat View 1.x configuration lines (`[Albus=red, Minerva=green]`,
/// `{mw=60, mode=minimal}`): every comma-separated entry is `key=value` with a
/// non-empty key and a single-word value. Strict on purpose, so a markdown link
/// or a stray brace in prose is never swallowed.
fn is_legacy_config(line: &str) -> bool {
    let line = line.trim();
    let inner = line
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .or_else(|| {
            line.strip_prefix('{')
                .filter(|rest| !rest.starts_with('{'))
                .and_then(|rest| rest.strip_suffix('}'))
        });
    let Some(inner) = inner else {
        return false;
    };
    !inner.contains(['[', ']', '{', '}', '(', ')'])
        && inner.split(',').all(|entry| {
            entry.split_once('=').is_some_and(|(key, value)| {
                let (key, value) = (key.trim(), value.trim());
                !key.is_empty()
                    && !value.is_empty()
                    && !value.contains('=')
                    && !value.contains(char::is_whitespace)
            })
        })
}

/// A message header before its side and continuation have been resolved.
struct RawMessage {
    header: String,
    body: String,
    body_offset: usize,
    subtext: String,
}

enum RawItem {
    Message(RawMessage),
    Resolved(ChatItem),
}

/// Parses the text of a chat block. Never panics; never drops content other
/// than syntax, alignment lines and 1.x configuration lines.
pub fn parse(src: &str) -> Vec<ChatItem> {
    let lines = split_lines(src);
    let closers: Vec<Option<usize>> = lines.iter().map(|&line| closer(src, line)).collect();
    // next_closer[i]: the first line at or after `i` that can close a message.
    let mut next_closer = vec![None; lines.len() + 1];
    for i in (0..lines.len()).rev() {
        next_closer[i] = if closers[i].is_some() {
            Some(i)
        } else {
            next_closer[i + 1]
        };
    }

    let mut raw: Vec<RawItem> = Vec::new();
    let mut right: HashSet<String> = HashSet::new();
    let mut center: HashSet<String> = HashSet::new();
    // Byte span of the markdown run being accumulated.
    let mut markdown: Option<(usize, usize)> = None;

    let flush = |markdown: &mut Option<(usize, usize)>, raw: &mut Vec<RawItem>| {
        if let Some((start, end)) = markdown.take() {
            let text = src[start..end].trim_end();
            if !text.is_empty() {
                raw.push(RawItem::Resolved(ChatItem::Markdown {
                    text: text.to_string(),
                    offset: start,
                }));
            }
        }
    };

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let text = &src[line.start..line.end];

        if let Some(message) = try_message(src, &lines, &closers, &next_closer, i) {
            flush(&mut markdown, &mut raw);
            raw.push(RawItem::Message(message.0));
            i = message.1 + 1;
            continue;
        }

        if text.trim_end() == "..." {
            flush(&mut markdown, &mut raw);
            raw.push(RawItem::Resolved(ChatItem::Delimiter));
        } else if let Some(comment) = text.strip_prefix('#') {
            flush(&mut markdown, &mut raw);
            let leading = comment.len() - comment.trim_start().len();
            raw.push(RawItem::Resolved(ChatItem::Comment {
                text: fmt(comment),
                offset: line.start + 1 + leading,
            }));
        } else if let Some(names) = text.strip_prefix('>') {
            flush(&mut markdown, &mut raw);
            collect_names(names, &mut right);
        } else if let Some(names) = text.strip_prefix('^') {
            flush(&mut markdown, &mut raw);
            collect_names(names, &mut center);
        } else if is_legacy_config(text) {
            flush(&mut markdown, &mut raw);
        } else if let Some((_, end)) = markdown.as_mut() {
            *end = line.end;
        } else if !text.trim().is_empty() {
            // A run never starts on a blank line, so its offset is the line its
            // first visible text is on.
            markdown = Some((line.start, line.end));
        }
        i += 1;
    }
    flush(&mut markdown, &mut raw);

    resolve(raw, &right, &center)
}

/// Tries to read a message opening on line `i`. Returns it with the index of
/// the line it closed on.
fn try_message(
    src: &str,
    lines: &[Line],
    closers: &[Option<usize>],
    next_closer: &[Option<usize>],
    i: usize,
) -> Option<(RawMessage, usize)> {
    let line = lines[i];
    let text = &src[line.start..line.end];
    let after_open = text.strip_prefix("{{")?;
    let header_len = after_open.find('|')?;
    let header = &after_open[..header_len];
    // `{{` + header + `|`
    let body_start = line.start + 2 + header_len + 1;

    let (close_line, pipe) = match closers[i] {
        Some(pipe) if pipe >= body_start => (i, pipe),
        _ => {
            let j = next_closer[i + 1]?;
            (j, closers[j]?)
        }
    };
    let close = lines[close_line];
    let subtext_end = close.start + src[close.start..close.end].trim_end().len() - 2;

    let raw_body = &src[body_start..pipe];
    let leading = raw_body.len() - raw_body.trim_start().len();
    Some((
        RawMessage {
            header: fmt(header),
            body: fmt(raw_body),
            body_offset: body_start + leading,
            subtext: fmt(&src[pipe + 1..subtext_end]),
        },
        close_line,
    ))
}

fn collect_names(list: &str, into: &mut HashSet<String>) {
    into.extend(
        list.split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string),
    );
}

/// Assigns sides and continuation flags. Alignment lines may come anywhere in
/// the block, even after the messages they affect, hence a second pass.
fn resolve(raw: Vec<RawItem>, right: &HashSet<String>, center: &HashSet<String>) -> Vec<ChatItem> {
    let mut side = Side::Left;
    let mut speaker = String::new();
    let mut previous_was_message = false;
    raw.into_iter()
        .map(|item| match item {
            RawItem::Resolved(item) => {
                previous_was_message = false;
                item
            }
            RawItem::Message(message) => {
                let continuation = message.header.is_empty()
                    || (previous_was_message && message.header == speaker);
                if !message.header.is_empty() {
                    // As in the plugin, an empty header keeps the last side.
                    side = if right.contains(&message.header) {
                        Side::Right
                    } else if center.contains(&message.header) {
                        Side::Center
                    } else {
                        Side::Left
                    };
                    speaker = message.header;
                }
                previous_was_message = true;
                ChatItem::Message(ChatMessage {
                    side,
                    speaker: speaker.clone(),
                    continuation,
                    body: message.body,
                    body_offset: message.body_offset,
                    subtext: message.subtext,
                })
            }
        })
        .collect()
}

/// Stable colour slot for a speaker: FNV-1a over the name's bytes. Stable
/// across runs and platforms, so a speaker keeps their colour from page to page
/// and between a server render and a static build.
pub fn speaker_color(name: &str) -> u8 {
    let hash = name.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });
    // The modulus is < 256, so the narrowing cannot truncate.
    (hash % u32::from(SPEAKER_COLORS)) as u8
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // Writing to a String cannot fail.
    let _ = pulldown_cmark_escape::escape_html(&mut out, text);
    out
}

/// Opens the whole block. `line` is the fence's source line, emitted as
/// `data-mbr-line` so a review note on a bubble can name the block it is in.
pub fn open_html(line: Option<u32>) -> String {
    match line {
        Some(line) => {
            format!(
                "<div class=\"mbr-chat\" role=\"log\" aria-label=\"Chat\" data-mbr-line=\"{line}\">\n"
            )
        }
        None => "<div class=\"mbr-chat\" role=\"log\" aria-label=\"Chat\">\n".to_string(),
    }
}

pub const CLOSE_HTML: &str = "</div>\n";
pub const DELIMITER_HTML: &str = "<div class=\"mbr-chat-delim\" role=\"separator\"></div>\n";
pub const COMMENT_OPEN_HTML: &str = "<div class=\"mbr-chat-comment\">\n";
pub const MARKDOWN_OPEN_HTML: &str = "<div class=\"mbr-chat-md\">\n";

/// Everything before a message's body: the bubble and its speaker label.
pub fn message_open_html(message: &ChatMessage) -> String {
    let mut html = format!(
        "<div class=\"mbr-chat-msg {} mbr-chat-c{}{}\">\n",
        message.side.class(),
        speaker_color(&message.speaker),
        if message.continuation {
            " mbr-chat-cont"
        } else {
            ""
        }
    );
    if !message.continuation && !message.speaker.is_empty() {
        html.push_str("<div class=\"mbr-chat-name\">");
        html.push_str(&escape(&message.speaker));
        html.push_str("</div>\n");
    }
    html.push_str("<div class=\"mbr-chat-body\">\n");
    html
}

/// Everything after a message's body: the subtext line and the bubble's close.
pub fn message_close_html(message: &ChatMessage) -> String {
    if message.subtext.is_empty() {
        "</div>\n</div>\n".to_string()
    } else {
        format!(
            "</div>\n<div class=\"mbr-chat-meta\">{}</div>\n</div>\n",
            escape(&message.subtext)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn messages(items: &[ChatItem]) -> Vec<&ChatMessage> {
        items
            .iter()
            .filter_map(|item| match item {
                ChatItem::Message(m) => Some(m),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn info_string_first_word_must_be_chat() {
        assert!(is_chat_info("chat"));
        assert!(is_chat_info(" chat title"));
        assert!(!is_chat_info("chat-old"));
        assert!(!is_chat_info("chat-webvtt"));
        assert!(!is_chat_info("rust"));
        assert!(!is_chat_info(""));
    }

    #[test]
    fn single_line_message() {
        let items = parse("{{Alice|Hello there|5:42 PM}}\n");
        assert_eq!(
            items,
            vec![ChatItem::Message(ChatMessage {
                side: Side::Left,
                speaker: "Alice".into(),
                continuation: false,
                body: "Hello there".into(),
                body_offset: 8,
                subtext: "5:42 PM".into(),
            })]
        );
    }

    #[test]
    fn multi_line_body_ends_at_first_closing_line() {
        let src = "{{Bob|line one\n\nline two|now}}\n{{Alice|reply|}}";
        let items = parse(src);
        let ms = messages(&items);
        assert_eq!(ms.len(), 2);
        assert_eq!(ms[0].body, "line one\n\nline two");
        assert_eq!(ms[0].subtext, "now");
        assert_eq!(ms[1].body, "reply");
        assert_eq!(&src[ms[1].body_offset..ms[1].body_offset + 5], "reply");
    }

    #[test]
    fn parts_may_be_empty() {
        let items = parse("{{||}}");
        let ms = messages(&items);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].body, "");
        assert_eq!(ms[0].subtext, "");
        assert!(ms[0].continuation, "an empty header is a continuation");
    }

    #[test]
    fn body_may_contain_pipes_and_escaped_pipes() {
        // Lazy body + pipe-free subtext: only the last `|` closes.
        let items = parse("{{A|see [[Page|alias]] and a \\| b|5pm}}");
        let ms = messages(&items);
        assert_eq!(ms[0].body, "see [[Page|alias]] and a | b");
        assert_eq!(ms[0].subtext, "5pm");
    }

    #[test]
    fn trailing_whitespace_after_close_is_allowed() {
        assert_eq!(messages(&parse("{{A|b|c}}   \t")).len(), 1);
        // But not text.
        assert!(messages(&parse("{{A|b|c}} trailing")).is_empty());
    }

    #[test]
    fn message_must_start_in_column_zero() {
        let items = parse("  {{A|b|c}}");
        assert!(messages(&items).is_empty());
        assert!(matches!(&items[0], ChatItem::Markdown { .. }));
    }

    #[test]
    fn unclosed_message_degrades_to_markdown() {
        let items = parse("{{A|never closed\nmore text");
        assert_eq!(
            items,
            vec![ChatItem::Markdown {
                text: "{{A|never closed\nmore text".into(),
                offset: 0
            }]
        );
    }

    #[test]
    fn header_pipe_cannot_close_its_own_message() {
        // `{{A|}}` has no second `|`, so the body runs on to the next closer.
        let items = parse("{{A|}}\nstill body|t}}");
        let ms = messages(&items);
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].body, "}}\nstill body");
        assert_eq!(ms[0].subtext, "t");
    }

    #[test]
    fn delimiter_and_comment() {
        let items = parse("...\n#  Monday  \n...   ");
        assert_eq!(
            items,
            vec![
                ChatItem::Delimiter,
                ChatItem::Comment {
                    text: "Monday".into(),
                    offset: 7
                },
                ChatItem::Delimiter
            ]
        );
    }

    #[test]
    fn alignment_lines_set_sides_even_when_they_come_later() {
        let src = "{{Alice|hi|}}\n{{Bob|yo|}}\n{{Mod|rules|}}\n> Bob, Carol\n^ Mod";
        let ms_items = parse(src);
        let ms = messages(&ms_items);
        assert_eq!(ms[0].side, Side::Left);
        assert_eq!(ms[1].side, Side::Right);
        assert_eq!(ms[2].side, Side::Center);
    }

    #[test]
    fn blockquote_inside_a_body_is_not_an_alignment_line() {
        // The plugin would put Alice on the right here.
        let items = parse("{{Alice|quoting:\n> Alice\n|}}");
        let ms = messages(&items);
        assert_eq!(ms[0].side, Side::Left);
        assert_eq!(ms[0].body, "quoting:\n> Alice");
    }

    #[test]
    fn empty_header_and_repeated_name_continue_the_speaker() {
        let src = "> Bob\n{{Bob|a|}}\n{{|b|}}\n{{Bob|c|}}\n...\n{{Bob|d|}}";
        let items = parse(src);
        let ms = messages(&items);
        assert_eq!(
            ms.iter()
                .map(|m| (m.speaker.as_str(), m.side, m.continuation))
                .collect::<Vec<_>>(),
            vec![
                ("Bob", Side::Right, false),
                ("Bob", Side::Right, true),
                ("Bob", Side::Right, true),
                // A delimiter breaks the run, so the name is shown again.
                ("Bob", Side::Right, false),
            ]
        );
    }

    #[test]
    fn other_lines_become_markdown_runs() {
        let src = "Intro paragraph.\n\nSecond **para**.\n{{A|b|}}\n\n\nAfter [link](x.md)\n";
        let items = parse(src);
        assert_eq!(
            items[0],
            ChatItem::Markdown {
                text: "Intro paragraph.\n\nSecond **para**.".into(),
                offset: 0
            }
        );
        assert!(matches!(items[1], ChatItem::Message(_)));
        let ChatItem::Markdown { text, offset } = &items[2] else {
            panic!("expected markdown, got {:?}", items[2]);
        };
        assert_eq!(text, "After [link](x.md)");
        assert_eq!(&src[*offset..*offset + 5], "After");
    }

    #[test]
    fn legacy_config_lines_are_ignored() {
        let items =
            parse("[Albus Dumbledore=orange, Minerva=red]\n{mw=60, mode=minimal}\n{{A|b|}}");
        assert_eq!(messages(&items).len(), 1);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn markdown_that_looks_bracketed_is_kept() {
        for line in [
            "[a link](x.md)",
            "[x=1](y)",
            "[just text]",
            "{not=config with spaces}",
            "{{A=b}}",
        ] {
            assert!(!is_legacy_config(line), "{line}");
            assert_eq!(parse(line).len(), 1, "{line}");
        }
    }

    #[test]
    fn crlf_line_endings() {
        let items = parse("{{A|b|c}}\r\n...\r\n");
        assert_eq!(messages(&items)[0].subtext, "c");
        assert_eq!(items[1], ChatItem::Delimiter);
    }

    #[test]
    fn speaker_color_is_stable_and_in_range() {
        assert_eq!(speaker_color("Alice"), speaker_color("Alice"));
        for name in ["", "Alice", "Bob", "Ж", "a very long name indeed"] {
            assert!(speaker_color(name) < SPEAKER_COLORS);
        }
        // Pinned so a change to the hash (which would recolour every speaker in
        // every existing note) is a deliberate act.
        assert_eq!(speaker_color("Alice"), 7);
    }

    #[test]
    fn html_escapes_header_and_subtext() {
        let message = ChatMessage {
            side: Side::Right,
            speaker: "<b>x</b>".into(),
            continuation: false,
            body: String::new(),
            body_offset: 0,
            subtext: "\"<i>\"".into(),
        };
        let open = message_open_html(&message);
        assert!(open.contains("&lt;b&gt;x&lt;/b&gt;"), "{open}");
        assert!(open.contains("mbr-chat-right"), "{open}");
        let close = message_close_html(&message);
        assert!(close.contains("&quot;&lt;i&gt;&quot;"), "{close}");
    }

    #[test]
    fn continuation_hides_the_name_label() {
        let message = ChatMessage {
            side: Side::Left,
            speaker: "Bob".into(),
            continuation: true,
            body: String::new(),
            body_offset: 0,
            subtext: String::new(),
        };
        let open = message_open_html(&message);
        assert!(!open.contains("mbr-chat-name"), "{open}");
        assert!(open.contains("mbr-chat-cont"), "{open}");
    }

    fn alnum_counts(text: &str) -> std::collections::BTreeMap<char, usize> {
        let mut counts = std::collections::BTreeMap::new();
        for c in text.chars().filter(|c| c.is_alphanumeric()) {
            *counts.entry(c).or_insert(0) += 1;
        }
        counts
    }

    fn item_text(items: &[ChatItem]) -> String {
        items
            .iter()
            .map(|item| match item {
                ChatItem::Message(m) => format!("{} {}", m.body, m.subtext),
                ChatItem::Delimiter => String::new(),
                ChatItem::Comment { text, .. } | ChatItem::Markdown { text, .. } => text.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    proptest! {
        #[test]
        fn never_panics(src in "\\PC*") {
            let _ = parse(&src);
        }

        #[test]
        fn never_panics_on_syntax_soup(src in "[{}|#>^.\\[\\]=\\\\ a\n\r]{0,80}") {
            let _ = parse(&src);
        }

        /// No content is lost. Headers are excluded from the comparison
        /// (they carry no `|`/newline and go to `speaker`, which is inherited
        /// for continuations), so the generator keeps letters out of them by
        /// putting digits only in bodies/subtexts/prose and letters only in
        /// headers — and alignment/config lines can't be generated at all,
        /// since no line can start with `>`, `^`, `[` or a lone `{`.
        #[test]
        fn every_body_and_prose_character_survives(
            lines in proptest::collection::vec(
                prop_oneof![
                    "[0-9 ]{0,12}",
                    "\\{\\{[a-z]{0,4}\\|[0-9 \\|]{0,12}",
                    "[0-9 ]{0,8}\\|[0-9 ]{0,4}\\}\\}",
                    "\\{\\{[a-z]{0,4}\\|[0-9 ]{0,8}\\|[0-9 ]{0,4}\\}\\}",
                    Just("...".to_string()),
                    "#[0-9 ]{0,8}",
                ],
                0..12,
            )
        ) {
            let src = lines.join("\n");
            let items = parse(&src);
            let digits = |text: &str| -> std::collections::BTreeMap<char, usize> {
                alnum_counts(text).into_iter().filter(|(c, _)| c.is_ascii_digit()).collect()
            };
            prop_assert_eq!(digits(&item_text(&items)), digits(&src));
        }

        /// Every offset the parser hands out is a char boundary inside the
        /// block, and the body it points at starts there.
        #[test]
        fn offsets_point_at_their_text(src in "[{}|#a\n ]{0,60}") {
            for item in parse(&src) {
                match item {
                    ChatItem::Message(m) if !m.body.is_empty() => {
                        prop_assert!(src.is_char_boundary(m.body_offset));
                        prop_assert!(src[m.body_offset..].starts_with(m.body.chars().next().unwrap_or(' ')));
                    }
                    ChatItem::Comment { text, offset } | ChatItem::Markdown { text, offset } if !text.is_empty() => {
                        prop_assert!(src[offset..].starts_with(&text[..1]));
                    }
                    _ => {}
                }
            }
        }
    }
}
