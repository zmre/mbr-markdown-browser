---
title: Page Styles and Types
description: Change a page's entire look with the style and type frontmatter fields
order: 4
---

# Page Styles and Types

Two frontmatter fields decide how a whole page looks. `style` names a display
style, `type` names what the note is, and mbr turns both into classes on the
page's `<body>` element, which is what the CSS keys off.

## How It Works

Every entry in `style`, plus a slugified copy of `type`, becomes a class in the
body class list. The type leads, the styles follow, and duplicates are collapsed:

```yaml
---
title: Kickoff
type: Meeting Notes
style: outline
---
```

That page renders as `<body class="meeting-notes outline">`. Any rule in
`theme.css` or your own `.mbr/user.css` that targets `.outline` or
`.meeting-notes` now applies to the entire document.

## The `style` Field

`style` is a display choice. Give it a single value:

```yaml
---
title: Q3 Planning
style: outline
---
```

Or give it several, as a YAML list:

```yaml
---
title: Q3 Planning
style: [outline, handout]
---
```

A space-separated string works too, so `style: outline handout` produces the same
two classes. Every value lands in the class list exactly as written, which means
your own class names are as welcome as the built-in ones.

## The `type` Field

`type` is the note's classification: what it *is*, rather than how it should look.
It is aligned with the
[OKF spec](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md),
which treats a note's kind as a first-class, self-descriptive field.

Because a type is written for humans and a class is written for CSS, mbr
lowercases the value and slugifies it before adding it to the class list.
Everything that is not a letter or a number separates words, and the words are
rejoined by single dashes:

| `type:` value        | Body class          |
|----------------------|---------------------|
| `person`             | `person`            |
| `slides`             | `slides`            |
| `Meeting Notes`      | `meeting-notes`     |
| `Field Note!`        | `field-note`        |
| `Book Review (2024)` | `book-review-2024`  |
| `Q&A`                | `q-a`               |

The result is always a class you can write a selector for directly: no doubled
dashes from adjacent punctuation, and no dash left dangling at either end.

The slug joins the same list `style` feeds, so `type: slides` now does everything
`style: slides` used to do. Writing both is harmless: the duplicate is collapsed
and the class appears once.

A few details worth knowing:

- **`type` must be a single string.** A list or a number is ignored for the body
  class, though the value still flows through to templates and `site.json`.
- **A type that slugifies to nothing adds nothing.** `type: "!!!"` leaves no
  empty class behind.
- **Templates see `type` as you wrote it.** Only the body class is slugified, so
  a template condition like `{% if type == "person" %}` still matches.

`type` is more than a class name. It also drives features such as the person
infobox and the genealogy charts. See
[The `type` field](relationships/#the-type-field) for the rest of what it does.

## Which One Should I Use?

| Use      | For                          | Examples                            |
|----------|------------------------------|-------------------------------------|
| `type:`  | What the note **is**         | `person`, `recipe`, `meeting-notes` |
| `style:` | How the page should **look** | `outline`, `kanban`, your own class |

Prefer `type` when the answer is a kind of note, and reach for `style` when the
answer is a presentation choice that is not the note's kind. A board of cards is a
good example of the latter: a project note displayed as a
[kanban](kanban/) is still a project note, so it keeps `style: kanban`.

Nothing stops a page from carrying both. A deck of slides about a person can be
`type: person` with `style: slides`.

## Styles That Ship With mbr

| Style     | What it does                                                                       | Read more                      |
|-----------|------------------------------------------------------------------------------------|--------------------------------|
| `outline` | Auto-numbered headings, alternating list markers, tighter typography, print styles | [Below](#the-outline-style)    |
| `slides`  | Turns the page into a Reveal.js deck, with horizontal rules separating slides      | [Presentation Slides](slides/) |
| `kanban`  | Lays lists out as trello-like columns, one card per bullet                         | [Kanban Display](kanban/)      |

## The Outline Style

`outline` reformats a document the way you would format a written outline or a
handout. It is the most self-contained of the built-in styles: pure CSS, no
JavaScript, and nothing about your markdown has to change.

```yaml
---
title: Design Review
style: outline
---
```

### Numbered Headings

Headings are numbered by CSS counters, so the numbers live in the stylesheet
rather than in your text. Reorder a section and everything renumbers itself,
with no stale `3.2.1.` left behind in the markdown.

| Heading  | Numbering              |
|----------|------------------------|
| `#`      | Not numbered           |
| `##`     | `1.`, `2.`, `3.`       |
| `###`    | `1.1.`, `1.2.`, `1.3.` |
| `####`   | `1.1.1.`, `1.1.2.`     |
| `#####`  | `1.1.1.1.`             |
| `######` | Not numbered           |

The top-level `#` is the document's title, so numbering starts at your `##`
sections. Each level restarts when its parent advances.

### Alternating List Markers

Ordered lists change marker style with depth, which keeps a deep outline legible
in a way that four levels of `1.` never manages:

| Nesting level     | Marker                          |
|-------------------|---------------------------------|
| First             | `A.` `B.` `C.` (upper alpha)    |
| Second            | `1.` `2.` `3.` (decimal)        |
| Third             | `a.` `b.` `c.` (lower alpha)    |
| Fourth and deeper | `i.` `ii.` `iii.` (lower roman) |

Bullet lists stay a plain disc at every depth, and nested lists lose their extra
top and bottom margins so a nested structure reads as one block rather than as
several.

### Typography

- **16px text on a 1.6 line height**, with a consistent 16px gap between
  paragraphs, lists, tables, quotes, and code blocks.
- **Compact headings** at weight 600 and a tight 1.1 line height. `#` and `##`
  carry a hairline rule beneath them.
- **Quiet blockquotes**, set in grey behind a 4px left rule.
- **Bold italic definition terms**, with their definitions indented beneath.

### Printing

`outline` ships a print stylesheet, which is the point of a handout. Paper gets a
white background, no page border, and 12px text; images, tables, and figures are
kept from splitting across a page break; and code blocks print in full instead of
being clipped to their scroll box. The site-wide print rules still apply on top of
these, so collapsed sections and closed answers are expanded before printing.
See [Printing](../customization/themes/#printing).

> [!NOTE]
> The outline rules use fixed light-tone values for the heading rules and
> blockquote colors rather than Pico variables, because they are tuned for reading
> and printing. On a dark theme you may want to override those few colors in
> `.mbr/user.css`.

## Writing Your Own Style

A custom style is nothing more than a class you define and a `style:` value that
matches it. Put the CSS in `.mbr/user.css`, which is loaded after the default
theme so your rules win:

```css
/* .mbr/user.css */
.handout main {
  max-width: 42rem;
  font-family: Georgia, serif;
}
```

```yaml
---
title: Workshop Handout
style: handout
---
```

Because the class list is applied verbatim, a `style:` value that no rule matches
is simply an inert class. That is a feature: you can tag pages now and style them
later, and a typo costs you nothing but a class nobody reads.

## See Also

- [Presentation Slides](slides/) - The `slides` style in full
- [Kanban Display](kanban/) - The `kanban` style in full
- [Relationships & Genealogy](relationships/#the-type-field) - What else `type` drives
- [CSS Theming](../customization/themes/) - Themes, `user.css`, and full theme overrides
