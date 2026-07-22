---
title: Relationships & Genealogy
description: Declare named, typed relationships between notes in frontmatter
---

# Relationships & Genealogy

mbr tracks two kinds of connections between notes:

- **Content links** — ordinary markdown links (`[text](url)`), tracked as
  inbound/outbound "backlinks". These say that two notes are connected.
- **Typed relationships** — named, directed edges declared in YAML frontmatter.
  These say *what* connects two notes (a `parent`, a `spouse`, a `sibling`…),
  and can carry their own attributes (marriage dates, places, notes).

The driving example is **genealogy**: each note is a person, edges are
`parent` / `child` / `spouse` / `sibling`, birth/death dates live on the person
notes, and marriage/divorce dates live on the relationship.

Typed relationships are on by default and cost nothing when unused. Disable them
with `relationship_tracking = false` (or `--no-relationship-tracking`).

## Why

I use markdown to track everything. Lately my wife has been into genealogy, but uses various sites and one offline program to track things. I just don't believe these will all be around in ten years and I certainly don't want to pay subscription fees for that time. 

To me, a family tree is more about stories than names and dates, which makes markdown notes perfect. We can track structure between them (something we already do anyway) and visualize that structure, but otherwise, they're just notes.  This also gives a chance to double own on alignment with [OKF](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md) allowing notes to be used for more than just prose. It's a test case for prose with optional structures.

We'll see where this goes, but really this note type could be used for other things, too, like tracking contacts.  Which would allow linking to referenced people and having their contact info at my fingertips.  This seems so much better than my current system that intermingles people from different places and contexts.

## The `type` field

Notes may declare a first-class, self-descriptive `type` (like `slides`):

```yaml
---
type: person
title: John Doe
---
```

Unknown types are tolerated — `type` is a plain string that flows through
frontmatter unchanged and is exposed to templates and `site.json`.

## Person fields

A `type: person` note may carry optional, self-descriptive attributes. All are
ordinary frontmatter — they flow through to templates and `site.json` unchanged —
but mbr gives a few of them first-class treatment:

| Field | Type | Purpose |
|--------|------|---------|
| `born` | date/string | Birth date; shown as the start of the lifespan (e.g. *1925–1999*). |
| `died` | date/string | Death date; shown as the end of the lifespan. |
| `born_place` | string | Birthplace; displayed with the lifespan on the person's page. |
| `image` | path or URL | Portrait; displayed on the person's page. |
| `gender` | string | Styles the family-tree nodes (e.g. `male`, `female`); any value is accepted. |
| `aliases` | array of strings | Alternate and maiden names (see below). |

```yaml
---
type: person
title: Mary Doe
born: 1927-03-19
died: 2010-08-05
born_place: "Boulder, CO"
gender: female
image: /people/mary.jpg
aliases:
  - Mary Smith          # maiden name
  - "Mrs. John Doe"
relationships:
  - type: spouse
    to: "[[John Doe]]"
---
```

### Aliases

`aliases` lists alternate names for a person — most usefully **maiden names** and
married names. They do two things:

- **Endpoint resolution.** A relationship endpoint that names an alias — written
  as a `[[Wikilink]]` or a path — resolves to this note, exactly like its
  `title` or filename stem. So a parent can point a `child` edge at
  `to: "[[Mary Smith]]"` (her maiden name) even though her note is titled
  *Mary Doe*, and it still links up.
- **Search.** Aliases are searchable, so looking up a maiden name finds the
  person.

Resolution order is **title → alias → filename stem** (all case-insensitive). As
with titles, an ambiguous name shared by multiple notes resolves
deterministically to the note with the lexicographically smallest URL.

All of these fields are optional; a person note with none of them behaves exactly
as before.

## The `relationships` field

Relationships are a single generic array of edge objects, so the same mechanism
works for any domain (genealogy is just one):

```yaml
---
type: person
title: John Doe
born: 1925-06-02          # node attribute on the person — ordinary frontmatter
died: 1999-11-20
relationships:
  - type: spouse           # required predicate
    to: "[[Mary Smith]]"   # endpoint
    married: 1948-06-01    # arbitrary edge attributes, preserved verbatim
    place: "Denver, CO"
  - type: child
    to: "[[Alice Doe]]"    # "Alice is my child"
  - type: parent
    to: "[[George Doe]]"   # "George is my parent"
---
```

