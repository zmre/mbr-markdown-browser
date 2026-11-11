---
title: Links, Backlinks and the Info Panel
description: The link explorer (f, F, T), backlinks, the info panel and its link graph
order: 3
---

# Links, Backlinks and the Info Panel

**Works in:** GUI · Server · Static site · iOS

mbr keeps track of which notes link to which, in both directions, with no
special syntax: ordinary markdown links and [wikilinks](../markdown/links.md)
both count.

## Link Explorer

![The link explorer over a note, on its Links Out tab, with Links In and ToC tabs and a filter field](../images/screens/link-explorer.png)

*The link explorer (`f`): links out, links in and the table of contents.*

Press **`f`**, **`F`**, or **`T`** to open the Link Explorer - a quick navigation modal for jumping between pages and headings.

### Tabs

| Key | Tab | Description |
|-----|-----|-------------|
| `f` | **Links Out** | All links from the current page to other pages |
| `F` | **Links In** | Backlinks - pages that link to the current page |
| `T` | **ToC** | Table of contents - all headings in the current document |

### Features

- **Fuzzy search** - Type to filter items with intelligent matching
- **Visibility awareness** - In ToC mode, currently visible headings are prioritized and marked
- **Internal vs External** - External links open in new tabs, internal links navigate in place. In [GUI mode](../modes/gui.md#external-links) there are no tabs, so external links go to your default browser instead
- **Link handling is your browser's** - Server mode never touches external links itself; see [External links](../modes/server/#external-links)
- **Tab cycling** - Press `Tab` to cycle between the three views

### Layout

```
┌──────────────────────────────────────────┐
│  [Links Out]  [Links In]  [ToC]          │
├──────────────────────────────────────────┤
│  🔍 Filter...                            │
├──────────────────────────────────────────┤
│  → Getting Started                       │
│  → Installation Guide                    │
│  → API Reference               ↗ (ext)   │
│  ← docs/intro (backlink)                 │
├──────────────────────────────────────────┤
│  ^n ^p navigate  Tab switch  Enter open  │
└──────────────────────────────────────────┘
```

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `f` | Open Links Out tab |
| `F` (Shift+f) | Open Links In (backlinks) tab |
| `T` (Shift+t) | Open Table of Contents tab |
| `Tab` | Cycle to next tab |
| `Shift+Tab` | Cycle to previous tab |
| `↑` / `↓` | Navigate items |
| `Ctrl+n` / `Ctrl+p` | Navigate items (vim-style) |
| `Ctrl+d` / `Ctrl+u` | Scroll the list |
| `Enter` | Navigate to selected item |
| `Escape` | Close the explorer |

### Link Tracking

The Links Out and Links In tabs require link tracking to be enabled (default). This is controlled by the `link_tracking` config option:

```toml
# .mbr/config.toml
link_tracking = true  # default
```

When enabled, mbr automatically tracks:
- **Outbound links** - All markdown links `[text](url)` in the current page
- **Inbound links** - All pages that link to the current page (backlinks)

This enables wiki-style bidirectional link navigation without requiring any special syntax.

## The Info Panel

![The info panel open beside a note, with the link graph at the top and the note's metadata below](../images/screens/info-panel.png)

*The info panel (`Ctrl/Cmd+G`): a mini link graph, frontmatter and document stats.*

Press **`Ctrl+G`** (`Cmd+G` on macOS), or use the info button in the header,
to open the info panel beside the page. It shows:

- a **link graph** of the page's neighbourhood: notes it links to and notes
  that link to it, two hops out by default
  ([`graph_depth`](../reference/configuration/#navigation-settings), 1–5);
  click a node to go there, or expand it to a full-size graph
- the page's **links out and in**
- its **relationships**, when it has any
  ([Relationships](relationships.md))
- its **frontmatter**, as a table
- document stats

The graph needs link tracking; with `--no-link-tracking` the panel has no
graph section.

![The link graph expanded to a dialog, centred on Mary Doe, with her family, colleagues and meeting notes two hops out](../images/screens/mini-graph-modal.png)

*The link graph, expanded from the info panel.*

## Broken links

Links whose target does not exist are listed in the page-problems panel
(the ⚠ badge in the header, server and GUI), along with broken images and
other problems; see
[Per-Page Error Indicator](../reference/configuration/#per-page-error-indicator--server---gui-only).

When a link's target is missing, the panel also offers up to three **Did you
mean** links: notes elsewhere in the site with the same name (the usual case
when a note was moved to another folder) or with a name that was extended or
shortened (`meeting` ↔ `meeting-notes`). Fix the link by hand, or follow one to
check it is the right note. Nothing is offered for a link that only lacks its
trailing slash: its target exists.

### When a page is not found

Following a link to a page that is gone shows mbr's 404 page, which suggests
where the page probably went:

- **Notes with the same name elsewhere** — the last part of the missing URL
  (`/work/project-plan/` → `project-plan`, an index note's folder name for
  `/clients/acme/index.md`) compared with every note's file name and URL,
  ignoring case and treating spaces, `-` and `_` alike. A missing PDF or image
  linked directly is looked for by its exact file name.
- **Notes whose name was extended or shortened** — `meeting` finds
  `meeting-notes` and `weekly-meeting-notes`, but never a name that merely
  contains it mid-word, and only for names of four or more characters.
- **Otherwise, its folder** — when nothing matches but the missing page's own
  folder still exists, the 404 page links to that folder and lists the notes
  directly in it. If the folder is gone too, nothing is suggested (it does not
  look further up; the page's Home button and search are there for that).

Matches that share more leading folders with the missing URL come first; at
most six are shown. Everything is computed in the browser from the site index,
so it works the same in the GUI, the server, the iOS app and a static build's
`404.html` (set [`base_url`](../reference/configuration.md#deployment-path)
when the site is published under a path, such as a GitHub project page).
A custom `.mbr/error.html` keeps working as before; it can add the suggestions
by including the [`_not_found_suggestions.html`](../customization/templates.md#404-suggestions)
partial.

![The Page Problems panel open beside a note, listing one broken internal link](../images/screens/page-errors.png)

*The page-problems panel.*
