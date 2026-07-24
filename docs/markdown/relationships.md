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
| `born` | date/string | Birth date; shown on its own line in the person infobox with a 📅 icon, humanized when it's an ISO date (e.g. `1855-10-30` → *October 30, 1855*). Other formats display as written. |
| `died` | date/string | Death date; shown on its own line beneath `born`, humanized the same way. |
| `born_place` | string | Birthplace; displayed on the person's page beneath the dates. |
| `image` | path or URL | Portrait; displayed on the person's page. |
| `gender` | string | Styles the genealogy charts (e.g. `male`, `female`) — card tinting and parent-line colors; any value is accepted. |
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

## Visualizing relationships

mbr ships two complementary visualizations. Both are lazy-loaded (they cost
nothing until used) and both work identically in server, GUI, and static-build
modes:

- **Genealogy charts** — interactive family charts rendered inline on
  `type: person` pages.
- **Link graph in the sidebar** — a force-directed mini graph of the current
  note's neighbourhood (content links *and* typed relationships), shown at the
  top of the info panel on every note.

> **Breaking change:** the old mermaid-based `<mbr-relationships>` element has
> been removed. A custom `.mbr/_display_enhancements.html` override that still
> references it silently renders nothing (unknown elements are inert), and its
> `depth` / `max-nodes` attributes are superseded by the `graph_depth` config
> option and the new elements. Mermaid diagrams in ordinary code blocks are
> unaffected.

### Genealogy charts on person pages

Notes with `type: person` render an interactive family chart — the
`<mbr-genealogy>` web component, emitted by `_display_enhancements.html` for
person pages only. It draws from the resolved relationships in `site.json` and
renders **nothing** when the person has no resolved relationships, so a person
note without edges never produces an empty box.

A selector in the top-left corner of the chart switches between two views; the
choice persists in localStorage (`mbr_genealogy_chart`):

**Family chart** (default) — built on the
[family-chart](https://github.com/donatso/family-chart) library (ISC license):

- SVG person cards with **portraits** from the `image` frontmatter field.
- Shows **ancestors and descendants two generations each** around the current
  person.
- **Expand/collapse** of non-blood branches via per-card mini-tree and
  link-break toggles.
- Native pan/zoom; clicking a card navigates to that person; `⤢` recenters.

**Timeline tree** — a custom time-aware layout:

- Ancestors above, descendants below the current person (two generations each).
- A **year axis** on the left positions each person by birth year. People
  without a `born` date fall back to their generation's median year, then to
  estimated 28-year generations; if no one has dates the axis is hidden.
- Lines are colored by parent — **blue father-lines**, **pink mother-lines**
  (gray when `gender` is unknown) — and couples are joined by **marriage bars**.
- Pan, zoom, and click-to-navigate as above.

The chart JS is a lazy chunk (`/.mbr/components/mbr-genealogy.min.js`, ~204 kB
min / ~61 kB gz). Person pages prefetch it, but it loads and renders only when
the chart scrolls near the viewport, so it never blocks page render.

**Roadmap:** the chart selector is designed for additional chart types — a
bubble map of birth places, hierarchical edge bundling across all notes, and an
ancestors/descendants sunburst are planned.

### When the charts appear

The inline chart appears only on `type: person` notes that have at least one
resolved relationship. Notes with other `type` values (`type: character`,
`type: service`, …) get no inline chart — their typed relationships appear in
the **sidebar link graph** (below) and in the info panel's textual
**Relationships** section instead.

To gate the chart differently or place it elsewhere, override
`.mbr/_display_enhancements.html` in your repository:

```html
<!-- .mbr/_display_enhancements.html -->
{% if type and type == "person" %}<mbr-genealogy></mbr-genealogy>{% endif %}
```

### Link graph in the sidebar

The info panel (`Ctrl+g` / `Cmd+g`) shows a **force-directed mini graph** at
the top: the current note plus its neighbourhood of inbound links, outbound
internal links, and typed relationships. It appears on any note that has a
`links.json` — not just typed notes — so it doubles as a visual backlink map.

- Nodes are **colored by degree**: the focus note uses the theme's primary
  color, and 1st-, 2nd-, … degree neighbours step down a ramp of the same hue.
- **Hovering** a node shows a card with the note's title and description
  (desktop only); **clicking** navigates; **dragging** repositions.
- The `⤢` button opens a **full-screen view** with pan/zoom, node labels, and a
  **depth stepper** (`1`–`5`) to widen or narrow the neighbourhood on the fly.

The default depth comes from the `graph_depth` config option (default `2`,
valid `1`–`5`, env `MBR_GRAPH_DEPTH`).

The graph is assembled **client-side** by a breadth-first walk over each
neighbour's per-page `links.json` (about 4 fetches in flight at a time, capped
at 80 nodes, cached for the session). Because it needs no dedicated server
endpoint, it works identically in **static builds**. In server mode, mbr
additionally bounds concurrent inbound-link repository scans so a burst of
`links.json` requests can't stampede the server.

The graph JS is a lazy chunk (`/.mbr/components/mbr-graph.min.js`, d3-force,
~57 kB min / ~19 kB gz) loaded only when the info panel first opens. If link
tracking is disabled (`--no-link-tracking` / `link_tracking = false`), there is
no `links.json` and the graph section simply doesn't appear.

## Configuration

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `relationship_tracking` | bool | `true` | Enable typed relationship tracking |
| `relationship_types` | array | genealogy defaults | Relation types and their semantics/labels |
| `graph_depth` | number | `2` | Default neighbourhood depth (`1`–`5`) for the sidebar link graph (env `MBR_GRAPH_DEPTH`) |

CLI: `--no-relationship-tracking` disables the feature for a single run. See the
[Configuration Reference](../../reference/configuration/#relationship-settings) and
[CLI Reference](../../reference/cli/).