### Endpoints and direction

Each edge connects a **subject** and an **object**:

- `to:` names the object. When `from` is omitted the subject defaults to the
  current note, so **`type: X, to: [[Y]]` reads "Y is my X"**.
- `from:` names the subject. When `to` is omitted the object defaults to the
  current note, so **`type: X, from: [[Y]]` reads "I am Y's X"**.
- If **both** are present, the edge connects two *other* notes (useful for
  authoring a graph from a single index note).
- If **neither** is present, the edge is skipped with a warning.

For genealogy the most natural form is `to:` from each person's own note:

```yaml
- type: parent
  to: "[[Sam Doe]]"     # Sam is my parent  → I show up under Sam's Children
- type: child
  to: "[[Alice Doe]]"   # Alice is my child → I show up under Alice's Parents
```

### Endpoint values

Endpoints may be:

- A `[[Wikilink]]` — resolved by note **title** first, then by filename
  **stem** (case-insensitive). `[[Target|Alias]]` is supported (the target is
  used for resolution).
- A **path** — absolute (`/people/john.md`) or relative (`../john.md`),
  resolved the same way markdown links are.

If an endpoint can't be resolved, the raw string is kept for display and a
warning is emitted — a broken relationship never fails a render or build.
Ambiguous names (two notes with the same title) resolve deterministically to
the note with the lexicographically smallest URL.

Body `[[Wikilinks]]` in page text resolve the same global way as relationship
endpoints: the current folder is preferred, otherwise the first matching note in
any folder (by title, alias, then filename stem) is used.

### Reserved keys and attributes

On each edge object, `type`, `to`, `from`, and `label` are reserved. **Every
other key is a free-form edge attribute**, preserved verbatim (dates, places,
notes, numbers, booleans…) and shown in the info panel and exposed as JSON.

## Automatic reverse edges

You declare each edge **once**; the reverse is derived automatically, exactly
like backlink derivation. This is driven by the `relationship_types` registry,
which classifies each type as:

- **symmetric** — the reverse reads the same (`spouse`, `sibling`).
- **inverse pair** — the reverse reads as the paired type (`parent` ↔ `child`).
- **directed** (anything not listed) — still tracked and visible from both
  sides, but not relabelled.

So if George's note declares `type: child, to: [[John Doe]]` ("John is my
child"), then:

- George's note shows John under **Children**.
- John's note shows George under **Parents** (derived, no editing needed).

Reciprocal declarations are de-duplicated: if John *also* declares
`type: parent, to: [[George Doe]]`, both sides collapse to the same single edge.

The built-in genealogy defaults are:

```toml
relationship_types = [
    { name = "parent", inverse = "child", label = "Parent", label_plural = "Parents" },
    { name = "child", inverse = "parent", label = "Child", label_plural = "Children" },
    { name = "spouse", symmetric = true, label = "Spouse", label_plural = "Spouses" },
    { name = "sibling", symmetric = true, label = "Sibling", label_plural = "Siblings" },
]
```

Add your own domain types (e.g. `{ name = "manager", inverse = "report" }`) in
`.mbr/config.toml`.

## Genealogy walkthrough

A three-generation family, one note per person, each edge declared once:

```yaml
# people/george.md
---
type: person
title: George Doe
born: 1898-02-11
relationships:
  - type: spouse
    to: "[[Martha Doe]]"
    married: 1920-04-10
  - type: child
    to: "[[John Doe]]"
  - type: child
    to: "[[Robert Doe]]"
---
```

```yaml
# people/john.md
---
type: person
title: John Doe
born: 1925-06-02
relationships:
  - type: spouse
    to: "[[Mary Smith]]"
    married: 1948-06-01
    place: "Denver, CO"
  - type: sibling
    to: "[[Robert Doe]]"
  - type: child
    to: "[[Alice Doe]]"
  - type: child
    to: "[[Sam Doe]]"
---
```

