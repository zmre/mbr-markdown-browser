//! What a render does with HTML the author typed into a note.
//!
//! Markdown lets a note carry raw HTML (`<details>`, `<kbd>`, a README's
//! `<p align="center">`), and by default mbr writes it out verbatim
//! ([`RawHtmlPolicy::Passthrough`]): the note's author is trusted like the
//! person serving it. An app that serves notes written by someone else on the
//! same origin as its own pages (a shared customer note, a synced contract)
//! must not let a `<script>` in one run with the app's privileges, so it can
//! choose [`RawHtmlPolicy::Sanitize`] or [`RawHtmlPolicy::Escape`].
//!
//! # Only the author's HTML
//!
//! The policy applies to the events the markdown **parser** produces
//! (`Event::Html`/`Event::InlineHtml`, and the attributes of a heading's
//! `{…}` block), before mbr's own passes run. mbr's own markup — media embeds,
//! sections, task checkboxes, wikilinks, chat bubbles — is injected as
//! `Event::Html` by those later passes, so it never reaches the sanitizer and
//! cannot be damaged by it. `--- {attrs}` section attributes are author
//! attributes too; the renderer filters them with [`is_safe_author_attribute`].
//!
//! # Why not sanitize each fragment with ammonia directly
//!
//! pulldown-cmark hands raw HTML over in pieces: an inline `<kbd>Ctrl</kbd>`
//! is three events, and a `<details>` block often closes in a later block,
//! after markdown that belongs inside it. ammonia parses what it is given as a
//! whole fragment with the HTML5 tree builder, which closes every open element
//! (`<kbd>` becomes `<kbd></kbd>`) and drops table parts out of context. So a
//! fragment is split into tokens with html5ever's tokenizer (the parser ammonia
//! itself uses) and each **start tag** is sanitized by ammonia on its own, in a
//! context where the tag is legal; end tags are kept only for allowed elements
//! and text is re-escaped. The browser then balances the result exactly as it
//! would have balanced the original.
//!
//! The output contains only allowed elements, none of which is a raw-text
//! element (`script`, `style`, `textarea`, `title`, `iframe`, …), so whatever
//! follows it is parsed as ordinary markup.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::sync::LazyLock;

use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use pulldown_cmark::{CowStr, Event, Tag as MdTag, TagEnd};
use serde::{Deserialize, Serialize};

/// What to do with raw HTML written in a note.
///
/// Ordered from most to least permissive, so [`RawHtmlPolicy::strictest`]
/// (`max`) combines two settings without weakening either.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum RawHtmlPolicy {
    /// Written verbatim (mbr's behaviour, and the default).
    #[default]
    Passthrough,
    /// Allowed formatting elements and attributes are kept; scripts, event
    /// handlers, `style`, frames, forms and script URLs are removed.
    Sanitize,
    /// Shown as text, the way a code span shows it. HTML comments stay hidden.
    Escape,
}

impl RawHtmlPolicy {
    /// The stricter of two policies: what an embedding app uses to make sure a
    /// repository's own `config.toml` can raise the policy but never lower it.
    ///
    /// ```
    /// use mbr_core::raw_html::RawHtmlPolicy;
    /// let from_repo = RawHtmlPolicy::Passthrough;
    /// assert_eq!(from_repo.strictest(RawHtmlPolicy::Sanitize), RawHtmlPolicy::Sanitize);
    /// assert_eq!(RawHtmlPolicy::Escape.strictest(RawHtmlPolicy::Sanitize), RawHtmlPolicy::Escape);
    /// ```
    #[must_use]
    pub fn strictest(self, other: Self) -> Self {
        self.max(other)
    }
}

/// Elements whose content is never markup in a browser. Their content is
/// dropped together with the element: kept as text it would be a script's
/// source shown on the page.
fn raw_text_kind(name: &str) -> Option<TokenSinkResult<()>> {
    match name {
        "script" => Some(TokenSinkResult::RawData(RawKind::ScriptData)),
        "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => {
            Some(TokenSinkResult::RawData(RawKind::Rawtext))
        }
        "textarea" | "title" => Some(TokenSinkResult::RawData(RawKind::Rcdata)),
        "plaintext" => Some(TokenSinkResult::Plaintext),
        _ => None,
    }
}

