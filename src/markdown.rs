use crate::attrs::ParsedAttrs;
use crate::errors::MarkdownError;
use crate::link_index::{OutboundLink, is_internal_link, split_url_anchor};
use crate::link_transform::{LinkTransformConfig, transform_link};
use crate::media::MediaEmbed;
use crate::oembed::PageInfo;
use crate::oembed_cache::OembedCache;
use crate::tasks::{self, TaskStatus};
use crate::vid::Vid;
use crate::wikilink::{parse_tag_link, transform_wikilinks};
use crate::wikilink_index::WikilinkIndex;
use pulldown_cmark::{
    BlockQuoteKind, CowStr, Event, HeadingLevel, LinkType, MetadataBlockKind, Options,
    Parser as MDParser, Tag, TagEnd, TextMergeStream, TextMergeWithOffset,
};
use regex::Regex;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use yaml_rust2::{Yaml, YamlLoader};

/// Markdown parser options.
///
/// Uses `Options::all()` to enable all pulldown-cmark features including wikilinks.
///
/// Wikilink processing flow:
/// 1. `transform_wikilinks` runs FIRST on raw markdown, converting tag-style wikilinks
///    like `[[Tags:rust]]` to standard markdown links `[rust](/tags/rust/)`
/// 2. pulldown-cmark then parses the result, handling plain wikilinks like `[[Whatever]]`
///    natively with its ENABLE_WIKILINKS support
///
/// This hybrid approach allows us to:
/// - Support custom tag-source links (`[[Source:value]]`)
/// - Preserve standard wikilink behavior for plain `[[page]]` links
pub(crate) fn markdown_options() -> Options {
    Options::all()
}

/// UTF-8 byte-order mark. Some editors (notably on Windows) prepend it to
/// markdown files. pulldown-cmark treats it as ordinary content, so a BOM in
/// front of `---` suppresses the YAML metadata block entirely: frontmatter is
/// dropped from site.json, search, and the relationship graph, while the page
/// visibly renders the frontmatter as an em-dash heading. Every entry point
/// that hands text to the parser strips it first.
const BOM: char = '\u{feff}';

/// Returns `input` without a leading [`BOM`].
fn strip_bom(input: &str) -> &str {
    input.strip_prefix(BOM).unwrap_or(input)
}

/// Owned counterpart of [`strip_bom`]: removes a leading [`BOM`] in place.
///
/// Uses `drain` rather than reallocating, and touches nothing at all for the
/// overwhelmingly common BOM-less case.
fn strip_bom_in_place(input: &mut String) {
    if strip_bom(input).len() != input.len() {
        input.drain(..BOM.len_utf8());
    }
}

/// Loads the first YAML document from `text`, returning `None` when the text
/// fails to parse *or* contains no document at all.
///
/// yaml-rust2 returns `Ok(vec![])` for a metadata block whose body is only
/// comments (`# tags: [draft]`), so indexing `[0]` on the result panics — and
/// release builds abort (`panic = 'abort'`). Always go through this helper.
fn load_first_yaml_doc(text: &str) -> Option<Yaml> {
    YamlLoader::load_from_str(text)
        .ok()
        .and_then(|docs| docs.into_iter().next())
}

/// Result of parsing a markdown file without rendering to HTML.
///
/// Owns the source string so consumers can iterate over events
/// without lifetime concerns. Use [`events()`](Self::events) to
/// get the pulldown-cmark event stream.
#[derive(Debug, Clone)]
pub struct ParsedDocument {
    /// The (possibly wikilink-transformed) markdown source.
    pub source: String,
    /// Frontmatter metadata extracted from the document.
    pub frontmatter: SimpleMetadata,
    /// Table of contents (headings with anchor IDs).
    pub headings: Vec<HeadingInfo>,
    /// Whether the document starts with an H1 heading.
    pub has_h1: bool,
    /// Word count (excluding code blocks and metadata).
    pub word_count: usize,
}

impl ParsedDocument {
    /// Returns an iterator over pulldown-cmark events for this document.
    ///
    /// The events use the same parser options as mbr's HTML renderer,
    /// ensuring consistent parsing behavior.
    pub fn events(&self) -> TextMergeStream<'_, MDParser<'_>> {
        let parser = MDParser::new_ext(&self.source, markdown_options());
        TextMergeStream::new(parser)
    }
}

/// Parse a markdown file into a [`ParsedDocument`] without rendering to HTML.
///
/// Reads the file, extracts frontmatter and headings, and returns the parsed
/// document. Consumers can iterate over the event stream via
/// [`ParsedDocument::events()`] to render in any format (terminal, HTML, etc.).
///
/// Wikilink transforms are not applied (no tag sources configured in this path).
pub fn parse<P: AsRef<Path>>(file: P) -> Result<ParsedDocument, MarkdownError> {
    let file = file.as_ref();
    let mut markdown_input = fs::read_to_string(file).map_err(|e| MarkdownError::ReadFailed {
        path: file.to_path_buf(),
        source: e,
    })?;
    strip_bom_in_place(&mut markdown_input);

    // Task markup is skipped: this entry point returns an event stream for
    // callers to render themselves, and only reads the events here for
    // headings, frontmatter and word counts.
    let (events, headings, _section_attrs) =
        collect_events_and_headings(&markdown_input, TaskMarkup::Skip);
    let has_h1 = headings.first().is_some_and(|h| h.level == 1);

    // Single pass: extract frontmatter and count words
    let mut frontmatter = SimpleMetadata::new();
    let mut word_count: usize = 0;
    let mut in_yaml = false;
    let mut in_code_block = false;
    let mut in_metadata_block = false;
    for event in &events {
        match event {
            Event::Start(Tag::MetadataBlock(MetadataBlockKind::YamlStyle)) => {
                in_yaml = true;
                in_metadata_block = true;
            }
            Event::End(TagEnd::MetadataBlock(MetadataBlockKind::YamlStyle)) => {
                in_yaml = false;
                in_metadata_block = false;
            }
            Event::Text(text) if in_yaml => {
                let metadata_parsed = load_first_yaml_doc(text);
                frontmatter = yaml_frontmatter_simplified(&metadata_parsed);
                in_yaml = false;
            }
            Event::Start(Tag::MetadataBlock(_)) => in_metadata_block = true,
            Event::End(TagEnd::MetadataBlock(_)) => in_metadata_block = false,
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => in_code_block = false,
            Event::Text(text) if !in_code_block && !in_metadata_block => {
                word_count += text.split_whitespace().count();
            }
            _ => {}
        }
    }

    if !frontmatter.contains_key("title") && has_h1 {
        frontmatter.insert(
            "title".to_string(),
            serde_json::Value::String(headings[0].text.clone()),
        );
    }

    Ok(ParsedDocument {
        source: markdown_input,
        frontmatter,
        headings,
        has_h1,
        word_count,
    })
}

/// Represents a heading in the document for table of contents generation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HeadingInfo {
    pub level: u8,
    pub text: String,
    pub id: String,
}

/// Result of rendering a markdown file to HTML.
///
/// Contains the rendered HTML along with metadata extracted during parsing.
#[derive(Debug, Clone)]
pub struct MarkdownRenderResult {
    /// Frontmatter metadata (from YAML block at top of file)
    pub frontmatter: SimpleMetadata,
    /// YAML frontmatter parse error, if the metadata block failed to parse.
    ///
    /// When `Some`, the whole frontmatter map was discarded (yaml-rust2 aborts
    /// the document on the first error), so valid fields are silently lost.
    /// Surfaced to the reader via the per-page errors endpoint and to the
    /// builder via a stderr summary.
    pub frontmatter_error: Option<String>,
    /// Table of contents (headings extracted from document)
    pub headings: Vec<HeadingInfo>,
    /// Rendered HTML content
    pub html: String,
    /// Links discovered during rendering (for backlink tracking)
    pub outbound_links: Vec<OutboundLink>,
    /// True if the document's first heading is an H1 (affects title rendering)
    pub has_h1: bool,
    /// Word count of the document (excluding code blocks and metadata)
    pub word_count: usize,
    /// Sentence count of the document (excluding code blocks and metadata).
    ///
    /// Approximated by scanning for terminal punctuation (`.!?`) in text
    /// events, plus one-per-block for paragraphs, headings, and list items
    /// whose final text did not end in terminal punctuation.
    pub sentence_count: usize,
    /// Syllable count of the document (excluding code blocks and metadata).
    ///
    /// Computed via [`crate::readability::count_syllables`] for each
    /// whitespace-delimited word during rendering.
    pub syllable_count: usize,
    /// Bare body wikilinks whose name is shared by several notes, deduped.
    ///
    /// Resolution is unchanged (first-wins); these are surfaced to the reader
    /// via the per-page errors endpoint so an accidental namesake link can be
    /// spotted. Always empty when the render had no wikilink index (CLI /
    /// QuickLook paths).
    pub ambiguous_wikilinks: Vec<crate::wikilink_index::AmbiguousWikilink>,
}

struct EventState {
    #[allow(dead_code)] // Reserved for future use (resolving relative paths)
    root_path: PathBuf,
    /// Path of the file being processed, used to name the file in diagnostics.
    ///
    /// Without it a YAML frontmatter warning is unactionable: the reader is told
    /// *that* a metadata block failed to parse but not *which* of their thousands
    /// of notes to open.
    file_path: PathBuf,
    /// Track the current media embed type (if any) for proper closing tags
    current_media: Option<MediaEmbed>,
    in_metadata: bool,
    in_link: bool, // Track when inside a link (including autolinks like <http://...>)
    metadata_source: Option<MetadataBlockKind>,
    metadata_parsed: Option<Yaml>,
    /// Configuration for transforming relative links
    link_transform_config: LinkTransformConfig,
    /// Global name index for Obsidian-style body-wikilink (`[[Name]]`)
    /// resolution. `None` when there is no repo context (CLI/QuickLook paths);
    /// only bare wikilinks not found in the current folder consult it.
    wikilink_index: Option<Arc<WikilinkIndex>>,
    /// Pre-fetched oembed results for bare URLs (populated during parallel fetch phase)
    prefetched_oembed: HashMap<String, PageInfo>,
    /// True in server/GUI mode, false in build/CLI mode
    server_mode: bool,
    /// True when dynamic video transcoding is enabled
    transcode_enabled: bool,
    /// Collected outbound links from the document
    collected_links: Vec<OutboundLink>,
    /// Current link destination URL being processed (set on Start(Link))
    current_link_dest: Option<String>,
    /// Current link text being accumulated
    current_link_text: String,
    /// Valid tag sources for detecting tag links (e.g., "tags", "performers")
    valid_tag_sources: HashSet<String>,
    /// Word count accumulator for text content
    word_count: usize,
    /// Track if we're inside a code block (to exclude from word count)
    in_code_block: bool,
    /// Sentence count accumulator (via terminal punctuation + block-end bumps)
    sentence_count: usize,
    /// Syllable count accumulator (summed per counted word)
    syllable_count: usize,
    /// Whether the last observed non-metadata/non-code text ended with
    /// terminal punctuation. Used to bump `sentence_count` at the end of
    /// paragraphs, headings, and list items whose final text lacked a `.!?`.
    block_needs_sentence_bump: bool,
    /// Captured YAML frontmatter parse error, if the metadata block failed to
    /// parse. When set, the entire frontmatter was discarded (so otherwise
    /// valid fields like `style` are lost); surfaced to the user via the
    /// per-page error reporting and a build-mode summary.
    frontmatter_error: Option<String>,
    /// Bare body wikilinks on this page whose name is shared by several notes,
    /// deduped. Resolution is unaffected; these are reported so the author knows
    /// mbr picked one arbitrarily.
    ambiguous_wikilinks: Vec<crate::wikilink_index::AmbiguousWikilink>,
}

/// Frontmatter metadata as a flat key/value map.
///
/// This is a [`BTreeMap`] rather than a `HashMap` so that serialization is
/// deterministic. `tera`'s `preserve_order` feature turns on
/// `serde_json/indexmap`, which makes JSON object key order equal *insertion*
/// order — with a randomly-seeded `HashMap` that made `window.frontmatter` and
/// every `frontmatter` object in `.mbr/site.json` reshuffle on each run, so two
/// builds of an identical repository produced different bytes. Ordering was
/// never YAML source order, so alphabetical is a strict improvement.
pub type SimpleMetadata = BTreeMap<String, serde_json::Value>;

/// Scan a slice of text for sentence-terminating punctuation (`.!?`).
///
/// Returns `(count, ends_with_terminator)` where:
///
/// * `count` — the number of in-text sentence terminators, defined as a `.!?`
///   that is followed by either whitespace or the end of the slice, and which
///   is not part of a run of terminators (so `...` and `?!` count once).
/// * `ends_with_terminator` — whether the last non-whitespace character is one
///   of `.!?`. This is used by the render loop to decide whether to credit
///   the enclosing block (paragraph/heading/item) with one extra sentence.
///
/// The heuristic is intentionally simple: it does not attempt to detect
/// abbreviations like "Dr." or "e.g." — these false positives are unlikely to
/// materially shift the FRE/FKGL band for a document of any meaningful length.
fn count_sentence_terminators(text: &str) -> (usize, bool) {
    let bytes = text.as_bytes();
    let mut count: usize = 0;
    let mut prev_was_terminator = false;
    for (i, &b) in bytes.iter().enumerate() {
        let is_terminator = matches!(b, b'.' | b'!' | b'?');
        if is_terminator && !prev_was_terminator {
            // Count only when the terminator is followed by whitespace or is
            // the last non-whitespace character. This avoids counting every
            // `.` in URLs and numeric contexts.
            let next_is_boundary = bytes[i + 1..]
                .iter()
                .find(|&&c| !matches!(c, b'.' | b'!' | b'?'))
                .is_none_or(|&c| c.is_ascii_whitespace());
            if next_is_boundary {
                count += 1;
            }
        }
        prev_was_terminator = is_terminator;
    }

    let ends_with_terminator = text
        .trim_end()
        .chars()
        .next_back()
        .is_some_and(|c| matches!(c, '.' | '!' | '?'));

    (count, ends_with_terminator)
}

/// Extracts the first H1 heading text from markdown content.
///
/// This is used to provide a title fallback when no frontmatter title exists.
/// Only extracts the first H1 found; subsequent H1s are ignored.
pub fn extract_first_h1(markdown_input: &str) -> Option<String> {
    // Use minimal parser options: only YAML metadata (to skip frontmatter blocks)
    // ATX headings are parsed by default without any feature flags
    let parser = MDParser::new_ext(markdown_input, Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    let parser = TextMergeStream::new(parser);

    let mut in_h1 = false;
    let mut h1_text = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) => {
                in_h1 = true;
            }
            // Inline code spans are part of the visible heading text, so a
            // title like "# The `main` function" must not lose the code word.
            Event::Text(text) | Event::Code(text) if in_h1 => {
                h1_text.push_str(&text);
            }
            Event::End(TagEnd::Heading(HeadingLevel::H1)) => {
                if !h1_text.is_empty() {
                    return Some(h1_text);
                }
                in_h1 = false;
            }
            _ => {}
        }
    }
    None
}

/// Em dash character (U+2014) - what `---` becomes with smart punctuation
const EM_DASH: &str = "\u{2014}";

/// Maps a non-standard [remark-hint](https://github.com/sergioramos/remark-hint)
/// paragraph prefix to its GitHub-alert equivalent.
///
/// Returns the [`BlockQuoteKind`] and the text with the marker stripped, or
/// `None` when the text does not begin with a recognized hint marker.
fn detect_hint_prefix(text: &str) -> Option<(BlockQuoteKind, &str)> {
    // Dispatch on the first byte so the common (non-hint) paragraph bails out after
    // a single comparison instead of attempting every prefix.
    let (prefix, kind) = match text.as_bytes().first()? {
        b'!' => ("!> ", BlockQuoteKind::Tip),
        b'?' => ("?> ", BlockQuoteKind::Warning),
        b'x' => ("x> ", BlockQuoteKind::Caution),
        _ => return None,
    };
    text.strip_prefix(prefix).map(|rest| (kind, rest))
}

/// Transform events: detect `--- {attrs}` pattern and convert to Rule + attrs.
///
/// When pulldown-cmark (with TextMergeStream) sees `--- {#id .class}` on a single line,
/// it produces:
/// - Start(Paragraph)
/// - Text("— {#id .class}") (em dash + space + attrs, merged into one Text)
/// - End(Paragraph)
///
/// This function detects that pattern and transforms it into a single Rule event,
/// extracting the attributes for section rendering.
///
/// Returns (transformed_events, section_attrs) where section_attrs maps section
/// index to parsed attributes.
///
/// Note: This logic is now inlined into `collect_events_and_headings` for the main
/// render path. This standalone function is kept for potential standalone use.
#[allow(dead_code)]
fn transform_rule_attrs(events: Vec<Event<'_>>) -> (Vec<Event<'_>>, HashMap<usize, ParsedAttrs>) {
    let mut result = Vec::with_capacity(events.len());
    let mut section_attrs = HashMap::new();
    let mut section_index = 0;
    let mut i = 0;

    while i < events.len() {
        // Detect pattern: Start(Paragraph), Text("— {attrs}"), End(Paragraph)
        // TextMergeStream merges adjacent Text events, so we see a single Text event
        if i + 2 < events.len()
            && let (Event::Start(Tag::Paragraph), Event::Text(text), Event::End(TagEnd::Paragraph)) =
                (&events[i], &events[i + 1], &events[i + 2])
            // Check: text starts with em dash + space + "{" and ends with "}"
            && text.starts_with(EM_DASH)
            && let Some(attrs_str) = text.strip_prefix(EM_DASH)
            && attrs_str.starts_with(" {")
            && attrs_str.ends_with('}')
            && let Some(attrs) = ParsedAttrs::parse(attrs_str.trim())
        {
            // Transform: emit Rule instead of paragraph
            result.push(Event::Rule);
            section_index += 1;
            section_attrs.insert(section_index, attrs);
            i += 3; // Skip all 3 events
            continue;
        }

        // Track real Rule events for section counting
        if matches!(&events[i], Event::Rule) {
            section_index += 1;
        }

        result.push(events[i].clone());
        i += 1;
    }

    (result, section_attrs)
}

/// Byte offset → 1-based line number lookups over a markdown source.
///
/// Built once per document and only when the document actually contains a task
/// checkbox, so prose-only pages — the overwhelming majority — never pay for
/// the scan. Lookups binary-search rather than counting newlines per marker,
/// which keeps a document of a thousand tasks from becoming quadratic.
struct LineIndex {
    /// Byte offset of every `\n`, ascending.
    newlines: Vec<usize>,
}

