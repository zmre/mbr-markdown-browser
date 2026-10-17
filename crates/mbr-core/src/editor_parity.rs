//! Editor parity harness, Rust half: the renderer's block structure of every
//! fixture in `tests/fixtures/editor-roundtrip/`, pinned as a golden outline.
//!
//! The in-browser editor (Milkdown/Crepe) parses markdown with remark; the
//! renderer and `flashcards.rs` parse it with pulldown-cmark. When the two
//! disagree about where a card, a definition or a list item ends, a save from
//! the editor silently restructures the file. This test writes down what the
//! *renderer* sees; `components/src/editor-parity.test.ts` builds the same
//! outline from the editor's mdast and compares it to the same golden file.
//! The golden is the renderer's view by definition — it is generated here and
//! only here.
//!
//! # The outline format
//!
//! `<fixture>.outline.json` is a JSON array of block nodes in document
//! (pre-)order, one per line. Nesting is the `depth` field (0 = top level), so
//! a diff names exactly the block that moved. Every node has, in this order:
//!
//! * `depth` — nesting depth.
//! * `kind` — one of `heading`, `paragraph`, `blockquote`, `list`, `item`,
//!   `dl`, `dt`, `dd`, `code`, `table`, `rule`, `section_break`, `html`,
//!   `footnote`.
//! * `lines` — `[first, last]`, 1-based, **inclusive**, in whole-file lines
//!   (frontmatter counts). `first` is the line of the first non-whitespace
//!   byte of the block's source range and `last` the line of the last one, so
//!   neither side's habit of including trailing blank lines or line endings in
//!   a range matters.
//!
//! and only the fields its kind defines (absent, never `null`):
//!
//! * `heading`: `level` (1–6); `id` and `classes` when `{#id .class}`
//!   heading attributes are present.
//! * `blockquote`: `alert` (`note`/`tip`/`important`/`warning`/`caution`)
//!   for a GitHub alert.
//! * `list`: `ordered`; `start` (ordered only); `tight`.
//! * `item`: `task` — the marker character, one of `" "`, `"x"`, `"-"`,
//!   `">"` (`[X]` reports `"x"`) — only on a task item. `[-]`/`[>]` are mbr's
//!   extension, recognized exactly as the renderer does
//!   (`split_extended_marker`).
//! * `dl`: `tight`.
//! * `code`: `fenced`; `lang` (first word of the info string) when present.
//! * `table`: `align` (per column: `none`/`left`/`center`/`right`) and `rows`
//!   (header row included).
//! * `footnote`: `label`.
//!
//! Structure rules that make the two parsers comparable:
//!
//! * Frontmatter (the YAML metadata block) is not a node: the editor never
//!   sees it.
//! * Inline content is never descended into. A block container's inline
//!   content is a `paragraph`, even where pulldown-cmark emits none (the items
//!   of a tight list, a tight `dd`) — mdast always has one, and tightness is
//!   carried by `tight` instead. `dt` and `heading` hold inline content
//!   directly and have no paragraph children.
//! * A paragraph whose whole text is `--- {attrs}` is a `section_break` (the
//!   renderer's rule, `markdown::section_rule_attrs`); a plain `---` is a
//!   `rule`. Table cells and rows are summarized, not listed.
//!
//! Regenerate the goldens after an intended renderer change with
//! `MBR_UPDATE_OUTLINES=1 cargo test -p mbr-core editor_parity`, and review
//! the diff: a changed golden changes what the editor is held to.

use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, Parser, Tag, TagEnd, TextMergeWithOffset,
};
use serde::Serialize;

use crate::markdown::{LineIndex, markdown_options, section_rule_attrs, strip_bom};

/// Environment variable that rewrites the goldens instead of comparing.
const UPDATE_ENV: &str = "MBR_UPDATE_OUTLINES";