/// `id` values that could shadow a global mbr's scripts read through the
/// window's named-property access (`<div id="__MBR_EXPORT__">` would make every
/// page think it is being exported). Matched exactly; the prefixes below cover
/// the rest of mbr's names.
const RESERVED_IDS: &[&str] = &[
    "frontmatter",
    "headings",
    "extendedMeta",
    "mermaid",
    "hljs",
    "katex",
    "Reveal",
    "ipc",
];

fn is_reserved_id(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    lower.starts_with("__") || lower.starts_with("mbr") || RESERVED_IDS.contains(&id)
}

/// The ammonia policy for one start tag: its defaults (formatting, lists,
/// tables, links, images; `http`/`https`/`mailto`/… URLs, relative URLs kept
/// for mbr's link rewriting) plus picture/video/audio, `class`, `id`, `dir`
/// and `align`.
static SANITIZER: LazyLock<ammonia::Builder<'static>> = LazyLock::new(|| {
    let mut builder = ammonia::Builder::default();
    builder
        .add_tags(["picture", "source", "video", "audio"])
        .add_generic_attributes(["class", "id", "dir", "align"])
        .add_tag_attributes("details", ["open"])
        .add_tag_attributes("img", ["srcset", "sizes"])
        .add_tag_attributes("source", ["src", "srcset", "sizes", "type", "media"])
        .add_tag_attributes(
            "video",
            [
                "src",
                "poster",
                "controls",
                "width",
                "height",
                "loop",
                "muted",
                "playsinline",
                "preload",
            ],
        )
        .add_tag_attributes("audio", ["src", "controls", "loop", "muted", "preload"])
        .attribute_filter(|_element, attribute, value| {
            if attribute == "id" && is_reserved_id(value) {
                None
            } else {
                Some(value.into())
            }
        });
    builder
});

/// The elements [`SANITIZER`] keeps, for deciding which end tags survive.
static ALLOWED_TAGS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| SANITIZER.clone_tags());

/// Markup that makes `name` legal where the HTML5 tree builder meets it, so
/// ammonia's parser keeps the tag instead of dropping it as misplaced.
fn parse_context(name: &str) -> &'static str {
    match name {
        "tr" => "<table><tbody>",
        "td" | "th" => "<table><tbody><tr>",
        "thead" | "tbody" | "tfoot" | "caption" | "colgroup" => "<table>",
        "col" => "<table><colgroup>",
        "rp" | "rt" | "rtc" => "<ruby>",
        _ => "",
    }
}

/// Collects tokens, switching the tokenizer into the raw-text states a
/// browser would use, so `<script>if (a<b)</script>` is one text token.
#[derive(Default)]
struct Collect {
    tokens: RefCell<Vec<Token>>,
}

impl TokenSink for Collect {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<()> {
        let switch = match &token {
            Token::TagToken(tag) if tag.kind == TagKind::StartTag => raw_text_kind(&tag.name),
            _ => None,
        };
        self.tokens.borrow_mut().push(token);
        switch.unwrap_or(TokenSinkResult::Continue)
    }
}

fn tokenize(html: &str) -> Vec<Token> {
    let input = BufferQueue::default();
    input.push_back(html.into());
    let tokenizer = Tokenizer::new(Collect::default(), TokenizerOpts::default());
    // `Script` is never returned by `Collect`, so one feed consumes it all.
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.tokens.take()
}

fn push_escaped(out: &mut String, text: &str, in_attribute: bool) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if in_attribute => out.push_str("&quot;"),
            '\u{0}' => {}
            _ => out.push(c),
        }
    }
}

fn serialize_start_tag(tag: &Tag) -> String {
    let mut out = String::with_capacity(16);
    out.push('<');
    out.push_str(&tag.name);
    for attribute in &tag.attrs {
        out.push(' ');
        out.push_str(&attribute.name.local);
        out.push_str("=\"");
        push_escaped(&mut out, &attribute.value, true);
        out.push('"');
    }
    out.push('>');
    out
}

/// One start tag through ammonia: `None` when the element is not allowed,
/// else the tag with only the attributes (and URLs) ammonia kept.
fn sanitize_start_tag(tag: &Tag) -> Option<String> {
    if !ALLOWED_TAGS.contains(&*tag.name) {
        return None;
    }
    let probe = format!("{}{}", parse_context(&tag.name), serialize_start_tag(tag));
    let cleaned = SANITIZER.clean(&probe).to_string();
    tokenize(&cleaned)
        .into_iter()
        .find_map(|token| match token {
            Token::TagToken(kept) if kept.kind == TagKind::StartTag && kept.name == tag.name => {
                Some(serialize_start_tag(&kept))
            }
            _ => None,
        })
}

