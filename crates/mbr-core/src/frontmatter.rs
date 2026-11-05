//! Where a note's frontmatter is: the one definition every parser entry point
//! shares.
//!
//! pulldown-cmark's `ENABLE_YAML_STYLE_METADATA_BLOCKS` (and the `+++` twin)
//! accepts a metadata block at **any** unindented `---` line with a non-blank
//! line after it and a closing `---`/`...` further down, not only at the top
//! of the document. In a slides deck every `---` followed by `## Slide` is
//! therefore a "metadata block": the slide vanished from the page, its `*`
//! list became a YAML alias error, and its (usually empty) YAML replaced the
//! real frontmatter. GitHub, Obsidian and Jekyll only read frontmatter at the
//! very start of the file, and so does mbr.
//!
//! pulldown-cmark has no "only at the start" switch, so the leading block is
//! found here by [`leading_metadata`] — a byte scan of the first block that
//! copies pulldown-cmark 0.13's own rules (`scanners::scan_metadata_block`,
//! `firstpass::parse_metadata_block`) exactly, pinned by a proptest against the
//! parser itself — and the body is parsed **with both metadata options off**,
//! so a later `---` is a thematic break or a setext underline, as CommonMark
//! says. [`parse_with_offsets`] puts the two halves back together as one event
//! stream with absolute byte offsets, so callers see the same
//! `Start(MetadataBlock)` / `Text` / `End(MetadataBlock)` they always did, with
//! no copy of the source.

use pulldown_cmark::{CowStr, Event, MetadataBlockKind, Options, Parser, Tag, TagEnd};
use std::ops::Range;

/// Both metadata-block options. The body is always parsed without them.
const METADATA_OPTIONS: Options = Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
    .union(Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS);

/// The metadata block at the start of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeadingMetadata {
    /// `---` (YAML style) or `+++` (pluses style).
    pub kind: MetadataBlockKind,
    /// The block as pulldown-cmark reports it: from the opening delimiter to
    /// the end of the closing delimiter and its trailing spaces, the closing
    /// line's terminator excluded.
    pub block: Range<usize>,
    /// The lines between the two delimiter lines, terminators included.
    pub content: Range<usize>,
    /// First byte of the body: just past the closing line's terminator.
    pub body_start: usize,
}

/// The metadata block at the very start of `source`, if there is one and
/// `options` enables its kind.
///
/// pulldown-cmark's rules, byte for byte: the first line is exactly three
/// delimiters followed only by ASCII whitespace; the next line is neither
/// blank nor a closing line; the block ends at the first later line that is
/// exactly three delimiters (or `...` for the YAML style) followed only by
/// spaces. No closing line, no block. A leading BOM is not skipped here:
/// callers strip it first (`markdown::strip_bom`).
///
/// One pass over the block's lines and no allocation; a document that does not
/// start with `---`/`+++` costs one byte comparison.
pub fn leading_metadata(source: &str, options: Options) -> Option<LeadingMetadata> {
    let bytes = source.as_bytes();
    let delimiter = *bytes.first()?;
    let kind = match delimiter {
        b'-' if options.contains(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS) => {
            MetadataBlockKind::YamlStyle
        }
        b'+' if options.contains(Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS) => {
            MetadataBlockKind::PlusesStyle
        }
        _ => return None,
    };
    if run_of(bytes, delimiter) != 3 {
        return None;
    }
    let content_start = next_line(bytes, 3);
    if !bytes[3..content_start].iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    // The first content line can be neither blank nor the end of the block.
    // At end of input it is an empty line, i.e. blank.
    let first = &bytes[content_start..];
    if closing_line(first, delimiter).is_some() || blank_line(first).is_some() {
        return None;
    }

    let mut line = next_line(bytes, content_start);
    while line < bytes.len() {
        if let Some(closer_len) = closing_line(&bytes[line..], delimiter) {
            let block_end = line + closer_len;
            return Some(LeadingMetadata {
                kind,
                block: 0..block_end,
                content: content_start..line,
                body_start: block_end + blank_line(&bytes[block_end..]).unwrap_or(0),
            });
        }
        line = next_line(bytes, line);
    }
    None
}

