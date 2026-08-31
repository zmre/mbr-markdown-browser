// Copyright 2015 Google Inc. All rights reserved.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

//! # MBR HTML Renderer
//!
//! Modified from pulldown-cmark's html.rs to support MBR-specific features.
//!
//! ## MBR Extensions
//!
//! All extensions are controlled via [`HtmlConfig`] and can be enabled/disabled:
//!
//! | Extension | Config Flag | Description |
//! |-----------|-------------|-------------|
//! | **Section wrapping** | `enable_sections` | Wraps content in `<section>` tags with `<hr>` as dividers |
//! | **Mermaid diagrams** | `enable_mermaid` | Renders \`\`\`mermaid blocks as `<pre class="mermaid">` |
//!
//! ## Usage
//!
//! ```rust,ignore
//! // Use MBR defaults (all extensions enabled)
//! html::push_html_mbr(&mut output, events);
//!
//! // Or configure explicitly
//! let config = HtmlConfig { enable_sections: true, enable_mermaid: false };
//! html::push_html_with_config(&mut output, events, config);
//! ```
//!
//! ## Upstream Tracking
//!
//! Based on: <https://github.com/pulldown-cmark/pulldown-cmark/blob/master/pulldown-cmark/src/html.rs>
//!
//! Key differences from upstream:
//! - Added [`HtmlConfig`] for extension configuration
//! - Added `section_started` field for section tracking
//! - Added `container_depth` field so section boundaries are only emitted at
//!   top level (a `</section>` inside a blockquote or list mis-nests)
//! - Added `codeblock_state` field for mermaid closing tag handling
//! - Link and image destinations with a script-capable scheme (`javascript:`,
//!   `vbscript:`, non-image `data:`) are replaced with an inert value
//! - Images are emitted with `loading="lazy" decoding="async"`
//! - Removed `ContainerBlock` handling (not used in MBR)
//!
//! Output is pinned by a golden corpus in `tests/render_golden.rs`.

use std::collections::HashMap;

use crate::attrs::ParsedAttrs;
use crate::tasks::{Annotations, TaskPriority, TaskStatus};
use chrono::NaiveDateTime;
use pulldown_cmark_escape::IoWriter;
use pulldown_cmark_escape::{FmtWriter, StrWrite, escape_href, escape_html, escape_html_body_text};

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, CowStr,
    Event::{self, *},
    LinkType, Tag, TagEnd,
};

// ============================================================================
// MBR EXTENSION: Configuration
// ============================================================================

/// The source line one block-level element was written on, addressed by the
/// element's `Start` event's index in the stream handed to the writer.
///
/// Built by [`crate::markdown`], which is the only place the parser's byte
/// offsets exist. See `markdown::BlockLines` for the table and for why the
/// index survives the three render passes.
#[derive(Debug, Clone, Copy)]
pub struct BlockLine {
    /// Index of the block's `Start` event in the event stream.
    pub at: u32,
    /// 1-based source line the block starts on.
    pub line: u32,
}

/// Configuration for MBR-specific HTML extensions.
///
/// This struct controls all custom behavior beyond pulldown-cmark's standard
/// HTML output. Use [`HtmlConfig::mbr_defaults()`] for standard MBR behavior,
/// or construct manually for fine-grained control.
#[derive(Debug, Default, Clone)]
pub struct HtmlConfig {
    /// Wrap content in `<section>` tags, using `<hr>` as dividers.
    ///
    /// When enabled:
    /// - Output starts with `<section>\n`
    /// - Each **top-level** `---` (Rule) becomes `</section>\n<hr />\n<section>\n`
    /// - A `---` nested inside a blockquote, list item, footnote definition,
    ///   table cell, or definition renders as a plain `<hr />`: splitting the
    ///   section there would close it outside its container and eject the
    ///   remaining content from the quote/list on reparse
    /// - Output ends with `</section>\n`
    pub enable_sections: bool,

    /// Render \`\`\`mermaid code blocks as `<pre class="mermaid">` without
    /// the `<code>` wrapper, allowing mermaid.js to render diagrams directly.
    pub enable_mermaid: bool,

    /// Attributes for each section (by 1-based index).
    ///
    /// Section 0 is the first section (before any `---`), section 1 is after the
    /// first `---`, etc. When rendering, attributes are applied to the `<section>` tag.
    ///
    /// Use syntax like `--- {#id .class data-attr="value"}` in markdown to set attrs.
    pub section_attrs: HashMap<usize, ParsedAttrs>,

    /// Source lines for block-level elements, ascending by `at`.
    ///
    /// **Empty means the feature is off**, which is what every caller that does
    /// not opt in gets: `HtmlConfig::default()` and [`HtmlConfig::mbr_defaults`]
    /// both leave it empty, so the writer's output is byte-for-byte what it was
    /// before `data-mbr-line` existed.
    ///
    /// When non-empty, the writer emits ` data-mbr-line="N"` on the six block
    /// tags listed in `markdown::is_review_block_start` — the single definition
    /// of that set, which the arms here must not drift from.
    pub block_lines: Vec<BlockLine>,
}

impl HtmlConfig {
    /// Standard MBR configuration with all extensions enabled and no section attrs.
    pub fn mbr_defaults() -> Self {
        Self {
            enable_sections: true,
            enable_mermaid: true,
            section_attrs: HashMap::new(),
            block_lines: Vec::new(),
        }
    }

    /// Standard MBR configuration with section attributes.
    pub fn mbr_with_section_attrs(section_attrs: HashMap<usize, ParsedAttrs>) -> Self {
        Self {
            enable_sections: true,
            enable_mermaid: true,
            section_attrs,
            block_lines: Vec::new(),
        }
    }

    /// Standard MBR configuration with section attributes and `data-mbr-line`
    /// source lines.
    ///
    /// An empty `block_lines` is exactly [`HtmlConfig::mbr_with_section_attrs`],
    /// so a caller that computed no lines pays nothing and emits nothing.
    pub fn mbr_with_attrs_and_block_lines(
        section_attrs: HashMap<usize, ParsedAttrs>,
        block_lines: Vec<BlockLine>,
    ) -> Self {
        Self {
            enable_sections: true,
            enable_mermaid: true,
            section_attrs,
            block_lines,
        }
    }
}

// ============================================================================
// Internal Types
// ============================================================================

enum TableState {
    Head,
    Body,
}

struct HtmlWriter<'a, I, W> {
    /// Iterator supplying events.
    iter: I,

    /// Writer to write to.
    writer: W,

    /// Whether or not the last write wrote a newline.
    end_newline: bool,

    /// Whether if inside a metadata block (text should not be written)
    in_non_writing_block: bool,

    // MBR EXTENSION: Mermaid support - tracks the closing tag for code blocks
    codeblock_state: Option<CowStr<'a>>,

    table_state: TableState,
    table_alignments: Vec<Alignment>,
    table_cell_index: usize,
    numbers: HashMap<CowStr<'a>, usize>,

    // MBR EXTENSION: Configuration for extensions
    config: HtmlConfig,

    // MBR EXTENSION: Section wrapping - tracks if opening section was emitted
    section_started: bool,

    // MBR EXTENSION: Section attributes - tracks current section index (0-based)
    current_section: usize,

    // MBR EXTENSION: Section wrapping - how many block containers (blockquote,
    // list, list item, table cell, footnote definition, definition list) are
    // currently open. A `<section>` boundary may only be emitted at depth 0.
    container_depth: usize,

    // MBR EXTENSION: `data-mbr-line` - how many events have been pulled off
    // `iter`, which is the key `config.block_lines` is addressed by. Maintained
    // by `next_event` and by nothing else; see that method.
    event_index: usize,

    // MBR EXTENSION: `data-mbr-line` - cursor into `config.block_lines`. The
    // writer's lookups are non-decreasing (it walks the stream front to back),
    // so this walks rather than binary-searching, exactly like
    // `markdown::TextLineCursor`.
    block_line_next: usize,

    // MBR EXTENSION: `data-mbr-line` - hoisted `!config.block_lines.is_empty()`
    // so the off path (every build, CLI and QuickLook render) is one branch.
    emit_block_lines: bool,
}

/// Block containers that a `<section>` boundary must never be emitted inside.
///
/// Splitting a section in the middle of one of these emits `</section>` before
/// the container's own closing tag, which a browser resolves by hoisting the
/// remaining content out of the quote/list entirely.
fn is_block_container(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::BlockQuote(_)
            | Tag::List(_)
            | Tag::Item
            | Tag::TableCell
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListDefinition
    )
}

/// [`is_block_container`] for the closing half of the event stream.
fn is_block_container_end(tag: &TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::BlockQuote(_)
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::TableCell
            | TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListDefinition
    )
}

// ============================================================================
// MBR EXTENSION: Task list markup
// ============================================================================

/// Closing tag for the span [`task_text_open`] opens.
pub(crate) const TASK_TEXT_CLOSE: &str = "</span>";