/// Sanitizes one piece of author-written HTML (a whole HTML block, or one
/// inline tag) under [`RawHtmlPolicy::Sanitize`]. See the module docs.
#[must_use]
pub fn sanitize_fragment(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    // Inside an element whose content is dropped: its name.
    let mut dropping: Option<String> = None;
    for token in tokenize(html) {
        match token {
            Token::TagToken(tag) => match tag.kind {
                TagKind::StartTag => {
                    if dropping.is_some() {
                        continue;
                    }
                    if raw_text_kind(&tag.name).is_some() {
                        dropping = Some(tag.name.to_string());
                    } else if let Some(kept) = sanitize_start_tag(&tag) {
                        out.push_str(&kept);
                    }
                }
                TagKind::EndTag => {
                    if let Some(name) = &dropping {
                        if *name == *tag.name {
                            dropping = None;
                        }
                    } else if ALLOWED_TAGS.contains(&*tag.name) {
                        out.push_str("</");
                        out.push_str(&tag.name);
                        out.push('>');
                    }
                }
            },
            Token::CharacterTokens(text) if dropping.is_none() => {
                push_escaped(&mut out, &text, false);
            }
            // Comments, doctypes, NULs, parse errors, dropped content.
            _ => {}
        }
    }
    out
}

/// The text [`RawHtmlPolicy::Escape`] shows for one piece of raw HTML: the
/// source as written, minus HTML comments (which are never visible, so
/// showing them would publish what the author hid). Not escaped: it becomes an
/// `Event::Text`, which the HTML writer escapes.
#[must_use]
pub fn escape_fragment(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start + 4..].find("-->") {
            Some(end) => rest = &rest[start + 4 + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Whether an author-written attribute on an element mbr generates (a
/// heading's `{…}`, a section's `--- {…}`) may be written under a policy
/// other than passthrough: no event handlers (`on*`), no `srcdoc` or
/// `formaction`, and no value with a script-capable URL scheme
/// (`javascript:`, `vbscript:`, non-image `data:`).
#[must_use]
pub fn is_safe_author_attribute(name: &str, value: Option<&str>) -> bool {
    let name = name.to_ascii_lowercase();
    !(name.starts_with("on")
        || name == "srcdoc"
        || name == "formaction"
        || value.is_some_and(crate::html::is_blocked_destination))
}

/// Applies a [`RawHtmlPolicy`] to a parser's `(event, range)` stream.
///
/// Placed between the parser and pass 1, so it sees only what the author
/// wrote. HTML blocks are buffered whole (pulldown-cmark splits them per line,
/// and a tag may span lines). Under [`RawHtmlPolicy::Passthrough`] every event
/// is returned untouched.
pub(crate) struct RawHtmlFilter<'a, I> {
    inner: I,
    policy: RawHtmlPolicy,
    pending: VecDeque<(Event<'a>, Range<usize>)>,
}

impl<'a, I> RawHtmlFilter<'a, I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    pub(crate) fn new(inner: I, policy: RawHtmlPolicy) -> Self {
        Self {
            inner,
            policy,
            pending: VecDeque::new(),
        }
    }

    /// Reads the rest of an HTML block (after its `Start`), queueing whatever
    /// the policy makes of it.
    fn buffer_block(&mut self, start: Range<usize>) {
        let mut html = String::new();
        let mut end = start.end;
        let mut trailing = Vec::new();
        for (event, range) in self.inner.by_ref() {
            end = range.end;
            match event {
                Event::Html(text) | Event::InlineHtml(text) => html.push_str(&text),
                Event::End(TagEnd::HtmlBlock) => break,
                // pulldown-cmark puts nothing else in an HTML block; keep it
                // rather than lose it if that ever changes.
                other => trailing.push((other, range)),
            }
        }
        let block = start.start..end;
        match self.policy {
            RawHtmlPolicy::Passthrough => unreachable!("passthrough never buffers"),
            RawHtmlPolicy::Sanitize => {
                let clean = sanitize_fragment(&html);
                if !clean.trim().is_empty() {
                    self.pending.extend([
                        (Event::Start(MdTag::HtmlBlock), start.clone()),
                        (Event::Html(CowStr::from(clean)), block.clone()),
                        (Event::End(TagEnd::HtmlBlock), block),
                    ]);
                }
            }
            RawHtmlPolicy::Escape => {
                let text = escape_fragment(&html);
                let text = text.trim_end();
                if !text.trim().is_empty() {
                    self.pending.extend([
                        (Event::Start(MdTag::Paragraph), start.clone()),
                        (Event::Text(CowStr::from(text.to_string())), block.clone()),
                        (Event::End(TagEnd::Paragraph), block),
                    ]);
                }
            }
        }
        self.pending.extend(trailing);
    }

    /// One inline piece of raw HTML, or `None` when nothing is left of it.
    fn inline(&self, html: &str) -> Option<Event<'a>> {
        match self.policy {
            RawHtmlPolicy::Passthrough => Some(Event::InlineHtml(CowStr::from(html.to_string()))),
            RawHtmlPolicy::Sanitize => {
                let clean = sanitize_fragment(html);
                (!clean.is_empty()).then(|| Event::InlineHtml(CowStr::from(clean)))
            }
            RawHtmlPolicy::Escape => {
                let text = escape_fragment(html);
                (!text.is_empty()).then(|| Event::Text(CowStr::from(text)))
            }
        }
    }
}

impl<'a, I> Iterator for RawHtmlFilter<'a, I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    type Item = (Event<'a>, Range<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(queued) = self.pending.pop_front() {
                return Some(queued);
            }
            let (event, range) = self.inner.next()?;
            if self.policy == RawHtmlPolicy::Passthrough {
                return Some((event, range));
            }
            match event {
                Event::Start(MdTag::HtmlBlock) => self.buffer_block(range),
                Event::Html(html) | Event::InlineHtml(html) => {
                    if let Some(event) = self.inline(&html) {
                        return Some((event, range));
                    }
                }
                Event::Start(MdTag::Heading {
                    level,
                    id,
                    classes,
                    mut attrs,
                }) => {
                    attrs.retain(|(name, value)| is_safe_author_attribute(name, value.as_deref()));
                    return Some((
                        Event::Start(MdTag::Heading {
                            level,
                            id,
                            classes,
                            attrs,
                        }),
                        range,
                    ));
                }
                other => return Some((other, range)),
            }
        }
    }
}