/// `source` without its leading metadata block (per `options`).
pub fn body_of(source: &str, options: Options) -> &str {
    leading_metadata(source, options).map_or(source, |meta| &source[meta.body_start..])
}

/// Parses `source` with `options`, recognising a metadata block **only** at
/// the start of the document, and yields each event with its byte range in
/// `source`.
///
/// The leading block arrives as `Start(MetadataBlock)`, one `Text` holding the
/// block's raw content (line terminators as authored — pulldown-cmark would
/// split a CRLF into two text events, which every consumer merges and YAML
/// reads the same), and `End(MetadataBlock)`. Everything after it is parsed
/// with the metadata options removed, so a mid-document `---` is ordinary
/// markdown.
pub fn parse_with_offsets(
    source: &str,
    options: Options,
) -> impl Iterator<Item = (Event<'_>, Range<usize>)> {
    let leading = leading_metadata(source, options);
    let body_start = leading.as_ref().map_or(0, |meta| meta.body_start);
    let metadata_events = leading.map(|meta| {
        let content = &source[meta.content.clone()];
        [
            (
                Event::Start(Tag::MetadataBlock(meta.kind)),
                meta.block.clone(),
            ),
            (Event::Text(CowStr::Borrowed(content)), meta.content),
            (Event::End(TagEnd::MetadataBlock(meta.kind)), meta.block),
        ]
    });
    let body = Parser::new_ext(&source[body_start..], options - METADATA_OPTIONS)
        .into_offset_iter()
        .map(move |(event, range)| (event, range.start + body_start..range.end + body_start));
    metadata_events.into_iter().flatten().chain(body)
}

/// [`parse_with_offsets`] without the ranges.
pub fn parse(source: &str, options: Options) -> impl Iterator<Item = Event<'_>> {
    parse_with_offsets(source, options).map(|(event, _)| event)
}

/// Number of consecutive `byte`s at the start of `bytes`.
fn run_of(bytes: &[u8], byte: u8) -> usize {
    bytes.iter().take_while(|&&b| b == byte).count()
}

/// Index just past the `\n` ending the line that starts at `from`, or the end
/// of input (pulldown-cmark's `scan_nextline`: only `\n` ends a line here).
fn next_line(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |at| from + at + 1)
}

/// pulldown-cmark's `scan_eol`: end of input, `\n`, `\r\n` or a lone `\r`.
fn line_end(bytes: &[u8]) -> Option<usize> {
    match bytes {
        [] => Some(0),
        [b'\n', ..] => Some(1),
        [b'\r', b'\n', ..] => Some(2),
        [b'\r', ..] => Some(1),
        _ => None,
    }
}

/// pulldown-cmark's `scan_blank_line`: spaces, tabs, `\x0b` or `\x0c`, then a
/// line end. Returns the bytes it covers.
fn blank_line(bytes: &[u8]) -> Option<usize> {
    let spaces = bytes
        .iter()
        .take_while(|&&b| matches!(b, b' ' | b'\t' | 0x0b | 0x0c))
        .count();
    line_end(&bytes[spaces..]).map(|eol| spaces + eol)
}

