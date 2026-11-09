---
title: Documentation Reorganization Proposal
description: A new structure for docs/ that separates markdown syntax from features, splits the long pages, and adds iOS and agenda docs
date: 2026-11-08
---

# Documentation Reorganization Proposal

## Status (2026-11-08): implemented

Done as proposed, with these differences:

- **`benchmarks/` stayed at the top level.** Its page fetches `data.json` from
  its own folder and `scripts/save-benchmarks.sh` writes there; the reference
  index links to it.
- **The desktop screenshot fixture is `scripts/docs-screenshots/fixture/`**, not
  `docs/.screenshots/`: the iOS app bundles `docs/` as its sample vault, and the
  fixture does not belong in it. `scripts/docs-screenshots.sh` takes every
  browser shot from it.
- **No "moved" stub pages** at the old URLs.
- **iOS distribution:** build from source only, for now.
- **`apple/ios/README.md` keeps its Build section** (short version) and points
  to `docs/reference/ios-development.md` for the full guide.
- **Screenshots taken:** 22 desktop (calendar, agenda, tasks and grids, search,
  links, info-panel graph, page problems, sidebar, tags, outline style,
  flashcards, relationship charts, properties editor) and 8 iOS (page, vault
  list, editor, tasks, flashcards, settings, downloading, offline), plus the
  UI-test shots (menus, native search, edge swipe). QuickLook and GUI shots
  are by hand (the checklist is in `docs/reference/development.md`), and
  editor-new-note, editor-move, review-export, slides speaker view and the
  priority-3 list are not done yet.

## Summary

- **`markdown/` becomes syntax only**: what you type in a note. Features move
  to a new `features/` section.
- **The 776-line `markdown/index.md` becomes an overview page plus about a
  dozen syntax pages.** The overview still lists every construct on one page.
- **Pages that aren't modes leave `modes/`.** Editing, properties and review
  notes move to `features/`. The sidebar, search and link explorer come out of
  `modes/server.md`, because they work in every mode. Server endpoints move to
  `reference/http-api.md`.
- **New pages:** day-agenda syntax (`* 9:00 - 10:00 Standup`), the iOS app (use
  and build), links/wikilinks, and a frontmatter page that also covers tag
  syntax.
- **New screenshots:** 12 iOS shots and about 45 desktop shots, in three
  priority tiers, each assigned to a page. The rule: take a screenshot only of
  what a reader of the docs site can't see by scrolling.

## Problems with today's layout

| Problem | Where |
|---------|-------|
| Features are filed as "Markdown Extensions" | Slides, flashcards, kanban, contacts, relationships, page styles, calendar and the task browser are all in `markdown/` |
| One page holds most of the real syntax | `markdown/index.md`: frontmatter, alerts, tasks, chat, pull quotes, marginalia, mermaid, math (127 lines), code, tables, footnotes, definition lists (94 lines), anchors, sections, autolinks |
| Syntax is buried in feature pages | Agenda entries are a section inside `markdown/calendar.md`; `TODO:` incomplete markers are inside the task *browser* page; tag syntax is in `reference/tags.md` |
| Task syntax is split in two | Checkboxes and dates in `markdown/index.md`, incomplete markers in `markdown/tasks.md`, each repeating part of the other |
| `modes/` holds features | `editing.md`, `editing-frontmatter.md`, `review.md` |
| `modes/server.md` (630 lines) documents everything | The sidebar, search and link explorer (which also work in GUI and static builds), plus 171 lines of HTTP endpoints |
| No iOS docs | Only `apple/ios/README.md` (developer notes) and `TESTING.md` |
| Files filed in odd places | `releasing.md` at the top level; `markdown/test-slides.md` is an example deck with a test-sounding name |

## Proposed tree