/// Renders the checkbox that stands in for a markdown task marker.
///
/// `line` is the 1-based source line the checkbox was written on. It is `Some`
/// on the document render path, where [`crate::markdown`] recovers it from the
/// parser's byte offsets, and `None` on the raw [`push_html_mbr`] path, which
/// has no source to measure against.
///
/// The checkbox is `disabled` in **every** mode — server, GUI, static build and
/// QuickLook alike. Making it live is a frontend concern: a later phase removes
/// the attribute at runtime when edit mode is on, which keeps config plumbing
/// out of the render path and keeps a static build byte-identical to a served
/// page.
pub(crate) fn task_checkbox_html(status: TaskStatus, line: Option<u32>) -> String {
    // Long enough for the widest form (`canceled`, four-digit line) without a
    // reallocation.
    let mut html = String::with_capacity(128);
    html.push_str("<input type=\"checkbox\" class=\"mbr-task-check\"");
    if let Some(line) = line {
        // `id` is the anchor the task browser deep-links to (`#mbr-task-42`);
        // `data-mbr-task-line` is what a line patch sends back to the server.
        let _ = write!(
            html,
            " id=\"mbr-task-{line}\" data-mbr-task-line=\"{line}\""
        );
    }
    // `checked` alone cannot distinguish canceled from open, so the status is
    // also written out for CSS and for the toggle handler.
    let _ = write!(
        html,
        " data-mbr-task-status=\"{}\"",
        task_status_slug(status)
    );
    if status == TaskStatus::Done {
        html.push_str(" checked");
    }
    html.push_str(" disabled>");
    html
}

/// Opening tag for the span wrapping a task's display text.
///
/// The status class goes here rather than on the `<li>` so that a canceled
/// parent task strikes through its own text only, leaving its subtasks and its
/// annotation chips untouched. (`<li>` carries no class at all: pulldown-cmark
/// gives the writer no attribute channel for list items, and CSS reaches it
/// with `:has()` — see the task rules in `templates/theme.css`.)
pub(crate) const fn task_text_open(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Canceled => "<span class=\"mbr-task-text mbr-task-canceled\">",
        TaskStatus::Open | TaskStatus::Done => "<span class=\"mbr-task-text\">",
    }
}

/// Renders the annotations stripped out of a task's text as chips.
///
/// Returns an empty string for a task with no annotations, which is the common
/// case and the one worth not allocating for.
pub(crate) fn task_annotations_html(annotations: &Annotations) -> String {
    let mut html = String::new();

    if let Some((class, label)) = task_priority_chip(annotations.priority) {
        // Empty content on purpose: the dot is drawn by CSS, so the label has
        // to reach a screen reader some other way.
        let _ = write!(
            html,
            " <span class=\"mbr-task-pri {class}\" role=\"img\" aria-label=\"{label}\" title=\"{label}\"></span>"
        );
    }
    for tag in &annotations.tags {
        html.push_str(" <span class=\"mbr-task-tag\">#");
        // The tag grammar is `[A-Za-z0-9_-]+`, so this can never change a byte.
        // It stays because markup assembled by hand is exactly where a later
        // grammar widening turns into an injection.
        let _ = escape_html(&mut html, tag);
        html.push_str("</span>");
    }
    if let Some(due) = annotations.due {
        push_task_time(&mut html, "mbr-task-due", due, annotations.due_has_time);
    }
    if let Some(done) = annotations.done {
        push_task_time(
            &mut html,
            "mbr-task-completed",
            done,
            annotations.done_has_time,
        );
    }
    if let Some(moved_to) = annotations.moved_to {
        push_task_time(
            &mut html,
            "mbr-task-moved",
            moved_to.and_time(chrono::NaiveTime::MIN),
            false,
        );
    }

    html
}

/// The value written to `data-mbr-task-status`.
const fn task_status_slug(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Open => "open",
        TaskStatus::Done => "done",
        TaskStatus::Canceled => "canceled",
    }
}

/// Modifier class and accessible label for a priority dot, or `None` for the
/// default priority, which draws nothing.
const fn task_priority_chip(priority: TaskPriority) -> Option<(&'static str, &'static str)> {
    match priority {
        TaskPriority::Normal => None,
        TaskPriority::High => Some(("mbr-task-pri-high", "High priority")),
        TaskPriority::Urgent => Some(("mbr-task-pri-urgent", "Urgent priority")),
    }
}

/// Appends one `<time>` chip.
///
/// The visible label deliberately omits the year: a chip reading "Aug 5" is
/// legible at a glance, and the unabbreviated value is already in `datetime`
/// for anything that needs to compute with it. Nothing here is relative to
/// *today* — a server render is cached and a static build is frozen, so an
/// "overdue" class baked in at render time would be a lie by tomorrow. Marking
/// overdue is left to the frontend, which knows what day it is.
fn push_task_time(html: &mut String, class: &str, value: NaiveDateTime, has_time: bool) {
    let (attr, label) = if has_time {
        (
            value.format("%Y-%m-%dT%H:%M").to_string(),
            value.format("%b %-d, %-I:%M %p").to_string(),
        )
    } else {
        (
            value.format("%Y-%m-%d").to_string(),
            value.format("%b %-d").to_string(),
        )
    };
    let _ = write!(
        html,
        " <time class=\"{class}\" datetime=\"{attr}\">{label}</time>"
    );
}

// ============================================================================
// MBR EXTENSION: Destination scheme filtering
// ============================================================================

/// Written in place of a link or image destination whose URL scheme can execute
/// script. The element and its text survive; only the navigation is made inert.
const BLOCKED_DESTINATION: &str = "about:blank#blocked";

/// Length of the longest scheme [`is_blocked_destination`] rejects (`javascript`).
const MAX_BLOCKED_SCHEME_LEN: usize = 10;

/// True when `dest` carries a URL scheme that can execute script.
///
/// Markdown is routinely authored by someone other than the person serving it,
/// so `[click me](javascript:…)` must not become a live href. `data:` is rejected
/// too, except for `data:image/*` — inline images are legitimate (and preserved
/// verbatim by `link_transform`), whereas `data:text/html` is a script vector.
///
/// The scheme is normalized the way a browser normalizes one before resolving a
/// URL: leading C0 control characters and spaces are ignored, and embedded tab,
/// line feed, and carriage return characters are removed anywhere they appear.
/// Without that, `" java\tscript:alert(1)"` would slip through while still
/// navigating.
fn is_blocked_destination(dest: &str) -> bool {
    let mut scheme = [0u8; MAX_BLOCKED_SCHEME_LEN];
    let mut len = 0usize;
    let mut found_colon = false;

    for c in dest.trim_start_matches(|c: char| c <= ' ').chars() {
        match c {
            '\t' | '\n' | '\r' => continue,
            ':' => {
                found_colon = true;
                break;
            }
            _ if c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.') => {
                if len == MAX_BLOCKED_SCHEME_LEN {
                    // Longer than every scheme we reject, so it cannot match.
                    return false;
                }
                scheme[len] = c.to_ascii_lowercase() as u8;
                len += 1;
            }
            // Not a scheme character: the destination is relative or malformed.
            _ => return false,
        }
    }

    if !found_colon {
        return false;
    }

    match &scheme[..len] {
        b"javascript" | b"vbscript" => true,
        b"data" => !is_inline_image_data_url(dest),
        _ => false,
    }
}

/// True for `data:image/…` destinations, the only `data:` shape MBR allows.
fn is_inline_image_data_url(dest: &str) -> bool {
    let Some((_, payload)) = dest.split_once(':') else {
        return false;
    };
    let mut actual = payload
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .map(|c| c.to_ascii_lowercase());
    "image/"
        .chars()
        .all(|expected| actual.next() == Some(expected))
}