/// Drops the unsafe attributes of every `--- {attrs}` section under a policy
/// other than passthrough (see [`is_safe_author_attribute`]).
pub(crate) fn filter_section_attrs(
    section_attrs: HashMap<usize, crate::attrs::ParsedAttrs>,
    policy: RawHtmlPolicy,
) -> HashMap<usize, crate::attrs::ParsedAttrs> {
    if policy == RawHtmlPolicy::Passthrough {
        return section_attrs;
    }
    section_attrs
        .into_iter()
        .map(|(index, mut attrs)| {
            attrs
                .attrs
                .retain(|(name, value)| is_safe_author_attribute(name, value.as_deref()));
            (index, attrs)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_is_removed_with_its_content() {
        let out = sanitize_fragment("<p>hi</p><script>alert(1)</script><b>ok</b>");
        assert_eq!(out, "<p>hi</p><b>ok</b>");
    }

    #[test]
    fn unterminated_script_drops_the_rest() {
        assert_eq!(sanitize_fragment("a<script>alert(1)"), "a");
    }

    #[test]
    fn raw_text_elements_are_removed_with_their_content() {
        for tag in ["style", "textarea", "title", "iframe", "noscript", "xmp"] {
            let out = sanitize_fragment(&format!("<{tag}><img src=x onerror=alert(1)></{tag}>ok"));
            assert_eq!(out, "ok", "{tag}");
        }
    }

    #[test]
    fn event_handlers_and_script_urls_are_removed() {
        let out = sanitize_fragment(
            r#"<a href="javascript:alert(1)" onclick="x()">t</a><img src=x onerror=y>"#,
        );
        assert!(!out.contains("javascript"), "{out}");
        assert!(!out.contains("onclick"), "{out}");
        assert!(!out.contains("onerror"), "{out}");
        assert!(out.contains("<img src=\"x\">"), "{out}");
    }

    #[test]
    fn entity_obfuscated_script_url_is_removed() {
        let out = sanitize_fragment(r#"<a href="jav&#x61;script:alert(1)">t</a>"#);
        assert!(!out.contains("href"), "{out}");
    }

    #[test]
    fn unbalanced_formatting_survives_piecewise() {
        // pulldown-cmark hands an inline element over as separate events.
        assert_eq!(sanitize_fragment("<kbd>"), "<kbd>");
        assert_eq!(sanitize_fragment("</kbd>"), "</kbd>");
        assert_eq!(sanitize_fragment("<details open>"), "<details open=\"\">");
        assert_eq!(sanitize_fragment("</details>"), "</details>");
    }

    #[test]
    fn table_parts_survive_out_of_context() {
        assert_eq!(sanitize_fragment("<tr>"), "<tr>");
        assert_eq!(sanitize_fragment("<td colspan=2>"), "<td colspan=\"2\">");
        assert_eq!(sanitize_fragment("<thead>"), "<thead>");
        assert_eq!(
            sanitize_fragment("<table>\n<tr><td>a</td></tr>\n</table>"),
            "<table>\n<tr><td>a</td></tr>\n</table>"
        );
    }

    #[test]
    fn relative_urls_class_and_align_are_kept() {
        let out = sanitize_fragment(
            r#"<p align="center" class="x"><img src="docs/a.png" width="40"></p>"#,
        );
        assert_eq!(
            out,
            r#"<p align="center" class="x"><img src="docs/a.png" width="40"></p>"#
        );
    }

    #[test]
    fn style_attribute_and_forms_are_removed() {
        let out = sanitize_fragment(
            r#"<span style="position:fixed">x</span><form action="/x"><input></form>"#,
        );
        assert_eq!(out, "<span>x</span>");
    }

    #[test]
    fn reserved_ids_are_dropped() {
        assert_eq!(sanitize_fragment(r#"<div id="__MBR_EXPORT__">"#), "<div>");
        assert_eq!(sanitize_fragment(r#"<div id="mermaid">"#), "<div>");
        assert_eq!(
            sanitize_fragment(r#"<a id="faq">"#),
            r#"<a id="faq" rel="noopener noreferrer">"#
        );
    }

    #[test]
    fn text_is_reescaped_and_entities_kept() {
        assert_eq!(
            sanitize_fragment("a &amp; b &lt;c&gt;"),
            "a &amp; b &lt;c&gt;"
        );
        assert_eq!(sanitize_fragment("<b>1 < 2</b>"), "<b>1 &lt; 2</b>");
    }

    #[test]
    fn comments_and_svg_are_removed() {
        assert_eq!(sanitize_fragment("<!-- secret --><b>x</b>"), "<b>x</b>");
        let out = sanitize_fragment("<svg><script>alert(1)</script></svg>");
        assert_eq!(out, "");
    }

    #[test]
    fn escape_keeps_source_but_hides_comments() {
        assert_eq!(escape_fragment("<b>x</b><!-- hidden -->y"), "<b>x</b>y");
        assert_eq!(escape_fragment("a<!-- open"), "a");
    }

    #[test]
    fn author_attributes() {
        assert!(!is_safe_author_attribute("onclick", Some("x()")));
        assert!(!is_safe_author_attribute("OnLoad", None));
        assert!(!is_safe_author_attribute("srcdoc", Some("<b>")));
        assert!(!is_safe_author_attribute(
            "data-background-iframe",
            Some("javascript:alert(1)")
        ));
        assert!(is_safe_author_attribute("style", Some("--pico-color:cyan")));
        assert!(is_safe_author_attribute(
            "data-background-image",
            Some("img/a.png")
        ));
    }

    #[test]
    fn strictest_never_lowers() {
        use RawHtmlPolicy::*;
        for a in [Passthrough, Sanitize, Escape] {
            for b in [Passthrough, Sanitize, Escape] {
                let s = a.strictest(b);
                assert!(s >= a && s >= b);
            }
        }
    }

    #[test]
    fn policy_serde_names() {
        let policy: RawHtmlPolicy = serde_json::from_str("\"sanitize\"").unwrap();
        assert_eq!(policy, RawHtmlPolicy::Sanitize);
        assert_eq!(
            serde_json::to_string(&RawHtmlPolicy::Escape).unwrap(),
            "\"escape\""
        );
    }
}
