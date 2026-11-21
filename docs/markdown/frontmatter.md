---
title: Frontmatter
description: "YAML metadata at the top of a note: title, tags, type, dates and your own fields"
order: 1
---

# Frontmatter

Add metadata to any markdown file:

```yaml
---
title: My Document
description: A helpful guide
tags: documentation, guide
date: 2025-01-09
author: Your Name
custom_field: Any value
---

# Content starts here
```

Frontmatter must be at the very top of the file: the opening `---` is the
first line (a UTF-8 byte-order mark before it is fine). The block needs a
non-blank first line and a closing `---` or `...`. This is how GitHub,
Obsidian and Jekyll read it. A `---` anywhere else is ordinary markdown: a
horizontal rule (a section or slide break), or the underline of a setext
heading when it sits directly under a line of text. So in a slides deck

```markdown
---
## Quick navigation

* In-links
* Out-links

---
```

is a slide with a heading and a list, never a second metadata block.

## What it is used for

- **Page titles**: the browser tab and, when the note has no `# Heading`, the
  page heading
- **Descriptions**: search results and link previews
- **Tags**: [tag pages](../features/tags/) and the sidebar's tag tree
- **Dates**: where a note sits on the [calendar](../features/calendar/#note-dates)
  and [date search](../features/search/#search-by-date)
- **Sorting**: `order` and `pinned` in [file sorting](../customization/sorting/)
- **Note kinds**: `type: slides`, `type: flashcard`, `type: person`,
  `type: organization` turn on [slides](../features/slides/),
  [flashcards](../features/flashcards/) and [contacts](../features/contacts/)
- **Page styles**: `style` and `type` become the
  [body class](../features/page-styles/) that restyles the whole page
- **Relationships**: typed links between notes, see
  [Relationships](../features/relationships/)
- **Links**: a value that is a `[[wikilink]]` or a path to a file links to it,
  see [Links in frontmatter](#links-in-frontmatter)
- **Your own fields**: searchable as `field:value`, and available to
  [templates](../customization/templates/) as `frontmatter_json`

## Fields mbr reads

| Field | Purpose |
|-------|---------|
| `title` | Page title; also a name [wikilinks](links/#wikilinks) can use |
| `description` | Meta description, search snippet |
| `tags` | Tags (see below) |
| `date` | The note's date (before `created`); also publication date |
| `created` | The note's date when there is no `date` |
| `aliases` | Other names the note answers to in wikilinks and relationships |
| `type` | The kind of note; also becomes a [body class](../features/page-styles/) |
| `style` | Display [style(s)](../features/page-styles/) applied as body classes |
| `order`, `pinned` | Position in listings, see [File Sorting](../customization/sorting/) |
| `relationships` | Typed links to other notes: a list of `{type, to}` edges, or the [short form](../features/relationships/#the-short-form) `reports_to: "[[X]]"`; see [Relationships](../features/relationships/) |
| `slides` | Per-deck settings, see [Deck Settings](../features/slides/#deck-settings) |
| `template` | An `.html` file, relative to the note's folder, that replaces the page layout for this note; see [Custom Page Templates](../features/custom-page-templates/) |
| `author` | Author name, available to templates |
| A wikilink or path value | A link to that note or file; see [Links in frontmatter](#links-in-frontmatter) |
| Any other field | Searchable, and available to templates, each as its own variable and all of them, structure intact, as `frontmatter` (see [Custom Page Templates](../features/custom-page-templates/#variables)) |

Contacts read many more (`company`, `emails`, `phones`, `dates`, …); see
[Contacts](../features/contacts/).

Field names are case-sensitive, as in YAML, with one exception: `type`, and the
contact `dates`, `born` and `died`, are read in any case. `Type: person` works
like `type: person` everywhere — templates, the body class, `site.json`, search
and the charts all see it as `type` — and if a note spells one of them twice
(`Type:` and `type:`), the first one wins. Every other field keeps the case you
wrote it in.

## Tags

Any of these forms works:

```yaml
# YAML list
tags: [rust, programming, tutorial]

# YAML block list
tags:
  - rust
  - programming

# Comma-separated string
tags: rust, programming, tutorial
```

A `/` makes a hierarchy: `project/frontend` sits under `project` in the
sidebar's tag tree. Other fields can be tag sources too (`category`,
`performers`, a nested `taxonomy.tags`); see [Tags](../features/tags/) for tag
pages and configuration.

## Aliases

```yaml
aliases: [Bob, Robert Wall]
```

A note answers to each alias in [wikilinks](links/#wikilinks) and in
relationship targets, after titles and before file names. Person notes can
label them (`maiden_name: Mary Smith`); see
[Aliases](../features/contacts/#aliases).

## Links in frontmatter

A value that is **only** a wikilink, or **only** a path to a file, is a link:

```yaml
---
customer: "[[Customer X]]"
owner: "[[People/Ann Lee|Ann]]"
contracts:
  - "[[Customer X MSA 2026]]"
  - "/customers/contracts/Customer X SOW 3 renewal (signed).pdf"
  - ./scans/po-75334.pdf
---
```

- **Wikilinks**: `[[Target]]` or `[[Target|Label]]`, resolved exactly like a
  [wikilink in the body](links/#wikilinks): a note of that name in the same
  folder first, else the first note with that title, alias or file name.
  Quote it: an unquoted `[[…]]` is a YAML list.
- **Paths**: a value starting with `/` (from the repository root), `./` or
  `../` (from the note's folder), to a file of any kind: a note, a PDF, an
  image. Values that do not start that way are never paths, so `1/2` or
  `see notes/` stay text.
- At the **top level** of the frontmatter, or as an element of a **list of
  strings**. Text around the link (`"see [[X]]"`), a value inside a nested
  map and a list of maps are not links.
- `template`, `relationships` and the private contact fields (`emails`,
  `phones`, `addresses`, …) are never links.

What mbr does with them (backlinks, the info panel, broken-link reports,
templates) is in [Frontmatter links](../features/links/#frontmatter-links).

## Editing frontmatter

The in-browser editor has a form for frontmatter that keeps your YAML exactly
as written; see [Editing Properties](../features/editing-properties/).

## Long frontmatter

There is no size limit you are likely to meet. When mbr indexes a note it
reads only the first 8 KB, which holds almost any frontmatter; when the block
has not closed by then (an invoice with many line items, a contact with many
addresses), it reads on until it closes, up to 1 MB. A block still open after
1 MB is ignored like any block that never closes, with a warning naming the
file.

## Malformed frontmatter

One YAML mistake (a duplicated key is the easy one) discards the **whole**
block, not just the bad line: the note loses its title, tags, `type` and
relationships at once. mbr logs a warning naming the file and the line, and
`mbr -b` reports it. See
[Frontmatter parse errors](../features/relationships/#frontmatter-parse-errors).