/// One block in the outline. Field order is the serialized key order.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct OutlineNode {
    depth: usize,
    kind: &'static str,
    lines: [u32; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    classes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alert: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ordered: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tight: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fenced: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    align: Option<Vec<&'static str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

impl OutlineNode {
    fn new(depth: usize, kind: &'static str) -> Self {
        Self {
            depth,
            kind,
            lines: [0, 0],
            level: None,
            id: None,
            classes: None,
            alert: None,
            ordered: None,
            start: None,
            tight: None,
            task: None,
            fenced: None,
            lang: None,
            align: None,
            rows: None,
            label: None,
        }
    }
}

/// An open block on the builder's stack.
struct Open {
    /// Index of its node in the output, or `None` for a tag that is not a
    /// node (table head/row/cell, metadata block).
    node: Option<usize>,
    /// Source range start of the opening event.
    start: usize,
    /// Whether inline content directly inside it needs a synthetic paragraph.
    wraps_inline: bool,
}

/// Builds the outline of `source` — see the module docs for the format.
fn outline(source: &str) -> Vec<OutlineNode> {
    let body = strip_bom(source);
    let shift = source.len() - body.len();
    let lines = LineIndex::build(source);
    let span = |range: Range<usize>| -> [u32; 2] { trimmed_lines(source, &lines, range) };

    let mut nodes: Vec<OutlineNode> = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    // The synthetic paragraph around inline content in a tight item or dd:
    // its node index and the source range covered so far.
    let mut synthetic: Option<(usize, Range<usize>)> = None;
    // The open paragraph: its node index, its inline event count, and whether
    // its first inline event was text matching `--- {attrs}`.
    let mut paragraph: Option<(usize, usize, bool)> = None;
    // True from `Start(Item)` until the item's first content, carried through
    // a loose item's `Start(Paragraph)` — the renderer's `at_item_start`.
    let mut at_item_start = false;

    let parser =
        TextMergeWithOffset::new(Parser::new_ext(body, markdown_options()).into_offset_iter());
    for (event, range) in parser {
        let range = range.start + shift..range.end + shift;
        let depth = stack.iter().filter(|open| open.node.is_some()).count();
        let was_at_item_start = at_item_start;
        at_item_start = matches!(event, Event::Start(Tag::Item))
            || (at_item_start && matches!(event, Event::Start(Tag::Paragraph)));

        let is_block = match &event {
            Event::Start(tag) => is_block_tag(tag),
            Event::End(tag) => is_block_tag_end(tag),
            Event::Rule => true,
            _ => false,
        };

        if !is_block {
            // Inline content: text, inline tags, markers, breaks, math, html.
            let task = match &event {
                Event::TaskListMarker(done) => Some(if *done { "x" } else { " " }),
                Event::Text(text) if was_at_item_start => extended_marker(text),
                _ => None,
            };
            if let Some(task) = task
                && let Some(item) = innermost(&nodes, &stack, "item")
            {
                nodes[item].task = Some(task);
            }
            if let Some((_, count, matched)) = paragraph.as_mut() {
                if *count == 0 {
                    *matched =
                        matches!(&event, Event::Text(text) if section_rule_attrs(text).is_some());
                }
                *count += 1;
            }
            if stack.last().is_some_and(|open| open.wraps_inline) {
                match synthetic.as_mut() {
                    Some((_, covered)) => covered.end = range.end,
                    None => {
                        nodes.push(OutlineNode::new(depth, "paragraph"));
                        synthetic = Some((nodes.len() - 1, range));
                    }
                }
            }
            continue;
        }

        // Any block event ends a synthetic paragraph.
        if let Some((index, covered)) = synthetic.take() {
            nodes[index].lines = span(covered);
        }

        match event {
            Event::Start(tag) => {
                if matches!(tag, Tag::Paragraph) {
                    // A real paragraph directly in an item / dd makes its
                    // list loose: pulldown-cmark only wraps loose items.
                    mark_loose(&mut nodes, &stack);
                }
                if matches!(tag, Tag::TableHead | Tag::TableRow)
                    && let Some(table) = innermost(&nodes, &stack, "table")
                    && let Some(rows) = nodes[table].rows.as_mut()
                {
                    *rows += 1;
                }
                let index = start_node(&tag, depth).map(|node| {
                    nodes.push(node);
                    nodes.len() - 1
                });
                if matches!(tag, Tag::Paragraph) {
                    paragraph = index.map(|index| (index, 0, false));
                }
                stack.push(Open {
                    node: index,
                    start: range.start,
                    wraps_inline: matches!(tag, Tag::Item | Tag::DefinitionListDefinition),
                });
            }
            Event::End(tag) => {
                let Some(open) = stack.pop() else { continue };
                if let Some(index) = open.node {
                    nodes[index].lines = span(open.start..range.end);
                }
                if matches!(tag, TagEnd::Paragraph)
                    && let Some((index, 1, true)) = paragraph
                {
                    nodes[index].kind = "section_break";
                }
                paragraph = None;
            }
            Event::Rule => {
                let mut node = OutlineNode::new(depth, "rule");
                node.lines = span(range);
                nodes.push(node);
            }
            _ => {}
        }
    }
    nodes
}

/// The innermost open node of `kind`, if any.
fn innermost(nodes: &[OutlineNode], stack: &[Open], kind: &str) -> Option<usize> {
    stack
        .iter()
        .rev()
        .filter_map(|open| open.node)
        .find(|&index| nodes[index].kind == kind)
}

/// Lines of the first and last non-whitespace bytes of `range`.
fn trimmed_lines(source: &str, lines: &LineIndex, range: Range<usize>) -> [u32; 2] {
    let text = &source[range.clone()];
    let lead = text.len() - text.trim_start().len();
    let content = text.trim();
    if content.is_empty() {
        let line = lines.line_of(range.start);
        return [line, line];
    }
    let first = range.start + lead;
    [
        lines.line_of(first),
        lines.line_of(first + content.len() - 1),
    ]
}

/// mbr's `[-]` / `[>]` task marker at the start of an item's first text run,
/// recognized by the renderer's own rule.
fn extended_marker(text: &str) -> Option<&'static str> {
    crate::markdown::split_extended_marker(text)?;
    Some(if text.starts_with("[-]") { "-" } else { ">" })
}