/// Bytes per line assumed when sizing a [`LineIndex`]. Markdown is prose, so
/// this is a close enough guess that the vector rarely has to grow.
const ASSUMED_LINE_BYTES: usize = 32;

/// Ceiling on that guess, so a file that is one enormous line (a minified blob
/// with a `.md` extension, say) cannot reserve megabytes it will never use.
const MAX_RESERVED_LINES: usize = 1 << 16;

impl LineIndex {
    fn build(source: &str) -> Self {
        // `match_indices` over a `char` pattern takes the standard library's
        // vectorised byte search, which measured ~5x faster than the obvious
        // `bytes().enumerate().filter(..)` loop (32.5us against 6.4us on a
        // 60 kB document) -- and that loop was, before this, the single largest
        // cost of task rendering on a large page.
        let mut newlines =
            Vec::with_capacity((source.len() / ASSUMED_LINE_BYTES).min(MAX_RESERVED_LINES));
        newlines.extend(source.match_indices('\n').map(|(offset, _)| offset));
        Self { newlines }
    }

    /// The 1-based line containing byte `offset`.
    fn line_of(&self, offset: usize) -> u32 {
        // `partition_point` counts the newlines strictly before `offset`, which
        // is the number of complete lines preceding it.
        let preceding = self.newlines.partition_point(|&newline| newline < offset);
        u32::try_from(preceding + 1).unwrap_or(u32::MAX)
    }
}

/// Whether [`collect_events_and_headings`] should also rewrite task list items
/// into mbr's checkbox-and-chips markup.
///
/// [`TaskMarkup::Skip`] exists for the callers that only want the event stream:
/// the repository-wide backlink scan runs over every markdown file in the
/// repository, and running the annotation grammar over every task line of every
/// one of them buys it nothing — it collects link destinations, which this
/// rewrite never touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskMarkup {
    Render,
    Skip,
}

/// Decodes a task checkbox from `event`, or `None` if it is not one.
///
/// `at_item_start` must be true only when `event` is the first inline event of
/// a list item. That is the sole position where the non-standard `[-]` and
/// `[>]` markers are recognised, and gating on it is also what keeps a `[-]`
/// written inside a fenced code block, a heading, or ordinary prose from
/// turning into a checkbox — none of those is a list item's first inline event.
fn task_marker_status(event: &Event<'_>, at_item_start: bool) -> Option<TaskStatus> {
    match event {
        // pulldown-cmark recognises `[ ]` and `[x]` natively.
        Event::TaskListMarker(true) => Some(TaskStatus::Done),
        Event::TaskListMarker(false) => Some(TaskStatus::Open),
        Event::Text(text) if at_item_start => split_extended_marker(text).map(|(status, _)| status),
        _ => None,
    }
}

/// Splits mbr's `[-]` (canceled) or `[>]` (moved elsewhere) marker off the
/// front of a list item's first text run, returning the status and the text
/// that follows it.
///
/// pulldown-cmark only understands `[ ]` and `[x]`, so these two reach the
/// renderer as ordinary text and mbr has to pick them out itself. The marker
/// must be followed by whitespace or end the run, matching the grammar in
/// [`crate::tasks`], so `[-]x` is not a checkbox.
fn split_extended_marker(text: &str) -> Option<(TaskStatus, &str)> {
    let rest = text
        .strip_prefix("[-]")
        .or_else(|| text.strip_prefix("[>]"))?;
    match rest.as_bytes().first() {
        None => Some((TaskStatus::Canceled, rest)),
        // One ASCII byte, so the split is always on a char boundary.
        Some(b' ' | b'\t') => Some((TaskStatus::Canceled, &rest[1..])),
        Some(_) => None,
    }
}

/// True for events that belong to a block's inline content.
///
/// A task's text span closes at the first event that is not one of these — a
/// nested `Start(List)`, the item's `End(Paragraph)`, or its `End(Item)` — so
/// the annotation chips land at the end of the task's own line rather than
/// after its subtasks.
fn is_inline_event(event: &Event<'_>) -> bool {
    match event {
        Event::Text(_)
        | Event::Code(_)
        | Event::InlineMath(_)
        | Event::DisplayMath(_)
        | Event::InlineHtml(_)
        | Event::FootnoteReference(_)
        | Event::SoftBreak
        | Event::HardBreak
        | Event::TaskListMarker(_) => true,
        Event::Start(tag) => matches!(
            tag,
            Tag::Emphasis
                | Tag::Strong
                | Tag::Strikethrough
                | Tag::Superscript
                | Tag::Subscript
                | Tag::Link { .. }
                | Tag::Image { .. }
        ),
        Event::End(tag) => matches!(
            tag,
            TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Superscript
                | TagEnd::Subscript
                | TagEnd::Link
                | TagEnd::Image
        ),
        // `Event::Html` is a raw *block*, `Event::Rule` a thematic break.
        _ => false,
    }
}

/// Merged pass 1: parse markdown, extract headings with anchor IDs, detect
/// `--- {attrs}` rule patterns, and rewrite task list items -- all in a single
/// iteration over the parser output.
///
/// Returns (events, headings, section_attrs).
///
/// This merges what were previously separate passes (heading extraction loop +
/// `transform_rule_attrs`) into one. The rule-attrs detection uses a 3-element
/// look-back buffer: when we encounter `End(Paragraph)`, we check if the preceding
/// two events form the `Start(Paragraph), Text("em-dash + attrs")` pattern.
///
/// Task rewriting is folded in here rather than run as its own pass for two
/// reasons. The source line each checkbox came from is only knowable from the
/// parser's byte ranges, which exist nowhere else; and a separate pass would
/// have to allocate and move a second copy of the whole event vector, which
/// measured as most of the cost of the feature on a task-heavy document.
/// Everything else discards its range immediately.
///
/// # Task output shape
///
/// For `- [ ] fix **this** !! #bug @due(2026-08-05)` on line 1:
///
/// ```html
/// <li><input type="checkbox" class="mbr-task-check" id="mbr-task-1"
///            data-mbr-task-line="1" data-mbr-task-status="open" disabled>
/// <span class="mbr-task-text">fix <strong>this</strong></span>
/// <span class="mbr-task-pri mbr-task-pri-high" …></span>
/// <span class="mbr-task-tag">#bug</span>
/// <time class="mbr-task-due" datetime="2026-08-05">Aug 5</time></li>
/// ```
///
/// # Caveat: offsets are into the transformed source
///
/// The ranges index `markdown_input`, which is the wikilink-substituted source
/// rather than the file on disk. Line numbers survive that substitution because
/// `transform_wikilinks` only ever rewrites within a single line: a wikilink
/// that spans a line break is not a wikilink (see `wikilink::push_transformed`),
/// so no rewrite can add or remove a newline.
fn collect_events_and_headings(
    markdown_input: &str,
    task_markup: TaskMarkup,
) -> (
    Vec<Event<'_>>,
    Vec<HeadingInfo>,
    HashMap<usize, ParsedAttrs>,
) {
    let parser = MDParser::new_ext(markdown_input, markdown_options()).into_offset_iter();
    let parser = TextMergeWithOffset::new(parser);

    let mut events = Vec::new();
    let mut headings = Vec::new();
    let mut anchor_ids: HashMap<String, usize> = HashMap::new();
    let mut in_heading_text: Option<String> = None;
    let mut section_attrs = HashMap::new();
    let mut section_index = 0;
    let mut hint_open = false;
    // Built on the first checkbox, so a document without tasks never scans for
    // newlines at all.
    let mut line_index: Option<LineIndex> = None;
    let mut at_item_start = false;
    let mut pending_task: Option<PendingTask> = None;

    for (event, range) in parser {
        let was_at_item_start = at_item_start;
        // A loose list item wraps its content in a paragraph, so the marker
        // arrives one event later there than in a tight list.
        at_item_start = matches!(event, Event::Start(Tag::Item))
            || (at_item_start && matches!(event, Event::Start(Tag::Paragraph)));

        if task_markup == TaskMarkup::Render {
            if let Some(status) = task_marker_status(&event, was_at_item_start) {
                // A marker always opens a fresh item, so nothing should still
                // be open; closing defensively beats emitting a stray `<span>`.
                if let Some(open) = pending_task.take() {
                    close_task(&mut events, open);
                }
                let index = line_index.get_or_insert_with(|| LineIndex::build(markdown_input));
                let line = index.line_of(range.start);

                events.push(Event::Html(CowStr::from(crate::html::task_checkbox_html(
                    status,
                    Some(line),
                ))));
                events.push(Event::Html(CowStr::from(crate::html::task_text_open(
                    status,
                ))));

                let mut task = PendingTask {
                    text_at: Vec::new(),
                };
                // `[-]` / `[>]` are not parser-recognised markers: the checkbox
                // and the first run of display text arrive as one text event, so
                // the remainder has to be put back.
                if let Event::Text(text) = &event
                    && let Some((_, rest)) = split_extended_marker(text)
                {
                    task.text_at.push(events.len());
                    events.push(Event::Text(CowStr::from(rest.to_string())));
                }
                pending_task = Some(task);
                // The marker event itself is replaced, so it is never pushed.
                continue;
            }

            if let Some(task) = pending_task.as_mut() {
                if is_inline_event(&event) {
                    // Pushed here rather than falling through to the match, so
                    // the recorded index is provably the slot the run lands in.
                    // None of the arms below applies to a task's inline content:
                    // the heading arms need an open heading and the hint arm a
                    // `Start(Paragraph)` immediately behind, which the span-open
                    // event displaces.
                    if matches!(event, Event::Text(_)) {
                        task.text_at.push(events.len());
                    }
                    events.push(event);
                    continue;
                }
                // End of the task's own line: close the text span and emit the
                // chips before whatever block comes next (a nested subtask
                // list, the item's end, a second paragraph).
                if let Some(task) = pending_task.take() {
                    close_task(&mut events, task);
                }
            }
        }

        match &event {
            // --- Heading extraction ---
            Event::Start(Tag::Heading { .. }) => {
                in_heading_text = Some(String::new());
                events.push(event);
            }
            // Heading label accumulation. `Event::Code` (inline code spans) and
            // `Event::InlineMath` carry visible heading text and must be
            // included, otherwise "The `main` function" yields the label
            // "The  function" and the anchor id `the--function`.
            //
            // Deliberately NOT accumulated: `Event::InlineHtml` (its payload is
            // raw markup like `<kbd>`, whose inner text already arrives as a
            // separate `Event::Text`) and `Event::FootnoteReference` (the label
            // is a citation marker, not part of the heading's name).
            Event::Text(text) | Event::Code(text) | Event::InlineMath(text)
                if in_heading_text.is_some() =>
            {
                if let Some(ref mut heading_text) = in_heading_text {
                    heading_text.push_str(text);
                }
                events.push(event);
            }

            // --- remark-hint syntax detection (inline) ---
            // A paragraph whose first text run starts with `!> `/`?> `/`x> ` becomes the
            // matching GitHub-style alert blockquote (Tip/Warning/Caution).
            Event::Text(text) if matches!(events.last(), Some(Event::Start(Tag::Paragraph))) => {
                if let Some((kind, rest)) = detect_hint_prefix(text) {
                    events.pop(); // remove the Start(Paragraph)
                    events.push(Event::Start(Tag::BlockQuote(Some(kind))));
                    events.push(Event::Start(Tag::Paragraph));
                    events.push(Event::Text(CowStr::from(rest.to_owned())));
                    hint_open = true;
                    continue;
                }
                events.push(event);
            }
            Event::End(TagEnd::Heading(heading_level)) => {
                if let Some(text) = in_heading_text.take() {
                    let id = generate_anchor_id(&text, &mut anchor_ids);
                    let level_num = match heading_level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    };

                    headings.push(HeadingInfo {
                        level: level_num,
                        text: text.clone(),
                        id: id.clone(),
                    });

                    // Walk backward to find the matching Start(Heading) and inject the ID
                    for i in (0..events.len()).rev() {
                        if let Event::Start(Tag::Heading {
                            level,
                            id: _,
                            classes,
                            attrs,
                        }) = &events[i]
                        {
                            events[i] = Event::Start(Tag::Heading {
                                level: *level,
                                id: Some(CowStr::from(id)),
                                classes: classes.clone(),
                                attrs: attrs.clone(),
                            });
                            break;
                        }
                    }
                }
                events.push(event);
            }

            // --- Rule attrs detection (inline) ---
            // Detect End(Paragraph) and look back for the 3-event pattern:
            //   Start(Paragraph), Text("em-dash + {attrs}"), End(Paragraph)
            Event::End(TagEnd::Paragraph) => {
                // Close an open remark-hint alert: emit the paragraph end followed by
                // the blockquote end. A hint paragraph never matches the em-dash rule
                // pattern, so handling it first is safe.
                if hint_open {
                    events.push(event);
                    events.push(Event::End(TagEnd::BlockQuote(None)));
                    hint_open = false;
                    continue;
                }

                let len = events.len();
                // Need at least 2 prior events to form the pattern
                if len >= 2 {
                    let is_rule_attrs = matches!(
                        (&events[len - 2], &events[len - 1]),
                        (Event::Start(Tag::Paragraph), Event::Text(_))
                    ) && {
                        if let Event::Text(text) = &events[len - 1] {
                            text.starts_with(EM_DASH)
                                && text.strip_prefix(EM_DASH).is_some_and(|rest| {
                                    rest.starts_with(" {") && rest.ends_with('}')
                                })
                        } else {
                            false
                        }
                    };

                    if is_rule_attrs {
                        // Extract and parse attrs from the text event
                        let parsed = if let Event::Text(text) = &events[len - 1] {
                            text.strip_prefix(EM_DASH)
                                .and_then(|rest| ParsedAttrs::parse(rest.trim()))
                        } else {
                            None
                        };

                        // Remove the Start(Paragraph) and Text events
                        events.pop(); // Text
                        events.pop(); // Start(Paragraph)

                        // Emit a Rule event instead
                        events.push(Event::Rule);
                        section_index += 1;

                        if let Some(attrs) = parsed {
                            section_attrs.insert(section_index, attrs);
                        }
                        // Skip pushing the End(Paragraph) event
                        continue;
                    }
                }
                events.push(event);
            }

            // Track real Rule events for section counting
            Event::Rule => {
                section_index += 1;
                events.push(event);
            }

            _ => {
                events.push(event);
            }
        }
    }

    // A document that ends mid-item still has to close its span.
    if let Some(task) = pending_task.take() {
        close_task(&mut events, task);
    }

    (events, headings, section_attrs)
}

/// A task item whose checkbox has been emitted and whose text span is still open.
struct PendingTask {
    /// Positions in the output of the text runs making up the display text.
    ///
    /// Collected rather than stripped as they arrive: the annotation grammar
    /// has end-anchored rules (the trailing `> YYYY-MM-DD`, the whitespace
    /// collapse), so no run can be rewritten until the last one has been seen.
    text_at: Vec<usize>,
}

