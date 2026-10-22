---
title: Editing Properties (Frontmatter)
description: Edit a note's YAML frontmatter as a form or as YAML, with suggestions from your own notes
order: 5.5
---

# Editing Properties (Frontmatter)

The [in-browser editor](editing/) shows a note's YAML frontmatter as
**Properties**, in a panel above the body. A note without frontmatter shows
**Add properties** instead. The panel has two tabs that edit the same YAML:

- **Form**: one row per property, each with a widget that suits its value
  (a date picker for a date, chips for tags, a note picker for a link).
- **YAML**: the frontmatter as source, with syntax highlighting, completion
  and error checking.

You do not need to know mbr's vocabulary in advance. The form offers the
fields mbr understands, the fields of the
[OKF spec](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md),
and the keys and values your other notes already use.

## Your YAML is kept as you wrote it

The form never rewrites the whole block. Each edit changes only the bytes of
the value you edited. Everything else stays as it was: comments, key order,
quoting, flow style (`[a, b]`), blank lines, and keys mbr knows nothing about.
Frontmatter you do not touch at all is saved byte for byte (see
[What a save writes](editing/#what-a-save-writes)).

After every edit the editor parses the result again and checks that it means
what the edit meant. In the rare case that an edit cannot be made in place,
the editor rewrites the block in a standard YAML layout and the form says so:
"One edit could not be made in place, so the YAML was rewritten in a standard
layout."

Some YAML features have no form equivalent: anchors (`&x`), aliases (`*x`),
explicit tags (`!!str`), merge keys (`<<`), and comments inside a list or map.
A value that uses any of them is shown read-only, as its YAML source, with an
**Edit as YAML** button. That opens a small YAML editor holding only that
value. **Apply** (or `Cmd+Enter` / `Ctrl+Enter`) writes it back in place, and
**Cancel** (or `Esc`) leaves it unchanged. A comment at the end of a scalar's
line (`mobile: "+1 555 0100"  # cell`) does not need this: editing the value
keeps the comment.

**Save** includes edits you have not finished: an **Edit as YAML** you did
not apply, a tag typed without `Enter`, a rename still being typed. If the
YAML you typed does not parse, the note is not saved and the status line says
which value to fix. Closing the editor asks about them like any other unsaved
change.

If the YAML does not parse, the form is read-only. It lists each error with
its line and column and offers **Fix it in YAML**. The form never writes over
YAML it could not read.

## The form

Each row shows an icon for the kind of value, the property name, and the value
widget. The **⋯** menu on a row holds **Rename**, **Edit as YAML**,
**Change type to …** (for example, text to a number or a date, or a single
value to a list), and **Remove**. Maps and lists expand into nested rows, each
with its own **Add**, so a value of any shape can be edited.

| Value | Widget |
|-------|--------|
| Text | A text field. Long text (a `description`, or anything with line breaks) gets a text area. A value you wrote without quotes gets quotes only while it needs them, such as while a trailing space is typed |
| Number | A number field |
| `true` / `false` | A checkbox |
| Date (`2026-10-21`) | A date picker that writes the date only |
| Date and time | A date-and-time picker. The UTC offset written with the time is shown beside it and kept when you edit. A time with no offset gets your local offset, because OKF requires one: `2026-06-30T14:00:00-06:00`, or `Z` for UTC |
| Partial date | Year, month and day, any of them blank, for contact dates: `1927-03-19`, `1927-03`, `1927` or `--03-19`. An impossible combination, like a day with no month, writes nothing and says why |
| Tags | Chips. Type a tag and press `Enter` or `,` to add it; your vault's tags are suggested. A tag field written as one comma-separated string stays a string |
| A choice (`type`, `status`) | A field with suggestions that still accepts any text |
| Note link (`company`) | A note picker that writes `"[[Note Title]]"`. Free text is still allowed |
| List | One field per item. `Enter` moves to the next item, and in the last one adds a new item |
| Anything else | Nested rows |

**+ Add property** (or `Cmd+;` / `Ctrl+;`) opens a list of property names.
The order is: mbr's fields, then the contact fields when the note is a
`person` or `organization`, then the Attested Computation fields when the note
is that type, then OKF's fields, and last the keys your other notes use, most
used first. Fields the note already has are left out. You can also type any
name. A new property starts with an empty widget of the right kind.

Problems show as soft hints next to the value. They never block a save:

- an OKF `status` other than `draft`, `stable` or `deprecated`;
- a required value that is missing: a source without `resource`, a
  relationship without `type`, a review without `by`, an Attested Computation
  without `runtime`;
- a date and time with no UTC offset;
- a relationship type that is not in `relationship_types`.

### Choosing a type

`type` is the one field that turns a note into something mbr treats
specially. Its suggestions come first, then your vault's own types, most used
first:

| `type` | Effect |
|--------|--------|
| `slides` | The note plays as a slideshow ([Slides](../markdown/slides/)) |
| `flashcard` | Every definition list becomes a study deck ([Flashcards](../markdown/flashcards/)) |
| `person` | A contact card, and relationship charts below the note ([Contacts](../markdown/contacts/)) |
| `organization` | A contact card and an org chart |

Any other value is fine. OKF types are open-ended (`Metric`, `Playbook`,
`BigQuery Table`), and an unknown type gets no warning. Every type also
becomes a class on the page's `<body>` (see
[Page Styles and Types](../markdown/styles/)). The form has no helper for
`style`, which stays an ordinary field.

### mbr's fields

- `title`, `description`, `summary`
- Your tag fields: `tags`, or whichever fields the repository's
  [`tag_sources`](../reference/tags/) names
- `aliases`: other names for the note, each with an optional label
  (`nickname`, `maiden_name`)
- `date`, and `order` and `pinned` for [sorting](../customization/sorting/)
- `relationships` (below)

### Relationships

Each relationship in `relationships` is a row that reads from the note's own
side: **type**, then **note**. `parent` · `[[Sam]]` means "Sam is this note's
parent" (`type: parent, to: "[[Sam]]"`). The row has:

- a **type**, chosen from the repository's
  [`relationship_types`](../markdown/relationships/), each with its label and
  category;
- the **note**, from a note picker;
- an optional **label**, shown instead of the type's own label;
- a **Flip relationship direction** checkbox. Checking it writes `from:` in
  place of `to:`, so `parent` · `[[Sam]]` then means this note is Sam's parent;
  unchecking writes `to:` back. Only the key changes, and the sentence under
  the row says which way the relationship now reads. A symmetric type
  (`spouse`, `sibling`, `colleague`) has no checkbox, since its direction
  means nothing.

A relationship that names two other notes (both `from:` and `to:`) shows both
notes, labelled "from" and "to", and has no checkbox.

Under the row, the relationship reads back as a sentence, including the
reverse relationship mbr derives. On Jane's note, `type: parent, to: "[[Sam]]"`
reads "Sam is Jane's parent · Jane is Sam's child". Any other key on a
relationship (`married:`, `place:`) is kept and shown as an ordinary row under
it. A type that is not in the registry is allowed, with a warning that no
reverse relationship will be derived.

### Contact fields

When `type` is `person` or `organization`, the form offers the
[contact fields](../markdown/contacts/): name parts, `company` (a note link,
which on a person also makes an `employer` relationship), `job_title`,
`department`, `image`, and the labeled fields `emails`, `phones`, `urls`,
`social`, `im`, `addresses` and `dates`.

A labeled field is a list of entries, each with a label and a value. The
labels are suggested (`mobile`, `work`, `home` for phones; `birthday`, `death`,
`anniversary` for dates). An address has its parts (street, city, region,
postcode, country). mbr reads three ways of writing a labeled field: a map
(`work: x`), a list of one-key maps (`- work: x`), or plain values. The form
reads all three, and a new entry is written the way the note already writes
the others. A field that is empty becomes a map.

`born` and `died` are still shown when a note has them, but new notes get
`dates.birthday` and `dates.death` instead.

### OKF fields

Every note is offered the optional fields of
[OKF v0.2](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md),
after mbr's own:

| Field | What it holds |
|-------|---------------|
| `status` | `draft`, `stable` or `deprecated`. A note without one counts as `stable` |
| `resource` | What the note describes: a URL or a path |
| `sources` | Where the knowledge came from: a list of sources, each with `resource` (required) and optionally `id`, `title`, `author`, `usage_count`, `last_modified` and `usage_window` |
| `usage_window` | `from` and `to`, both dates and times |
| `generated` | `by` (who or what wrote the note) and `at`. `at` starts at the current time |
| `verified` | Reviews, each with `by` and `at`. A single review written as one map (not a list) stays a map until you add a second review |
| `stale_after` | The date and time after which the note is stale |

When `type` is `Attested Computation`, the form also offers `runtime`
(required), `parameters` (each with `name`, `type`, `required`),
`computation`, `executor` (`resource` and `receipt`) and `attester`.

The OKF spec asks tools to keep keys they do not know. The editor keeps every
key, so a note stays valid OKF after mbr edits it.

**Actors.** The `by` and `author` fields name an actor: `human:<id>`,
`process:<id>`, or `<agent>/<version>`. The suggestions are the actors your
vault already uses and the `human:` and `process:` prefixes. The **Me** button
writes `human:<your id>`. The first time you use it, it fills in `human:` and
asks you to type your id. After that the id is remembered until you leave the
page. It is never stored.

## The YAML tab

The YAML tab is a code editor for the whole frontmatter block. It has syntax
highlighting, undo, and line wrapping.

- **Completion** has the same knowledge as the form. Where you are typing a
  key, it offers property names. Where you are typing a value, it offers that
  field's values: types, statuses, tags, relationship types, actors, the
  current time for a date and time, and note titles for a link. Completion
  appears as you type, and `Ctrl+Space` opens it.
- **Errors** are underlined as you type, with the parser's message, using the
  same parser as the form. An alias that names no anchor is an error too.
- **What you type is what is saved.** Unlike the form, the YAML tab saves your
  text exactly as you typed it. Switch back to the form and it reads your text
  again.

## Suggestions come from your notes

The first time you click into the properties panel, the editor reads the
site index (`/.mbr/site.json`, the same one the browse and search panels
use). It then knows:

- which keys your notes use and how often, what kind of value each usually
  holds, and its most common values;
- your relationship types;
- your notes' titles, for note links.

The site index leaves out contact details (`emails`, `phones` and so on) and
flattens nested values, so suggestions inside those come from mbr's built-in
knowledge rather than from your notes. No configuration is involved, and
nothing is stored.

## Keyboard shortcuts

| Key | Where | Action |
|-----|-------|--------|
| `Cmd+;` / `Ctrl+;` | Form | Add a property |
| `←` / `→` | On the Form / YAML tabs | Switch tabs |
| `Enter` / `Tab` | Renaming a property | Apply the new name |
| `Esc` | Renaming a property | Cancel the rename |
| `Enter` | In a list item | Move to the next item; in the last one, add an item |
| `Enter` or `,` | In a tag field | Add the tag |
| `Backspace` | In an empty tag field | Remove the last tag |
| `↑` / `↓`, `Esc` | In a row's ⋯ menu | Move through the menu, close it |
| `Ctrl+Space` | YAML tab | Open completion |
| `Esc` | YAML tab, completion open | Close the completion list (the editor stays open) |
| `Cmd+Enter` / `Ctrl+Enter` | Edit as YAML | Apply |
| `Esc` | Edit as YAML | Cancel |

The editor's own keys still apply: `Cmd+S` / `Ctrl+S` saves, and `Esc`
elsewhere closes the editor. See [In-Browser Editing](editing/#keyboard-shortcuts).