/// Marks the list or dl holding the innermost open item / dd as loose.
fn mark_loose(nodes: &mut [OutlineNode], stack: &[Open]) {
    let mut nodes_up = stack.iter().rev().filter_map(|open| open.node);
    if let Some(parent) = nodes_up.next()
        && matches!(nodes[parent].kind, "item" | "dd")
        && let Some(list) = nodes_up.next()
    {
        nodes[list].tight = Some(false);
    }
}

fn is_block_tag(tag: &Tag<'_>) -> bool {
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

fn is_block_tag_end(tag: &TagEnd) -> bool {
    !matches!(
        tag,
        TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link
            | TagEnd::Image
    )
}

/// The node a block tag opens, or `None` for tags that are not outline nodes.
fn start_node(tag: &Tag<'_>, depth: usize) -> Option<OutlineNode> {
    let node = |kind| OutlineNode::new(depth, kind);
    Some(match tag {
        Tag::Paragraph => node("paragraph"),
        Tag::Heading {
            level, id, classes, ..
        } => OutlineNode {
            level: Some(*level as u8),
            id: id.as_ref().map(|id| id.to_string()),
            classes: (!classes.is_empty()).then(|| classes.iter().map(|c| c.to_string()).collect()),
            ..node("heading")
        },
        Tag::BlockQuote(kind) => OutlineNode {
            alert: kind.map(alert_name),
            ..node("blockquote")
        },
        Tag::CodeBlock(kind) => OutlineNode {
            fenced: Some(matches!(kind, CodeBlockKind::Fenced(_))),
            lang: match kind {
                CodeBlockKind::Fenced(info) => info.split_whitespace().next().map(str::to_string),
                CodeBlockKind::Indented => None,
            },
            ..node("code")
        },
        Tag::HtmlBlock => node("html"),
        Tag::List(start) => OutlineNode {
            ordered: Some(start.is_some()),
            start: *start,
            tight: Some(true),
            ..node("list")
        },
        Tag::Item => node("item"),
        Tag::FootnoteDefinition(label) => OutlineNode {
            label: Some(label.to_string()),
            ..node("footnote")
        },
        Tag::DefinitionList => OutlineNode {
            tight: Some(true),
            ..node("dl")
        },
        Tag::DefinitionListTitle => node("dt"),
        Tag::DefinitionListDefinition => node("dd"),
        Tag::Table(alignments) => OutlineNode {
            align: Some(alignments.iter().map(alignment_name).collect()),
            rows: Some(0),
            ..node("table")
        },
        Tag::TableHead
        | Tag::TableRow
        | Tag::TableCell
        | Tag::MetadataBlock(_)
        | Tag::Emphasis
        | Tag::Strong
        | Tag::Strikethrough
        | Tag::Superscript
        | Tag::Subscript
        | Tag::Link { .. }
        | Tag::Image { .. } => return None,
    })
}

fn alert_name(kind: BlockQuoteKind) -> &'static str {
    match kind {
        BlockQuoteKind::Note => "note",
        BlockQuoteKind::Tip => "tip",
        BlockQuoteKind::Important => "important",
        BlockQuoteKind::Warning => "warning",
        BlockQuoteKind::Caution => "caution",
    }
}