/// Closes an open task: strips the annotations out of its text runs, rewrites
/// them in place, and appends the closing span plus the annotation chips.
fn close_task(output: &mut Vec<Event<'_>>, task: PendingTask) {
    let (stripped, annotations) = {
        let runs: Vec<&str> = task
            .text_at
            .iter()
            .map(|&index| match &output[index] {
                Event::Text(text) => text.as_ref(),
                // Only text events are recorded, so this is unreachable.
                _ => "",
            })
            .collect();
        tasks::strip_annotations_across_runs(&runs)
    };

    for (&index, text) in task.text_at.iter().zip(stripped) {
        output[index] = Event::Text(CowStr::from(text));
    }

    output.push(Event::Html(CowStr::from(crate::html::TASK_TEXT_CLOSE)));
    let chips = crate::html::task_annotations_html(&annotations);
    if !chips.is_empty() {
        output.push(Event::Html(CowStr::from(chips)));
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn render(
    file: PathBuf,
    root_path: &Path,
    oembed_timeout_ms: u64,
    link_transform_config: LinkTransformConfig,
    server_mode: bool,
    transcode_enabled: bool,
    valid_tag_sources: HashSet<String>,
    mark_incomplete: bool,
    incomplete_markers: &[String],
    wikilink_index: Option<Arc<WikilinkIndex>>,
) -> Result<MarkdownRenderResult, MarkdownError> {
    render_with_cache(
        file,
        root_path,
        oembed_timeout_ms,
        link_transform_config,
        None,
        server_mode,
        transcode_enabled,
        valid_tag_sources,
        mark_incomplete,
        incomplete_markers,
        wikilink_index,
    )
    .await
}

/// Renders markdown to HTML with optional OEmbed caching support.
///
/// When `oembed_cache` is provided, cached results are used when available and
/// new results are cached for future use. URLs are fetched in parallel for improved
/// performance when multiple bare URLs are present in the document.
///
/// - `server_mode`: True in server/GUI mode, false in build/CLI mode
/// - `transcode_enabled`: True when dynamic video transcoding is enabled
/// - `valid_tag_sources`: Set of valid tag source names for wikilink transformation
#[allow(clippy::too_many_arguments)]
pub async fn render_with_cache(
    file: PathBuf,
    root_path: &Path,
    oembed_timeout_ms: u64,
    link_transform_config: LinkTransformConfig,
    oembed_cache: Option<Arc<OembedCache>>,
    server_mode: bool,
    transcode_enabled: bool,
    valid_tag_sources: HashSet<String>,
    mark_incomplete: bool,
    incomplete_markers: &[String],
    wikilink_index: Option<Arc<WikilinkIndex>>,
) -> Result<MarkdownRenderResult, MarkdownError> {
    // Read markdown input. Use tokio's async filesystem API so this (potentially
    // slow) read does not block a tokio worker thread in the async render path.
    let mut raw_markdown_input =
        tokio::fs::read_to_string(&file)
            .await
            .map_err(|e| MarkdownError::ReadFailed {
                path: file.clone(),
                source: e,
            })?;
    strip_bom_in_place(&mut raw_markdown_input);

    // Transform [[Source:value]] wikilinks to standard markdown links before parsing
    let markdown_input = if valid_tag_sources.is_empty() {
        raw_markdown_input
    } else {
        transform_wikilinks(&raw_markdown_input, &valid_tag_sources)
    };

    // Single merged pass: collect events, extract headings with anchor IDs,
    // detect `--- {attrs}` rule patterns, and rewrite task list items (merging
    // what were previously the heading extraction loop, transform_rule_attrs
    // and a separate task pass into one iteration). Running the task rewrite
    // here rather than after `process_all_events` also means the annotations
    // are gone before text is counted for readability and scanned for bare URLs.
    let (events_with_ids, headings, section_attrs) =
        collect_events_and_headings(&markdown_input, TaskMarkup::Render);

    // Detect if the first heading is an H1 (used for conditional title rendering in templates)
    let has_h1 = headings.first().is_some_and(|h| h.level == 1);

    // No-network embeds (YouTube/Giphy/gist/bare media) are pure CPU and require
    // no I/O, so they are produced regardless of `oembed_timeout_ms` — the docs
    // promise they keep working when oembed is disabled. Only network OpenGraph
    // enrichment is gated by the timeout; `prefetch_oembed_urls` must stay behind
    // that gate because calling it at timeout 0 would fill the cache with empty
    // `PageInfo`s. Keep this block in sync with `render_sync`, which duplicates
    // the same pipeline for the rayon/build path.
    let mut prefetched_oembed = collect_local_embeds(&events_with_ids);
    if oembed_timeout_ms > 0 {
        for (url, info) in
            prefetch_oembed_urls(&events_with_ids, oembed_timeout_ms, &oembed_cache).await
        {
            prefetched_oembed.entry(url).or_insert(info);
        }
    }

    // Pass 2: process events through our custom logic (link transforms, media embeds, etc.)
    let (processed_events, state) = process_all_events(
        events_with_ids,
        root_path,
        &file,
        link_transform_config,
        prefetched_oembed,
        server_mode,
        transcode_enabled,
        valid_tag_sources,
        wikilink_index,
    );

    // Pass 3 (optional): wrap blocks starting with TK/TODO/FIXME/XXX in
    // <span class="mbr-incomplete">…</span>. Off by default in build mode.
    let processed_events = if mark_incomplete {
        match build_incomplete_marker_regex(incomplete_markers) {
            Some(re) => mark_incomplete_blocks(processed_events, &re),
            None => processed_events,
        }
    } else {
        processed_events
    };

    // Generate HTML output and extract frontmatter
    finalize_render(
        processed_events,
        state,
        section_attrs,
        &markdown_input,
        headings,
        has_h1,
    )
}

/// Runs process_event over all events, returning the processed events and final state.
///
/// This is the shared event processing pass used by both `render_with_cache` (async)
/// and `render_sync`. It handles link transforms, media embeds, YAML frontmatter,
/// vid shortcodes, bare URL oembed lookups, and word counting.
///
/// `file_path` is only used to name the file in diagnostics (e.g. a YAML
/// frontmatter parse warning); it is never read from.
#[allow(clippy::too_many_arguments)]
fn process_all_events<'a>(
    events: Vec<Event<'a>>,
    root_path: &Path,
    file_path: &Path,
    link_transform_config: LinkTransformConfig,
    prefetched_oembed: HashMap<String, PageInfo>,
    server_mode: bool,
    transcode_enabled: bool,
    valid_tag_sources: HashSet<String>,
    wikilink_index: Option<Arc<WikilinkIndex>>,
) -> (Vec<Event<'a>>, EventState) {
    let mut state = EventState {
        root_path: root_path.to_path_buf(),
        file_path: file_path.to_path_buf(),
        current_media: None,
        in_metadata: false,
        in_link: false,
        metadata_source: None,
        metadata_parsed: None,
        link_transform_config,
        wikilink_index,
        prefetched_oembed,
        server_mode,
        transcode_enabled,
        collected_links: Vec::new(),
        current_link_dest: None,
        current_link_text: String::new(),
        valid_tag_sources,
        word_count: 0,
        in_code_block: false,
        sentence_count: 0,
        syllable_count: 0,
        block_needs_sentence_bump: false,
        frontmatter_error: None,
        ambiguous_wikilinks: Vec::new(),
    };
    let mut processed_events = Vec::with_capacity(events.len());

    for event in events {
        let (processed, new_state) = process_event(event, state);
        state = new_state;
        processed_events.push(processed);
    }

    (processed_events, state)
}

const INCOMPLETE_SPAN_OPEN: &str = "<span class=\"mbr-incomplete\">";
const INCOMPLETE_SPAN_CLOSE: &str = "</span>";

/// Build a `^(?:M1|M2|...)\b` regex from `markers`. Empty markers → None
/// (caller should skip the pass). Markers are escaped via `regex::escape`.
pub(crate) fn build_incomplete_marker_regex(markers: &[String]) -> Option<Regex> {
    let parts: Vec<String> = markers
        .iter()
        .filter(|m| !m.is_empty())
        .map(|m| regex::escape(m))
        .collect();
    if parts.is_empty() {
        return None;
    }
    let pattern = format!("^(?:{})\\b", parts.join("|"));
    Regex::new(&pattern).ok()
}

/// Wrap inline content of blocks whose first text matches `marker_re` in
/// `<span class="mbr-incomplete">…</span>`.
///
/// Eligible (innermost) blocks: `Paragraph`, `Heading{..}`, `Item`, `TableCell`.
/// Other container tags (`BlockQuote`, `List`, `Table`, code blocks, etc.) are
/// skipped — their inner Paragraph (or absence thereof) is what we evaluate.
fn mark_incomplete_blocks<'a>(events: Vec<Event<'a>>, marker_re: &Regex) -> Vec<Event<'a>> {
    struct Frame {
        start_idx: usize,
        has_seen_text: bool,
        marker_open: bool,
    }

    let mut output: Vec<Event<'a>> = Vec::with_capacity(events.len());
    let mut stack: Vec<Frame> = Vec::new();

    for event in events {
        match &event {
            Event::Start(Tag::Paragraph)
            | Event::Start(Tag::Heading { .. })
            | Event::Start(Tag::Item)
            | Event::Start(Tag::TableCell) => {
                let start_idx = output.len();
                output.push(event);
                stack.push(Frame {
                    start_idx,
                    has_seen_text: false,
                    marker_open: false,
                });
            }
            Event::End(TagEnd::Paragraph)
            | Event::End(TagEnd::Heading(_))
            | Event::End(TagEnd::Item)
            | Event::End(TagEnd::TableCell) => {
                if let Some(frame) = stack.pop()
                    && frame.marker_open
                {
                    output.push(Event::Html(CowStr::from(INCOMPLETE_SPAN_CLOSE)));
                }
                output.push(event);
            }
            Event::Text(text) => {
                if let Some(top) = stack.last_mut()
                    && !top.has_seen_text
                {
                    top.has_seen_text = true;
                    if marker_re.is_match(text.trim_start()) {
                        // Insert span open immediately after this frame's Start event.
                        output.insert(
                            top.start_idx + 1,
                            Event::Html(CowStr::from(INCOMPLETE_SPAN_OPEN)),
                        );
                        top.marker_open = true;
                    }
                }
                output.push(event);
            }
            _ => {
                output.push(event);
            }
        }
    }

    output
}

/// Generates final HTML output and constructs the MarkdownRenderResult.
///
/// Shared finalization logic for both `render_with_cache` and `render_sync`:
/// deduplicates outbound links, generates HTML via `push_html_mbr_with_attrs`,
/// extracts frontmatter, and injects H1 title fallback.
fn finalize_render(
    processed_events: Vec<Event<'_>>,
    state: EventState,
    section_attrs: HashMap<usize, ParsedAttrs>,
    markdown_input: &str,
    headings: Vec<HeadingInfo>,
    has_h1: bool,
) -> Result<MarkdownRenderResult, MarkdownError> {
    // Write to a new String buffer with MBR extensions (sections, mermaid)
    let mut html_output = String::with_capacity(markdown_input.len() * 2);

    // Deduplicate outbound links by target URL - if a page links to the same
    // target multiple times, we only keep the first occurrence
    let mut seen_targets: HashSet<String> = HashSet::new();
    let deduplicated_links: Vec<OutboundLink> = state
        .collected_links
        .into_iter()
        .filter(|link| seen_targets.insert(link.to.clone()))
        .collect();

    crate::html::push_html_mbr_with_attrs(
        &mut html_output,
        processed_events.into_iter(),
        section_attrs,
    );

    // Extract frontmatter and inject H1 title if no frontmatter title exists
    let mut frontmatter = yaml_frontmatter_simplified(&state.metadata_parsed);
    if !frontmatter.contains_key("title")
        && let Some(h1_text) = headings
            .first()
            .filter(|h| h.level == 1)
            .map(|h| h.text.clone())
    {
        frontmatter.insert("title".to_string(), serde_json::Value::String(h1_text));
    }

    Ok(MarkdownRenderResult {
        frontmatter,
        frontmatter_error: state.frontmatter_error,
        headings,
        html: html_output,
        outbound_links: deduplicated_links,
        has_h1,
        word_count: state.word_count,
        sentence_count: state.sentence_count,
        syllable_count: state.syllable_count,
        ambiguous_wikilinks: state.ambiguous_wikilinks,
    })
}

/// Synchronous version of `render_with_cache()` for use from rayon threads.
///
/// Performs the same rendering pipeline but without async: file reading (already sync),
/// wikilink transformation, merged heading + rule-attrs pass, process_event pass, and
/// HTML generation.
///
/// No-network embeds (Giphy, GitHub gist, and bare-URL media) ARE produced in the
/// sync path — they are pure CPU (regex/string) and require no I/O, so they work in
/// build mode regardless of `oembed_timeout_ms`. Only OpenGraph network fetches are
/// skipped here; when `oembed_timeout_ms > 0` and a cache is present, previously
/// cached network results are also merged in (but never fetched fresh).
#[allow(clippy::too_many_arguments)]
pub fn render_sync(
    file: PathBuf,
    root_path: &Path,
    oembed_timeout_ms: u64,
    link_transform_config: LinkTransformConfig,
    oembed_cache: Option<Arc<OembedCache>>,
    server_mode: bool,
    transcode_enabled: bool,
    valid_tag_sources: HashSet<String>,
    mark_incomplete: bool,
    incomplete_markers: &[String],
    wikilink_index: Option<Arc<WikilinkIndex>>,
) -> Result<MarkdownRenderResult, MarkdownError> {
    // Read markdown input
    let mut raw_markdown_input =
        fs::read_to_string(&file).map_err(|e| MarkdownError::ReadFailed {
            path: file.clone(),
            source: e,
        })?;
    strip_bom_in_place(&mut raw_markdown_input);

    // Transform [[Source:value]] wikilinks to standard markdown links before parsing
    let markdown_input = if valid_tag_sources.is_empty() {
        raw_markdown_input
    } else {
        transform_wikilinks(&raw_markdown_input, &valid_tag_sources)
    };

    // Single merged pass: collect events, extract headings with anchor IDs,
    // detect `--- {attrs}` rule patterns, and rewrite task list items.
    let (events_with_ids, headings, section_attrs) =
        collect_events_and_headings(&markdown_input, TaskMarkup::Render);

    // Detect if the first heading is an H1
    let has_h1 = headings.first().is_some_and(|h| h.level == 1);

    // No-network embeds (Giphy/gist/media) are cheap and require no I/O, so
    // compute them even in the sync/build path (they work regardless of
    // oembed_timeout_ms). Network OpenGraph results are only pulled from cache
    // here (the sync path never performs network fetches).
    // Mirrors the equivalent block in `render_with_cache`; the two entry points
    // duplicate the whole pipeline and must be changed together.
    let mut prefetched_oembed = collect_local_embeds(&events_with_ids);
    if oembed_timeout_ms > 0
        && let Some(ref cache) = oembed_cache
    {
        for (url, info) in collect_cached_oembed(&events_with_ids, cache) {
            prefetched_oembed.entry(url).or_insert(info);
        }
    }

    // Pass 2: process events through our custom logic (link transforms, media embeds, etc.)
    let (processed_events, state) = process_all_events(
        events_with_ids,
        root_path,
        &file,
        link_transform_config,
        prefetched_oembed,
        server_mode,
        transcode_enabled,
        valid_tag_sources,
        wikilink_index,
    );

    // Pass 3 (optional): wrap blocks starting with TK/TODO/FIXME/XXX in
    // <span class="mbr-incomplete">…</span>. Off by default in build mode.
    let processed_events = if mark_incomplete {
        match build_incomplete_marker_regex(incomplete_markers) {
            Some(re) => mark_incomplete_blocks(processed_events, &re),
            None => processed_events,
        }
    } else {
        processed_events
    };

    // Generate HTML output and extract frontmatter
    finalize_render(
        processed_events,
        state,
        section_attrs,
        &markdown_input,
        headings,
        has_h1,
    )
}

/// Extract only the outbound links of a markdown file.
///
/// Runs the same sync pipeline as [`render_sync`] — BOM strip, tag-wikilink
/// substitution, event collection, `process_all_events` (which is where link
/// transformation and `[[wikilink]]` resolution happen) — and then stops,
/// skipping HTML generation and frontmatter extraction. Callers that need the
/// rendered page should use [`render_sync`]; this exists for the server's
/// repository-wide backlink index, which parses every markdown file once and
/// only ever looks at the collected links.
///
/// Links are deduplicated by target exactly as [`finalize_render`] does, so a
/// page's link list is identical whichever entry point produced it.
///
/// Network oembed is not consulted (the sync pipeline never fetches anyway).
/// That can change which *external* links are collected — a bare URL that
/// would have become an embed stays an autolink — but never the internal ones,
/// which are all the backlink index inverts.
pub fn extract_outbound_links_sync(
    file: PathBuf,
    root_path: &Path,
    link_transform_config: LinkTransformConfig,
    server_mode: bool,
    valid_tag_sources: HashSet<String>,
    wikilink_index: Option<Arc<WikilinkIndex>>,
) -> Result<Vec<OutboundLink>, MarkdownError> {
    let mut raw_markdown_input =
        fs::read_to_string(&file).map_err(|e| MarkdownError::ReadFailed {
            path: file.clone(),
            source: e,
        })?;
    strip_bom_in_place(&mut raw_markdown_input);

    let markdown_input = if valid_tag_sources.is_empty() {
        raw_markdown_input
    } else {
        transform_wikilinks(&raw_markdown_input, &valid_tag_sources)
    };

    // Task markup is skipped: it rewrites text runs, never link destinations,
    // so it cannot change which links this function collects -- and this runs
    // over every markdown file in the repository.
    let (events_with_ids, _headings, _section_attrs) =
        collect_events_and_headings(&markdown_input, TaskMarkup::Skip);

    let prefetched_oembed = collect_local_embeds(&events_with_ids);

    let (_processed_events, state) = process_all_events(
        events_with_ids,
        root_path,
        &file,
        link_transform_config,
        prefetched_oembed,
        server_mode,
        false, // transcode_enabled: irrelevant to link collection
        valid_tag_sources,
        wikilink_index,
    );

    let mut seen_targets: HashSet<String> = HashSet::new();
    Ok(state
        .collected_links
        .into_iter()
        .filter(|link| seen_targets.insert(link.to.clone()))
        .collect())
}