Even though **Sam's** note declares no relationships, opening it shows his
parents (John, Mary) and his sibling (Alice) — all derived from edges declared
elsewhere. Open the info panel (`Ctrl+g`) to see relationships grouped by
predicate, with edge attributes like *married 1948-06-01 · place Denver, CO*.

## How it's exposed

- **`links.json`** (per page, loaded by the info panel): a `relationships` array
  of resolved edges alongside `inbound`/`outbound` links.
- **`site.json`**: a top-level `relationship_types` registry, plus a
  `relationships` array on each `markdown_files` entry (resolved URLs included),
  so downstream tooling can consume the whole graph.

Each resolved relationship includes: `rel_type` (as declared), `predicate`
(viewpoint label used for grouping), `neighbor` (resolved URL, empty if
unresolved), `neighbor_title`, `neighbor_raw`, `resolved`, `direction`
(`outgoing`/`incoming`), optional `label`, `attributes`, and `derived` (true
when produced by the reverse-edge rule rather than declared on this note).

## Visualizing relationships (graph / family tree)

Notes that declare a `type` also render a **relationship graph** — a family
tree for genealogy, or a general relationship graph for any other domain. This
is the `<mbr-relationships>` web component, which reuses the built-in mermaid
renderer (no extra downloads or dependencies) and draws directly from
`site.json`, so it works identically in the live server and in static builds.

Starting from the current note, the component walks outward through the
resolved edges and draws the neighbourhood it finds:

- **Parent → child** edges are drawn top-down, so ancestors sit above
  descendants (the diagram uses mermaid's `graph TD`).
- **Spouse / sibling** (and any other *symmetric* type) are drawn as
  **undirected dotted links**, labelled with the relationship (e.g. *Spouse*).
- **Other directed types** (anything not in the registry) are drawn as
  **labelled arrows** from subject to object.
- The **current note is highlighted** so you can orient yourself in the tree.
- Node labels show the person's title and, when present, their lifespan from
  the `born`/`died` frontmatter (e.g. *John Doe (1925–1999)*).
- Nodes are **tinted by `gender`** when that field is set, so a family tree
  reads at a glance.

Reciprocal and derived edges are **de-duplicated**: an edge between two notes is
drawn exactly once, regardless of which note (or how many notes) declared it.

### When the graph appears

The graph is included on any note that declares a `type` frontmatter field
(`type: person`, `type: character`, `type: service`, …). It renders **nothing**
when the focused note has no resolved relationships, so simply having a `type`
never produces an empty box. Gating on `type` (rather than only `person`) keeps
the visualization available to *any* domain that models a typed graph, while
still staying out of ordinary prose notes that declare no `type`.

To restrict it further (for example to people only), override
`.mbr/_display_enhancements.html` in your repository and change the include
condition to `{% if type == "person" %}`.

### Recursion depth and size

By default the graph expands **3 relationship hops** out from the focused note.
For a typical family that is enough to show the whole tree from any person. You
can change the depth (and a safety cap on the number of nodes for very large
graphs) by setting attributes when you override the template:

```html
<!-- .mbr/_display_enhancements.html -->
{% if type %}<mbr-relationships depth="4" max-nodes="120"></mbr-relationships>{% endif %}
```

- `depth` — how many hops to expand from the focus (default `3`, clamped to
  `1`–`6`).
- `max-nodes` — hard cap on graph size for performance on huge repositories
  (default `80`).

### Couple / marriage layout limitation

mermaid flowcharts have no native concept of a "couple" node, so a married pair
is not joined by the classic horizontal marriage bar with children hanging
beneath both parents. Instead, **each parent draws its own arrow down to a
shared child**, and the marriage itself is shown as a dotted *Spouse* link
between the two. This keeps the feature dependency-free (a deliberate trade-off)
while still conveying every relationship correctly.

## Configuration

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `relationship_tracking` | bool | `true` | Enable typed relationship tracking |
| `relationship_types` | array | genealogy defaults | Relation types and their semantics/labels |

CLI: `--no-relationship-tracking` disables the feature for a single run. See the
[Configuration Reference](../../reference/configuration/#relationship-settings) and
[CLI Reference](../../reference/cli/).