fn alignment_name(alignment: &Alignment) -> &'static str {
    match alignment {
        Alignment::None => "none",
        Alignment::Left => "left",
        Alignment::Center => "center",
        Alignment::Right => "right",
    }
}

/// The golden file's text: a JSON array, one compact node per line.
fn render_outline(nodes: &[OutlineNode]) -> String {
    let mut out = String::from("[\n");
    for (i, node) in nodes.iter().enumerate() {
        let json = serde_json::to_string(node).expect("outline nodes always serialize");
        let sep = if i + 1 < nodes.len() { "," } else { "" };
        let _ = writeln!(out, "  {json}{sep}");
    }
    out.push_str("]\n");
    out
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/editor-roundtrip")
}

#[test]
fn renderer_outlines_match_goldens() {
    let dir = fixtures_dir();
    let updating = std::env::var_os(UPDATE_ENV).is_some();
    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    fixtures.sort();
    assert!(!fixtures.is_empty(), "no fixtures in {}", dir.display());

    let mut failures = Vec::new();
    for fixture in &fixtures {
        let source = std::fs::read_to_string(fixture).expect("fixture is UTF-8");
        let actual = render_outline(&outline(&source));
        let golden = fixture.with_extension("outline.json");
        if updating {
            std::fs::write(&golden, &actual).expect("write golden");
            continue;
        }
        match std::fs::read_to_string(&golden) {
            Ok(expected) if expected == actual => {}
            Ok(expected) => failures.push(format!(
                "{}:\n--- golden\n{expected}--- actual\n{actual}",
                fixture.display()
            )),
            Err(e) => failures.push(format!(
                "missing golden {} ({e}); regenerate with {UPDATE_ENV}=1",
                golden.display()
            )),
        }
    }

    // A golden whose fixture was deleted or renamed would be compared by the
    // TS half against nothing; refuse it here.
    let orphans: Vec<String> = std::fs::read_dir(&dir)
        .expect("fixtures dir")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.to_string_lossy().ends_with(".outline.json"))
        .filter(|path| {
            let name = path.to_string_lossy();
            !Path::new(&name.replace(".outline.json", ".md")).exists()
        })
        .map(|path| path.display().to_string())
        .collect();
    assert!(
        orphans.is_empty(),
        "outline goldens without a fixture: {orphans:?}"
    );

    assert!(
        failures.is_empty(),
        "{} outline(s) differ from their goldens. If the renderer change is \
         intended, regenerate with `{UPDATE_ENV}=1 cargo test -p mbr-core editor_parity` \
         and review the diff.\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// `(depth, kind, first, last)` of every node — the shape the unit tests pin.
fn shape(source: &str) -> Vec<(usize, &'static str, u32, u32)> {
    outline(source)
        .into_iter()
        .map(|node| (node.depth, node.kind, node.lines[0], node.lines[1]))
        .collect()
}

#[test]
fn tight_items_get_a_synthetic_paragraph_and_loose_ones_a_real_one() {
    let tight = outline("- a\n- b\n");
    let loose = outline("- a\n\n- b\n");
    assert_eq!(tight[0].tight, Some(true));
    assert_eq!(loose[0].tight, Some(false));
    // Same nodes and lines either way: tightness lives on the list alone.
    assert_eq!(
        shape("- a\n- b\n"),
        [
            (0, "list", 1, 2),
            (1, "item", 1, 1),
            (2, "paragraph", 1, 1),
            (1, "item", 2, 2),
            (2, "paragraph", 2, 2),
        ]
    );
    assert_eq!(shape("- a\n\n- b\n")[3], (1, "item", 3, 3));
}

#[test]
fn every_task_marker_is_reported_in_tight_and_loose_items() {
    let markers = |source: &str| -> Vec<Option<&'static str>> {
        outline(source)
            .into_iter()
            .filter(|node| node.kind == "item")
            .map(|node| node.task)
            .collect()
    };
    let tight = "- [ ] a\n- [x] b\n- [X] c\n- [-] d\n- [>] e\n- [-]x f\n- g\n";
    let expected = [
        Some(" "),
        Some("x"),
        Some("x"),
        Some("-"),
        Some(">"),
        None,
        None,
    ];
    assert_eq!(markers(tight), expected);
    assert_eq!(markers(&tight.replace('\n', "\n\n")), expected);
}