/// pulldown-cmark's `scan_closing_metadata_block`: exactly three delimiters
/// (or `...` when the delimiter is `-`), then spaces only, then a line end.
/// Returns the length up to the line end, which it does not include.
fn closing_line(bytes: &[u8], delimiter: u8) -> Option<usize> {
    let delimited =
        run_of(bytes, delimiter) == 3 || (delimiter == b'-' && run_of(bytes, b'.') == 3);
    if !delimited {
        return None;
    }
    let end = 3 + run_of(&bytes[3..], b' ');
    line_end(&bytes[end..]).map(|_| end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const ALL: Options = METADATA_OPTIONS;

    fn lead(source: &str) -> Option<LeadingMetadata> {
        leading_metadata(source, ALL)
    }

    /// pulldown-cmark's own reading of the block at offset 0, with the
    /// metadata options on: (kind, block range, merged content, the events
    /// after it).
    type Reference<'a> = (
        Option<(MetadataBlockKind, Range<usize>, String)>,
        Vec<(Event<'a>, Range<usize>)>,
    );

    fn reference(source: &str, options: Options) -> Reference<'_> {
        let mut events: Vec<_> = Parser::new_ext(source, options)
            .into_offset_iter()
            .collect();
        match events.first() {
            Some((Event::Start(Tag::MetadataBlock(kind)), range)) if range.start == 0 => {
                let kind = *kind;
                let block = range.clone();
                let end = events
                    .iter()
                    .position(|(e, _)| matches!(e, Event::End(TagEnd::MetadataBlock(_))))
                    .expect("a started block ends");
                let content: String = events[1..end]
                    .iter()
                    .map(|(e, _)| match e {
                        Event::Text(t) => t.to_string(),
                        other => panic!("unexpected event in metadata: {other:?}"),
                    })
                    .collect();
                let rest = events.split_off(end + 1);
                (Some((kind, block, content)), rest)
            }
            _ => (None, events),
        }
    }

    #[test]
    fn yaml_block_at_start() {
        let meta = lead("---\ntitle: x\n---\n# Body\n").unwrap();
        assert_eq!(meta.kind, MetadataBlockKind::YamlStyle);
        assert_eq!(meta.block, 0..16);
        assert_eq!(meta.content, 4..13);
        assert_eq!(meta.body_start, 17);
    }

    #[test]
    fn dots_close_a_yaml_block() {
        let source = "---\ntitle: x\n...\nbody";
        let meta = lead(source).unwrap();
        assert_eq!(&source[meta.content.clone()], "title: x\n");
        assert_eq!(&source[meta.body_start..], "body");
    }

    #[test]
    fn dots_do_not_close_a_pluses_block() {
        assert_eq!(lead("+++\na = 1\n...\n"), None);
        let meta = lead("+++\na = 1\n+++\n").unwrap();
        assert_eq!(meta.kind, MetadataBlockKind::PlusesStyle);
    }

    #[test]
    fn options_select_the_kinds() {
        let yaml = "---\na: b\n---\n";
        let pluses = "+++\na = 1\n+++\n";
        let yaml_only = Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
        assert!(leading_metadata(yaml, yaml_only).is_some());
        assert_eq!(leading_metadata(pluses, yaml_only), None);
        assert_eq!(leading_metadata(yaml, Options::empty()), None);
    }

    #[test]
    fn not_a_block() {
        for source in [
            "",
            "---",
            "---\n",
            "---\n\na: b\n---\n",    // blank first line
            "---\n---\n",            // empty block
            "---\na: b\n",           // never closed
            "----\na: b\n----\n",    // four delimiters
            " ---\na: b\n---\n",     // indented
            "--- x\na: b\n---\n",    // text after the opener
            "# T\n---\na: b\n---\n", // not at the start
            "---\na: b\n --- \n",    // indented closer
            "---\na: b\n---\t\n",    // a tab after the closer
        ] {
            assert_eq!(lead(source), None, "{source:?}");
        }
    }

    #[test]
    fn crlf_and_trailing_whitespace() {
        let source = "--- \t\r\ntitle: x\r\n---  \r\nbody";
        let meta = lead(source).unwrap();
        assert_eq!(&source[meta.content.clone()], "title: x\r\n");
        assert_eq!(&source[meta.body_start..], "body");
        assert_eq!(&source[meta.block.clone()], "--- \t\r\ntitle: x\r\n---  ");
    }

    #[test]
    fn a_later_block_is_ordinary_markdown() {
        let source = "---\ntype: slides\n---\n# Deck\n\n---\n## Flashcards\n---\n";
        let events: Vec<_> = parse(source, ALL).collect();
        let blocks = events
            .iter()
            .filter(|e| matches!(e, Event::Start(Tag::MetadataBlock(_))))
            .count();
        assert_eq!(blocks, 1);
        assert!(events.contains(&Event::Text("Flashcards".into())));
        assert!(events.contains(&Event::Rule));
    }

    #[test]
    fn offsets_are_absolute() {
        let source = "---\na: b\n---\n# Head\n";
        let heading = parse_with_offsets(source, ALL)
            .find(|(e, _)| matches!(e, Event::Start(Tag::Heading { .. })))
            .unwrap();
        assert_eq!(&source[heading.1], "# Head\n");
    }

    #[test]
    fn body_of_strips_only_the_leading_block() {
        assert_eq!(body_of("---\na: b\n---\nx", ALL), "x");
        assert_eq!(body_of("x\n---\na: b\n---\n", ALL), "x\n---\na: b\n---\n");
    }

    /// Lines a document start is made of: delimiters and near misses, blank
    /// and whitespace-only lines, YAML, markdown.
    fn line() -> impl Strategy<Value = &'static str> {
        prop::sample::select(vec![
            "---", "+++", "...", "----", "--", "--- ", "---  ", "---\t", " ---", "---x", "+++ ",
            ". ..", "", " ", "\t", "\x0b", "\x0c", "a: b", "title: T", "* x", "- y", "## X", "# H",
            "x", "```", "<div>", "[r]: /u", "> q",
        ])
    }

    fn terminator() -> impl Strategy<Value = &'static str> {
        prop::sample::select(vec!["\n", "\n", "\n", "\r\n", "\r"])
    }

    /// Mostly `---`/`+++` first lines, so most cases exercise a block.
    fn opener() -> impl Strategy<Value = &'static str> {
        prop_oneof![
            4 => prop::sample::select(vec!["---", "+++", "--- ", "---\t", "---\x0c"]),
            1 => line(),
        ]
    }

    fn document() -> impl Strategy<Value = String> {
        (
            (opener(), terminator()),
            prop::collection::vec((line(), terminator()), 0..10),
        )
            .prop_flat_map(|(first, rest)| {
                let lines: Vec<_> = std::iter::once(first).chain(rest).collect();
                prop::bool::ANY.prop_map(move |final_terminator| {
                    let mut source = String::new();
                    for (i, (text, end)) in lines.iter().enumerate() {
                        source.push_str(text);
                        if i + 1 < lines.len() || final_terminator {
                            source.push_str(end);
                        }
                    }
                    source
                })
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4000))]

        /// The scanner finds exactly the block pulldown-cmark finds at
        /// offset 0 — same kind, same range, same content — for every option
        /// set mbr parses with.
        #[test]
        fn scanner_matches_pulldown_cmark(source in document(), yaml in any::<bool>(), pluses in any::<bool>()) {
            let mut options = Options::all() - METADATA_OPTIONS;
            options.set(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS, yaml);
            options.set(Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS, pluses);
            let (expected, _) = reference(&source, options);
            let ours = leading_metadata(&source, options).map(|meta| {
                (meta.kind, meta.block, source[meta.content].replace("\r\n", "\n"))
            });
            prop_assert_eq!(ours, expected);
        }

        /// When pulldown-cmark finds no block after the leading one, the
        /// combined stream is exactly its stream: the body parsed on its own
        /// continues where the parser would have.
        #[test]
        fn stream_matches_pulldown_cmark_without_later_blocks(source in document()) {
            let (expected_lead, expected_rest) = reference(&source, Options::all());
            prop_assume!(!expected_rest
                .iter()
                .any(|(e, _)| matches!(e, Event::Start(Tag::MetadataBlock(_)))));
            let mut ours: Vec<_> = parse_with_offsets(&source, Options::all()).collect();
            if expected_lead.is_some() {
                prop_assert!(matches!(ours.first(), Some((Event::Start(Tag::MetadataBlock(_)), _))));
                ours.drain(..3);
            }
            prop_assert_eq!(ours, expected_rest);
        }

        /// A block anywhere but the start never changes what the start says.
        #[test]
        fn a_later_block_never_changes_the_leading_one(
            head in document(),
            yaml in "[a-z]{1,8}: [a-z*&!|]{0,8}",
        ) {
            // An unclosed opener in `head` may legitimately be closed by the
            // appended `---`, so only a block `head` already has is pinned.
            if let Some(before) = lead(&head) {
                let source = format!("{head}\n\n---\n{yaml}\n---\n");
                // `body_start` may move: a closer at end of input gains a
                // terminator.
                let after = lead(&source).map(|meta| (meta.kind, meta.block, meta.content));
                prop_assert_eq!(after, Some((before.kind, before.block, before.content)));
            }
        }
    }
}
