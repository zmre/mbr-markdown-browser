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
//! - Added `codeblock_state` field for mermaid closing tag handling
//! - Removed `ContainerBlock` handling (not used in MBR)

use std::collections::HashMap;

use crate::attrs::ParsedAttrs;
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
    /// - Each `---` (Rule) becomes `</section>\n<hr />\n<section>\n`
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
}

impl HtmlConfig {
    /// Standard MBR configuration with all extensions enabled and no section attrs.
    pub fn mbr_defaults() -> Self {
        Self {
            enable_sections: true,
            enable_mermaid: true,
            section_attrs: HashMap::new(),
        }
    }

    /// Standard MBR configuration with section attributes.
    pub fn mbr_with_section_attrs(section_attrs: HashMap<usize, ParsedAttrs>) -> Self {
        Self {
            enable_sections: true,
            enable_mermaid: true,
            section_attrs,
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

        while let Some(event) = self.iter.next() {
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
                    if self.config.enable_sections {
                        // Increment section index before opening the next section
                        self.current_section += 1;
                        let attrs_str = self
                            .config
                            .section_attrs
                            .get(&self.current_section)
                            .map(|a| a.to_html_attr_string())
                            .unwrap_or_default();
                        self.write(&format!("</section>\n<hr />\n<section{}>\n", attrs_str))?;
                    } else {
                        // Standard pulldown-cmark behavior
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
                TaskListMarker(true) => {
                    self.write("<input disabled=\"\" type=\"checkbox\" checked=\"\"/>\n")?;
                }
                TaskListMarker(false) => {
                    self.write("<input disabled=\"\" type=\"checkbox\"/>\n")?;
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
        match tag {
            Tag::HtmlBlock => Ok(()),
            Tag::Paragraph => {
                if self.end_newline {
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
            Tag::Table(alignments) => {
                self.table_alignments = alignments;
                self.write("<table>")
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
                if self.end_newline {
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
                match info {
                    CodeBlockKind::Fenced(info) => {
                        let lang = info.split(' ').next().unwrap_or_default();
                        if lang.is_empty() {
                            self.write("<pre><code>")
                        // MBR EXTENSION: Mermaid diagram support
                        } else if self.config.enable_mermaid && lang == "mermaid" {
                            self.codeblock_state = Some("</pre>".into());
                            self.write("<pre class=\"mermaid\">")
                        } else {
                            self.write("<pre><code class=\"language-")?;
                            escape_html(&mut self.writer, lang)?;
                            self.write("\">")
                        }
                    }
                    CodeBlockKind::Indented => self.write("<pre><code>"),
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
            Tag::Item => {
                if self.end_newline {
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
                if self.end_newline {
                    self.write("<dt>")
                } else {
                    self.write("\n<dt>")
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
                escape_href(&mut self.writer, &dest_url)?;
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
                escape_href(&mut self.writer, &dest_url)?;
                self.write("\" alt=\"")?;
                self.raw_text()?;
                if !title.is_empty() {
                    self.write("\" title=\"")?;
                    escape_html(&mut self.writer, &title)?;
                }
                self.write("\" />")
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
        while let Some(event) = self.iter.next() {
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
    write_html_fmt(s, iter).unwrap()
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
    write_html_fmt_with_config(s, iter, HtmlConfig::mbr_defaults()).unwrap()
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
    write_html_fmt_with_config(s, iter, HtmlConfig::mbr_with_section_attrs(section_attrs)).unwrap()
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
    write_html_fmt_with_config(s, iter, config).unwrap()
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
    use pulldown_cmark::Parser;

    /// Helper to render markdown with custom config
    fn render_with_config(markdown: &str, config: HtmlConfig) -> String {
        let parser = Parser::new(markdown);
        let mut html = String::new();
        push_html_with_config(&mut html, parser, config);
        html
    }

    #[test]
    fn test_sections_disabled() {
        // With enable_sections: false, no section wrappers should be added
        let config = HtmlConfig {
            enable_sections: false,
            enable_mermaid: false,
            section_attrs: HashMap::new(),
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
}