#[test]
fn section_rule_needs_a_paragraph_of_its_own() {
    assert_eq!(shape("a\n\n--- {#x}\n\nb\n")[1], (0, "section_break", 3, 3));
    // Directly under a paragraph it is that paragraph's second line.
    assert_eq!(shape("a\n--- {#x}\n"), [(0, "paragraph", 1, 2)]);
    // A plain rule is a rule; text around the braces is a paragraph.
    assert_eq!(shape("a\n\n---\n")[1], (0, "rule", 3, 3));
    assert_eq!(shape("--- {#x} tail\n")[0].1, "paragraph");
}

/// The spellings `components/src/editor-plugins/section-attrs.ts` treats as a
/// section break (and the near misses it does not), pinned on this side so the
/// two cannot drift.
#[test]
fn section_rule_spellings_the_editor_mirrors() {
    let kind = |source: &str| shape(source)[0].1;
    for source in [
        "--- {.a}\n",
        "\u{2014} {.a}\n",
        "&mdash; {.a}\n",
        "--- {}\n",
        "--- {data-background-iframe=https://example.com/x}\n",
    ] {
        assert_eq!(kind(source), "section_break", "{source:?}");
    }
    for source in [
        "---{x}\n",
        "---  {x}\n",
        "---- {x}\n",
        "-- {x}\n",
        "\\--- {x}\n",
        "--- {a *b*}\n",
        "--- {a `b`}\n",
    ] {
        assert_eq!(kind(source), "paragraph", "{source:?}");
    }
    assert_eq!(shape("> a\n>\n> --- {.x}\n")[2], (1, "section_break", 3, 3));
}