```
docs/
├── index.md                         Home: what mbr is, quick links (rewritten table)
├── getting-started/
│   ├── index.md                     Install (adds an "iPhone and iPad" line)
│   └── quickstart.md
│
├── markdown/                        MARKDOWN SYNTAX: only what you type
│   ├── index.md                     Overview: every construct, one line each, links out.
│   │                                Keeps the small standard ones inline: tables,
│   │                                strikethrough, smart punctuation, autolinks,
│   │                                heading anchors + {#id}
│   ├── frontmatter.md               YAML frontmatter, recognized fields, tags syntax,
│   │                                `style` / `type` as fields (links to features)
│   ├── links.md                     NEW: relative links, trailing-slash URLs,
│   │                                extension-less links, wikilinks + resolution order
│   │                                + ambiguity, links to viewer pages
│   ├── alerts.md                    GitHub alerts, remark-hint shorthand
│   ├── tasks.md                     Checkbox markers, annotations, date formats
│   │                                (Obsidian Tasks, Dataview, org planning lines),
│   │                                move markers, TODO:/FIXME incomplete markers
│   ├── agenda.md                    NEW: day-calendar entries (`* 9:00 - 10:00 Event`)
│   ├── definition-lists.md          FAQ rendering, blank-line rule, printing, restyling
│   ├── math.md                      KaTeX inline/display, math fences, AsciiMath
│   ├── diagrams.md                  Mermaid: types, layout and look
│   ├── code.md                      Syntax highlighting, languages
│   ├── asides.md                    Pull quotes, marginalia, footnotes
│   ├── sections.md                  `--- {#id .class key=value}`, the first section
│   ├── media.md                     Image syntax for video/audio/PDF, vid shortcode,
│   │                                YouTube / Giphy / OpenGraph auto-embed, paths
│   └── chat.md                      ```chat blocks
│
├── features/                        WHAT MBR DOES WITH YOUR NOTES
│   ├── index.md                     Feature map + "works in" matrix (GUI/server/static/iOS/QuickLook)
│   ├── browsing.md                  Sidebar, folders, recent, pinned, breadcrumbs, next/prev
│   ├── search.md                    Full-text search, query syntax, facets, date search
│   ├── links.md                     Link explorer (f/F/T), backlinks, info panel, link graph
│   ├── tags.md                      Tag pages, hierarchy, static-build tag pages
│   ├── calendar.md                  Periodic notes, note dates, d/[/], day companion, overlay
│   ├── tasks.md                     The task browser (syntax lives in markdown/tasks.md)
│   ├── slides.md                    Presentations (+ deck settings)
│   ├── slides-example.md            Today's test-slides.md, renamed
│   ├── flashcards.md
│   ├── kanban.md
│   ├── page-styles.md               `style` / `type` body classes, outline style
│   ├── contacts.md                  Contact card, labeled fields, privacy
│   ├── relationships.md             Typed relationships, genealogy, org/family charts
│   ├── editing.md                   ← modes/editing.md
│   ├── editing-properties.md        ← modes/editing-frontmatter.md
│   └── review.md                    ← modes/review.md
│
├── modes/                           HOW YOU RUN MBR
│   ├── index.md                     Comparison table gains iOS and QuickLook columns
│   ├── gui.md
│   ├── server.md                    Trimmed to starting/configuring/troubleshooting
│   ├── build.md
│   ├── quicklook.md
│   └── ios.md                       NEW: the iPhone/iPad app, for users
│
├── customization/                   (unchanged)
│   ├── index.md
│   ├── themes.md
│   ├── templates.md
│   ├── components.md
│   └── sorting.md
│
└── reference/
    ├── index.md
    ├── cli.md
    ├── configuration.md             Kept as one page (see below)
    ├── keyboard-shortcuts.md
    ├── http-api.md                  NEW: endpoints from modes/server.md
    ├── integration.md
    ├── architecture.md
    ├── development.md               Gains "Building the iOS app" (or links to it)
    ├── ios-development.md           NEW: build, bindings, self-test, screenshots
    ├── benchmarks.md                ← benchmarks/index.md (data.json moves beside it)
    └── releasing.md                 ← releasing.md
```

Sidebar order comes from `order:` frontmatter, which `.mbr/config.toml` already
sorts by. Proposed top-level order: Getting Started (1), Modes (2), Markdown
Syntax (3), Features (4), Customization (5), Reference (6).

## Page-by-page moves

### From `markdown/index.md`

| Section | Lines | Goes to |
|---------|------:|---------|
| Standard Extensions table | 23 | `markdown/index.md` (stays; wikilinks row links to `links.md`) |
| YAML Frontmatter, Quick navigation | 59 | `markdown/frontmatter.md` |
| GitHub-style Alerts | 46 | `markdown/alerts.md` |
| Tasks | 127 | `markdown/tasks.md` |
| Chat Transcripts (stub) | 13 | removed; overview links to `markdown/chat.md` |
| Pull Quotes, Marginalia | 35 | `markdown/asides.md` |
| Mermaid Diagrams | 53 | `markdown/diagrams.md` |
| Math with KaTeX | 127 | `markdown/math.md` |
| Syntax Highlighting | 30 | `markdown/code.md` |
| Tables | 16 | stays on the overview |
| Footnotes | 18 | `markdown/asides.md` |
| Definition Lists | 94 | `markdown/definition-lists.md` |
| Heading Anchors | 14 | stays on the overview |
| Section Attributes | 79 | `markdown/sections.md` |
| Auto-linking | 9 | stays on the overview |

### Other moves

| From | To | Change |
|------|----|--------|
| `markdown/tasks.md` (browser) | `features/tasks.md` | "Incomplete markers" syntax half → `markdown/tasks.md`; keeps the panel behaviour |
| `markdown/calendar.md` | `features/calendar.md` | "Agenda entries" → `markdown/agenda.md`; "Searching by date" → `features/search.md`; companion/overlay stay |
| `markdown/slides.md` | `features/slides.md` | none |
| `markdown/test-slides.md` | `features/slides-example.md` | retitled "Slides Example" (already its title) |
| `markdown/flashcards.md` | `features/flashcards.md` | "Writing cards" stays here, linking to `markdown/definition-lists.md` |
| `markdown/kanban.md` | `features/kanban.md` | none |
| `markdown/styles.md` | `features/page-styles.md` | none |
| `markdown/contacts.md` | `features/contacts.md` | Dedupe "Person fields" with relationships: contacts owns fields and card |
| `markdown/relationships.md` | `features/relationships.md` | Owns `relationships:`, inverse pairs, charts; drops person fields (link to contacts) |
| `markdown/media.md` | `markdown/media.md` | stays; "Floating media" and troubleshooting stay too |
| `markdown/chat.md` | `markdown/chat.md` | stays |
| `modes/editing*.md`, `modes/review.md` | `features/` | none |
| `modes/server.md` § File Browser Sidebar, Directory Browsing, Recent Files | `features/browsing.md` | |
| `modes/server.md` § Full-Text Search | `features/search.md` | |
| `modes/server.md` § Link Explorer, Link Tracking | `features/links.md` | Plus the info panel (Ctrl-g), now only in shortcuts and contacts |
| `modes/server.md` § Server Endpoints | `reference/http-api.md` | |
| `reference/tags.md` | `features/tags.md` + `markdown/frontmatter.md` | Syntax (YAML forms, wikilink tags) → frontmatter; pages/config/templates → features |
| `benchmarks/`, `releasing.md` | `reference/` | |

## Splitting decisions

- **Split** wherever a section is something a reader would search for by name
  and link to: math, definition lists, sections, diagrams, alerts, tasks,
  agenda. Pages of 40–130 lines each.
- **Group the small ones**: pull quotes + marginalia + footnotes as `asides.md`;
  tables, strikethrough, smart punctuation, autolinks and anchors stay on the
  overview. A page of ten lines is a click with nothing behind it.
- **Keep `reference/configuration.md` as one page** (1,770 lines). It is a
  lookup page, read with find-in-page and anchors, and CLAUDE.md names it as
  the one file to update on every config change. Instead, every feature page
  ends with a short **Configuration** section that lists its keys and links
  to the anchors.
- **Keep contacts and relationships apart**: they are used apart (an org chart
  needs no genealogy; a family tree needs no phone numbers). Remove the
  overlap so each topic has one owner.
- **Keep the `markdown/` folder name** for syntax. `/markdown/` and the anchors
  on syntax that stays there keep working; only features change URL.

## A common shape for pages

Feature pages:

1. One paragraph: what it is, with a screenshot.
2. **Works in**: a one-line badge row (GUI · Server · Static · iOS · QuickLook).
3. How to use it (keys, buttons).
4. The syntax it reads, as a link to `markdown/…`, not a copy.
5. Configuration (keys + links into `reference/configuration.md`).
6. See also.

Syntax pages:

1. The syntax, in a code block.
2. A live example (the docs site renders itself).
3. Rules and edge cases.
4. Where it shows up (which features read it).
5. Compatibility with other tools (Obsidian, GitHub, Dataview…).

## New page: `markdown/agenda.md`

Agenda entries are documented today, but only as a section in the middle of
the calendar page. The new page:

- **Title:** Day Agenda Entries. **Description:** Time-stamped list items that
  become a day timeline.
- **The syntax**: a top-level list item starting with a time or range; nested
  bullets are details:

  ```markdown
  * 09:00 - 09:15 Standup
      * Employees, Bob Wall, Patrick Walsh
      * Slack call
  * 10:30 - 11:00 Bob / Patrick 1:1
  * 2pm Dentist
  - [ ] 16:00 - 16:30 Write the weekly update
  ```

- **Time forms** table (24-hour, `09.00`, am/pm, `noon`/`midnight`, ranges with
  `-`/`–`/`to`, a leading `at`/`from`).
- **Separators** after the time (space, `:`, ` - `); end inherits am/pm; an end
  at or before the start is ignored.
- **Task items** as entries (checkbox state shown).
- **Not entries**, each with its reason: `2026 goals`, `10 reasons`,
  `1:1 with Bob`, `10:30am-ish`, a time with no title, nested lists, quotes,
  tables, `<details>`, chat blocks.
- **Where they show up**: time tint in the note (Custom Highlight API), now/next
  marker on today's day note, the day companion's timeline (with a screenshot),
  other days' agendas in the companion, QuickLook's day calendar, deep links
  `#mbr-agenda-N`.
- **Any note, not only daily notes**: how a note gets its date (link to
  `features/calendar.md#note-dates`).
- **Compatibility**: Obsidian Day Planner (`09.00`), plain bullet journals.
- **Configuration**: `[calendar] agenda_chips`, `companion`.

`features/calendar.md` keeps a three-line summary that links here.

## New page: `modes/ios.md` (users)

**MBR for iPhone and iPad** — browse a notes folder from iCloud Drive, Dropbox
or On My iPhone with the desktop's renderer.

1. **What it is**: the same renderer and front end as desktop; no server, no
   network; iOS/iPadOS 26.
2. **Getting it**: TestFlight / build from source (link to
   `reference/ios-development.md`).
3. **Opening a vault**: pick a folder, the sample vault, opening one file from
   Files/Share (and "Grant folder access" to see its neighbours), switching
   vaults.
4. **Finding your way**: the native bar (title, breadcrumb menu, ⋯ actions,
   `+` new note, Back with long-press history), edge swipes (left: Browse,
   right: Info), native search.
5. **What works**: a matrix from `features/index.md` — editing, tasks,
   flashcards with spaced repetition, slides (full screen), calendar actions
   (Today's Note, Calendar, Day Panel), review notes, `.mbr/` customization.
6. **iCloud and other providers**: not-yet-downloaded notes ("Downloading from
   iCloud…" page), offline, folders that fill in, Refresh, what happens when a
   note changes on another device.
7. **Staying fresh**: no live reload; Refresh menu, returning to the app,
   automatic reload of the page on screen.
8. **Customization**: `.mbr/config.toml`, templates and CSS work; a broken
   config shows a "Configuration ignored" notice instead of failing.
9. **Privacy and storage**: the index cache in the app's own Caches, nothing
   written to your notes folder except your edits.
10. **Troubleshooting**: empty folders on iCloud, stuck "Downloading", config
    notice.

## New page: `reference/ios-development.md` (builders)

Built from `apple/ios/README.md`'s Build section, rewritten for someone who has
never built it. `apple/ios/README.md` then links here for builds and keeps the
internals (layout, bridge, mobile API).

1. **Requirements**: macOS, Xcode with an iOS simulator runtime
   (`xcodebuild -downloadPlatform iOS`), the nix dev shell (iOS Rust std is in
   `rustToolchainDev`; `build.sh` re-execs under `nix develop` if missing),
   `xcodegen`.
2. **Quick start**:

   ```bash
   cd apple/ios
   ./build.sh run      # build everything, boot "iPhone 17", install, launch
   ```

3. **All `build.sh` modes**: default, `run`, `device`, `rust`, `uitest`,
   `MBR_IOS_DEVICE=<udid>` (register a phone once), `MBR_IOS_NO_SIGN=1`.
4. **Running on your own phone**: team and bundle id (`X2SAY6ACE6`,
   `com.zmre.mbr.ios`) and how to change them in `project.yml` for a fork;
   Developer Mode on the phone; `devicectl` install and console logs.
5. **What the build does**: two staticlibs → UniFFI bindings → xcframework →
   XcodeGen → `xcodebuild`; where outputs land (`target/ios-derived-data/`).
6. **Regenerated bindings**: `Generated/` is checked in; `git add -f` because a
   common global gitignore pattern (`*.sw*`) matches `.swift`.
7. **Checking Rust only**: `cargo check -p mbr-ffi --features mobile --lib
   --target aarch64-apple-ios-sim`; clippy runs on the host only.
8. **Why the Xcode shims** (short; links to `design/ios-architecture-research.md`
   §12.2): never export Xcode's `DEVELOPER_DIR`/`SDKROOT` globally.
9. **Testing**: self-test modes table, `./build.sh uitest`, the device
   checklist (`apple/ios/TESTING.md`).
10. **Linting Swift**: `nix build .#swiftfmt .#swiftlint-check`.
11. **Taking screenshots** (below).
12. **Troubleshooting**: missing simulator runtime, signing errors, stale
    `Generated/`, linker errors from a globally exported SDK.

## Screenshots

### iOS (new, `docs/images/screens/ios/`)

The UI tests already print `SHOT <name>` and hold still at each state, and
`-mbrSelfTest screens` prints `SCREEN <name>`. A small script,
`apple/ios/scripts/screenshots.sh`, watches that output and runs
`xcrun simctl io <device> screenshot` at each marker, for iPhone 17 and
an iPad, in light and dark (`simctl ui <device> appearance dark`). The sample
vault is the content, so no personal notes appear.

| Shot | Source state | Used on |
|------|--------------|---------|
| `ios-page.png` (light + dark) | `screens` page | `modes/ios.md` hero, home page, README |
| `ios-ipad-page.png` | `screens` page on iPad, sidebar docked | `modes/ios.md` |
| `ios-vault-list.png` | `screens` list | Opening a vault |
| `ios-title-menu.png` | UI test `title-menu` | Finding your way (breadcrumbs) |
| `ios-more-menu.png` | UI test `more-menu` | Finding your way (actions) |
| `ios-native-search.png` | UI test `native-search` | Native search |
| `ios-swipe-browse.png` | UI test `swipe-browse` | Edge swipes |
| `ios-editor.png` | `screens` editor | What works / editing |
| `ios-tasks.png` | `screens` tasks overlay | What works |
| `ios-flashcards.png` | `screens` deck | What works |
| `ios-downloading.png` | self-test `download` | iCloud section |
| `ios-settings.png` | `screens` settings | Customization |

### Desktop: the gap audit

**Rule for choosing shots.** The docs site renders itself, so syntax with a
live example (alerts, math, mermaid, tables, definition lists, chat) needs no
screenshot. A reader of the published site already sees it. Screenshots go
where a reader **cannot** see the thing by scrolling:

- anything server/GUI-only (editing, the task browser, review notes);
- anything behind a key or button (overlays, panels, menus);
- anything outside the browser (QuickLook, Finder, the GUI's native menus, iOS);
- anything that depends on a state the docs don't have (today's daily note,
  a person page with relatives, a not-yet-downloaded note).

**What exists today**: 18 shots, on 17 of 38 pages. Pages with none: QuickLook
(its only images are example paths), GUI mode, calendar, page styles, properties
editor, tags, modes overview, getting started, customization, sorting.

The tables below list every gap, by priority.

#### Priority 1: pages that describe something visual and show nothing

| Shot | Shows | Page |
|------|-------|------|
| `quicklook-preview.png` | A note previewed with the space bar in Finder | `modes/quicklook.md` |
| `quicklook-thumbnails.png` | Finder icon view with rendered markdown thumbnails and a `cover:` thumbnail | `modes/quicklook.md` § Thumbnails |
| `quicklook-contact.png` | A person note's contact card in QuickLook | `modes/quicklook.md` § Contacts |
| `quicklook-day-calendar.png` | Day calendar header + agenda timeline in a daily-note preview | `modes/quicklook.md` § Day calendar, `markdown/agenda.md` |
| `gui-window.png` | The GUI window with its native menu bar (macOS) | `modes/gui.md` |
| `gui-find-bar.png` | Find in page with highlighted matches | `modes/gui.md` § Find in Page |
| `gui-open-picker.png` | The folder picker on launch without a path | `modes/gui.md` § Switching Directories |
| `day-companion.png` | Daily note with the docked companion: mini month, agenda timeline, notes from this day | `features/calendar.md`, `markdown/agenda.md` |
| `agenda-in-note.png` | Agenda entries in a note: time tint, now/next marker | `markdown/agenda.md` |
| `calendar-overlay-month.png` | `c` overlay, Month view, plotted by note date | `features/calendar.md` |
| `calendar-overlay-week.png` | Week view with timed entries | `features/calendar.md` |
| `periodic-nav.png` | The prev/next/up bar at the top of a weekly note | `features/calendar.md` § Navigating |
| `properties-form.png` | The frontmatter form: type, tags chips, a date | `features/editing-properties.md` |
| `properties-relationships.png` | Relationship rows with the sentence under them | `features/editing-properties.md` § Relationships |
| `properties-yaml.png` | The YAML tab with completion open | `features/editing-properties.md` § YAML tab |
| `page-style-outline.png` | A page in `style: outline` (numbered headings) | `features/page-styles.md` |
| `sidebar-docked.png` | Docked sidebar: shortcuts, recent, tags, folders, calendar section | `features/browsing.md` |
| `tag-page.png` | A tag page listing its notes, with the tag tree | `features/tags.md` |

#### Priority 2: features shown once but with an important second state

| Shot | Shows | Page |
|------|-------|------|
| `task-grid-week.png` | Task browser, By Due → Week | `features/tasks.md` § By Due |
| `task-grid-month.png` | By Due → Month, with a day's popover open | `features/tasks.md` |
| `tasks-in-note.png` | Tasks in a note: due/done chips, priority, a cancelled task, a `TODO:` wash | `markdown/tasks.md` (chips need a toggle to show state, which the static site can't) |
| `link-explorer.png` | `f` / `F` fuzzy nav with outbound and inbound tabs | `features/links.md` |
| `mini-graph-modal.png` | The link graph expanded from the info panel | `features/links.md` |
| `timeline-tree.png` | Relationship chart: Timeline tree | `features/relationships.md` |
| `all-people-graph.png` | Relationship chart: All people | `features/relationships.md` |
| `search-date.png` | Search with the When select and `matched_date` labels | `features/search.md` § Date search |
| `search-folder-picker.png` | Search scoped with the folder picker | `features/search.md` |
| `flashcards-reading.png` | A deck page in reading view: progress pies and "Reviewed 4×" summaries | `features/flashcards.md` § Progress |
| `flashcards-fsrs.png` | Back of a card with the four rating buttons | `features/flashcards.md` § Spaced repetition |
| `review-export.png` | Review panel with the export preview | `features/review.md` § Export |
| `editor-new-note.png` | ⋯ → New note folder picker | `features/editing.md` § Starting a new note |
| `editor-move.png` | Rename / Move picker | `features/editing.md` § Moving |
| `page-errors.png` | The page-problems panel listing a broken link and an ambiguous wikilink | `features/links.md` or `reference/…` (wherever "Data problems" lands) |
| `slides-speaker-view.png` | Speaker view with notes | `features/slides.md` § Speaker Notes |

#### Priority 3: helpful, not urgent

| Shot | Shows | Page |
|------|-------|------|
| `modes-side-by-side.png` | GUI, server in a browser, and a static build next to each other | `modes/index.md` |
| `home-listing.png` | Home / section listing with subfolders and recent notes | `features/browsing.md` |
| `media-viewer-pdf.png` | `/.mbr/pdfs/?path=…` viewer page | `markdown/media.md` § Linking to a viewer page |
| `video-transcript.png` | Video with the interactive transcript open | `markdown/media.md` § Interactive Transcript |
| `pdf-embed.png` | A PDF embed with cover image | `markdown/media.md` § PDF |
| `opengraph-card.png` | A bare link enriched with OpenGraph | `markdown/media.md` (the docs build has oembed on, but at 300 ms it may fall back) |
| `narrow-layout.png` | A page at 390px: collapsed breadcrumbs, ▶ FAB | `features/browsing.md` |
| `custom-theme.png` | Before/after a `.mbr/user.css` override | `customization/index.md` |
| `custom-template.png` | A template override (custom footer) | `customization/templates.md` |
| `dark-mode.png` | Pairs with existing `hero-light.png` where a dark mode is discussed | `customization/themes.md` § Dark Mode |

Existing shots move with their pages, no renames.

**Making them.** The existing shots (#348) were taken by hand. For
repeatability, add `scripts/docs-screenshots.sh`, which:

- serves a fixture repository, `docs/.screenshots/` (hidden so the docs build
  skips it), holding a dated daily note with agenda entries, a deck with a
  history, a person with relatives, tasks with due dates, and a long note;
- drives the pages with `agent-browser` (already used by `check-overflow.sh`) at
  1280×800, light and dark, opening overlays by key;
- pins the date with the `today` the client sends where it can, so the calendar
  shots are stable.

QuickLook, Finder and GUI shots stay manual, with a checklist in
`reference/development.md` that names each shot and how to stage it. The overflow script
(`components/scripts/check-overflow.sh`) runs over the reorganized site
afterwards, since new pages bring new tables.

## Home page and README

- `docs/index.md` Quick Links becomes four groups (Syntax, Features, Modes,
  Reference) instead of one 18-row table, and "Navigation Features" links into
  `features/`.
- The root `README.md`'s doc links are updated, and it gets one iOS screenshot.

## Things outside `docs/` that change

| File | Change |
|------|--------|
| `apple/ios/MBRMobile/SelfTest+Scenarios.swift:287` | Loads `/markdown/flashcards/` → `/features/flashcards/` |
| `apple/ios/README.md` | Build section shrinks to a link to `reference/ios-development.md` |
| `CLAUDE.md` | Doc paths in "When to Update Documentation" (`docs/markdown/styles.md`, `kanban.md`, …) and a new rule: syntax changes update `docs/markdown/`, feature changes `docs/features/` |
| `components/src/frontmatter/fields.ts`, `mbr-widgets.ts`, `components/src/flashcards/concentric.ts`, `crates/mbr-core/templates/theme.css` | Comments naming `docs/markdown/*.md` |
| `README.md` | Doc links |
| `tests/common/mod.rs`, `crates/mbr-core/src/link_index.rs` | No change: they build their own fixture repos |

## Verification

- `cargo test --test docs_anchors`: every `#fragment` in `docs/` lands on an
  emitted id (catches renamed anchors across the moves).
- `mbr -b --fail-on-broken-links docs`: no broken page links.
- A grep for each moved path over the repository, outside `docs/`.
- Read-through in the GUI at 1280px and 390px for the sidebar order.

## Order of work

1. Moves only (`git mv`), with link fixes. One commit, so history follows.
2. Split `markdown/index.md` and the task/calendar syntax halves.
3. New pages: agenda, links, frontmatter, `features/index.md`, `http-api.md`.
4. iOS pages and the screenshot script; take the shots.
5. Desktop screenshots (priority 1, then 2), home page, README, CLAUDE.md.
   Priority 3 can follow later.
6. Verification pass.

## Open questions

- **Old URLs**: the published docs site's `/markdown/slides/` and similar would
  404. mbr has no redirect mechanism for static builds. One option is to keep a
  tiny stub page at each old path ("Moved to …"), which costs 9 files.
- **iOS distribution**: is there a TestFlight link to give, or is "build from
  source" the only route for now?
- **`syntax/` vs `markdown/`**: this proposal keeps `markdown/` for URL
  stability; `syntax/` reads better in the sidebar if URL churn is acceptable
  (the sidebar title can say "Markdown Syntax" either way).