/// Compute no-network oembed results (Giphy, gist, bare-URL media) for all
/// bare URLs in `events`. Pure/synchronous — safe for the build (rayon) path.
fn collect_local_embeds(events: &[Event<'_>]) -> HashMap<String, PageInfo> {
    collect_bare_urls(events)
        .into_iter()
        .filter_map(|url| PageInfo::local_embed(&url).map(|info| (url, info)))
        .collect()
}

/// Collect oembed results from cache only (no network fetches).
///
/// Used by `render_sync` to leverage cached oembed data without blocking on I/O.
fn collect_cached_oembed(events: &[Event<'_>], cache: &OembedCache) -> HashMap<String, PageInfo> {
    let urls = collect_bare_urls(events);
    let mut results = HashMap::new();
    for url in urls {
        if let Some(info) = cache.get(&url) {
            results.insert(url, info);
        }
    }
    results
}

/// Pre-pass to collect all bare URLs that need oembed fetching.
///
/// This identifies text events that look like bare URLs (start with "http", no spaces,
/// and not inside a link element). These URLs are then fetched in parallel for better
/// performance.
///
/// Code blocks are skipped: a URL that only appears in a code sample is content,
/// not a link, and fetching it would make mbr issue an outbound request for text
/// the author never intended to embed.
fn collect_bare_urls(events: &[Event<'_>]) -> HashSet<String> {
    let mut urls = HashSet::new();
    let mut in_link = false;
    let mut in_metadata = false;
    let mut in_code_block = false;

    for event in events {
        match event {
            Event::Start(Tag::Link { .. }) => in_link = true,
            Event::End(TagEnd::Link) => in_link = false,
            Event::Start(Tag::MetadataBlock(_)) => in_metadata = true,
            Event::End(TagEnd::MetadataBlock(_)) => in_metadata = false,
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => in_code_block = false,
            Event::Text(text)
                if !in_link
                    && !in_metadata
                    && !in_code_block
                    && text.starts_with("http")
                    && !text.contains(' ')
                    && !text.trim_start().starts_with("{{") =>
            {
                urls.insert(text.to_string());
            }
            _ => {}
        }
    }

    urls
}

/// Maximum number of oembed URLs fetched concurrently for a single document.
///
/// Small on purpose: unbounded fan-out (one in-flight request per distinct bare
/// URL) pressures the file-descriptor limit and turns a single markdown file
/// into a traffic amplifier against whatever host it names.
const OEMBED_FETCH_CONCURRENCY: usize = 8;

/// Maximum number of distinct bare URLs a single document fetches oembed
/// metadata for. URLs beyond the cap render as plain links.
const MAX_OEMBED_FETCHES_PER_DOC: usize = 100;

/// Applies [`MAX_OEMBED_FETCHES_PER_DOC`] to the list of URLs still needing a
/// network fetch.
///
/// Sorts before truncating so the surviving subset is deterministic:
/// `collect_bare_urls` returns a `HashSet`, whose iteration order varies from
/// process to process, which would otherwise make static builds irreproducible.
/// The sort is skipped entirely when the document is under the cap.
fn cap_fetch_list(mut urls: Vec<String>) -> Vec<String> {
    if urls.len() > MAX_OEMBED_FETCHES_PER_DOC {
        tracing::warn!(
            "oembed prefetch: {} bare URLs exceeds the per-document cap of {}; \
             the remainder will render as plain links",
            urls.len(),
            MAX_OEMBED_FETCHES_PER_DOC
        );
        urls.sort_unstable();
        urls.truncate(MAX_OEMBED_FETCHES_PER_DOC);
    }
    urls
}

/// Fetches oembed data for a collection of URLs in parallel.
///
/// Uses the cache when available to avoid redundant network requests.
/// New results are stored in the cache for future use.
///
/// Concurrency is bounded by [`OEMBED_FETCH_CONCURRENCY`] and the number of
/// fetches per document by [`MAX_OEMBED_FETCHES_PER_DOC`].
async fn prefetch_oembed_urls(
    events: &[Event<'_>],
    oembed_timeout_ms: u64,
    oembed_cache: &Option<Arc<OembedCache>>,
) -> HashMap<String, PageInfo> {
    let urls = collect_bare_urls(events);

    if urls.is_empty() {
        return HashMap::new();
    }

    tracing::debug!("oembed prefetch: found {} bare URLs to fetch", urls.len());

    // Partition URLs into cached and uncached
    let (cached, uncached): (Vec<_>, Vec<_>) = urls
        .into_iter()
        .partition(|url| oembed_cache.as_ref().and_then(|c| c.get(url)).is_some());

    let mut results = HashMap::new();

    // Add cached results
    if let Some(cache) = oembed_cache {
        for url in cached {
            if let Some(info) = cache.get(&url) {
                results.insert(url, info);
            }
        }
    }

    // Fetch uncached URLs with bounded concurrency
    let to_fetch = cap_fetch_list(uncached);
    if !to_fetch.is_empty() {
        use futures::stream::StreamExt;

        tracing::debug!(
            "oembed prefetch: {} cached, {} to fetch",
            results.len(),
            to_fetch.len()
        );

        let fetched: Vec<_> = futures::stream::iter(to_fetch)
            .map(|url| async move {
                tracing::debug!("oembed fetch start: {}", url);
                let result = PageInfo::new_from_url(&url, oembed_timeout_ms)
                    .await
                    .unwrap_or_else(|_| PageInfo {
                        url: url.clone(),
                        ..Default::default()
                    });
                tracing::debug!("oembed fetch complete: {}", url);
                (url, result)
            })
            .buffer_unordered(OEMBED_FETCH_CONCURRENCY)
            .collect()
            .await;

        // Store results and cache them
        for (url, info) in fetched {
            if let Some(cache) = oembed_cache {
                cache.insert(url.clone(), info.clone());
            }
            results.insert(url, info);
        }
    }

    results
}

fn yaml_frontmatter_simplified(y: &Option<Yaml>) -> SimpleMetadata {
    match y.as_ref().and_then(|yaml| yaml.as_hash()) {
        Some(hash) => yaml_hash_to_metadata(hash),
        None => SimpleMetadata::new(),
    }
}

/// Converts a YAML hash to simplified metadata, borrowing instead of cloning.
fn yaml_hash_to_metadata(hash: &yaml_rust2::yaml::Hash) -> SimpleMetadata {
    let mut hm = SimpleMetadata::new();
    for (k, v) in hash.iter() {
        match (k, v) {
            (Yaml::String(key), Yaml::String(value)) => {
                tracing::trace!("Frontmatter: {key} = {value}");
                hm.insert(key.clone(), serde_json::Value::String(value.clone()));
            }
            (Yaml::String(key), Yaml::Array(vals)) => {
                // Preserve arrays as JSON arrays instead of joining them
                let arr: Vec<serde_json::Value> = vals
                    .iter()
                    .filter_map(|val| val.as_str())
                    .map(|s| serde_json::Value::String(s.to_string()))
                    .collect();
                tracing::trace!("Frontmatter: {key} = {:?}", &arr);
                hm.insert(key.clone(), serde_json::Value::Array(arr));
            }
            (Yaml::String(key), Yaml::Hash(nested_hash)) => {
                tracing::trace!("Frontmatter: {key} = (nested hash)");
                // Recursively parse nested hashes and flatten with dot notation
                let nested = yaml_hash_to_metadata(nested_hash);
                for (k, v) in nested {
                    hm.insert(key.to_string() + "." + k.as_str(), v);
                }
            }
            (Yaml::String(key), Yaml::Integer(val)) => {
                tracing::trace!("Frontmatter: {key} = {val}");
                hm.insert(key.clone(), serde_json::json!(val));
            }
            (Yaml::String(key), Yaml::Real(val)) => {
                tracing::trace!("Frontmatter: {key} = {val}");
                hm.insert(key.clone(), serde_json::Value::String(val.clone()));
            }
            (Yaml::String(key), Yaml::Boolean(val)) => {
                tracing::trace!("Frontmatter: {key} = {val}");
                hm.insert(key.clone(), serde_json::json!(val));
            }
            (Yaml::String(key), other_val) => {
                tracing::trace!("Frontmatter: {key} = {:?}", &other_val);
                if let Some(str_val) = other_val.as_str() {
                    hm.insert(key.clone(), serde_json::Value::String(str_val.to_string()));
                }
            }
            (k, v) => {
                tracing::warn!("Unexpected frontmatter key-value: {:?} = {:?}", k, v);
            }
        }
    }
    hm
}

/// Maximum bytes to read when extracting frontmatter metadata.
/// Frontmatter should always be at the top of the file, so 8KB is plenty.
const FRONTMATTER_MAX_BYTES: usize = 8 * 1024;

/// Frontmatter extracted from a file: the simplified metadata map plus the
/// typed relationships parsed from the raw YAML.
///
/// Returning both from a single read avoids a second file read for the typed
/// relationship path (the simplified map is lossy for array-of-object fields).
#[derive(Debug, Clone, Default)]
pub struct FileMetadata {
    /// The simplified frontmatter metadata (string/array/scalar values).
    pub metadata: SimpleMetadata,
    /// Typed relationships declared in frontmatter (unresolved endpoints).
    pub relationships: Vec<crate::relationships::RawRelationship>,
}

pub fn extract_metadata_from_file<P: AsRef<Path>>(path: P) -> Result<FileMetadata, MarkdownError> {
    let path = path.as_ref();
    // Only read the first 8KB - frontmatter is always at the top
    let mut file = File::open(path).map_err(|e| MarkdownError::ReadFailed {
        path: path.to_path_buf(),
        source: e,
    })?;
    let file_len = file.metadata().map(|m| m.len() as usize).unwrap_or(0);
    let read_len = file_len.min(FRONTMATTER_MAX_BYTES);
    let mut buffer = vec![0u8; read_len];
    file.read_exact(&mut buffer)
        .map_err(|e| MarkdownError::ReadFailed {
            path: path.to_path_buf(),
            source: e,
        })?;
    let decoded = String::from_utf8_lossy(&buffer);
    let markdown_input = strip_bom(&decoded);
    let parser = MDParser::new_ext(markdown_input, Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    let parser = TextMergeStream::new(parser);
    let mut in_metadata = false;
    let mut hm = SimpleMetadata::new();
    let mut relationships = Vec::new();
    for event in parser.take(4) {
        match &event {
            Event::Start(Tag::MetadataBlock(MetadataBlockKind::YamlStyle)) => {
                in_metadata = true;
            }
            Event::End(TagEnd::MetadataBlock(MetadataBlockKind::YamlStyle)) => {
                break;
            }
            Event::Text(text) if in_metadata => {
                // A parse failure discards the *entire* frontmatter: `type`,
                // `aliases`, `relationships` and all. This is the scan path that
                // feeds the relationship index, so a silent drop here shows up
                // as a flood of "unresolved relationship endpoint" warnings from
                // other notes that referenced this one by an alias that never
                // made it into the index. Log it, naming the file, then fall
                // back to empty metadata exactly as before so the scan continues.
                let metadata_parsed = match YamlLoader::load_from_str(text) {
                    Ok(docs) => docs.into_iter().next(),
                    Err(e) => {
                        tracing::warn!(
                            path = %path.display(),
                            "Failed to parse YAML frontmatter: {e}; the whole \
                             frontmatter block (including any `aliases` and \
                             `relationships`) is ignored for this note"
                        );
                        None
                    }
                };

                if let Some(ref yaml) = metadata_parsed {
                    relationships = crate::relationships::parse_relationships(yaml);
                }
                hm = yaml_frontmatter_simplified(&metadata_parsed);
                break;
            }
            _ => {}
        }
    }

    // If no frontmatter title, try to extract the first H1 from the content
    if !hm.contains_key("title")
        && let Some(h1_text) = extract_first_h1(markdown_input)
    {
        hm.insert("title".to_string(), serde_json::Value::String(h1_text));
    }

    Ok(FileMetadata {
        metadata: hm,
        relationships,
    })
}

/// Generates a URL-safe anchor ID from heading text.
///
/// Handles duplicates by appending `-2`, `-3`, … and guarantees the emitted id
/// is unique across the whole document. `anchor_ids` therefore doubles as the
/// set of already-issued ids: a bare per-base counter is not sufficient, because
/// the composed suffix can collide with a *different* heading whose own slug
/// happens to match it (`["Step 1", "Step 1", "Step 1-2"]` would otherwise hand
/// out `step-1-2` twice).
fn generate_anchor_id(text: &str, anchor_ids: &mut HashMap<String, usize>) -> String {
    // Convert to lowercase and replace spaces and special chars with dashes
    let base_id = text
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else if c.is_whitespace() {
                '-'
            } else {
                // Remove special characters
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");

    // Handle empty IDs
    let base_id = if base_id.is_empty() {
        "heading".to_string()
    } else {
        base_id
    };

    // Walk the suffix upward until the composed candidate is genuinely unused.
    let mut count = anchor_ids.get(&base_id).copied().unwrap_or(0);
    let candidate = loop {
        count += 1;
        let candidate = if count == 1 {
            base_id.clone()
        } else {
            format!("{}-{}", base_id, count)
        };
        if !anchor_ids.contains_key(&candidate) {
            break candidate;
        }
    };

    anchor_ids.insert(base_id, count);
    // Reserve the emitted id itself so a later heading that slugifies straight
    // to it is pushed onto a different suffix instead of colliding.
    anchor_ids.entry(candidate.clone()).or_insert(0);
    candidate
}

/// Processes a single markdown event, transforming it as needed.
///
/// This function is now synchronous because all async work (oembed fetching)
/// is done in the prefetch phase. Bare URLs are looked up in the prefetched
/// results instead of being fetched inline.
fn process_event(
    event: pulldown_cmark::Event<'_>,
    mut state: EventState,
) -> (pulldown_cmark::Event<'_>, EventState) {
    match &event {
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            // Transform the URL first for trailing-slash URL convention
            // This applies to all images/media, not just regular images
            let transformed_url = transform_link(dest_url, &state.link_transform_config);

            match MediaEmbed::from_url_and_title(&transformed_url, title) {
                Some(media) => {
                    // the link title is actually the next Text event so need to split this to only produce the open tags
                    let html = media.to_html(true, state.server_mode, state.transcode_enabled);
                    state.current_media = Some(media);
                    (Event::Html(html.into()), state)
                }
                _ => {
                    let new_event = Event::Start(Tag::Image {
                        link_type: *link_type,
                        dest_url: CowStr::from(transformed_url),
                        title: title.clone(),
                        id: id.clone(),
                    });
                    (new_event, state)
                }
            }
        }
        Event::End(TagEnd::Image) => {
            if let Some(media) = state.current_media.take() {
                (Event::Html(media.html_close().into()), state)
            } else {
                (event, state)
            }
        }
        Event::Start(Tag::MetadataBlock(v)) => {
            state.metadata_source = Some(*v);
            state.in_metadata = true;
            (event.clone(), state)
        }
        Event::End(TagEnd::MetadataBlock(_)) => {
            state.in_metadata = false;
            (event.clone(), state)
        }
        // Track when we're inside a link (including autolinks like <http://...>)
        // and transform the link URL for trailing-slash URL convention
        // Also detect and transform tag links like [text](Tags:rust) -> [text](/tags/rust/)
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            state.in_link = true;
            // Store the original destination URL for link tracking
            state.current_link_dest = Some(dest_url.to_string());
            state.current_link_text.clear();

            // First check if this is a tag link (e.g., Tags:rust, performers:Joshua Jay)
            // If so, transform to the tag URL path (/tags/rust/, /performers/joshua_jay/)
            let transformed_url =
                if let Some(wikilink) = parse_tag_link(dest_url, &state.valid_tag_sources) {
                    transform_link(&wikilink.url_path(), &state.link_transform_config)
                } else {
                    // Not a tag link. For bare-name body wikilinks (`[[Name]]`),
                    // apply Obsidian-style global resolution: current folder first,
                    // else the first matching file anywhere. `resolve_wikilink`
                    // returns Some only for the global-fallback case, so same-folder
                    // links keep the default relative transform byte-for-byte.
                    let is_bare_wikilink =
                        matches!(link_type, LinkType::WikiLink { .. }) && !dest_url.contains('/');
                    let global = if is_bare_wikilink {
                        state.wikilink_index.as_ref().and_then(|idx| {
                            idx.resolve_wikilink(
                                dest_url,
                                &state.link_transform_config.current_page_url,
                                state.link_transform_config.is_index_file,
                            )
                        })
                    } else {
                        None
                    };
                    // Record namesake ambiguity separately from `global`: a
                    // same-folder link needs no rewrite (`global` is None) yet can
                    // still have resolved arbitrarily between two case-variant
                    // files, and a global-fallback link that *did* rewrite may have
                    // had several candidates to choose from.
                    let ambiguous = if is_bare_wikilink {
                        state.wikilink_index.as_ref().and_then(|idx| {
                            idx.ambiguity_for(
                                dest_url,
                                &state.link_transform_config.current_page_url,
                                state.link_transform_config.is_index_file,
                            )
                        })
                    } else {
                        None
                    };
                    if let Some(found) = ambiguous
                        && !state.ambiguous_wikilinks.contains(&found)
                    {
                        state.ambiguous_wikilinks.push(found);
                    }
                    match global {
                        Some(abs) => {
                            // Override the recorded outbound target with the absolute
                            // URL so link validation and backlinks resolve correctly
                            // (its leading `/` makes `resolve_outbound_links` leave it
                            // untouched, and the path resolver then finds it).
                            state.current_link_dest = Some(abs.clone());
                            transform_link(&abs, &state.link_transform_config)
                        }
                        None => transform_link(dest_url, &state.link_transform_config),
                    }
                };

            let new_event = Event::Start(Tag::Link {
                link_type: *link_type,
                dest_url: CowStr::from(transformed_url),
                title: title.clone(),
                id: id.clone(),
            });
            (new_event, state)
        }
        Event::End(TagEnd::Link) => {
            state.in_link = false;
            // Collect the outbound link
            if let Some(dest_url) = state.current_link_dest.take() {
                let (path, anchor) = split_url_anchor(&dest_url);
                let internal = is_internal_link(&dest_url);
                let link = OutboundLink {
                    to: path,
                    text: std::mem::take(&mut state.current_link_text),
                    anchor,
                    internal,
                };
                state.collected_links.push(link);
            }
            (event, state)
        }
        // Track code blocks to exclude from word count
        Event::Start(Tag::CodeBlock(_)) => {
            state.in_code_block = true;
            (event, state)
        }
        Event::End(TagEnd::CodeBlock) => {
            state.in_code_block = false;
            (event, state)
        }
        // Block boundaries for readability's sentence count: paragraphs,
        // headings, and list items whose last text did not end in `.!?` get
        // one implicit sentence credit. This avoids undercounting headings
        // ("Introduction") and terse bullet items ("Install Rust").
        Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item) => {
            if state.block_needs_sentence_bump {
                state.sentence_count += 1;
                state.block_needs_sentence_bump = false;
            }
            (event, state)
        }
        Event::Text(text) => {
            // Accumulate link text when inside a link
            if state.in_link {
                state.current_link_text.push_str(text);
            }
            // Count words, sentences, and syllables in text content
            // (excluding metadata and code blocks).
            if !state.in_metadata && !state.in_code_block {
                for word in text.split_whitespace() {
                    state.word_count += 1;
                    state.syllable_count += crate::readability::count_syllables(word);
                }
                let (sentences_in_text, ends_with_terminator) = count_sentence_terminators(text);
                state.sentence_count += sentences_in_text;
                // Track whether the enclosing block still needs a sentence
                // bump at its End tag. Trailing whitespace is ignored: we
                // care whether the last non-space character is `.!?`.
                let trimmed = text.trim_end();
                if !trimmed.is_empty() {
                    state.block_needs_sentence_bump = !ends_with_terminator;
                }
            }
            if state.in_metadata {
                match YamlLoader::load_from_str(text) {
                    Ok(docs) => state.metadata_parsed = docs.into_iter().next(),
                    Err(e) => {
                        // Invalid YAML aborts the whole frontmatter block, so
                        // otherwise-valid fields (e.g. `style: slides`) are
                        // silently lost. Capture the error so it can be
                        // surfaced to the user instead of disappearing, and name
                        // the file — the error alone does not identify which of
                        // a repository's notes to go and fix.
                        tracing::warn!(
                            path = %state.file_path.display(),
                            "Failed to parse YAML frontmatter: {e}"
                        );
                        state.frontmatter_error = Some(e.to_string());
                    }
                }
                (event, state)
            } else if state.in_code_block {
                // Code blocks are verbatim: the vid shortcode and bare-URL
                // oembed rewrites below must never fire on sample code. Checked
                // after `in_metadata` (not folded into it) so we never run the
                // YAML loader over code text.
                (event, state)
            } else if !state.in_link && text.starts_with("http") && !text.contains(' ') {
                // Only process bare URLs that are NOT inside a link element.
                // URLs in <http://...> autolinks or [text](url) links are already
                // handled by markdown and shouldn't trigger oembed fetching.
                //
                // Look up the prefetched result instead of fetching inline.
                let url_str = text.to_string();
                let info = state
                    .prefetched_oembed
                    .get(&url_str)
                    .cloned()
                    .unwrap_or_else(|| PageInfo {
                        url: url_str,
                        ..Default::default()
                    });
                (Event::Html(info.html().into()), state)
            } else if text.trim_start().starts_with("{{") {
                if let Some(mut vid) = Vid::from_vid(text) {
                    vid.url = transform_link(&vid.url, &state.link_transform_config);
                    (
                        Event::Html(
                            vid.to_html(false, state.server_mode, state.transcode_enabled)
                                .into(),
                        ),
                        state,
                    )
                } else {
                    (event, state)
                }
            } else {
                (event, state)
            }
        }
        _ => (event, state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    async fn render_markdown(content: &str) -> String {
        render_markdown_with_config(content, false, HashSet::new()).await
    }

    async fn render_markdown_with_tags(content: &str, tag_sources: HashSet<String>) -> String {
        render_markdown_with_config(content, false, tag_sources).await
    }

    /// Renders with an explicit `server_mode`, for asserting that output does
    /// *not* vary between a served page and a static build.
    async fn render_markdown_with_mode(content: &str, server_mode: bool) -> String {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
        };
        render(
            path,
            &root,
            0,
            config,
            server_mode,
            false,
            HashSet::new(),
            false,
            &[],
            None,
        )
        .await
        .unwrap()
        .html
    }

    async fn render_markdown_with_config(
        content: &str,
        is_index_file: bool,
        tag_sources: HashSet<String>,
    ) -> String {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
        };
        // Tests run with server_mode=false, transcode_enabled=false, mark_incomplete=false
        let result = render(
            path,
            &root,
            100,
            config,
            false,
            false,
            tag_sources,
            false,
            &[],
            None,
        )
        .await
        .unwrap();
        result.html
    }

    /// Render with `mark_incomplete = true` and the given marker list.
    async fn render_markdown_marked(content: &str, markers: &[&str]) -> String {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
        };
        let owned: Vec<String> = markers.iter().map(|s| s.to_string()).collect();
        let result = render(
            path,
            &root,
            0,
            config,
            false,
            false,
            HashSet::new(),
            true,
            &owned,
            None,
        )
        .await
        .unwrap();
        result.html
    }

    async fn render_result(content: &str) -> MarkdownRenderResult {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
        };
        render(
            path,
            &root,
            0,
            config,
            false,
            false,
            HashSet::new(),
            false,
            &[],
            None,
        )
        .await
        .unwrap()
    }

    /// Renders `content` with an explicit current-page URL and wikilink index,
    /// for exercising Obsidian-style body-wikilink resolution.
    async fn render_with_wikilinks(
        content: &str,
        current_page_url: &str,
        url_depth: Option<usize>,
        wikilink_index: Option<Arc<WikilinkIndex>>,
    ) -> MarkdownRenderResult {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth,
            current_page_url: current_page_url.to_string(),
            markdown_page_probe: None,
        };
        render(
            path,
            &root,
            0,
            config,
            true, // server_mode
            false,
            HashSet::new(),
            false,
            &[],
            wikilink_index,
        )
        .await
        .unwrap()
    }

    fn wikilink_note(url: &str, title: &str, stem: &str) -> crate::relationships::NoteRelInput {
        crate::relationships::NoteRelInput {
            url: url.to_string(),
            title: title.to_string(),
            stem: stem.to_string(),
            aliases: Vec::new(),
            is_index: false,
            relationships: Vec::new(),
        }
    }

    #[tokio::test]
    async fn wikilink_global_fallback_rewrites_href_and_records_absolute_target() {
        // `Patrick Walsh.md` lives in a *different* folder than the page that
        // references `[[Patrick Walsh]]`, so the global fallback rewrites the
        // href to the file's absolute URL and records that absolute target.
        let index = Arc::new(WikilinkIndex::new());
        index.rebuild(&[
            wikilink_note("/walsh/patrick-walsh/", "Patrick Walsh", "patrick-walsh"),
            wikilink_note("/notes/family/", "Family", "family"),
        ]);

        let result = render_with_wikilinks(
            "See [[Patrick Walsh]] here.",
            "/notes/family/",
            None, // server mode: absolute URL left as-is
            Some(index),
        )
        .await;

        assert!(
            result.html.contains(r#"href="/walsh/patrick-walsh/""#),
            "expected absolute href, got: {}",
            result.html
        );
        assert_eq!(
            result.outbound_links[0].to, "/walsh/patrick-walsh/",
            "outbound target should be the absolute URL for validation/backlinks"
        );
    }

    #[tokio::test]
    async fn wikilink_same_folder_keeps_default_transform() {
        // `patrick-walsh.md` is a same-folder sibling of the referencing page,
        // so the default relative transform is kept byte-for-byte (no absolute
        // rewrite, and the recorded target stays the raw relative name).
        let index = Arc::new(WikilinkIndex::new());
        index.rebuild(&[
            wikilink_note("/notes/patrick-walsh/", "Patrick Walsh", "patrick-walsh"),
            wikilink_note("/notes/family/", "Family", "family"),
        ]);

        let result = render_with_wikilinks(
            "See [[patrick-walsh]] here.",
            "/notes/family/",
            None,
            Some(index),
        )
        .await;

        assert!(
            !result.html.contains(r#"href="/notes/patrick-walsh/""#),
            "same-folder wikilink must not be rewritten to absolute: {}",
            result.html
        );
        assert!(
            result.html.contains(r#"href="../patrick-walsh""#),
            "expected default relative href, got: {}",
            result.html
        );
        assert_eq!(result.outbound_links[0].to, "patrick-walsh");
    }

    #[tokio::test]
    async fn invalid_yaml_frontmatter_is_captured_not_swallowed() {
        // Regression: this frontmatter uses `*` list markers with TAB
        // indentation (invalid YAML). yaml-rust2 aborts the whole document, so
        // the otherwise-valid `style: slides` field is silently discarded.
        // We must capture the parse error rather than swallow it.
        let content =
            "---\ntitle: \"Hi\"\nstyle: slides\ntags:\n\t* presentation\n\t* ai\n---\n# Heading\n";
        let result = render_result(content).await;

        assert!(
            result.frontmatter_error.is_some(),
            "expected a captured frontmatter parse error, got None"
        );
        // The valid `style` field is lost because the whole block failed to
        // parse — documents the failure mode the error report explains.
        assert!(
            !result.frontmatter.contains_key("style"),
            "expected style to be discarded when frontmatter fails to parse"
        );
    }

    #[tokio::test]
    async fn valid_yaml_frontmatter_has_no_error() {
        let content = "---\ntitle: \"Hi\"\nstyle: slides\n---\n# Heading\n";
        let result = render_result(content).await;
        assert!(result.frontmatter_error.is_none());
        assert!(result.frontmatter.contains_key("style"));
    }

    /// Frontmatter with two `to:` keys in one `relationships:` entry — the exact
    /// shape that cost a user their whole `person` frontmatter.
    const DUPLICATE_KEY_FRONTMATTER: &str = concat!(
        "---\n",
        "type: person\n",
        "aliases:\n",
        "  - Johnny Doe\n",
        "relationships:\n",
        "  - type: parent\n",
        "    to: \"[[Mary Doe]]\"\n",
        "    to: \"[[Sam Doe]]\"\n",
        "---\n",
        "# John Doe\n",
    );

    #[test]
    fn duplicate_frontmatter_key_warning_names_the_file() {
        // Regression: the repo-scan path discarded the parse error with `.ok()`
        // and logged nothing at all, so the note lost `type`, `aliases` and every
        // relationship with no way to tell which of hundreds of files was at
        // fault. The dropped `aliases` then produced a flood of "unresolved
        // relationship endpoint" warnings from *other* notes.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("john-doe.md");
        std::fs::write(&path, DUPLICATE_KEY_FRONTMATTER).unwrap();

        let (result, logs) =
            crate::test_support::capture_tracing(|| extract_metadata_from_file(&path));

        // Fallback behaviour is unchanged: still `Ok`, with empty metadata, so
        // the repo scan carries on.
        let meta = result.expect("extraction must still succeed");
        assert!(meta.relationships.is_empty());
        assert!(!meta.metadata.contains_key("type"));
        assert!(!meta.metadata.contains_key("aliases"));

        assert!(
            logs.contains("Failed to parse YAML frontmatter"),
            "expected a frontmatter warning, got: {logs}"
        );
        assert!(
            logs.contains("john-doe.md"),
            "the warning must name the file, got: {logs}"
        );
    }

    #[test]
    fn render_frontmatter_warning_names_the_file() {
        // The render path captured the error but logged it without the path, so
        // a page-load warning still could not be traced to a file. Driven on this
        // thread so `capture_tracing`'s thread-local subscriber sees it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken-note.md");
        std::fs::write(&path, DUPLICATE_KEY_FRONTMATTER).unwrap();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (result, logs) = crate::test_support::capture_tracing(|| {
            runtime.block_on(render(
                path.clone(),
                dir.path(),
                0,
                LinkTransformConfig {
                    markdown_extensions: vec!["md".to_string()],
                    index_file: "index.md".to_string(),
                    is_index_file: false,
                    url_depth: None,
                    current_page_url: "/broken-note/".to_string(),
                    markdown_page_probe: None,
                },
                true,
                false,
                HashSet::new(),
                false,
                &[],
                None,
            ))
        });

        let result = result.expect("render must still succeed");
        assert!(result.frontmatter_error.is_some());
        assert!(
            logs.contains("Failed to parse YAML frontmatter"),
            "expected a frontmatter warning, got: {logs}"
        );
        assert!(
            logs.contains("broken-note.md"),
            "the warning must name the file, got: {logs}"
        );
    }

    #[tokio::test]
    async fn ambiguous_body_wikilink_is_reported_without_changing_resolution() {
        // Two notes titled "John Doe": the `[[John Doe]]` in the body resolves to
        // the smaller URL as always, and the arbitrary choice is now reported.
        let index = Arc::new(WikilinkIndex::new());
        index.rebuild(&[
            wikilink_note("/people/john-jr/", "John Doe", "john-jr"),
            wikilink_note("/people/john-sr/", "John Doe", "john-sr"),
        ]);

        let result = render_with_wikilinks(
            "His father was [[John Doe]], and also [[John Doe]] again.",
            "/notes/family/",
            None,
            Some(index),
        )
        .await;

        // Resolution unchanged.
        assert!(
            result.html.contains(r#"href="/people/john-jr/""#),
            "expected the first-wins target, got: {}",
            result.html
        );
        // Reported once, deduped across the two occurrences.
        assert_eq!(result.ambiguous_wikilinks.len(), 1);
        let found = &result.ambiguous_wikilinks[0];
        assert_eq!(found.raw, "[[John Doe]]");
        assert_eq!(found.resolved_to, "/people/john-jr/");
        assert_eq!(found.candidates, vec!["/people/john-sr/".to_string()]);
    }

    #[tokio::test]
    async fn unambiguous_body_wikilink_reports_nothing() {
        let index = Arc::new(WikilinkIndex::new());
        index.rebuild(&[
            wikilink_note("/people/john/", "John Doe", "john"),
            wikilink_note("/notes/family/", "Family", "family"),
        ]);

        let result = render_with_wikilinks(
            "See [[John Doe]] here.",
            "/notes/family/",
            None,
            Some(index),
        )
        .await;
        assert!(result.ambiguous_wikilinks.is_empty());
    }

    #[tokio::test]
    async fn wikilink_ambiguity_is_empty_without_an_index() {
        // CLI / QuickLook renders have no repo context at all.
        let result = render_result("See [[Anyone]] here.").await;
        assert!(result.ambiguous_wikilinks.is_empty());
    }

    #[test]
    fn extract_metadata_from_file_returns_relationships() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "---\ntype: person\nborn: 1901-05-02\nrelationships:\n  - type: child\n    from: \"[[Sam Doe]]\"\n---\n# John\n"
        )
        .unwrap();
        let result = extract_metadata_from_file(file.path()).unwrap();
        assert_eq!(
            result.metadata.get("type"),
            Some(&serde_json::Value::String("person".to_string()))
        );
        assert_eq!(result.relationships.len(), 1);
        assert_eq!(result.relationships[0].rel_type, "child");
        assert_eq!(result.relationships[0].from.as_deref(), Some("[[Sam Doe]]"));
    }

    #[test]
    fn sentence_terminator_basic_cases() {
        assert_eq!(count_sentence_terminators(""), (0, false));
        assert_eq!(count_sentence_terminators("Hello."), (1, true));
        assert_eq!(count_sentence_terminators("Hi! How are you?"), (2, true));
        // Ellipsis counts once.
        assert_eq!(count_sentence_terminators("Wait..."), (1, true));
        // Mid-sentence period not followed by whitespace shouldn't count.
        assert_eq!(count_sentence_terminators("v1.2.3 is out."), (1, true));
        // Missing trailing terminator.
        assert_eq!(count_sentence_terminators("No ending here"), (0, false));
    }

    #[tokio::test]
    async fn readability_counts_simple_paragraph() {
        let md = "The cat sat on the mat. The dog ran away.";
        let result = render_result(md).await;
        assert_eq!(result.word_count, 10);
        assert_eq!(result.sentence_count, 2);
        // Nine one-syllable words plus "away" (a-way, 2 syllables).
        assert_eq!(result.syllable_count, 11);
    }

    #[tokio::test]
    async fn readability_heading_without_terminator_bumps_sentence() {
        let md = "# Introduction\n\nHello world.";
        let result = render_result(md).await;
        assert_eq!(result.word_count, 3);
        // Heading ("Introduction") + "Hello world." = 2 sentences.
        assert_eq!(result.sentence_count, 2);
    }

    #[tokio::test]
    async fn readability_excludes_code_blocks() {
        let md = "Some prose here.\n\n```rust\nfn main() { println!(\"hi\"); }\n```\n";
        let result = render_result(md).await;
        assert_eq!(result.word_count, 3);
        assert_eq!(result.sentence_count, 1);
    }

    #[tokio::test]
    async fn readability_empty_document_has_zero_counts() {
        let result = render_result("").await;
        assert_eq!(result.word_count, 0);
        assert_eq!(result.sentence_count, 0);
        assert_eq!(result.syllable_count, 0);
    }

    // ---- task list rendering -------------------------------------------------

    /// The `<li>` body a task renders to, checkbox included, for exact-match
    /// assertions that stay readable.
    fn task_body(html: &str) -> String {
        let start = html.find("<li>").expect("a list item") + "<li>".len();
        let end = html.find("</li>").expect("a closed list item");
        html[start..end].to_string()
    }

    #[tokio::test]
    async fn canceled_marker_renders_a_checkbox_and_a_status_class() {
        // Replaces the old bare `<s>` hack: the class is what the theme styles,
        // and it sits on the text so a canceled parent does not strike out its
        // own subtasks.
        for md in ["- [-] canceled task", "* [-] canceled task"] {
            assert_eq!(
                task_body(&render_markdown(md).await),
                concat!(
                    r#"<input type="checkbox" class="mbr-task-check" id="mbr-task-1" "#,
                    r#"data-mbr-task-line="1" data-mbr-task-status="canceled" disabled>"#,
                    r#"<span class="mbr-task-text mbr-task-canceled">canceled task</span>"#
                ),
                "for {md:?}"
            );
        }
    }

    #[tokio::test]
    async fn moved_marker_is_canceled_and_shows_its_destination_date() {
        let html = render_markdown("- [>] moved along > 2026-08-04").await;
        assert_eq!(
            task_body(&html),
            concat!(
                r#"<input type="checkbox" class="mbr-task-check" id="mbr-task-1" "#,
                r#"data-mbr-task-line="1" data-mbr-task-status="canceled" disabled>"#,
                r#"<span class="mbr-task-text mbr-task-canceled">moved along</span>"#,
                r#" <time class="mbr-task-moved" datetime="2026-08-04">Aug 4</time>"#
            )
        );
    }

    #[tokio::test]
    async fn unchecked_and_checked_markers_carry_their_status() {
        let open = render_markdown("- [ ] unchecked item").await;
        assert!(
            open.contains(r#"data-mbr-task-status="open" disabled>"#),
            "{open}"
        );
        assert!(!open.contains(" checked"), "{open}");

        let done = render_markdown("- [x] checked item").await;
        assert!(
            done.contains(r#"data-mbr-task-status="done" checked disabled>"#),
            "{done}"
        );
    }

    #[tokio::test]
    async fn task_text_is_html_escaped() {
        let html = render_markdown("- [-] special chars: & < > \"").await;
        assert!(html.contains("special chars: &amp; &lt; &gt;"), "{html}");
    }

    #[tokio::test]
    async fn checkboxes_are_inert_in_every_mode() {
        // Interactivity is turned on by the frontend, not by the renderer, so
        // a static build and a served page emit identical markup.
        for server_mode in [false, true] {
            let html = render_markdown_with_mode("- [ ] a task", server_mode).await;
            assert!(html.contains(" disabled>"), "server_mode={server_mode}");
        }
    }

    #[tokio::test]
    async fn annotations_render_as_chips_instead_of_literal_text() {
        let html = render_markdown(
            "- [ ] write the report !!! #work @due(2026-08-05) @done(2026-08-04 12:11 PM)",
        )
        .await;
        assert_eq!(
            task_body(&html),
            concat!(
                r#"<input type="checkbox" class="mbr-task-check" id="mbr-task-1" "#,
                r#"data-mbr-task-line="1" data-mbr-task-status="open" disabled>"#,
                r#"<span class="mbr-task-text">write the report</span>"#,
                r#" <span class="mbr-task-pri mbr-task-pri-urgent" role="img" "#,
                r#"aria-label="Urgent priority" title="Urgent priority"></span>"#,
                r#" <span class="mbr-task-tag">#work</span>"#,
                r#" <time class="mbr-task-due" datetime="2026-08-05">Aug 5</time>"#,
                r#" <time class="mbr-task-completed" datetime="2026-08-04T12:11">Aug 4, 12:11 PM</time>"#
            )
        );
    }

    #[tokio::test]
    async fn a_task_with_no_annotations_emits_no_chips() {
        let html = render_markdown("- [ ] plain task").await;
        assert_eq!(
            task_body(&html),
            concat!(
                r#"<input type="checkbox" class="mbr-task-check" id="mbr-task-1" "#,
                r#"data-mbr-task-line="1" data-mbr-task-status="open" disabled>"#,
                r#"<span class="mbr-task-text">plain task</span>"#
            )
        );
    }

    /// The hard case: annotations arrive as text runs interleaved with inline
    /// formatting events, so they cannot be stripped from one flat string.
    #[tokio::test]
    async fn inline_formatting_survives_annotation_stripping() {
        let html = render_markdown("- [ ] fix **this** and *that* #bug").await;
        assert!(
            html.contains(
                r#"<span class="mbr-task-text">fix <strong>this</strong> and <em>that</em></span>"#
            ),
            "inline formatting and its spacing must survive: {html}"
        );
        assert!(
            html.contains(r#"<span class="mbr-task-tag">#bug</span>"#),
            "{html}"
        );
    }

    #[tokio::test]
    async fn links_inside_a_task_keep_working() {
        let html = render_markdown("- [ ] read [the guide](guide.md) !! @due(2026-08-05)").await;
        assert!(
            html.contains(r#"<a href="../guide/">the guide</a>"#),
            "{html}"
        );
        assert!(html.contains("mbr-task-pri-high"), "{html}");
        assert!(!html.contains("@due("), "{html}");
    }

    /// A trailing `< YYYY-MM-DD` says where a task came from. Nothing surfaces
    /// it, so it is stripped and dropped.
    #[tokio::test]
    async fn moved_from_marker_is_stripped_without_a_chip() {
        let html = render_markdown("- [ ] carried over < 2026-08-01").await;
        assert!(html.contains(">carried over</span>"), "{html}");
        assert!(!html.contains("2026-08-01"), "{html}");
    }

    #[tokio::test]
    async fn nested_subtasks_each_get_their_own_line_number() {
        let html = render_markdown("- [ ] parent\n\t- [ ] child one\n\t- [x] child two").await;
        for line in 1..=3 {
            assert!(
                html.contains(&format!(r#"data-mbr-task-line="{line}""#)),
                "missing line {line}: {html}"
            );
        }
        // The parent's text span must close before its subtask list, or the
        // chips would render underneath the children.
        assert!(
            html.contains("<span class=\"mbr-task-text\">parent</span>\n<ul>"),
            "the parent's text span must close before its subtask list: {html}"
        );
    }

    #[tokio::test]
    async fn a_marker_inside_a_fenced_code_block_is_left_alone() {
        let html = render_result("```\n- [-] not a checkbox\n- [ ] nor this\n```\n")
            .await
            .html;
        assert!(!html.contains("mbr-task-check"), "{html}");
        assert!(html.contains("- [-] not a checkbox"), "{html}");
        assert!(html.contains("- [ ] nor this"), "{html}");
    }

    #[tokio::test]
    async fn a_bracket_marker_outside_a_list_item_is_not_a_task() {
        // The old renderer turned any text starting with `[-] ` into a
        // checkbox, including ordinary prose.
        let html = render_result("[-] this is just a sentence\n").await.html;
        assert!(!html.contains("mbr-task-check"), "{html}");
        assert!(html.contains("[-] this is just a sentence"), "{html}");
    }

    // ---- task line numbers ---------------------------------------------------

    /// The 1-based lines advertised by the rendered checkboxes, in order.
    fn rendered_task_lines(html: &str) -> Vec<u32> {
        const ATTR: &str = "data-mbr-task-line=\"";
        html.match_indices(ATTR)
            .map(|(at, _)| {
                let rest = &html[at + ATTR.len()..];
                let end = rest.find('"').expect("unterminated attribute");
                rest[..end].parse().expect("numeric line")
            })
            .collect()
    }

    /// Line numbers must survive whatever sits between the tasks — headings,
    /// fences, blank lines, frontmatter — because they are derived from byte
    /// offsets rather than from counting events.
    #[tokio::test]
    async fn task_line_numbers_survive_intervening_blocks() {
        let md = concat!(
            "---\n",           // 1
            "title: T\n",      // 2
            "---\n",           // 3
            "\n",              // 4
            "# Heading\n",     // 5
            "\n",              // 6
            "- [ ] first\n",   // 7
            "\n",              // 8
            "```js\n",         // 9
            "// - [ ] fake\n", // 10
            "```\n",           // 11
            "\n",              // 12
            "Some prose.\n",   // 13
            "\n",              // 14
            "- [x] second\n",  // 15
            "- [-] third\n",   // 16
        );
        let html = render_result(md).await.html;
        assert_eq!(rendered_task_lines(&html), vec![7, 15, 16]);
        assert_eq!(
            rendered_task_lines(&html),
            crate::tasks::scan_source_tasks(md)
                .into_iter()
                .map(|task| task.line)
                .collect::<Vec<_>>(),
            "the renderer and the task index must agree about line numbers"
        );
    }

    #[tokio::test]
    async fn crlf_line_endings_do_not_shift_task_line_numbers() {
        let md = "- [ ] first\r\n\r\nprose\r\n\r\n- [x] second\r\n";
        let html = render_result(md).await.html;
        assert_eq!(rendered_task_lines(&html), vec![1, 5]);
    }

    /// The incomplete-block pass runs after the task rewrite, so it sees the
    /// annotation-stripped display text and must still wrap the item.
    #[tokio::test]
    async fn incomplete_markers_still_fire_inside_a_task() {
        let html = render_markdown_marked("- [ ] TODO: write it up #docs", &["TODO"]).await;
        assert!(html.contains(INCOMPLETE_SPAN_OPEN), "{html}");
        assert!(html.contains("TODO: write it up"), "{html}");
        assert!(
            html.contains(r#"<span class="mbr-task-tag">#docs</span>"#),
            "{html}"
        );
        // One open, one close: the two passes must not interleave their spans
        // into overlapping tags.
        assert_eq!(html.matches(INCOMPLETE_SPAN_OPEN).count(), 1, "{html}");
    }

    /// The parser is handed the *wikilink-transformed* source, not the file on
    /// disk, so `transform_wikilinks` sits between the bytes on disk and the
    /// offsets the line numbers come from. For an ordinary single-line wikilink
    /// it rewrites within the line and the numbers are unaffected.
    #[tokio::test]
    async fn wikilinks_on_a_task_line_do_not_shift_its_line_number() {
        let sources: HashSet<String> = ["Tags".to_string()].into_iter().collect();
        let md = concat!(
            "- [ ] read about [[Tags:rust]] #study\n",
            "- [ ] and [[Tags:async]] too\n",
            "\n",
            "- [x] last one\n",
        );
        let html = render_markdown_with_tags(md, sources).await;
        assert_eq!(rendered_task_lines(&html), vec![1, 2, 4]);
        assert!(html.contains("href=\"/tags/rust/\""), "{html}");
        assert!(
            html.contains(r#"<span class="mbr-task-tag">#study</span>"#),
            "{html}"
        );
    }

    /// A `[[Source:value]]` whose brackets straddle a line break is not a
    /// wikilink, so the substitution cannot swallow the newline and the lines
    /// below keep their numbers.
    ///
    /// This used to be a pinned known limitation: the transformed source came
    /// out a line shorter than the file on disk, every later task advertised a
    /// line number one too small, and a line patch aimed at one of those numbers
    /// would have edited the wrong line.
    #[tokio::test]
    async fn a_multi_line_tag_wikilink_does_not_shift_later_line_numbers() {
        let sources: HashSet<String> = ["Tags".to_string()].into_iter().collect();
        let md = "- [ ] see [[Tags:\nrust]] here\n- [x] second\n";

        let expected: Vec<u32> = crate::tasks::scan_source_tasks(md)
            .into_iter()
            .map(|task| task.line)
            .collect();
        assert_eq!(expected, vec![1, 3]);

        let html = render_markdown_with_tags(md, sources).await;
        assert_eq!(
            rendered_task_lines(&html),
            expected,
            "the renderer and the task index must agree about line numbers"
        );
        // ...and it was not rewritten into a tag link on the way through.
        assert!(!html.contains("/tags/rust/"), "{html}");
    }

    #[test]
    fn line_index_maps_offsets_to_one_based_lines() {
        //             0123 456 78
        let index = LineIndex::build("ab\nc\n\nd");
        assert_eq!(index.line_of(0), 1);
        assert_eq!(index.line_of(2), 1); // the newline itself ends line 1
        assert_eq!(index.line_of(3), 2);
        assert_eq!(index.line_of(5), 3); // the empty line
        assert_eq!(index.line_of(6), 4);
        // Past the end is still the last line rather than a panic.
        assert_eq!(index.line_of(999), 4);

        assert_eq!(LineIndex::build("").line_of(0), 1);
    }

    #[test]
    fn split_extended_marker_requires_whitespace_after_the_box() {
        assert_eq!(
            split_extended_marker("[-] canceled"),
            Some((TaskStatus::Canceled, "canceled"))
        );
        assert_eq!(
            split_extended_marker("[>]\tmoved"),
            Some((TaskStatus::Canceled, "moved"))
        );
        assert_eq!(
            split_extended_marker("[-]"),
            Some((TaskStatus::Canceled, ""))
        );
        for text in ["[-]x", "[x] done", "[ ] open", "[?] what", "prose"] {
            assert_eq!(split_extended_marker(text), None, "for {text:?}");
        }
    }

    #[tokio::test]
    async fn test_yaml_frontmatter() {
        let md = "---\ntitle: Test Title\n---\n\n# Heading";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(md.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
        };
        let result = render(
            path,
            &root,
            100,
            config,
            false,
            false,
            HashSet::new(),
            false,
            &[],
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            result.frontmatter.get("title"),
            Some(&serde_json::Value::String("Test Title".to_string()))
        );
    }

    // H1 extraction tests
    #[test]
    fn test_extract_first_h1_basic() {
        let md = "# Hello World\n\nSome content";
        let result = extract_first_h1(md);
        assert_eq!(result, Some("Hello World".to_string()));
    }

    #[test]
    fn test_extract_first_h1_with_inline_formatting() {
        let md = "# Hello **World**\n\nSome content";
        let result = extract_first_h1(md);
        assert_eq!(result, Some("Hello World".to_string()));
    }

    #[test]
    fn test_extract_first_h1_none_when_no_h1() {
        let md = "## This is H2\n\nSome content";
        let result = extract_first_h1(md);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_first_h1_returns_first_only() {
        let md = "# First H1\n\n# Second H1";
        let result = extract_first_h1(md);
        assert_eq!(result, Some("First H1".to_string()));
    }

    #[test]
    fn test_extract_first_h1_empty_doc() {
        let md = "";
        let result = extract_first_h1(md);
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_has_h1_true_when_first_heading_is_h1() {
        let md = "# Main Title\n\n## Subsection";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(md.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
        };
        let result = render(
            path,
            &root,
            100,
            config,
            false,
            false,
            HashSet::new(),
            false,
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(result.has_h1);
    }

    #[tokio::test]
    async fn test_has_h1_false_when_first_heading_is_h2() {
        let md = "## Subsection\n\n# Late H1";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(md.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
        };
        let result = render(
            path,
            &root,
            100,
            config,
            false,
            false,
            HashSet::new(),
            false,
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(!result.has_h1);
    }

    #[tokio::test]
    async fn test_title_fallback_from_h1() {
        // No frontmatter title, but has H1 - should extract title from H1
        let md = "# My Document Title\n\nSome content here.";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(md.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
        };
        let result = render(
            path,
            &root,
            100,
            config,
            false,
            false,
            HashSet::new(),
            false,
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(result.has_h1);
        assert_eq!(
            result.frontmatter.get("title"),
            Some(&serde_json::Value::String("My Document Title".to_string()))
        );
    }

    #[tokio::test]
    async fn test_frontmatter_title_takes_precedence() {
        // Frontmatter title should take precedence over H1
        let md = "---\ntitle: Frontmatter Title\n---\n\n# H1 Title";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(md.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
        };
        let result = render(
            path,
            &root,
            100,
            config,
            false,
            false,
            HashSet::new(),
            false,
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(result.has_h1);
        assert_eq!(
            result.frontmatter.get("title"),
            Some(&serde_json::Value::String("Frontmatter Title".to_string()))
        );
    }

    #[tokio::test]
    async fn test_no_title_when_no_frontmatter_and_no_h1() {
        // No frontmatter and no H1 - should have no title
        let md = "## Subsection\n\nSome content.";
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(md.as_bytes()).unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
        };
        let result = render(
            path,
            &root,
            100,
            config,
            false,
            false,
            HashSet::new(),
            false,
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(!result.has_h1);
        assert_eq!(result.frontmatter.get("title"), None);
    }

    // Media embed tests
    #[tokio::test]
    async fn test_video_embed_from_image_syntax() {
        let md = "![My Video](video.mp4)";
        let html = render_markdown(md).await;
        assert!(html.contains("<video"));
        assert!(html.contains("video.mp4"));
        assert!(html.contains("<figcaption>"));
        assert!(html.contains("My Video"));
        assert!(html.contains("</figcaption></figure>"));
    }

    #[tokio::test]
    async fn test_vid_shortcode_apostrophe_path_not_curly_encoded() {
        // Smart punctuation curls the apostrophe; the path must be normalized back
        // to ASCII so the percent-encoded URL matches the real filename on disk.
        let html = render_markdown("{{ vid(path=\"World's Best.mp4\") }}").await;
        assert!(html.contains("World%27s%20Best.mp4"), "got: {html}");
        assert!(
            !html.contains("%E2%80%99"),
            "curly apostrophe leaked into URL: {html}"
        );
    }

    #[tokio::test]
    async fn test_audio_embed_from_image_syntax() {
        let md = "![Episode 1](podcast.mp3)";
        let html = render_markdown(md).await;
        assert!(html.contains("<audio"));
        assert!(html.contains("audio-embed"));
        assert!(html.contains("podcast.mp3"));
        assert!(html.contains("<figcaption>"));
        assert!(html.contains("Episode 1"));
        assert!(html.contains("</figcaption></figure>"));
    }

    #[tokio::test]
    async fn test_youtube_embed_from_image_syntax() {
        let md = "![Watch this](https://www.youtube-nocookie.com/watch?v=dQw4w9WgXcQ)";
        let html = render_markdown(md).await;
        assert!(html.contains("youtube-embed"));
        assert!(html.contains("youtube-nocookie.com/embed/dQw4w9WgXcQ"));
        assert!(html.contains("<figcaption>"));
        assert!(html.contains("Watch this"));
        assert!(html.contains("</figcaption></figure>"));
    }

    #[tokio::test]
    async fn test_youtube_short_url_embed() {
        let md = "![](https://youtu.be/dQw4w9WgXcQ)";
        let html = render_markdown(md).await;
        assert!(html.contains("youtube-embed"));
        assert!(html.contains("youtube-nocookie.com/embed/dQw4w9WgXcQ"));
    }

    #[tokio::test]
    async fn test_pdf_embed_from_image_syntax() {
        let md = "![Important Document](report.pdf)";
        let html = render_markdown(md).await;
        assert!(html.contains("pdf-embed"));
        // URL is transformed for trailing-slash convention (../report.pdf for non-index files)
        assert!(
            html.contains(r#"data="../report.pdf""#),
            "PDF URL should be transformed. Got: {}",
            html
        );
        assert!(html.contains(r#"type="application/pdf""#));
        assert!(html.contains("data-pdf-fallback"));
        assert!(html.contains("<figcaption>"));
        assert!(html.contains("Important Document"));
        assert!(html.contains("</figcaption></figure>"));
    }

    #[tokio::test]
    async fn test_pdf_embed_with_path() {
        let md = "![](docs/manual.pdf)";
        let html = render_markdown(md).await;
        assert!(html.contains("pdf-embed"));
        // URL is transformed for trailing-slash convention (../docs/manual.pdf for non-index files)
        assert!(
            html.contains(r#"data="../docs/manual.pdf""#),
            "PDF URL should be transformed. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_regular_image_not_converted() {
        let md = "![Alt text](photo.jpg)";
        let html = render_markdown(md).await;
        assert!(html.contains("<img"));
        assert!(html.contains("photo.jpg"));
        assert!(!html.contains("<video"));
        assert!(!html.contains("<audio"));
        assert!(!html.contains("pdf-embed"));
    }

    #[tokio::test]
    async fn test_multiple_media_types_in_document() {
        let md = r#"
# My Media

![Video](clip.mp4)

![Audio](song.mp3)

![PDF](doc.pdf)

![Image](photo.png)
"#;
        let html = render_markdown(md).await;
        assert!(html.contains("<video"));
        assert!(html.contains("<audio"));
        assert!(html.contains("pdf-embed"));
        assert!(html.contains("<img"));
    }

    #[tokio::test]
    async fn test_vid_shortcode() {
        let md = r#"{{ vid(path="test/video.mp4") }}"#;
        let html = render_markdown(md).await;
        println!("Output HTML: {}", html);
        assert!(html.contains("<video"), "Should contain video element");
        assert!(
            html.contains("/videos/test/video.mp4"),
            "Should contain video path"
        );
    }

    #[tokio::test]
    async fn test_vid_shortcode_with_spaces() {
        let md = r#"{{ vid(path="Eric Jones/Eric Jones - Metal 3.mp4")}}"#;
        let html = render_markdown(md).await;
        println!("Output HTML: {}", html);
        assert!(html.contains("<video"), "Should contain video element");
        assert!(
            html.contains("/videos/Eric%20Jones"),
            "Should contain URL-encoded path"
        );
    }

    // Link transformation tests
    #[tokio::test]
    async fn test_link_transformation_regular_markdown() {
        // Regular markdown file (not index) - links get ../ prefix
        let md = "[Other Doc](other.md)";
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
        assert!(
            html.contains(r#"href="../other/""#),
            "Regular markdown should transform other.md to ../other/. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_link_transformation_index_file() {
        // Index file - links don't get ../ prefix
        let md = "[Other Doc](other.md)";
        let html = render_markdown_with_config(md, true, HashSet::new()).await;
        assert!(
            html.contains(r#"href="other/""#),
            "Index file should transform other.md to other/. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_link_transformation_preserves_absolute_urls() {
        let md = "[External](https://example.com)";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"href="https://example.com""#),
            "Absolute URLs should remain unchanged"
        );
    }

    #[tokio::test]
    async fn test_link_transformation_with_anchor() {
        let md = "[Section](other.md#section)";
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
        assert!(
            html.contains(r#"href="../other/#section""#),
            "Links with anchors should transform correctly. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_image_transformation_regular_markdown() {
        // Regular images (not media embeds) should also be transformed
        let md = "![Alt](images/photo.jpg)";
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
        assert!(
            html.contains(r#"src="../images/photo.jpg""#),
            "Image URLs should be transformed. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_image_transformation_index_file() {
        let md = "![Alt](images/photo.jpg)";
        let html = render_markdown_with_config(md, true, HashSet::new()).await;
        assert!(
            html.contains(r#"src="images/photo.jpg""#),
            "Index file image URLs shouldn't get ../. Got: {}",
            html
        );
    }

    // Media embed URL transformation tests
    #[tokio::test]
    async fn test_video_embed_url_transformation() {
        // Video embeds in regular markdown files should get ../ prefix
        let md = "![My Video](video.mp4)";
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
        assert!(
            html.contains("../video.mp4"),
            "Video URLs should be transformed with ../. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_video_embed_url_transformation_index_file() {
        // Video embeds in index files should NOT get ../ prefix
        let md = "![My Video](video.mp4)";
        let html = render_markdown_with_config(md, true, HashSet::new()).await;
        assert!(
            !html.contains("../video.mp4"),
            "Index file video URLs shouldn't get ../. Got: {}",
            html
        );
        assert!(
            html.contains("video.mp4"),
            "Video URL should be present. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_audio_embed_url_transformation() {
        // Audio embeds in regular markdown files should get ../ prefix
        let md = "![Podcast](episode.mp3)";
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
        assert!(
            html.contains("../episode.mp3"),
            "Audio URLs should be transformed with ../. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_pdf_embed_url_transformation() {
        // PDF embeds in regular markdown files should get ../ prefix
        let md = "![Document](report.pdf)";
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
        assert!(
            html.contains("../report.pdf"),
            "PDF URLs should be transformed with ../. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_pdf_embed_url_transformation_index_file() {
        // PDF embeds in index files should NOT get ../ prefix
        let md = "![Document](report.pdf)";
        let html = render_markdown_with_config(md, true, HashSet::new()).await;
        assert!(
            !html.contains("../report.pdf"),
            "Index file PDF URLs shouldn't get ../. Got: {}",
            html
        );
        assert!(
            html.contains("report.pdf"),
            "PDF URL should be present. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_media_embed_peer_file_transformation() {
        // Test the specific bug case: peer file in same folder as markdown
        // Markdown: docs/guide.md references peer-video.mp4 (docs/peer-video.mp4)
        // When served at /docs/guide/, browser sees ../peer-video.mp4 → /docs/peer-video.mp4 (correct!)
        let md = "![](peer-video.mp4)";
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
        assert!(
            html.contains("../peer-video.mp4"),
            "Peer file video should get ../ prefix. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_media_embed_explicit_relative_path() {
        // Test ./file.mp4 syntax also gets transformed correctly
        let md = "![](./peer-video.mp4)";
        let html = render_markdown_with_config(md, false, HashSet::new()).await;
        assert!(
            html.contains("../peer-video.mp4"),
            "./peer-video.mp4 should transform to ../peer-video.mp4. Got: {}",
            html
        );
    }

    // Section attributes tests
    #[tokio::test]
    async fn test_section_attrs_with_id() {
        // Test that --- {#id} applies ID to the next section
        let md = "First section\n\n--- {#intro}\n\nSecond section";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"<section id="intro">"#),
            "Section should have id='intro'. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_with_class() {
        // Test that --- {.highlight} applies class to the next section
        let md = "First section\n\n--- {.highlight}\n\nSecond section";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"<section class="highlight">"#),
            "Section should have class='highlight'. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_with_multiple_classes() {
        // Test multiple classes
        let md = "First section\n\n--- {.slide .center}\n\nSecond section";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"<section class="slide center">"#),
            "Section should have class='slide center'. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_with_data_attributes() {
        // Test data attributes
        let md = "First section\n\n--- {data-transition=\"slide\"}\n\nSecond section";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"data-transition="slide""#),
            "Section should have data-transition='slide'. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_mixed() {
        // Test ID, class, and data attribute together
        let md = "First section\n\n--- {#main .highlight data-bg=\"blue\"}\n\nSecond section";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"id="main""#),
            "Section should have id='main'. Got: {}",
            html
        );
        assert!(
            html.contains(r#"class="highlight""#),
            "Section should have class='highlight'. Got: {}",
            html
        );
        assert!(
            html.contains(r#"data-bg="blue""#),
            "Section should have data-bg='blue'. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_multiple_rules() {
        // Test multiple rules with attrs
        let md = "Section 0\n\n--- {#one}\n\nSection 1\n\n--- {#two}\n\nSection 2";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"<section id="one">"#),
            "First rule section should have id='one'. Got: {}",
            html
        );
        assert!(
            html.contains(r#"<section id="two">"#),
            "Second rule section should have id='two'. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_plain_rule_still_works() {
        // Test that plain --- without attrs still creates a section
        let md = "First section\n\n---\n\nSecond section";
        let html = render_markdown(md).await;
        // Should have at least 2 sections (one before and one after the rule)
        let section_count = html.matches("<section>").count();
        assert!(
            section_count >= 1,
            "Plain rule should create sections. Got: {}",
            html
        );
        assert!(
            html.contains("<hr />"),
            "Should contain <hr /> divider. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_em_dash_with_non_attrs_text() {
        // Test that --- followed by text that isn't attrs is rendered normally
        // This becomes paragraph with em dash + text (not transformed to Rule)
        let md = "Some text\n\n--- not attrs\n\nMore text";
        let html = render_markdown(md).await;
        // Should NOT have a <hr /> since it's not a valid rule pattern
        // The em dash paragraph should be preserved as text
        assert!(
            html.contains("—"),
            "Em dash should be preserved. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_empty_attrs() {
        // Test that --- {} creates a section without any attributes
        let md = "First section\n\n--- {}\n\nSecond section";
        let html = render_markdown(md).await;
        // Should have a plain section (no id, class, or attrs)
        // The section should close and reopen with just <section>
        assert!(
            html.contains("<section>"),
            "Empty attrs should create plain section. Got: {}",
            html
        );
        assert!(
            html.contains("<hr />"),
            "Should contain <hr /> divider. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_with_whitespace() {
        // Test that whitespace inside braces is handled
        let md = "First section\n\n--- {  #intro  .highlight  }\n\nSecond section";
        let html = render_markdown(md).await;
        assert!(
            html.contains(r#"id="intro""#),
            "Whitespace should not affect ID parsing. Got: {}",
            html
        );
        assert!(
            html.contains(r#"class="highlight""#),
            "Whitespace should not affect class parsing. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_curly_quotes() {
        // Test curly quotes from smart punctuation (pulldown-cmark converts " to "")
        // Build the attrs string with explicit curly quotes
        let md = "First section\n\n--- {data-x=\u{201C}value\u{201D}}\n\nSecond section";
        let html = render_markdown(md).await;
        // The curly quotes should be normalized to straight quotes in output
        assert!(
            html.contains(r#"data-x="value""#),
            "Curly quotes should be normalized. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_section_attrs_html_escaping() {
        // Test that attribute values with HTML special chars are escaped
        // Note: Can't use <script> directly as pulldown-cmark interprets it as HTML
        // Use & and ' which need escaping but don't break markdown parsing
        let md = "First section\n\n--- {data-val=\"a & b\"}\n\nSecond section";
        let html = render_markdown(md).await;
        // The & should be escaped to &amp;
        assert!(
            html.contains("&amp;"),
            "HTML special chars should be escaped. Got: {}",
            html
        );
        assert!(
            html.contains(r#"data-val="a &amp; b""#),
            "Value should have escaped &. Got: {}",
            html
        );
    }

    // ==================== Incomplete-block marker tests ====================

    const DEFAULT_MARKERS: &[&str] = &["TK", "TODO", "FIXME", "XXX"];

    #[test]
    fn test_build_incomplete_marker_regex_defaults() {
        let markers = default_incomplete_markers_for_test();
        let re = build_incomplete_marker_regex(&markers).expect("regex");
        assert!(re.is_match("TK"));
        assert!(re.is_match("TK rewrite this"));
        assert!(re.is_match("TODO foo"));
        assert!(re.is_match("FIXME(name)"));
        assert!(re.is_match("XXX:"));
        // Word boundary blocks TKTK / TODOs / Tk / lowercase.
        assert!(!re.is_match("TKTK"));
        assert!(!re.is_match("TODOs"));
        assert!(!re.is_match("Tk"));
        assert!(!re.is_match("todo"));
        assert!(!re.is_match("Tomato"));
    }

    #[test]
    fn test_build_incomplete_marker_regex_empty() {
        // Empty list → no regex (caller short-circuits).
        let markers: Vec<String> = Vec::new();
        assert!(build_incomplete_marker_regex(&markers).is_none());
        // Empty strings filtered out too.
        let markers = vec!["".to_string()];
        assert!(build_incomplete_marker_regex(&markers).is_none());
    }

    #[test]
    fn test_build_incomplete_marker_regex_escapes_metachars() {
        // Markers containing regex metacharacters must not crash regex
        // compilation. Without `regex::escape`, an unbalanced `(` would
        // produce an invalid pattern and `Regex::new` would return None.
        let markers = vec!["FOO(".to_string(), "BAR".to_string()];
        let re = build_incomplete_marker_regex(&markers).expect("regex compiles");
        // BAR still matches normally (word-boundary check applies).
        assert!(re.is_match("BAR foo"));
        // And the metachar marker doesn't break sibling markers.
        assert!(!re.is_match("Tomato"));
    }

    fn default_incomplete_markers_for_test() -> Vec<String> {
        DEFAULT_MARKERS.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn test_incomplete_paragraph() {
        let html = render_markdown_marked("TK rewrite this paragraph.", DEFAULT_MARKERS).await;
        assert!(
            html.contains(r#"<p><span class="mbr-incomplete">"#),
            "Paragraph should have span as first child. Got: {html}"
        );
        assert!(html.contains("TK rewrite"), "TK text preserved: {html}");
    }

    #[tokio::test]
    async fn test_incomplete_heading() {
        let html = render_markdown_marked("## TODO finish this", DEFAULT_MARKERS).await;
        assert!(
            html.contains(r#"<span class="mbr-incomplete">"#),
            "Span should be present in heading. Got: {html}"
        );
        // Span must be inside the <h2>, not wrapping it.
        assert!(html.contains("<h2"), "h2 element present: {html}");
        let h2_start = html.find("<h2").unwrap();
        let span_start = html.find(r#"<span class="mbr-incomplete">"#).unwrap();
        assert!(span_start > h2_start, "span should be inside h2: {html}");
    }

    #[tokio::test]
    async fn test_incomplete_tight_list_item() {
        let html = render_markdown_marked("- TK item one\n- normal item", DEFAULT_MARKERS).await;
        assert!(
            html.contains(r#"<li><span class="mbr-incomplete">"#),
            "Span should follow <li> for tight list: {html}"
        );
        // Only the TK item is wrapped.
        assert_eq!(
            html.matches(r#"<span class="mbr-incomplete">"#).count(),
            1,
            "Only one span expected: {html}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_loose_list_item() {
        // Blank line between items forces loose list — items wrap their content
        // in <p>. The inner <p> is the innermost block, so the span goes there.
        let md = "- TK draft this\n\n- finished item\n";
        let html = render_markdown_marked(md, DEFAULT_MARKERS).await;
        assert!(
            html.contains(r#"<p><span class="mbr-incomplete">"#),
            "Span should wrap inner <p> in loose list: {html}"
        );
        // The <li> itself should not have the span as a direct child.
        assert!(
            !html.contains(r#"<li><span class="mbr-incomplete">"#),
            "Loose-list <li> should not have direct span child: {html}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_table_cell() {
        let md = "| H |\n|---|\n| TK cell |\n";
        let html = render_markdown_marked(md, DEFAULT_MARKERS).await;
        assert!(
            html.contains(r#"<td><span class="mbr-incomplete">"#),
            "Span should follow <td> for incomplete cell: {html}"
        );
    }

    #[tokio::test]
    async fn test_remark_hint_tip() {
        let html = render_markdown("!> tip").await;
        assert!(
            html.contains(r#"<blockquote class="markdown-alert-tip">"#),
            "Expected tip alert blockquote: {html}"
        );
        assert!(
            html.contains("<p>tip</p>"),
            "Marker should be stripped: {html}"
        );
        assert!(!html.contains("!&gt;"), "Escaped marker leaked: {html}");
        assert!(!html.contains("!>"), "Raw marker leaked: {html}");
    }

    #[tokio::test]
    async fn test_remark_hint_warning() {
        let html = render_markdown("?> warn").await;
        assert!(
            html.contains(r#"<blockquote class="markdown-alert-warning">"#),
            "Expected warning alert blockquote: {html}"
        );
        assert!(
            html.contains("<p>warn</p>"),
            "Marker should be stripped: {html}"
        );
    }

    #[tokio::test]
    async fn test_remark_hint_caution() {
        let html = render_markdown("x> caution").await;
        assert!(
            html.contains(r#"<blockquote class="markdown-alert-caution">"#),
            "Expected caution alert blockquote: {html}"
        );
        assert!(
            html.contains("<p>caution</p>"),
            "Marker should be stripped: {html}"
        );
    }

    #[tokio::test]
    async fn test_remark_hint_multiline() {
        // A soft-wrapped paragraph: marker stripped from the first line, the rest
        // of the paragraph stays inside the same alert.
        let html = render_markdown("!> line one\nline two").await;
        assert!(
            html.contains(r#"<blockquote class="markdown-alert-tip">"#),
            "Expected tip alert blockquote: {html}"
        );
        assert!(html.contains("line one"), "First line retained: {html}");
        assert!(html.contains("line two"), "Second line retained: {html}");
        assert!(!html.contains("!&gt;"), "Escaped marker leaked: {html}");
        assert!(!html.contains("!>"), "Raw marker leaked: {html}");
    }

    #[tokio::test]
    async fn test_remark_hint_requires_trailing_space() {
        // No space after the marker -> not a hint.
        let html = render_markdown("!>no-space").await;
        assert!(
            !html.contains("markdown-alert"),
            "Should not be converted without trailing space: {html}"
        );
    }

    #[tokio::test]
    async fn test_remark_hint_only_at_paragraph_start() {
        // Mid-paragraph occurrence is not a hint.
        let html = render_markdown("text !> more").await;
        assert!(
            !html.contains("markdown-alert"),
            "Mid-paragraph marker should not be converted: {html}"
        );
    }

    #[tokio::test]
    async fn test_remark_hint_ignored_in_code_block() {
        // A fenced code block containing a hint-like line must render verbatim.
        let html = render_markdown("```\n!> foo\n```").await;
        assert!(
            !html.contains("markdown-alert"),
            "Code block content should not be converted: {html}"
        );
        assert!(
            html.contains("!&gt; foo") || html.contains("!> foo"),
            "Code content should render verbatim: {html}"
        );
    }

    #[tokio::test]
    async fn test_native_github_alert_still_works() {
        // Regression: native pulldown-cmark GitHub alerts continue to render.
        let html = render_markdown("> [!TIP]\n> hello").await;
        assert!(
            html.contains(r#"<blockquote class="markdown-alert-tip">"#),
            "Native GitHub alert should still render: {html}"
        );
        assert!(html.contains("hello"), "Alert content retained: {html}");
    }

    #[tokio::test]
    async fn test_incomplete_blockquote_paragraph() {
        // Blockquote itself is not eligible; its inner Paragraph is.
        let html = render_markdown_marked("> TK quote me", DEFAULT_MARKERS).await;
        assert!(
            html.contains(r#"<p><span class="mbr-incomplete">"#),
            "Inner <p> should carry the span, not <blockquote>: {html}"
        );
        assert!(
            !html.contains(r#"<blockquote><span"#),
            "Blockquote should not be span-wrapped: {html}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_with_strong_emphasis() {
        // Span goes immediately after <p>, so it wraps the <strong>.
        let html = render_markdown_marked("**TK** finish later", DEFAULT_MARKERS).await;
        assert!(
            html.contains(r#"<p><span class="mbr-incomplete"><strong>TK</strong>"#),
            "Span should wrap <strong>TK</strong>: {html}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_with_link() {
        // A link starting the paragraph: span should wrap the <a>.
        let html =
            render_markdown_marked("[TK](https://example.com) check this", DEFAULT_MARKERS).await;
        assert!(
            html.contains(r#"<p><span class="mbr-incomplete"><a "#),
            "Span should wrap the link: {html}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_negative_tomato() {
        // Word starting with "T" but not a marker.
        let html = render_markdown_marked("Tomato is red.", DEFAULT_MARKERS).await;
        assert!(
            !html.contains("mbr-incomplete"),
            "'Tomato' should not match: {html}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_negative_lowercase() {
        let html = render_markdown_marked("Tk lowercase ignored.", DEFAULT_MARKERS).await;
        assert!(
            !html.contains("mbr-incomplete"),
            "Mixed case 'Tk' should not match: {html}"
        );
        let html2 = render_markdown_marked("todo lowercase.", DEFAULT_MARKERS).await;
        assert!(
            !html2.contains("mbr-incomplete"),
            "lowercase 'todo' should not match: {html2}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_negative_word_boundary() {
        // TKTK and TODOs must not match (no word boundary at marker end).
        let html = render_markdown_marked("TKTK shouldn't match.", DEFAULT_MARKERS).await;
        assert!(
            !html.contains("mbr-incomplete"),
            "TKTK should not match: {html}"
        );
        let html2 = render_markdown_marked("TODOs are plural.", DEFAULT_MARKERS).await;
        assert!(
            !html2.contains("mbr-incomplete"),
            "'TODOs' should not match: {html2}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_negative_mid_paragraph() {
        let html =
            render_markdown_marked("This paragraph mentions TK in the middle.", DEFAULT_MARKERS)
                .await;
        assert!(
            !html.contains("mbr-incomplete"),
            "Mid-paragraph TK should not match: {html}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_negative_code_block() {
        // Code blocks never push a frame, so the TK inside is ignored.
        let md = "```\nTK code lines\n```\n";
        let html = render_markdown_marked(md, DEFAULT_MARKERS).await;
        assert!(
            !html.contains("mbr-incomplete"),
            "TK in code block should not match: {html}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_negative_frontmatter() {
        let md = "---\ntitle: TK rename later\n---\n\nNormal paragraph.";
        let html = render_markdown_marked(md, DEFAULT_MARKERS).await;
        assert!(
            !html.contains("mbr-incomplete"),
            "TK in frontmatter should not match: {html}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_disabled_no_span() {
        // mark_incomplete=false → never injects spans, even with markers present.
        let html = render_markdown("TK should not be highlighted.").await;
        assert!(
            !html.contains("mbr-incomplete"),
            "Disabled flag suppresses span: {html}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_custom_markers() {
        let html = render_markdown_marked("NOTE this draft.", &["NOTE"]).await;
        assert!(
            html.contains(r#"<p><span class="mbr-incomplete">"#),
            "Custom marker NOTE should match: {html}"
        );
        // TK is not in the custom marker list now.
        let html2 = render_markdown_marked("TK ignored under custom list.", &["NOTE"]).await;
        assert!(
            !html2.contains("mbr-incomplete"),
            "TK should not match when only NOTE configured: {html2}"
        );
    }

    #[tokio::test]
    async fn test_incomplete_empty_markers_no_op() {
        // Empty marker list short-circuits the pass: no spans injected.
        let html = render_markdown_marked("TK still here.", &[]).await;
        assert!(
            !html.contains("mbr-incomplete"),
            "Empty marker list should not inject spans: {html}"
        );
    }

    // Wikilink and tag link tests

    fn make_sources(sources: &[&str]) -> HashSet<String> {
        sources.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn test_wikilink_transformation() {
        // [[Tags:rust]] should become a link to /tags/rust/
        let sources = make_sources(&["tags"]);
        let md = "Check out [[Tags:rust]] for more info.";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="/tags/rust/""#),
            "Wikilink should transform to tag URL. Got: {}",
            html
        );
        assert!(
            html.contains(">rust<"),
            "Link text should be the tag value. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_wikilink_with_spaces() {
        // [[performers:Joshua Jay]] should become a link to /performers/joshua_jay/
        let sources = make_sources(&["performers"]);
        let md = "Watch [[performers:Joshua Jay]] perform!";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="/performers/joshua_jay/""#),
            "Wikilink with spaces should normalize URL. Got: {}",
            html
        );
        assert!(
            html.contains(">Joshua Jay<"),
            "Link text should preserve original case. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_wikilink_unknown_source_becomes_native_wikilink() {
        // [[category:books]] - category is not a valid tag source, so transform_wikilinks
        // leaves it alone. But pulldown-cmark's native wikilink support picks it up
        // and renders it as a link to "category:books".
        let sources = make_sources(&["tags"]);
        let md = "See [[category:books]] for more.";
        let html = render_markdown_with_tags(md, sources).await;
        // With native wikilink support, this becomes a link (not literal text)
        assert!(
            html.contains("<a"),
            "Wikilink should become a link via pulldown-cmark. Got: {}",
            html
        );
        // The link destination should be the wikilink content
        assert!(
            html.contains("category:books"),
            "Link should reference the wikilink content. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_markdown_tag_link() {
        // [text](Tags:rust) should become a link to /tags/rust/
        let sources = make_sources(&["tags"]);
        let md = "[Learn Rust](Tags:rust)";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="/tags/rust/""#),
            "Tag link should transform to tag URL. Got: {}",
            html
        );
        assert!(
            html.contains(">Learn Rust<"),
            "Link text should be preserved. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_markdown_tag_link_normalized() {
        // [Great performer](performers:joshua_jay) -> /performers/joshua_jay/
        // Note: Markdown link destinations can't contain unescaped spaces,
        // so tag values in [text](Source:value) format must be pre-normalized.
        // Use [[Source:value with spaces]] wikilink format for values with spaces.
        let sources = make_sources(&["performers"]);
        let md = "[Great performer](performers:joshua_jay)";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="/performers/joshua_jay/""#),
            "Tag link should transform to tag URL. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_url_scheme_not_treated_as_tag() {
        // [Example](https://example.com) should remain a regular URL
        let sources = make_sources(&["tags", "https"]); // Even if https is a source
        let md = "[Example](https://example.com)";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="https://example.com""#),
            "URL schemes should not be treated as tag sources. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_multiple_wikilinks() {
        // Multiple wikilinks in one document
        let sources = make_sources(&["tags"]);
        let md = "Learn [[Tags:rust]] and [[Tags:python]] today!";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="/tags/rust/""#),
            "First wikilink should work. Got: {}",
            html
        );
        assert!(
            html.contains(r#"href="/tags/python/""#),
            "Second wikilink should work. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_nested_tag_source() {
        // [[taxonomy.tags:rust]] for nested frontmatter fields
        let sources = make_sources(&["taxonomy.tags"]);
        let md = "See [[taxonomy.tags:rust]] for more.";
        let html = render_markdown_with_tags(md, sources).await;
        assert!(
            html.contains(r#"href="/taxonomy.tags/rust/""#),
            "Nested tag source should work. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_no_tag_sources_uses_native_wikilinks() {
        // When no tag sources configured, transform_wikilinks is skipped entirely.
        // pulldown-cmark's native wikilink support still applies, rendering
        // [[Tags:rust]] as a link to "Tags:rust".
        let sources = HashSet::new();
        let md = "See [[Tags:rust]] for more.";
        let html = render_markdown_with_tags(md, sources).await;
        // With native wikilink support, this becomes a link (not literal text)
        assert!(
            html.contains("<a"),
            "Wikilink should become a link via pulldown-cmark. Got: {}",
            html
        );
        assert!(
            html.contains("Tags:rust"),
            "Link should reference the wikilink content. Got: {}",
            html
        );
    }

    // Regression tests for plain wikilinks (no colon/source prefix)
    // These verify that pulldown-cmark's native wikilink support works correctly

    #[tokio::test]
    async fn test_plain_wikilink_works() {
        // Plain [[MyPage]] should become a link to MyPage
        let html = render_markdown("Check out [[MyPage]] for more.").await;
        assert!(
            html.contains("<a"),
            "Plain wikilink should become a link. Got: {}",
            html
        );
        assert!(
            html.contains("MyPage"),
            "Link should reference MyPage. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_plain_wikilink_with_spaces() {
        // [[My Page]] should work with spaces
        let html = render_markdown("See [[My Page]] here.").await;
        assert!(
            html.contains("<a"),
            "Wikilink with spaces should become a link. Got: {}",
            html
        );
        assert!(
            html.contains("My Page"),
            "Link should preserve the page name. Got: {}",
            html
        );
    }

    #[tokio::test]
    async fn test_tag_and_plain_wikilinks_together() {
        // Both tag-style and plain wikilinks should work in the same document
        let sources = make_sources(&["tags"]);
        let md = "See [[Tags:rust]] and also [[MyPage]] for info.";
        let html = render_markdown_with_tags(md, sources).await;
        // Tag wikilink should go to /tags/rust/
        assert!(
            html.contains(r#"href="/tags/rust/""#),
            "Tag wikilink should transform to /tags/rust/. Got: {}",
            html
        );
        // Plain wikilink should become a link to MyPage
        assert!(
            html.contains("MyPage"),
            "Plain wikilink should reference MyPage. Got: {}",
            html
        );
        // Should have two links
        let link_count = html.matches("<a").count();
        assert!(
            link_count >= 2,
            "Should have at least 2 links. Got {} in: {}",
            link_count,
            html
        );
    }

    #[tokio::test]
    async fn test_code_blocks_with_unsupported_language() {
        // Code blocks with unknown languages must still render valid HTML
        // so that hljs can gracefully skip them at runtime
        let md = "```unknownlang\nsome code\n```";
        let html = render_markdown(md).await;
        assert!(
            html.contains("<pre><code class=\"language-unknownlang\">"),
            "Unsupported language should still get a language class. Got: {}",
            html
        );
        assert!(html.contains("some code"));
    }

    #[tokio::test]
    async fn test_code_blocks_mixed_supported_and_unsupported_languages() {
        // When valid and invalid languages coexist, all blocks must render
        // with proper language classes so hljs can highlight what it can
        let md = concat!(
            "```rust\nfn main() {}\n```\n\n",
            "```garbage_lang_404\nfoo bar\n```\n\n",
            "```python\nprint(1)\n```",
        );
        let html = render_markdown(md).await;
        assert!(
            html.contains("language-rust"),
            "Rust block missing. Got: {}",
            html
        );
        assert!(
            html.contains("language-garbage_lang_404"),
            "Unsupported block missing. Got: {}",
            html
        );
        assert!(
            html.contains("language-python"),
            "Python block missing. Got: {}",
            html
        );
        assert!(html.contains("fn main"));
        assert!(html.contains("foo bar"));
        assert!(html.contains("print(1)"));
    }

    // ==================== Comment-only YAML frontmatter ====================

    #[test]
    fn yaml_loader_yields_no_documents_for_comment_only_frontmatter() {
        // Precondition for the two regressions below: yaml-rust2 parses a
        // comment-only block *successfully* but returns zero documents, so the
        // old `.map(|ys| ys[0].clone()).ok()` indexed out of bounds — and `.ok()`
        // cannot catch a panic. Release builds set `panic = 'abort'`, so one
        // user file with commented-out frontmatter SIGABRTed the process.
        assert!(
            YamlLoader::load_from_str("# tags: [draft]")
                .expect("comment-only YAML parses")
                .is_empty()
        );
        assert!(load_first_yaml_doc("# tags: [draft]").is_none());
    }

    #[test]
    fn parse_survives_comment_only_frontmatter() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"---\n# tags: [draft]\n---\n\nBody text.\n")
            .unwrap();
        let doc = parse(file.path()).expect("parse must not panic");
        assert!(
            doc.frontmatter.is_empty(),
            "comment-only frontmatter yields no metadata, got: {:?}",
            doc.frontmatter
        );
    }

    #[test]
    fn extract_metadata_survives_comment_only_frontmatter() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"---\n# tags: [draft]\n---\n\nBody text.\n")
            .unwrap();
        let meta = extract_metadata_from_file(file.path()).expect("must not panic");
        assert!(
            meta.metadata.is_empty(),
            "comment-only frontmatter yields no metadata, got: {:?}",
            meta.metadata
        );
        assert!(meta.relationships.is_empty());
    }

    // ==================== UTF-8 BOM handling ====================

    const BOM_DOC: &str = "\u{feff}---\ntitle: My Page\ntags:\n  - alpha\n---\n\nBody text.\n";

    #[tokio::test]
    async fn bom_prefixed_frontmatter_still_renders_as_metadata() {
        let result = render_result(BOM_DOC).await;
        assert_eq!(
            result.frontmatter.get("title"),
            Some(&serde_json::Value::String("My Page".to_string())),
            "BOM suppressed the metadata block"
        );
        assert!(result.frontmatter.contains_key("tags"));
        assert!(
            !result.html.contains(EM_DASH),
            "frontmatter leaked into the body as an em-dash heading: {}",
            result.html
        );
    }

    #[test]
    fn bom_prefixed_frontmatter_extracts_metadata() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(BOM_DOC.as_bytes()).unwrap();
        let meta = extract_metadata_from_file(file.path()).unwrap();
        assert_eq!(
            meta.metadata.get("title"),
            Some(&serde_json::Value::String("My Page".to_string()))
        );
        assert!(meta.metadata.contains_key("tags"));
    }

    #[test]
    fn bom_prefixed_frontmatter_parses() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(BOM_DOC.as_bytes()).unwrap();
        let doc = parse(file.path()).unwrap();
        assert_eq!(
            doc.frontmatter.get("title"),
            Some(&serde_json::Value::String("My Page".to_string()))
        );
    }

    #[test]
    fn strip_bom_leaves_bom_less_input_alone() {
        assert_eq!(strip_bom("# Heading"), "# Heading");
        assert_eq!(strip_bom("\u{feff}# Heading"), "# Heading");
        let mut owned = String::from("\u{feff}---\n");
        strip_bom_in_place(&mut owned);
        assert_eq!(owned, "---\n");
        let mut untouched = String::from("plain");
        strip_bom_in_place(&mut untouched);
        assert_eq!(untouched, "plain");
    }

    // ==================== Code-block text transforms ====================

    #[tokio::test]
    async fn vid_shortcode_inside_code_fence_is_not_expanded() {
        let html = render_result("```\n{{ vid(path=\"demo.mp4\") }}\n```\n")
            .await
            .html;
        assert!(
            !html.contains("<video"),
            "vid shortcode expanded inside a fence: {html}"
        );
        assert!(
            html.contains("vid(path="),
            "shortcode should render literally: {html}"
        );
    }

    #[tokio::test]
    async fn bare_url_inside_code_fence_is_not_embedded() {
        let html = render_result("```\nhttps://example.com/in-code\n```\n")
            .await
            .html;
        // Assert on `<a href=` rather than the exact URL: the buggy path fed the
        // code text (trailing newline included) into `PageInfo::html()`, so the
        // emitted href was escaped and would not match the literal URL.
        assert!(
            !html.contains("<a href="),
            "bare URL linkified inside a fence: {html}"
        );
        assert!(
            html.contains("https://example.com/in-code"),
            "URL should render literally: {html}"
        );
    }

    #[tokio::test]
    async fn canceled_checkbox_marker_inside_code_fence_is_not_transformed() {
        let html = render_result("```\n[-] not a checkbox\n```\n").await.html;
        assert!(
            !html.contains("mbr-task-check"),
            "checkbox transform fired inside a fence: {html}"
        );
        assert!(
            html.contains("[-] not a checkbox"),
            "line should render literally: {html}"
        );
    }

    #[tokio::test]
    async fn text_transforms_still_apply_outside_code_fences() {
        let vid_html = render_result("{{ vid(path=\"demo.mp4\") }}").await.html;
        assert!(
            vid_html.contains("<video"),
            "vid shortcode outside a fence must expand: {vid_html}"
        );

        let url_html = render_result("https://example.com/outside").await.html;
        assert!(
            url_html.contains("<a href=\"https://example.com/outside\""),
            "bare URL outside a fence must still be linkified: {url_html}"
        );

        let todo_html = render_result("- [-] canceled task").await.html;
        assert!(
            todo_html.contains("mbr-task-check"),
            "checkbox transform must still work outside a fence: {todo_html}"
        );
    }

    #[test]
    fn collect_bare_urls_skips_code_blocks() {
        // A URL that only appears in a code sample must never trigger an
        // outbound HTTP request.
        let (fenced, _, _) = collect_events_and_headings(
            "```\nhttps://example.com/in-code\n```\n",
            TaskMarkup::Skip,
        );
        assert!(
            collect_bare_urls(&fenced).is_empty(),
            "code-block URLs must not be queued for fetching"
        );

        let (prose, _, _) =
            collect_events_and_headings("https://example.com/outside\n", TaskMarkup::Skip);
        assert_eq!(
            collect_bare_urls(&prose).len(),
            1,
            "prose URLs must still be queued"
        );
    }

    // ==================== Async/sync embed parity ====================

    #[tokio::test]
    async fn local_embeds_render_at_timeout_zero_in_both_paths() {
        // No-network embeds (YouTube here) must survive `oembed_timeout_ms = 0`;
        // QuickLook hardcodes 0 into the async path and builds default to it.
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"https://youtu.be/dQw4w9WgXcQ\n").unwrap();
        let path = file.path().to_path_buf();
        let root = path.parent().unwrap().to_path_buf();
        let config = LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: false,
            url_depth: None,
            current_page_url: String::new(),
            markdown_page_probe: None,
        };

        let async_result = render_with_cache(
            path.clone(),
            &root,
            0,
            config.clone(),
            None,
            false,
            false,
            HashSet::new(),
            false,
            &[],
            None,
        )
        .await
        .unwrap();
        let sync_result = render_sync(
            path,
            &root,
            0,
            config,
            None,
            false,
            false,
            HashSet::new(),
            false,
            &[],
            None,
        )
        .unwrap();

        assert!(
            async_result.html.contains("youtube-embed"),
            "async path dropped the no-network embed: {}",
            async_result.html
        );
        assert_eq!(
            async_result.html, sync_result.html,
            "async and sync render paths must agree at oembed_timeout_ms = 0"
        );
    }

    // ==================== Heading text extraction ====================

    #[tokio::test]
    async fn heading_text_and_anchor_include_inline_code() {
        let result = render_result("## The `main` function\n").await;
        assert_eq!(result.headings[0].text, "The main function");
        assert_eq!(result.headings[0].id, "the-main-function");
    }

    #[test]
    fn extract_first_h1_includes_inline_code() {
        assert_eq!(
            extract_first_h1("# The `main` function\n"),
            Some("The main function".to_string())
        );
    }

    #[tokio::test]
    async fn heading_text_excludes_raw_inline_html() {
        // Deliberate: the raw `<kbd>` markup is not readable heading text, and
        // its inner text already arrives as a separate Text event.
        let result = render_result("## Press <kbd>Ctrl</kbd> now\n").await;
        assert_eq!(result.headings[0].text, "Press Ctrl now");
        assert_eq!(result.headings[0].id, "press-ctrl-now");
    }

    // ==================== Anchor id generation ====================

    #[test]
    fn generate_anchor_id_basic_slugification() {
        let mut anchor_ids = HashMap::new();
        assert_eq!(
            generate_anchor_id("Hello World", &mut anchor_ids),
            "hello-world"
        );
        assert_eq!(
            generate_anchor_id("Ünïcode Heading", &mut anchor_ids),
            "ünïcode-heading"
        );
        // Punctuation is dropped and can leave a doubled separator. Recorded as
        // the slugifier's current behavior, not endorsed as a good slug —
        // changing it would break every existing hand-written `#anchor` link.
        assert_eq!(
            generate_anchor_id("Hello, World!", &mut anchor_ids),
            "hello--world"
        );
    }

    #[test]
    fn generate_anchor_id_never_collides() {
        // Regression: the old per-base counter emitted `step-1-2` for both the
        // second "Step 1" and for "Step 1-2".
        let mut anchor_ids = HashMap::new();
        let headings = ["Step 1", "Step 1", "Step 1-2", "", "", "Ünïcode Ünïcode"];
        let ids: Vec<String> = headings
            .iter()
            .map(|h| generate_anchor_id(h, &mut anchor_ids))
            .collect();

        assert_eq!(
            ids.iter().collect::<HashSet<_>>().len(),
            ids.len(),
            "duplicate anchor ids: {ids:?}"
        );
        assert_eq!(ids[0], "step-1");
        assert_eq!(ids[1], "step-1-2");
        assert_eq!(ids[3], "heading");
        assert_eq!(ids[4], "heading-2");
    }

    // ==================== Bounded oembed fan-out ====================

    #[test]
    fn cap_fetch_list_limits_and_is_deterministic() {
        let urls: Vec<String> = (0..150)
            .map(|i| format!("https://example.com/{i:03}"))
            .collect();

        let mut expected = urls.clone();
        expected.sort_unstable();
        expected.truncate(MAX_OEMBED_FETCHES_PER_DOC);

        assert_eq!(
            cap_fetch_list(urls.clone()).len(),
            MAX_OEMBED_FETCHES_PER_DOC
        );
        assert_eq!(cap_fetch_list(urls.clone()), expected);

        // Truncation must not depend on iteration order (collect_bare_urls
        // returns a HashSet, whose order varies per process).
        let reversed: Vec<String> = urls.into_iter().rev().collect();
        assert_eq!(cap_fetch_list(reversed), expected);
    }

    #[test]
    fn cap_fetch_list_leaves_small_lists_untouched() {
        let urls = vec![
            "https://b.example".to_string(),
            "https://a.example".to_string(),
        ];
        assert_eq!(
            cap_fetch_list(urls.clone()),
            urls,
            "lists under the cap must be passed through unchanged"
        );
    }

    #[tokio::test]
    async fn prefetch_oembed_urls_resolves_local_embeds_without_network() {
        // Every URL here short-circuits in `PageInfo::new_from_url` via the
        // no-network embed path, so this exercises the bounded stream with no I/O.
        let md = "https://youtu.be/aaaaaaaaaaa\n\nhttps://youtu.be/bbbbbbbbbbb\n\nhttps://youtu.be/ccccccccccc\n";
        let (events, _, _) = collect_events_and_headings(md, TaskMarkup::Skip);
        let results = prefetch_oembed_urls(&events, 500, &None).await;
        assert_eq!(results.len(), 3);
        assert!(results.values().all(|info| info.embed_html.is_some()));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Anchor ids must be unique for any sequence of heading texts —
        /// duplicates silently break in-page TOC links and permalinks.
        #[test]
        fn anchor_ids_are_always_unique(
            headings in proptest::collection::vec(any::<String>(), 0..30)
        ) {
            let mut anchor_ids = HashMap::new();
            let ids: Vec<String> = headings
                .iter()
                .map(|h| generate_anchor_id(h, &mut anchor_ids))
                .collect();
            let unique: HashSet<&String> = ids.iter().collect();
            prop_assert_eq!(unique.len(), ids.len());
        }
    }
}