impl<'a, I, W> HtmlWriter<'a, I, W>
where
    I: Iterator<Item = Event<'a>>,
    W: StrWrite,
{
    fn new(iter: I, writer: W) -> Self {
        Self::new_with_config(iter, writer, HtmlConfig::default())
    }

    fn new_with_config(iter: I, writer: W, config: HtmlConfig) -> Self {
        let emit_block_lines = !config.block_lines.is_empty();
        Self {
            iter,
            writer,
            end_newline: true,
            in_non_writing_block: false,
            codeblock_state: None,
            table_state: TableState::Head,
            table_alignments: vec![],
            table_cell_index: 0,
            numbers: HashMap::new(),
            config,
            section_started: false,
            current_section: 0,
            container_depth: 0,
            event_index: 0,
            block_line_next: 0,
            emit_block_lines,
        }
    }

    /// MBR EXTENSION: the **only** way to pull an event off `iter`.
    ///
    /// # Why this must be the only way
    ///
    /// `config.block_lines` is keyed by an event's position in the stream, so
    /// the counter has to see every event the writer consumes — and the writer
    /// consumes events from *two* loops, not one: [`Self::run`] and
    /// [`Self::raw_text`], which drains an image's inner events into its `alt`
    /// attribute. A counter incremented in `run` alone would desynchronise
    /// permanently after the first image on the page, and every `data-mbr-line`
    /// after it would name the wrong source line — with well-formed HTML, no
    /// panic and no error anywhere. `block_line_index_survives_image_alt_text`
    /// pins this.
    #[inline]
    fn next_event(&mut self) -> Option<Event<'a>> {
        let event = self.iter.next();
        if event.is_some() {
            self.event_index += 1;
        }
        event
    }

    /// MBR EXTENSION: writes ` data-mbr-line="N"` for the event currently being
    /// rendered, when the table has a line for it.
    ///
    /// [`Self::event_index`] is post-incremented by [`Self::next_event`], so the
    /// event in hand sits at `event_index - 1`. That off-by-one is hidden here
    /// rather than repeated at each of the six call sites.
    fn write_block_line(&mut self) -> Result<(), W::Error> {
        let index = match self.event_index.checked_sub(1) {
            Some(index) => index,
            // Unreachable: a tag is only written for an event already pulled.
            None => return Ok(()),
        };
        // Advance the cursor past everything the writer has already gone by.
        while self
            .config
            .block_lines
            .get(self.block_line_next)
            .is_some_and(|entry| (entry.at as usize) < index)
        {
            self.block_line_next += 1;
        }
        // Copied out before `self.write`, which borrows `self` mutably.
        let line = self
            .config
            .block_lines
            .get(self.block_line_next)
            .filter(|entry| entry.at as usize == index)
            .map(|entry| entry.line);
        match line {
            Some(line) => {
                self.write(" data-mbr-line=\"")?;
                write!(&mut self.writer, "{}", line)?;
                self.write("\"")
            }
            None => Ok(()),
        }
    }

    /// Writes a new line.
    #[inline]
    fn write_newline(&mut self) -> Result<(), W::Error> {
        self.end_newline = true;
        self.writer.write_str("\n")
    }

    /// Writes a buffer, and tracks whether or not a newline was written.
    #[inline]
    fn write(&mut self, s: &str) -> Result<(), W::Error> {
        self.writer.write_str(s)?;

        if !s.is_empty() {
            self.end_newline = s.ends_with('\n');
        }
        Ok(())
    }

    /// MBR EXTENSION: Writes an `href`/`src` value, substituting
    /// [`BLOCKED_DESTINATION`] for schemes that can execute script.
    #[inline]
    fn write_destination(&mut self, dest: &str) -> Result<(), W::Error> {
        if is_blocked_destination(dest) {
            self.write(BLOCKED_DESTINATION)
        } else {
            escape_href(&mut self.writer, dest)
        }
    }

    /// MBR EXTENSION: opens a `<pre>`, carrying `data-mbr-line` when the feature
    /// is on. With it off this writes exactly `<pre>`, so the three code-block
    /// shapes stay byte-identical to what they were.
    fn write_pre_open(&mut self) -> Result<(), W::Error> {
        if self.emit_block_lines {
            self.write("<pre")?;
            self.write_block_line()?;
            self.write(">")
        } else {
            self.write("<pre>")
        }
    }

    fn run(mut self) -> Result<(), W::Error> {
        // MBR EXTENSION: Emit opening section tag with optional attrs
        if self.config.enable_sections {
            let attrs_str = self
                .config
                .section_attrs
                .get(&self.current_section)
                .map(|a| a.to_html_attr_string())
                .unwrap_or_default();
            self.write(&format!("<section{}>\n", attrs_str))?;
            self.section_started = true;
        }

        // MBR EXTENSION: `next_event`, never `self.iter.next()` — see that
        // method for why the counter must see every consumed event.
        while let Some(event) = self.next_event() {
            match event {
                Start(tag) => {
                    self.start_tag(tag)?;
                }
                End(tag) => {
                    self.end_tag(tag)?;
                }
                Text(text) => {
                    if !self.in_non_writing_block {
                        escape_html_body_text(&mut self.writer, &text)?;
                        self.end_newline = text.ends_with('\n');
                    }
                }
                Code(text) => {
                    self.write("<code>")?;
                    escape_html_body_text(&mut self.writer, &text)?;
                    self.write("</code>")?;
                }
                InlineMath(text) => {
                    self.write(r#"<span class="math math-inline">"#)?;
                    escape_html(&mut self.writer, &text)?;
                    self.write("</span>")?;
                }
                DisplayMath(text) => {
                    self.write(r#"<span class="math math-display">"#)?;
                    escape_html(&mut self.writer, &text)?;
                    self.write("</span>")?;
                }
                Html(html) | InlineHtml(html) => {
                    self.write(&html)?;
                }
                SoftBreak => {
                    self.write_newline()?;
                }
                HardBreak => {
                    self.write("<br />\n")?;
                }
                // MBR EXTENSION: Section dividers with optional attrs
                Rule => {
                    // The section index advances for every rule, nested or not, so
                    // that it stays aligned with the attribute map that
                    // `markdown.rs` builds by counting `Event::Rule` the same way.
                    if self.config.enable_sections {
                        self.current_section += 1;
                    }

                    if self.config.enable_sections && self.container_depth == 0 {
                        let attrs_str = self
                            .config
                            .section_attrs
                            .get(&self.current_section)
                            .map(|a| a.to_html_attr_string())
                            .unwrap_or_default();
                        self.write(&format!("</section>\n<hr />\n<section{}>\n", attrs_str))?;
                    } else {
                        // Standard pulldown-cmark behavior. Also used for rules
                        // nested inside a blockquote/list/footnote: closing the
                        // section there would close outside the container and
                        // eject everything after the rule from it.
                        if self.end_newline {
                            self.write("<hr />\n")?;
                        } else {
                            self.write("\n<hr />\n")?;
                        }
                    }
                }
                FootnoteReference(name) => {
                    let len = self.numbers.len() + 1;
                    self.write("<sup class=\"footnote-reference\"><a href=\"#")?;
                    escape_html(&mut self.writer, &name)?;
                    self.write("\">")?;
                    let number = *self.numbers.entry(name).or_insert(len);
                    write!(&mut self.writer, "{}", number)?;
                    self.write("</a></sup>")?;
                }
                // MBR EXTENSION: the checkbox carries mbr's task classes so the
                // theme styles it the same way here as on the document render
                // path. It cannot carry a source line: this writer is handed an
                // event stream, not the markdown it came from.
                // `markdown::collect_events_and_headings` replaces the marker
                // with a line-numbered checkbox before the events ever reach
                // here, so in the application this arm only runs for callers of
                // `push_html_mbr` that skip that pass.
                TaskListMarker(checked) => {
                    let status = if checked {
                        TaskStatus::Done
                    } else {
                        TaskStatus::Open
                    };
                    self.write(&task_checkbox_html(status, None))?;
                    self.write("\n")?;
                }
            }
        }

        // MBR EXTENSION: Emit closing section tag
        if self.config.enable_sections && self.section_started {
            self.write("</section>\n")?;
        }

        Ok(())
    }

    /// Writes the start of an HTML tag.
    fn start_tag(&mut self, tag: Tag<'a>) -> Result<(), W::Error> {
        // MBR EXTENSION: Section wrapping - track open block containers.
        if is_block_container(&tag) {
            self.container_depth += 1;
        }
        match tag {
            Tag::HtmlBlock => Ok(()),
            // MBR EXTENSION: `data-mbr-line`. One of the six tags in
            // `markdown::is_review_block_start`; keep the two sets in step.
            Tag::Paragraph => {
                if self.emit_block_lines {
                    self.write(if self.end_newline { "<p" } else { "\n<p" })?;
                    self.write_block_line()?;
                    self.write(">")
                } else if self.end_newline {
                    self.write("<p>")
                } else {
                    self.write("\n<p>")
                }
            }
            Tag::Heading {
                level,
                id,
                classes,
                attrs,
            } => {
                if self.end_newline {
                    self.write("<")?;
                } else {
                    self.write("\n<")?;
                }
                write!(&mut self.writer, "{}", level)?;
                // MBR EXTENSION: `data-mbr-line`, emitted after the level and
                // *before* the author's own id/classes/attrs. An HTML parser
                // keeps the FIRST occurrence of a duplicate attribute, so going
                // first is what makes ours authoritative against a heading
                // written as `## Title {data-mbr-line=99}`.
                if self.emit_block_lines {
                    self.write_block_line()?;
                }
                if let Some(id) = id {
                    self.write(" id=\"")?;
                    escape_html(&mut self.writer, &id)?;
                    self.write("\"")?;
                }
                let mut classes = classes.iter();
                if let Some(class) = classes.next() {
                    self.write(" class=\"")?;
                    escape_html(&mut self.writer, class)?;
                    for class in classes {
                        self.write(" ")?;
                        escape_html(&mut self.writer, class)?;
                    }
                    self.write("\"")?;
                }
                for (attr, value) in attrs {
                    self.write(" ")?;
                    escape_html(&mut self.writer, &attr)?;
                    if let Some(val) = value {
                        self.write("=\"")?;
                        escape_html(&mut self.writer, &val)?;
                        self.write("\"")?;
                    } else {
                        self.write("=\"\"")?;
                    }
                }
                self.write(">")
            }
            // MBR EXTENSION: `data-mbr-line` goes on the `<table>`, not on the
            // cells: every cell in a row shares one source line, so per-cell
            // attributes triple the byte cost of a table-heavy document to say
            // the same thing.
            Tag::Table(alignments) => {
                self.table_alignments = alignments;
                if self.emit_block_lines {
                    self.write("<table")?;
                    self.write_block_line()?;
                    self.write(">")
                } else {
                    self.write("<table>")
                }
            }
            Tag::TableHead => {
                self.table_state = TableState::Head;
                self.table_cell_index = 0;
                self.write("<thead><tr>")
            }
            Tag::TableRow => {
                self.table_cell_index = 0;
                self.write("<tr>")
            }
            Tag::TableCell => {
                match self.table_state {
                    TableState::Head => {
                        self.write("<th")?;
                    }
                    TableState::Body => {
                        self.write("<td")?;
                    }
                }
                match self.table_alignments.get(self.table_cell_index) {
                    Some(&Alignment::Left) => self.write(" style=\"text-align: left\">"),
                    Some(&Alignment::Center) => self.write(" style=\"text-align: center\">"),
                    Some(&Alignment::Right) => self.write(" style=\"text-align: right\">"),
                    _ => self.write(">"),
                }
            }
            Tag::BlockQuote(kind) => {
                let class_str = match kind {
                    None => "",
                    Some(kind) => match kind {
                        BlockQuoteKind::Note => " class=\"markdown-alert-note\"",
                        BlockQuoteKind::Tip => " class=\"markdown-alert-tip\"",
                        BlockQuoteKind::Important => " class=\"markdown-alert-important\"",
                        BlockQuoteKind::Warning => " class=\"markdown-alert-warning\"",
                        BlockQuoteKind::Caution => " class=\"markdown-alert-caution\"",
                    },
                };
                // MBR EXTENSION: `data-mbr-line`.
                if self.emit_block_lines {
                    self.write(if self.end_newline {
                        "<blockquote"
                    } else {
                        "\n<blockquote"
                    })?;
                    self.write(class_str)?;
                    self.write_block_line()?;
                    self.write(">\n")
                } else if self.end_newline {
                    self.write(&format!("<blockquote{}>\n", class_str))
                } else {
                    self.write(&format!("\n<blockquote{}>\n", class_str))
                }
            }
            Tag::CodeBlock(info) => {
                if !self.end_newline {
                    self.write_newline()?;
                }
                self.codeblock_state = Some("</code></pre>".into());
                // MBR EXTENSION: `data-mbr-line` belongs on the `<pre>` in every
                // shape below — it is the element that stands for the block.
                match info {
                    CodeBlockKind::Fenced(info) => {
                        let lang = info.split(' ').next().unwrap_or_default();
                        if lang.is_empty() {
                            self.write_pre_open()?;
                            self.write("<code>")
                        // MBR EXTENSION: Mermaid diagram support
                        } else if self.config.enable_mermaid && lang == "mermaid" {
                            self.codeblock_state = Some("</pre>".into());
                            self.write("<pre class=\"mermaid\"")?;
                            if self.emit_block_lines {
                                self.write_block_line()?;
                            }
                            self.write(">")
                        } else {
                            self.write_pre_open()?;
                            self.write("<code class=\"language-")?;
                            escape_html(&mut self.writer, lang)?;
                            self.write("\">")
                        }
                    }
                    CodeBlockKind::Indented => {
                        self.write_pre_open()?;
                        self.write("<code>")
                    }
                }
            }
            Tag::List(Some(1)) => {
                if self.end_newline {
                    self.write("<ol>\n")
                } else {
                    self.write("\n<ol>\n")
                }
            }
            Tag::List(Some(start)) => {
                if self.end_newline {
                    self.write("<ol start=\"")?;
                } else {
                    self.write("\n<ol start=\"")?;
                }
                write!(&mut self.writer, "{}", start)?;
                self.write("\">\n")
            }
            Tag::List(None) => {
                if self.end_newline {
                    self.write("<ul>\n")
                } else {
                    self.write("\n<ul>\n")
                }
            }
            // MBR EXTENSION: `data-mbr-line` on the `<li>`, never on the
            // enclosing `<ul>`/`<ol>` — a list's own line always equals its
            // first item's, so an attribute there is pure duplication.
            Tag::Item => {
                if self.emit_block_lines {
                    self.write(if self.end_newline { "<li" } else { "\n<li" })?;
                    self.write_block_line()?;
                    self.write(">")
                } else if self.end_newline {
                    self.write("<li>")
                } else {
                    self.write("\n<li>")
                }
            }
            Tag::DefinitionList => {
                if self.end_newline {
                    self.write("<dl>\n")
                } else {
                    self.write("\n<dl>\n")
                }
            }
            Tag::DefinitionListTitle => {
                // MBR EXTENSION: definition lists render as an FAQ-style
                // disclosure list -- the `<dd>` answer is collapsed until its
                // `<dt>` question is activated. That behaviour is pure CSS (see
                // the "Definition Lists" section of `templates/theme.css`), and
                // pure CSS can only observe activation as `:focus`, which a
                // `<dt>` cannot receive unless it is made focusable. The
                // `tabindex` is therefore load-bearing, not decoration: drop it
                // and every answer becomes permanently unreachable.
                if self.end_newline {
                    self.write("<dt tabindex=\"0\">")
                } else {
                    self.write("\n<dt tabindex=\"0\">")
                }
            }
            Tag::DefinitionListDefinition => {
                if self.end_newline {
                    self.write("<dd>")
                } else {
                    self.write("\n<dd>")
                }
            }
            Tag::Subscript => self.write("<sub>"),
            Tag::Superscript => self.write("<sup>"),
            Tag::Emphasis => self.write("<em>"),
            Tag::Strong => self.write("<strong>"),
            Tag::Strikethrough => self.write("<del>"),
            Tag::Link {
                link_type: LinkType::Email,
                dest_url,
                title,
                id: _,
            } => {
                self.write("<a href=\"mailto:")?;
                escape_href(&mut self.writer, &dest_url)?;
                if !title.is_empty() {
                    self.write("\" title=\"")?;
                    escape_html(&mut self.writer, &title)?;
                }
                self.write("\">")
            }
            Tag::Link {
                link_type: _,
                dest_url,
                title,
                id: _,
            } => {
                self.write("<a href=\"")?;
                self.write_destination(&dest_url)?;
                if !title.is_empty() {
                    self.write("\" title=\"")?;
                    escape_html(&mut self.writer, &title)?;
                }
                self.write("\">")
            }
            Tag::Image {
                link_type: _,
                dest_url,
                title,
                id: _,
            } => {
                self.write("<img src=\"")?;
                self.write_destination(&dest_url)?;
                self.write("\" alt=\"")?;
                self.raw_text()?;
                if !title.is_empty() {
                    self.write("\" title=\"")?;
                    escape_html(&mut self.writer, &title)?;
                }
                // MBR EXTENSION: notes are often image-heavy; defer off-screen
                // requests and keep decoding off the main thread.
                self.write("\" loading=\"lazy\" decoding=\"async\" />")
            }
            Tag::FootnoteDefinition(name) => {
                if self.end_newline {
                    self.write("<div class=\"footnote-definition\" id=\"")?;
                } else {
                    self.write("\n<div class=\"footnote-definition\" id=\"")?;
                }
                escape_html(&mut self.writer, &name)?;
                self.write("\"><sup class=\"footnote-definition-label\">")?;
                let len = self.numbers.len() + 1;
                let number = *self.numbers.entry(name).or_insert(len);
                write!(&mut self.writer, "{}", number)?;
                self.write("</sup>")
            }
            Tag::MetadataBlock(_) => {
                self.in_non_writing_block = true;
                Ok(())
            }
        }
    }

    fn end_tag(&mut self, tag: TagEnd) -> Result<(), W::Error> {
        // MBR EXTENSION: Section wrapping - track open block containers.
        // `saturating_sub` keeps a malformed event stream from panicking.
        if is_block_container_end(&tag) {
            self.container_depth = self.container_depth.saturating_sub(1);
        }
        match tag {
            TagEnd::HtmlBlock => {}
            TagEnd::Paragraph => {
                self.write("</p>\n")?;
            }
            TagEnd::Heading(level) => {
                self.write("</")?;
                write!(&mut self.writer, "{}", level)?;
                self.write(">\n")?;
            }
            TagEnd::Table => {
                self.write("</tbody></table>\n")?;
            }
            TagEnd::TableHead => {
                self.write("</tr></thead><tbody>\n")?;
                self.table_state = TableState::Body;
            }
            TagEnd::TableRow => {
                self.write("</tr>\n")?;
            }
            TagEnd::TableCell => {
                match self.table_state {
                    TableState::Head => {
                        self.write("</th>")?;
                    }
                    TableState::Body => {
                        self.write("</td>")?;
                    }
                }
                self.table_cell_index += 1;
            }
            TagEnd::BlockQuote(_) => {
                self.write("</blockquote>\n")?;
            }
            TagEnd::CodeBlock => {
                match self.codeblock_state.take() {
                    Some(closing) => self.write(closing.as_ref())?,
                    None => self.write("</code></pre>")?,
                }
                self.write("\n")?;
            }
            TagEnd::List(true) => {
                self.write("</ol>\n")?;
            }
            TagEnd::List(false) => {
                self.write("</ul>\n")?;
            }
            TagEnd::Item => {
                self.write("</li>\n")?;
            }
            TagEnd::DefinitionList => {
                self.write("</dl>\n")?;
            }
            TagEnd::DefinitionListTitle => {
                self.write("</dt>\n")?;
            }
            TagEnd::DefinitionListDefinition => {
                self.write("</dd>\n")?;
            }
            TagEnd::Emphasis => {
                self.write("</em>")?;
            }
            TagEnd::Superscript => {
                self.write("</sup>")?;
            }
            TagEnd::Subscript => {
                self.write("</sub>")?;
            }
            TagEnd::Strong => {
                self.write("</strong>")?;
            }
            TagEnd::Strikethrough => {
                self.write("</del>")?;
            }
            TagEnd::Link => {
                self.write("</a>")?;
            }
            TagEnd::Image => (), // shouldn't happen, handled in start
            TagEnd::FootnoteDefinition => {
                self.write("</div>\n")?;
            }
            TagEnd::MetadataBlock(_) => {
                self.in_non_writing_block = false;
            }
        }
        Ok(())
    }

    // run raw text, consuming end tag
    fn raw_text(&mut self) -> Result<(), W::Error> {
        let mut nest = 0;
        // MBR EXTENSION: this is the second of the writer's two event loops, and
        // the reason `next_event` exists. Consuming an image's inner events
        // without counting them would shift every later `data-mbr-line` by the
        // size of the alt text.
        while let Some(event) = self.next_event() {
            match event {
                Start(_) => nest += 1,
                End(_) => {
                    if nest == 0 {
                        break;
                    }
                    nest -= 1;
                }
                Html(_) => {}
                InlineHtml(text) | Code(text) | Text(text) => {
                    // Don't use escape_html_body_text here.
                    // The output of this function is used in the `alt` attribute.
                    escape_html(&mut self.writer, &text)?;
                    self.end_newline = text.ends_with('\n');
                }
                InlineMath(text) => {
                    self.write("$")?;
                    escape_html(&mut self.writer, &text)?;
                    self.write("$")?;
                }
                DisplayMath(text) => {
                    self.write("$$")?;
                    escape_html(&mut self.writer, &text)?;
                    self.write("$$")?;
                }
                SoftBreak | HardBreak | Rule => {
                    self.write(" ")?;
                }
                FootnoteReference(name) => {
                    let len = self.numbers.len() + 1;
                    let number = *self.numbers.entry(name).or_insert(len);
                    write!(&mut self.writer, "[{}]", number)?;
                }
                TaskListMarker(true) => self.write("[x]")?,
                TaskListMarker(false) => self.write("[ ]")?,
            }
        }
        Ok(())
    }
}

/// Iterate over an `Iterator` of `Event`s, generate HTML for each `Event`, and
/// push it to a `String`.
///
/// # Examples
///
/// ```
/// use pulldown_cmark::{html, Parser};
///
/// let markdown_str = r#"
/// hello
/// =====
///
/// * alpha
/// * beta
/// "#;
/// let parser = Parser::new(markdown_str);
///
/// let mut html_buf = String::new();
/// html::push_html(&mut html_buf, parser);
///
/// assert_eq!(html_buf, r#"<h1>hello</h1>
/// <ul>
/// <li>alpha</li>
/// <li>beta</li>
/// </ul>
/// "#);
/// ```
pub fn push_html<'a, I>(s: &mut String, iter: I)
where
    I: Iterator<Item = Event<'a>>,
{
    write_html_fmt(s, iter).expect("writing to a String cannot fail")
}

/// Iterate over an `Iterator` of `Event`s, generate HTML for each `Event`, and
/// write it out to an I/O stream.
///
/// **Note**: using this function with an unbuffered writer like a file or socket
/// will result in poor performance. Wrap these in a
/// [`BufWriter`](https://doc.rust-lang.org/std/io/struct.BufWriter.html) to
/// prevent unnecessary slowdowns.
///
/// # Examples
///
/// ```
/// use pulldown_cmark::{html, Parser};
/// use std::io::Cursor;
///
/// let markdown_str = r#"
/// hello
/// =====
///
/// * alpha
/// * beta
/// "#;
/// let mut bytes = Vec::new();
/// let parser = Parser::new(markdown_str);
///
/// html::write_html_io(Cursor::new(&mut bytes), parser);
///
/// assert_eq!(&String::from_utf8_lossy(&bytes)[..], r#"<h1>hello</h1>
/// <ul>
/// <li>alpha</li>
/// <li>beta</li>
/// </ul>
/// "#);
/// ```
pub fn write_html_io<'a, I, W>(writer: W, iter: I) -> std::io::Result<()>
where
    I: Iterator<Item = Event<'a>>,
    W: std::io::Write,
{
    HtmlWriter::new(iter, IoWriter(writer)).run()
}

/// Iterate over an `Iterator` of `Event`s, generate HTML for each `Event`, and
/// write it into Unicode-accepting buffer or stream.
///
/// # Examples
///
/// ```
/// use pulldown_cmark::{html, Parser};
///
/// let markdown_str = r#"
/// hello
/// =====
///
/// * alpha
/// * beta
/// "#;
/// let mut buf = String::new();
/// let parser = Parser::new(markdown_str);
///
/// html::write_html_fmt(&mut buf, parser);
///
/// assert_eq!(buf, r#"<h1>hello</h1>
/// <ul>
/// <li>alpha</li>
/// <li>beta</li>
/// </ul>
/// "#);
/// ```
pub fn write_html_fmt<'a, I, W>(writer: W, iter: I) -> core::fmt::Result
where
    I: Iterator<Item = Event<'a>>,
    W: core::fmt::Write,
{
    HtmlWriter::new(iter, FmtWriter(writer)).run()
}

// ============================================================================
// MBR EXTENSION: Public API
// ============================================================================

/// Push HTML with MBR extensions enabled (sections and mermaid support).
///
/// This is the primary function for MBR's markdown rendering. It wraps content
/// in `<section>` tags with `<hr>` as dividers, and renders mermaid code blocks
/// as `<pre class="mermaid">`.
///
/// # Example
///
/// ```rust,ignore
/// use mbr::html::push_html_mbr;
/// use pulldown_cmark::Parser;
///
/// let markdown = "First section\n\n---\n\nSecond section";
/// let parser = Parser::new(markdown);
/// let mut html = String::new();
/// push_html_mbr(&mut html, parser);
///
/// // Output:
/// // <section>
/// // <p>First section</p>
/// // </section>
/// // <hr />
/// // <section>
/// // <p>Second section</p>
/// // </section>
/// ```
pub fn push_html_mbr<'a, I>(s: &mut String, iter: I)
where
    I: Iterator<Item = Event<'a>>,
{
    write_html_fmt_with_config(s, iter, HtmlConfig::mbr_defaults())
        .expect("writing to a String cannot fail")
}

/// Push HTML with MBR extensions and section attributes.
///
/// Like [`push_html_mbr`] but with pre-parsed section attributes. Attributes
/// are applied to `<section>` tags based on their index (0-based).
///
/// # Example
///
/// ```rust,ignore
/// use mbr::html::push_html_mbr_with_attrs;
/// use mbr::attrs::ParsedAttrs;
/// use pulldown_cmark::Parser;
/// use std::collections::HashMap;
///
/// let markdown = "First section\n\n---\n\nSecond section";
/// let parser = Parser::new(markdown);
///
/// let mut section_attrs = HashMap::new();
/// section_attrs.insert(1, ParsedAttrs::parse("{#intro .highlight}").unwrap());
///
/// let mut html = String::new();
/// push_html_mbr_with_attrs(&mut html, parser, section_attrs);
///
/// // Output includes: <section id="intro" class="highlight">
/// ```
pub fn push_html_mbr_with_attrs<'a, I>(
    s: &mut String,
    iter: I,
    section_attrs: HashMap<usize, ParsedAttrs>,
) where
    I: Iterator<Item = Event<'a>>,
{
    write_html_fmt_with_config(s, iter, HtmlConfig::mbr_with_section_attrs(section_attrs))
        .expect("writing to a String cannot fail")
}

