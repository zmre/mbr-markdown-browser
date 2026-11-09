---
title: Section Attributes
description: Divide a note into sections with --- and give each an id, classes and attributes
order: 11
---

# Section Attributes

When `enable_sections` is active (default for server/GUI mode), horizontal rules (`---`) divide content into `<section>` elements. You can add attributes to the **following** section by placing an attribute block after the rule:

```markdown
--- {#intro .highlight}

This content is in a section with id="intro" and class="highlight".

--- {.slide data-transition="fade"}

This section has class="slide" and a custom data attribute.

---

Plain section (no attributes).
```

## Attribute Syntax

The attribute block follows [pulldown-cmark's heading attributes syntax](https://pulldown-cmark.github.io/pulldown-cmark/specs/heading_attrs.html), plus quoted values:

| Syntax | Result | Example |
|--------|--------|---------|
| `#id` | ID attribute | `{#intro}` → `id="intro"` |
| `.class` | CSS class | `{.highlight}` → `class="highlight"` |
| `key=value` | Custom attribute | `{data-x=y}` → `data-x="y"` |
| `key="value"` | Quoted value | `{title="Hello World"}` |
| `key='value'` | Single-quoted value | `{data-q='say "hi"'}` → `data-q="say &quot;hi&quot;"` |

Values are used exactly as written. Smart punctuation does not touch them, so `--`, `...` and quotes inside a value stay as typed: `--- {style="--pico-color: cyan;"}` sets the CSS custom property. A `#` or `.` inside a quoted value is part of the value (`style="color: #fff"` sets no id). Backslash escapes and HTML entities are not decoded inside the braces.

Multiple attributes can be combined:

```markdown
--- {#section-1 .slide .center data-transition="slide" data-background="#fff"}
```

## The First Section

Content before the first rule is section one, so an attribute rule at the very
top of a note (only frontmatter and blank lines before it) applies to that
first section rather than opening a new one. No empty section and no `<hr>` is
produced:

```markdown
---
title: Deck
---

--- {#cover .dark}

This content is in the first section, with id="cover" and class="dark".
```

A plain `---` at the top, or an attribute rule anywhere after content, divides
sections as usual.

## Use Cases

**Presentation slides:** Add Reveal.js-style attributes for slide transitions and backgrounds. For defaults that apply to a whole deck (or every deck in the repository), see [Deck Settings](../features/slides/#deck-settings).

**Styling:** Target specific sections with CSS using IDs or classes. Two
classes ship with mbr: `.media-float-left` and `.media-float-right` float the
section's image, video, audio, PDF or YouTube embed beside its text (see
[Floating Media Beside Text](media/#floating-media-beside-text)).

**JavaScript hooks:** Add data attributes for interactive behavior.

## Live Example

--- {#demo-section .highlighted-section}

This section has `id="demo-section"` and `class="highlighted-section"`. Inspect the HTML to verify!

---

Back to a plain section.