/// Wikilink edges `components/src/editor-plugins/wikilinks.ts` mirrors: what
/// pulldown-cmark makes a wikilink of, and its destination (target and
/// heading, raw — `\\` escapes nothing before the `|`).
#[test]
fn wikilink_edges_the_editor_mirrors() {
    use pulldown_cmark::LinkType;
    let links = |source: &str| -> Vec<(bool, String)> {
        Parser::new_ext(source, markdown_options())
            .filter_map(|event| match event {
                Event::Start(Tag::Link {
                    link_type: LinkType::WikiLink { .. },
                    dest_url,
                    ..
                }) => Some((false, dest_url.to_string())),
                Event::Start(Tag::Image {
                    link_type: LinkType::WikiLink { .. },
                    dest_url,
                    ..
                }) => Some((true, dest_url.to_string())),
                _ => None,
            })
            .collect()
    };
    let one = |embed: bool, dest: &str| vec![(embed, dest.to_string())];
    assert_eq!(links("[[Note]]"), one(false, "Note"));
    assert_eq!(links("[[Note|the alias]]"), one(false, "Note"));
    assert_eq!(links("[[Note#Heading]]"), one(false, "Note#Heading"));
    assert_eq!(links("[[#Local]]"), one(false, "#Local"));
    assert_eq!(links("![[diagram.png]]"), one(true, "diagram.png"));
    assert_eq!(links("[[a#b|c|d]]"), one(false, "a#b"));
    assert_eq!(links("[[a\\|b]]"), one(false, "a\\"));
    assert_eq!(links("[[[a]]"), one(false, "a"));
    assert_eq!(links("[[x|]]"), one(false, "x"));
    assert_eq!(links("[[a]b]]"), one(false, "a]b"));
    // Wider than the editor (which keeps these as text): a line break or a `[`.
    assert_eq!(links("[[a\nb]]"), one(false, "a\nb"));
    assert_eq!(links("[[a[b]]"), one(false, "a[b"));
    assert_eq!(links("Wow\\![[x]]"), one(false, "x"));
    for source in ["[[]]", "[[|x]]", "`[[x]]`"] {
        assert_eq!(links(source), [], "{source:?}");
    }
}

/// Heading attribute edges `components/src/editor-plugins/heading-attrs.ts`
/// mirrors: the nearest `{` opens the block, one-character tokens and
/// `key=value` pairs give no id or class, and the last `#id` wins.
#[test]
fn heading_attribute_edges_the_editor_mirrors() {
    let attrs = |source: &str| {
        let node = outline(source).remove(0);
        (node.id, node.classes)
    };
    assert_eq!(
        attrs("# Nest {#a {.b}\n"),
        (None, Some(vec!["b".to_string()]))
    );
    assert_eq!(
        attrs("# Both {#a .b .c #d}\n"),
        (
            Some("d".to_string()),
            Some(vec!["b".to_string(), "c".to_string()])
        )
    );
    assert_eq!(
        attrs("# Pairs {key=v # . .ok}\n").1,
        Some(vec!["ok".to_string()])
    );
    assert_eq!(attrs("# Esc {#a\\b}\n"), (None, None));
    assert_eq!(attrs("Setext {#s}\n===\n").0, Some("s".to_string()));
}

#[test]
fn definition_list_nesting_and_frontmatter_lines() {
    let source = "---\ntitle: x\n---\nTerm\n: one\n  * aid\n: two\n";
    assert_eq!(
        shape(source),
        [
            (0, "dl", 4, 7),
            (1, "dt", 4, 4),
            (1, "dd", 5, 6),
            (2, "paragraph", 5, 5),
            (2, "list", 6, 6),
            (3, "item", 6, 6),
            (4, "paragraph", 6, 6),
            (1, "dd", 7, 7),
            (2, "paragraph", 7, 7),
        ]
    );
}

#[test]
fn bom_and_crlf_do_not_shift_lines() {
    assert_eq!(
        shape("\u{feff}# A\r\n\r\nb\r\nc\r\n"),
        [(0, "heading", 1, 1), (0, "paragraph", 3, 4),]
    );
}

#[test]
fn golden_text_is_one_node_per_line() {
    let text = render_outline(&outline("# A {#a}\n\n| x |\n|:-:|\n| 1 |\n"));
    assert_eq!(
        text,
        "[\n  {\"depth\":0,\"kind\":\"heading\",\"lines\":[1,1],\"level\":1,\"id\":\"a\"},\n  \
         {\"depth\":0,\"kind\":\"table\",\"lines\":[3,5],\"align\":[\"center\"],\"rows\":2}\n]\n"
    );
}