/// Push HTML with explicit configuration.
///
/// Allows fine-grained control over which MBR extensions are enabled.
///
/// # Example
///
/// ```rust,ignore
/// use mbr::html::{push_html_with_config, HtmlConfig};
/// use pulldown_cmark::Parser;
///
/// let config = HtmlConfig {
///     enable_sections: true,
///     enable_mermaid: false,
/// };
///
/// let markdown = "Hello world";
/// let parser = Parser::new(markdown);
/// let mut html = String::new();
/// push_html_with_config(&mut html, parser, config);
/// ```
pub fn push_html_with_config<'a, I>(s: &mut String, iter: I, config: HtmlConfig)
where
    I: Iterator<Item = Event<'a>>,
{
    write_html_fmt_with_config(s, iter, config).expect("writing to a String cannot fail")
}

/// Internal: write HTML with explicit configuration.
fn write_html_fmt_with_config<'a, I, W>(writer: W, iter: I, config: HtmlConfig) -> core::fmt::Result
where
    I: Iterator<Item = Event<'a>>,
    W: core::fmt::Write,
{
    HtmlWriter::new_with_config(iter, FmtWriter(writer), config).run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulldown_cmark::{Options, Parser};

    /// Helper to render markdown with custom config
    fn render_with_config(markdown: &str, config: HtmlConfig) -> String {
        let parser = Parser::new(markdown);
        let mut html = String::new();
        push_html_with_config(&mut html, parser, config);
        html
    }

    // ---- task markup ---------------------------------------------------------

    #[test]
    fn task_checkbox_carries_its_line_when_one_is_known() {
        assert_eq!(
            task_checkbox_html(TaskStatus::Open, Some(42)),
            concat!(
                r#"<input type="checkbox" class="mbr-task-check" id="mbr-task-42" "#,
                r#"data-mbr-task-line="42" data-mbr-task-status="open" disabled>"#
            )
        );
        assert_eq!(
            task_checkbox_html(TaskStatus::Done, Some(1)),
            concat!(
                r#"<input type="checkbox" class="mbr-task-check" id="mbr-task-1" "#,
                r#"data-mbr-task-line="1" data-mbr-task-status="done" checked disabled>"#
            )
        );
    }

    #[test]
    fn task_checkbox_without_a_line_omits_the_anchor() {
        let html = task_checkbox_html(TaskStatus::Canceled, None);
        assert_eq!(
            html,
            r#"<input type="checkbox" class="mbr-task-check" data-mbr-task-status="canceled" disabled>"#
        );
        assert!(!html.contains("id="));
    }

    #[test]
    fn every_task_checkbox_is_inert() {
        for status in [TaskStatus::Open, TaskStatus::Done, TaskStatus::Canceled] {
            for line in [None, Some(7)] {
                assert!(
                    task_checkbox_html(status, line).ends_with(" disabled>"),
                    "{status:?} {line:?}"
                );
            }
        }
    }

    #[test]
    fn a_task_with_no_annotations_renders_no_chips() {
        assert_eq!(task_annotations_html(&Annotations::default()), "");
    }

    #[test]
    fn annotation_chips_render_in_a_fixed_order() {
        let annotations = Annotations {
            priority: TaskPriority::Urgent,
            tags: vec!["work".to_string(), "ops".to_string()],
            due: chrono::NaiveDate::from_ymd_opt(2026, 8, 5).and_then(|d| d.and_hms_opt(15, 0, 0)),
            due_has_time: true,
            done: chrono::NaiveDate::from_ymd_opt(2026, 8, 4).and_then(|d| d.and_hms_opt(0, 0, 0)),
            done_has_time: false,
            moved_to: chrono::NaiveDate::from_ymd_opt(2026, 9, 1),
        };
        assert_eq!(
            task_annotations_html(&annotations),
            concat!(
                r#" <span class="mbr-task-pri mbr-task-pri-urgent" role="img" "#,
                r#"aria-label="Urgent priority" title="Urgent priority"></span>"#,
                r#" <span class="mbr-task-tag">#work</span>"#,
                r#" <span class="mbr-task-tag">#ops</span>"#,
                r#" <time class="mbr-task-due" datetime="2026-08-05T15:00">Aug 5, 3:00 PM</time>"#,
                r#" <time class="mbr-task-completed" datetime="2026-08-04">Aug 4</time>"#,
                r#" <time class="mbr-task-moved" datetime="2026-09-01">Sep 1</time>"#
            )
        );
    }

    #[test]
    fn a_high_priority_dot_is_labelled_differently_from_an_urgent_one() {
        let high = task_annotations_html(&Annotations {
            priority: TaskPriority::High,
            ..Annotations::default()
        });
        assert!(high.contains("mbr-task-pri-high"), "{high}");
        assert!(high.contains(r#"aria-label="High priority""#), "{high}");
        assert!(!high.contains("mbr-task-pri-urgent"), "{high}");
    }

    /// The tag grammar cannot produce these today; the escape is there so a
    /// later widening of it does not silently become an injection.
    #[test]
    fn tag_chips_are_html_escaped() {
        let html = task_annotations_html(&Annotations {
            tags: vec!["<script>".to_string()],
            ..Annotations::default()
        });
        assert!(html.contains("&lt;script&gt;"), "{html}");
        assert!(!html.contains("<script>"), "{html}");
    }

    #[test]
    fn only_a_canceled_task_gets_the_canceled_text_class() {
        assert!(task_text_open(TaskStatus::Canceled).contains("mbr-task-canceled"));
        for status in [TaskStatus::Open, TaskStatus::Done] {
            assert_eq!(task_text_open(status), r#"<span class="mbr-task-text">"#);
        }
    }

    /// Renders with the same options the application uses (`Options::all()`),
    /// which is what enables tables, footnotes, task lists and math.
    fn render_mbr(markdown: &str) -> String {
        let parser = Parser::new_ext(markdown, Options::all());
        let mut html = String::new();
        push_html_mbr(&mut html, parser);
        html
    }

    /// Returns the slice strictly between the first `open` tag and the first
    /// `close` tag that follows it.
    ///
    /// Substring *counts* of `<section>`/`</section>` stay balanced even when the
    /// tags are mis-nested, so container nesting has to be inspected directly.
    fn between<'a>(html: &'a str, open: &str, close: &str) -> &'a str {
        let start = html
            .find(open)
            .unwrap_or_else(|| panic!("expected {open} in output:\n{html}"))
            + open.len();
        let end = html[start..]
            .find(close)
            .unwrap_or_else(|| panic!("expected {close} after {open} in output:\n{html}"))
            + start;
        &html[start..end]
    }

    #[test]
    fn test_sections_disabled() {
        // With enable_sections: false, no section wrappers should be added
        let config = HtmlConfig {
            enable_sections: false,
            enable_mermaid: false,
            section_attrs: HashMap::new(),
            block_lines: Vec::new(),
        };
        let html = render_with_config("Hello\n\n---\n\nWorld", config);

        // Should NOT have section tags
        assert!(
            !html.contains("<section"),
            "Sections disabled should not produce section tags. Got: {}",
            html
        );
        // Should still have hr
        assert!(
            html.contains("<hr />"),
            "Should still have hr divider. Got: {}",
            html
        );
    }

    #[test]
    fn test_sections_enabled_default() {
        // With mbr_defaults, sections should be enabled
        let config = HtmlConfig::mbr_defaults();
        let html = render_with_config("Hello\n\n---\n\nWorld", config);

        // Should have section tags
        assert!(
            html.contains("<section>"),
            "Sections enabled should produce section tags. Got: {}",
            html
        );
    }

    #[test]
    fn test_sections_with_attrs() {
        // Section attrs should be applied to the corresponding section
        let mut section_attrs = HashMap::new();
        section_attrs.insert(
            1,
            ParsedAttrs {
                id: Some("second".to_string()),
                classes: vec!["highlight".to_string()],
                attrs: vec![],
            },
        );
        let config = HtmlConfig::mbr_with_section_attrs(section_attrs);
        let html = render_with_config("First\n\n---\n\nSecond", config);

        // Second section should have the attrs
        assert!(
            html.contains(r#"id="second""#),
            "Section should have id. Got: {}",
            html
        );
        assert!(
            html.contains(r#"class="highlight""#),
            "Section should have class. Got: {}",
            html
        );
    }

    #[test]
    fn test_mermaid_disabled() {
        // With enable_mermaid: false, mermaid blocks render as normal code
        let config = HtmlConfig {
            enable_sections: false,
            enable_mermaid: false,
            section_attrs: HashMap::new(),
            block_lines: Vec::new(),
        };
        let html = render_with_config("```mermaid\ngraph TD\n```", config);

        // Should have standard code block structure
        assert!(
            html.contains("<pre><code"),
            "Mermaid disabled should use standard code. Got: {}",
            html
        );
    }

    #[test]
    fn test_mermaid_enabled() {
        // With enable_mermaid: true, mermaid blocks render as <pre class="mermaid">
        let config = HtmlConfig {
            enable_sections: false,
            enable_mermaid: true,
            section_attrs: HashMap::new(),
            block_lines: Vec::new(),
        };
        let html = render_with_config("```mermaid\ngraph TD\n```", config);

        // Should have mermaid-specific structure
        assert!(
            html.contains(r#"<pre class="mermaid">"#),
            "Mermaid enabled should use mermaid class. Got: {}",
            html
        );
        // Should NOT have <code> wrapper
        assert!(
            !html.contains("<code"),
            "Mermaid should not have code wrapper. Got: {}",
            html
        );
    }

    // ------------------------------------------------------------------
    // Section wrapping must not tear open block containers
    // ------------------------------------------------------------------

    #[test]
    fn test_rule_inside_blockquote_does_not_close_section() {
        let html = render_mbr("> before\n>\n> ---\n>\n> after");
        let inside = between(&html, "<blockquote>", "</blockquote>");

        assert!(
            !inside.contains("</section>"),
            "A rule inside a blockquote must not close the surrounding section \
             (the browser would hoist everything after it out of the quote). Got: {html}"
        );
        assert!(
            !inside.contains("<section"),
            "A rule inside a blockquote must not open a section. Got: {html}"
        );
        assert!(
            inside.contains("<hr />"),
            "The rule should still render as a plain <hr />. Got: {html}"
        );
        assert!(
            inside.contains("after"),
            "Content after the rule must stay inside the blockquote. Got: {html}"
        );
    }

    #[test]
    fn test_rule_inside_list_item_does_not_close_section() {
        let html = render_mbr("- a\n\n  ---\n\n- b");
        let inside = between(&html, "<li>", "</li>");

        assert!(
            !inside.contains("</section>"),
            "A rule inside a list item must not close the surrounding section. Got: {html}"
        );
        assert!(
            !inside.contains("<section"),
            "A rule inside a list item must not open a section. Got: {html}"
        );
        assert!(
            inside.contains("<hr />"),
            "The rule should still render as a plain <hr />. Got: {html}"
        );
        // The <ul> must also survive intact.
        let list = between(&html, "<ul>", "</ul>");
        assert!(
            !list.contains("section"),
            "The list must not be split by a section boundary. Got: {html}"
        );
    }

    #[test]
    fn test_rule_inside_definition_list_does_not_close_section() {
        let html = render_mbr("Term\n\n: def\n\n  ---\n\n  more\n");
        let inside = between(&html, "<dd>", "</dd>");

        assert!(
            !inside.contains("</section>"),
            "A rule inside a definition must not close the surrounding section. Got: {html}"
        );
        assert!(
            inside.contains("more"),
            "Content after the rule must stay inside the definition. Got: {html}"
        );
    }

    /// `templates/theme.css` renders definition lists as an FAQ-style
    /// disclosure list keyed off `dt:focus`, and only a focusable element can
    /// match `:focus`. Without `tabindex` on every `<dt>` there is no pure-CSS
    /// way to open a `<dd>`, so the answers would be permanently hidden.
    #[test]
    fn test_definition_list_titles_are_focusable() {
        let html = render_mbr("Tight\n: answer\n\nLoose\n\n: first para\n\n  second para\n");

        assert!(
            html.contains("<dl>"),
            "definition lists must still open a <dl>. Got: {html}"
        );
        assert!(
            html.contains("<dd>"),
            "definitions must still render as <dd>. Got: {html}"
        );
        assert_eq!(
            html.matches("<dt tabindex=\"0\">").count(),
            2,
            "every <dt> must be focusable so the CSS disclosure can open its \
             <dd>. Got: {html}"
        );
        assert!(
            !html.contains("<dt>"),
            "no <dt> may be emitted without tabindex -- its answer would be \
             unreachable. Got: {html}"
        );
    }

    /// The two `<dd>` body shapes the disclosure CSS has to collapse. A tight
    /// definition holds bare inline text; a loose one gets its body wrapped in
    /// `<p>`, whose margins would escape a zero-height box if the CSS did not
    /// establish a block formatting context. Pinned here so a parser upgrade
    /// that changes either shape shows up as a test failure rather than as a
    /// silently broken collapse.
    #[test]
    fn test_definition_bodies_are_tight_or_paragraph_wrapped() {
        let tight = render_mbr("Term\n: answer\n");
        assert!(
            tight.contains("<dd>answer</dd>"),
            "a tight definition should hold bare inline text. Got: {tight}"
        );

        let loose = render_mbr("Term\n\n: answer\n");
        assert!(
            loose.contains("<dd>\n<p>answer</p>\n</dd>"),
            "a loose definition should wrap its body in <p>. Got: {loose}"
        );
    }

    /// One question can own several answers. The CSS opens them with `dt:focus
    /// ~ dd` rather than `+ dd` precisely because of this shape -- with the
    /// adjacent combinator the second `<dd>` would stay `visibility: hidden`,
    /// which also makes it untabbable, so its content would be unreachable.
    #[test]
    fn test_one_title_can_own_several_definitions() {
        let html = render_mbr("Term\n: first\n: second\n");

        assert_eq!(
            html.matches("<dt tabindex=\"0\">").count(),
            1,
            "expected a single term. Got: {html}"
        );
        assert_eq!(
            html.matches("<dd>").count(),
            2,
            "expected the term to own two sibling <dd>s. Got: {html}"
        );
    }

    #[test]
    fn test_top_level_rule_still_splits_sections() {
        let html = render_mbr("Hello\n\n---\n\nWorld");

        assert!(
            html.contains("</section>\n<hr />\n<section>"),
            "A top-level rule must still split sections. Got: {html}"
        );
    }

    #[test]
    fn test_nested_rule_does_not_shift_section_attr_indices() {
        // markdown.rs numbers sections by counting every Event::Rule, nested or
        // not, so html.rs must advance its index for the nested rule too or the
        // attrs land on the wrong section.
        let mut section_attrs = HashMap::new();
        section_attrs.insert(
            2,
            ParsedAttrs {
                id: Some("third".to_string()),
                classes: vec![],
                attrs: vec![],
            },
        );
        let config = HtmlConfig::mbr_with_section_attrs(section_attrs);
        let parser = Parser::new_ext("> ---\n\n---\n\ntail", Options::all());
        let mut html = String::new();
        push_html_with_config(&mut html, parser, config);

        assert!(
            html.contains(r#"<section id="third">"#),
            "Section index must count the nested rule. Got: {html}"
        );
    }

    // ------------------------------------------------------------------
    // Script-capable destinations are neutralized
    // ------------------------------------------------------------------

    #[test]
    fn test_javascript_link_destination_is_neutralized() {
        let html = render_mbr("[click me](javascript:alert(1))");

        assert!(
            !html.contains("javascript:"),
            "javascript: href must not survive. Got: {html}"
        );
        assert!(
            html.contains(r#"<a href="about:blank#blocked">"#),
            "Blocked link should get an inert href. Got: {html}"
        );
        assert!(
            html.contains("click me"),
            "Link text must be preserved. Got: {html}"
        );
    }

    #[test]
    fn test_javascript_scheme_obfuscation_is_neutralized() {
        // Browsers strip leading control characters/spaces and every embedded
        // tab/newline/CR before resolving the URL, so these all still navigate.
        for dest in [
            "JaVaScRiPt:alert(1)",
            "  javascript:alert(1)",
            "java\tscript:alert(1)",
            "\u{1}javascript:alert(1)",
        ] {
            let html = render_mbr(&format!("[x](<{dest}>)"));
            assert!(
                html.contains(r#"href="about:blank#blocked""#),
                "{dest:?} should have been neutralized. Got: {html}"
            );
        }
    }

    #[test]
    fn test_vbscript_link_destination_is_neutralized() {
        let html = render_mbr("[x](vbscript:msgbox(1))");
        assert!(
            html.contains(r#"href="about:blank#blocked""#),
            "vbscript: href must be neutralized. Got: {html}"
        );
    }

    #[test]
    fn test_non_image_data_url_is_neutralized() {
        let html = render_mbr("[x](data:text/html;base64,PHNjcmlwdD4=)");
        assert!(
            html.contains(r#"href="about:blank#blocked""#),
            "data:text/html href must be neutralized. Got: {html}"
        );
    }

    #[test]
    fn test_inline_image_data_url_is_preserved() {
        // link_transform.rs deliberately passes data: through for inline images,
        // so data:image/* must keep working in both links and images.
        let html = render_mbr("![alt](data:image/png;base64,iVBORw0KGgo=)");
        assert!(
            html.contains(r#"src="data:image/png;base64,iVBORw0KGgo=""#),
            "data:image/* src must be preserved. Got: {html}"
        );

        let html = render_mbr("[x](data:image/png;base64,iVBORw0KGgo=)");
        assert!(
            html.contains(r#"href="data:image/png;base64,iVBORw0KGgo=""#),
            "data:image/* href must be preserved. Got: {html}"
        );
    }

    #[test]
    fn test_javascript_image_destination_is_neutralized() {
        let html = render_mbr("![alt](javascript:alert(1))");
        assert!(
            !html.contains("javascript:"),
            "javascript: src must not survive. Got: {html}"
        );
        assert!(
            html.contains(r#"<img src="about:blank#blocked""#),
            "Blocked image should get an inert src. Got: {html}"
        );
    }

    #[test]
    fn test_ordinary_destinations_are_untouched() {
        let cases = [
            (
                "[a](https://example.com/p?q=1)",
                "https://example.com/p?q=1",
            ),
            ("[a](http://example.com/)", "http://example.com/"),
            ("[a](../notes/other/)", "../notes/other/"),
            ("[a](/images/pic.png)", "/images/pic.png"),
            ("[a](#anchor)", "#anchor"),
            ("[a](mailto:me@example.com)", "mailto:me@example.com"),
            ("[a](tel:+15551234)", "tel:+15551234"),
            // A relative file that merely looks like a scheme name.
            ("[a](javascript)", "javascript"),
        ];
        for (markdown, expected_href) in cases {
            let html = render_mbr(markdown);
            assert!(
                html.contains(&format!(r#"href="{expected_href}""#)),
                "{markdown} should render href={expected_href:?}. Got: {html}"
            );
        }
    }

    #[test]
    fn test_autolink_email_is_untouched() {
        let html = render_mbr("<me@example.com>");
        assert!(
            html.contains(r#"href="mailto:me@example.com""#),
            "Email autolinks must keep working. Got: {html}"
        );
    }

    // ------------------------------------------------------------------
    // Image loading hints
    // ------------------------------------------------------------------

    #[test]
    fn test_images_get_lazy_loading_and_async_decoding() {
        let html = render_mbr("![alt text](/images/pic.png)");

        assert!(
            html.contains(r#"loading="lazy""#),
            "Images should be lazily loaded. Got: {html}"
        );
        assert!(
            html.contains(r#"decoding="async""#),
            "Images should decode asynchronously. Got: {html}"
        );
    }

    #[test]
    fn test_image_with_title_keeps_loading_hints() {
        let html = render_mbr(r#"![alt](/images/pic.png "A title")"#);

        assert!(
            html.contains(r#"title="A title" loading="lazy" decoding="async" />"#),
            "Title and loading hints should both be emitted. Got: {html}"
        );
    }

    // ------------------------------------------------------------------
    // Scheme normalization unit coverage
    // ------------------------------------------------------------------

    #[test]
    fn test_is_blocked_destination() {
        for dest in [
            "javascript:alert(1)",
            "JAVASCRIPT:alert(1)",
            " \u{c}javascript:alert(1)",
            "v\rbscript:x", // still `vbscript:` once CRs are removed
            "vbscript:x",
            "data:text/html,<script>",
            "data:,plain",
            "data:image",     // no slash: not `image/`
            "data:imag",      // truncated payload
            "javascript:",    // empty body still navigates
            "jAvAsCrIpT\n:x", // newline before the colon
        ] {
            assert!(is_blocked_destination(dest), "{dest:?} should be blocked");
        }

        for dest in [
            "",
            ":",
            "https://example.com",
            "HTTPS://example.com",
            "mailto:me@example.com",
            "../a/b",
            "/a/b",
            "#frag",
            "?q=1",
            "javascript",            // no colon: a relative path
            "a javascript:alert(1)", // space is not stripped mid-URL
            "not-javascript:x",
            "data:image/png;base64,AAAA",
            "DATA:IMAGE/PNG;base64,AAAA",
            "data:image/svg+xml,%3Csvg%3E",
            "blob:https://example.com/uuid",
            "verylongschemename:x",
        ] {
            assert!(
                !is_blocked_destination(dest),
                "{dest:?} should not be blocked"
            );
        }
    }

    // ---- data-mbr-line (review anchors) --------------------------------------

    /// Renders `events` with a `block_lines` table built from `(at, line)` pairs.
    fn render_with_block_lines(events: Vec<Event<'static>>, pairs: &[(u32, u32)]) -> String {
        let block_lines = pairs
            .iter()
            .map(|&(at, line)| BlockLine { at, line })
            .collect();
        let config = HtmlConfig {
            enable_sections: false,
            enable_mermaid: true,
            section_attrs: HashMap::new(),
            block_lines,
        };
        let mut html = String::new();
        push_html_with_config(&mut html, events.into_iter(), config);
        html
    }

    /// Parses `markdown` and renders it with a table built by finding the
    /// indices of the events matching `pick`.
    fn render_markdown_with_block_lines(
        markdown: &str,
        pick: impl Fn(&Event<'_>) -> bool,
    ) -> String {
        let events: Vec<Event<'static>> = Parser::new_ext(markdown, Options::all())
            .map(|e| owned_event(&e))
            .collect();
        let pairs: Vec<(u32, u32)> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| pick(e))
            .map(|(i, _)| (i as u32, (i as u32) + 1))
            .collect();
        render_with_block_lines(events, &pairs)
    }

    /// A `'static` clone of `event`, so test tables can outlive the parser.
    fn owned_event(event: &Event<'_>) -> Event<'static> {
        // Round-tripping through the writer is not needed; only the few shapes
        // these tests use have to survive, and `CowStr::from(String)` is owned.
        match event {
            Start(tag) => Start(owned_tag(tag)),
            End(tag) => End(*tag),
            Text(t) => Text(CowStr::from(t.to_string())),
            Code(t) => Code(CowStr::from(t.to_string())),
            Html(t) => Html(CowStr::from(t.to_string())),
            InlineHtml(t) => InlineHtml(CowStr::from(t.to_string())),
            InlineMath(t) => InlineMath(CowStr::from(t.to_string())),
            DisplayMath(t) => DisplayMath(CowStr::from(t.to_string())),
            FootnoteReference(t) => FootnoteReference(CowStr::from(t.to_string())),
            SoftBreak => SoftBreak,
            HardBreak => HardBreak,
            Rule => Rule,
            TaskListMarker(b) => TaskListMarker(*b),
        }
    }

    fn owned_tag(tag: &Tag<'_>) -> Tag<'static> {
        match tag {
            Tag::Paragraph => Tag::Paragraph,
            Tag::Heading {
                level,
                id,
                classes,
                attrs,
            } => Tag::Heading {
                level: *level,
                id: id.as_ref().map(|i| CowStr::from(i.to_string())),
                classes: classes
                    .iter()
                    .map(|c| CowStr::from(c.to_string()))
                    .collect(),
                attrs: attrs
                    .iter()
                    .map(|(k, v)| {
                        (
                            CowStr::from(k.to_string()),
                            v.as_ref().map(|v| CowStr::from(v.to_string())),
                        )
                    })
                    .collect(),
            },
            Tag::BlockQuote(kind) => Tag::BlockQuote(*kind),
            Tag::CodeBlock(CodeBlockKind::Fenced(info)) => {
                Tag::CodeBlock(CodeBlockKind::Fenced(CowStr::from(info.to_string())))
            }
            Tag::CodeBlock(CodeBlockKind::Indented) => Tag::CodeBlock(CodeBlockKind::Indented),
            Tag::List(start) => Tag::List(*start),
            Tag::Item => Tag::Item,
            Tag::Table(a) => Tag::Table(a.clone()),
            Tag::TableHead => Tag::TableHead,
            Tag::TableRow => Tag::TableRow,
            Tag::TableCell => Tag::TableCell,
            Tag::Emphasis => Tag::Emphasis,
            Tag::Strong => Tag::Strong,
            Tag::Strikethrough => Tag::Strikethrough,
            Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            } => Tag::Image {
                link_type: *link_type,
                dest_url: CowStr::from(dest_url.to_string()),
                title: CowStr::from(title.to_string()),
                id: CowStr::from(id.to_string()),
            },
            Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            } => Tag::Link {
                link_type: *link_type,
                dest_url: CowStr::from(dest_url.to_string()),
                title: CowStr::from(title.to_string()),
                id: CowStr::from(id.to_string()),
            },
            Tag::HtmlBlock => Tag::HtmlBlock,
            other => panic!("unhandled tag in test helper: {other:?}"),
        }
    }

    /// Each of the six tags carries the attribute, from a hand-built table.
    #[test]
    fn block_line_attribute_on_each_tag() {
        let events: Vec<Event<'static>> = vec![
            Start(Tag::Paragraph),       // 0
            Text(CowStr::Borrowed("p")), // 1
            End(TagEnd::Paragraph),      // 2
            Start(Tag::Heading {
                // 3
                level: pulldown_cmark::HeadingLevel::H2,
                id: None,
                classes: vec![],
                attrs: vec![],
            }),
            Text(CowStr::Borrowed("h")),                            // 4
            End(TagEnd::Heading(pulldown_cmark::HeadingLevel::H2)), // 5
            Start(Tag::List(None)),                                 // 6
            Start(Tag::Item),                                       // 7
            Text(CowStr::Borrowed("li")),                           // 8
            End(TagEnd::Item),                                      // 9
            End(TagEnd::List(false)),                               // 10
            Start(Tag::BlockQuote(None)),                           // 11
            End(TagEnd::BlockQuote(None)),                          // 12
            Start(Tag::CodeBlock(CodeBlockKind::Fenced(CowStr::Borrowed("")))), // 13
            End(TagEnd::CodeBlock),                                 // 14
            Start(Tag::Table(vec![Alignment::None])),               // 15
            Start(Tag::TableHead),                                  // 16
            Start(Tag::TableCell),                                  // 17
            End(TagEnd::TableCell),                                 // 18
            End(TagEnd::TableHead),                                 // 19
            End(TagEnd::Table),                                     // 20
        ];
        // A distinct line per block, so a mix-up cannot pass.
        let html = render_with_block_lines(
            events,
            &[
                (0, 11),  // <p>
                (3, 22),  // <h2>
                (6, 33),  // Tag::List — recorded here on purpose; see below
                (7, 44),  // <li>
                (11, 55), // <blockquote>
                (13, 66), // <pre>
                (15, 77), // <table>
                (17, 88), // Tag::TableCell — likewise
            ],
        );

        assert!(html.contains(r#"<p data-mbr-line="11">"#), "{html}");
        assert!(html.contains(r#"<h2 data-mbr-line="22">"#), "{html}");
        assert!(html.contains(r#"<li data-mbr-line="44">"#), "{html}");
        assert!(
            html.contains(r#"<blockquote data-mbr-line="55">"#),
            "{html}"
        );
        assert!(html.contains(r#"<pre data-mbr-line="66"><code>"#), "{html}");
        assert!(html.contains(r#"<table data-mbr-line="77">"#), "{html}");

        // The excluded tags stay bare even when the table names them, which is
        // what keeps `markdown::is_review_block_start` the single definition of
        // the set: an entry it never records cannot leak through the writer.
        assert!(html.contains("<ul>"), "no attribute on <ul>: {html}");
        assert!(html.contains("<th>"), "no attribute on <th>: {html}");
        assert!(!html.contains(r#""33""#), "{html}");
        assert!(!html.contains(r#""88""#), "{html}");
    }

    /// GOTCHA: `raw_text` is the writer's *second* event loop. If it consumed
    /// events without counting them, every `data-mbr-line` after the first image
    /// on the page would name the wrong line. Route `raw_text` back through
    /// `self.iter.next()` instead of `next_event` and this test fails.
    #[test]
    fn block_line_index_survives_image_alt_text() {
        // The alt text carries inline markup, so `raw_text` eats several events
        // (`Start(Emphasis)`, `Text`, `End(Emphasis)`, …) before the image's
        // `End`. Every one of them has to advance the counter.
        let markdown =
            "![alt *with* `markup` and **more**](img.png)\n\nSecond paragraph.\n\nThird one.\n";
        let events: Vec<Event<'static>> = Parser::new_ext(markdown, Options::all())
            .map(|e| owned_event(&e))
            .collect();

        // Find the two later paragraph starts and give them recognisable lines.
        let paragraph_starts: Vec<usize> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e, Start(Tag::Paragraph)))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            paragraph_starts.len(),
            3,
            "the image paragraph plus two more: {events:?}"
        );
        let pairs = [
            (paragraph_starts[0] as u32, 1),
            (paragraph_starts[1] as u32, 3),
            (paragraph_starts[2] as u32, 5),
        ];

        let html = render_with_block_lines(events, &pairs);
        assert!(
            html.contains(r#"<p data-mbr-line="3">Second paragraph.</p>"#),
            "the paragraph after the image must keep its own line: {html}"
        );
        assert!(
            html.contains(r#"<p data-mbr-line="5">Third one.</p>"#),
            "and so must the one after that: {html}"
        );
    }

    #[test]
    fn empty_block_lines_emits_no_attribute() {
        let html = render_with_block_lines(
            vec![
                Start(Tag::Paragraph),
                Text(CowStr::Borrowed("p")),
                End(TagEnd::Paragraph),
            ],
            &[],
        );
        assert_eq!(html, "<p>p</p>\n");
        assert!(!html.contains("data-mbr-line"));
    }

    /// An HTML parser keeps the *first* of a pair of duplicate attributes, so
    /// ours must precede any the author wrote.
    #[test]
    fn our_heading_line_precedes_author_attrs() {
        let html =
            render_markdown_with_block_lines("## Title {#custom .cls data-mbr-line=99}\n", |e| {
                matches!(e, Start(Tag::Heading { .. }))
            });
        let ours = html.find(r#"data-mbr-line="1""#).expect(&html);
        let theirs = html.find(r#"data-mbr-line="99""#).expect(&html);
        assert!(
            ours < theirs,
            "ours must come first so a parser prefers it: {html}"
        );
        // And it must not have displaced the author's id or classes.
        assert!(html.contains(r#"id="custom""#), "{html}");
        assert!(html.contains(r#"class="cls""#), "{html}");
    }

    #[test]
    fn mermaid_pre_carries_block_line() {
        let html = render_markdown_with_block_lines("```mermaid\ngraph TD\n```\n", |e| {
            matches!(e, Start(Tag::CodeBlock(_)))
        });
        assert!(
            html.contains(r#"<pre class="mermaid" data-mbr-line="1">"#),
            "{html}"
        );
        assert!(
            !html.contains("<code"),
            "mermaid keeps its bare <pre>: {html}"
        );
    }

    #[test]
    fn fenced_lang_pre_carries_block_line() {
        let html = render_markdown_with_block_lines("```rust\nfn main() {}\n```\n", |e| {
            matches!(e, Start(Tag::CodeBlock(_)))
        });
        assert!(
            html.contains(r#"<pre data-mbr-line="1"><code class="language-rust">"#),
            "{html}"
        );
    }
}
