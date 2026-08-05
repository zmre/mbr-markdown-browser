---
title: Task Browser
description: Find, filter and complete every task in your markdown repository
order: 6
---

# Task Browser

Press `t` — or click the clipboard icon in the header — to open the task
browser: a two-pane view of every task in the repository, filtered and grouped
however you need to look at them today.

The syntax it reads is documented in [Markdown Extensions](./#tasks): the four
markers, `@due(...)`, `@done(...)`, `#tag` and the `!!` / `!!!` priorities.
Nothing here requires any change to your notes — if you already write
`- [ ] something`, you already have a task list.

> [!IMPORTANT]
> The task browser is **server and GUI only** (`mbr -s`, `mbr -g`). Its index is
> built by reading live files, which a static build has no way to do, so a built
> site has no task panel and no `/.mbr/tasks` endpoint. See
> [Applicability](#why-not-in-static-builds) below.

## Layout

```
┌─ folders ────┬─ [filter field.....................] [⚙ filters] ─┐
│ ▼ 📁 Home 42 │        ( ▤ category | ▦ calendar )                │
│   ▶ docs  12 │  ┌──────────────────────────────────────────────┐ │
│   ▶ notes 30 │  │ Weekly notes            3/7 ▓▓▓░░░░          │ │
│              │  │   docs/notes                                 │ │
│              │  │  ● [ ] write the report  #work  🗓 Aug 5      │ │
│              │  │    [ ] follow up                             │ │
└──────────────┴──┴──────────────────────────────────────────────┘ │
```

**Left pane — folders.** The whole repository as a tree, each folder carrying
the number of matching tasks in it *and everything below it*. Selecting a folder
scopes the results to it and its subfolders; selecting **Home** clears the
scope. The counts ignore the folder selection on purpose, so picking one folder
never empties out the siblings you might want to switch to.

**Right pane — results.** A filter field, the two mode tabs, and the task list.

## The two modes

### Category (the default)

One heading per file, tasks in the order they appear in it. The heading is the
note's title, with its folder underneath in smaller type, and clicking it opens
that note.

The `x/y` and progress bar on the right count **every task in the file**,
including ones the current filter has hidden — so a file showing you one
matching task can still read `3/7`. That is deliberate: the number answers "how
far along is this note?", not "how much of what I am looking at is done?".

### Calendar

One heading per due-date bucket, in this order:

| Bucket | Contains | Progress bar |
|--------|----------|--------------|
| Overdue | Due on a day that has already passed | No — a backlog has no useful denominator |
| Today | Due today | Yes |
| Tomorrow | Due tomorrow | Yes |
| Upcoming | Everything later, with a subheading per date | Yes, on "Upcoming" as a whole |
| No due date | Everything undated | No |

Here the `x/y` counts everything matching your filters **except** the status
filter, so turning "Complete" on and off moves tasks in and out of the list
without moving the progress bar.

Canceled tasks are ignored entirely in calendar mode — they do not appear, do
not count, and cannot create a bucket. A canceled task has no meaningful
deadline.

## Filtering

Type in the filter field to narrow by text. Bare words match the task's text
**or** one of its tags, several words are ANDed together, and a `#tag` token
matches tags only:

| Query | Matches |
|-------|---------|
| `report` | Any task whose text contains "report", or tagged `#report…` |
| `the report` | Tasks matching **both** words |
| `#work` | Tasks tagged `#work` (prefix match, so `#wo` finds it while you type) |
| `report #work` | Both conditions |

The ⚙ button opens the rest of the filters:

- **Status** — a multi-select of Incomplete / Complete / Canceled. It starts on
  **Incomplete only**, and the last box cannot be cleared (an empty selection
  means "incomplete" to the server, which would be confusing to look at).
- **Priority** — Normal, High, Urgent. Empty means all.
- **Due** — Any, Overdue, Due today, Due tomorrow, Upcoming, or No due date.

Every filter change is a fresh query; nothing is filtered in the browser, so
the counts and the list can never disagree.

## Keyboard

The panel is driven the same way [search](../reference/keyboard-shortcuts/) is,
so it should feel familiar.

| Key | Action |
|-----|--------|
| `t` | Open the panel |
| `Ctrl+n` / `Ctrl+p`, `↓` / `↑` | Move between tasks and headings |
| `Enter` | Open the focused task in its file (or collapse/expand a focused heading) |
| `Space` | Complete ↔ reopen the focused task *(editing only)* |
| `x` | Cancel ↔ reopen the focused task *(editing only)* |
| `←` / `→` | Collapse / expand the focused group |
| `Tab` | Switch between the folder pane and the results pane |
| `Ctrl+d` / `Ctrl+u` | Scroll the active pane half a page |
| `Ctrl+f` / `Ctrl+b` | Scroll the active pane a full page |
| `Esc` | Close the filter options, then the panel |

Typing goes to the filter field, which keeps focus the whole time. `Space`,
`x`, `←` and `→` only take over once you have moved onto a task with the arrow
keys — and any keystroke in the filter field hands them back — so filtering for
"buy milk" works exactly as you would expect.

## Jumping to a task

`Enter` (or a click on the task's text) opens the note it lives in and scrolls
straight to the line, clear of the sticky header, with a brief highlight so you
can see which one you came for. The link is an ordinary fragment
(`/docs/notes/#mbr-task-42`), so it can be bookmarked, shared, or opened in a
new tab.

## Toggling a task

With [editing](../modes/editing/) enabled, tasks can be completed from either
place:

- **In the panel**: click a checkbox, press `Space`, or press `x` to cancel.
  Right-clicking a checkbox cancels too.
- **In the document**: left-click a rendered checkbox to complete or reopen it;
  right-click to cancel.

The card or checkbox moves immediately and the change is written to the file
behind it. If the write is refused, it moves back and tells you why — most
usefully when the line has changed on disk since the page was rendered, in which
case nothing is written and the panel refreshes itself.

The page does **not** reload for your own toggle. A `@done(...)` chip appears
(or disappears) in place, your scroll position and the panel's filters survive,
and on a server that requires an [edit
token](../modes/editing/#entering-the-token) the token — which is kept in memory
only — is still there for the next click.

Only the marker byte and the `@done(...)` stamp are rewritten. Indentation,
bullet style, spacing, your other annotations, the line's ending (a CRLF file
stays CRLF) and the presence or absence of a trailing newline are all preserved.
See [`tasks_stamp_done`](../reference/configuration/#task-settings) to turn the
stamp off, and [In-Browser Editing](../modes/editing/#task-toggling) for the
endpoint behind it.

Without editing enabled, the checkboxes stay inert and `Space` / `x` stay
unbound — no control appears that cannot do anything.

## Configuration

| Option | Default | Effect |
|--------|---------|--------|
| `tasks_enabled` | `true` | Turn the panel and its endpoint off (`--no-tasks`) |
| `tasks_stamp_done` | `true` | Maintain `@done(...)` when a task is completed |

See [Task Settings](../reference/configuration/#task-settings) for the details.

## Why not in static builds

Tasks are a live view. The index is built by reading the markdown files as they
are *now*, and it is kept current by the same file watcher that drives live
reload — so it has meaning only where those files exist and can change. A static
build is a snapshot; a task list frozen into one would start lying the moment
somebody ticked a box. Built pages therefore ship no task chunk at all, and the
panel's trigger renders nothing.

The task **syntax** is unaffected: priority dots, tag pills and date chips are a
markdown extension and render everywhere, static builds included.

## Cost when you do not use it

None worth measuring. The index is built lazily on the very first task query and
never at startup, holds only the files that actually contain tasks, and is then
kept fresh per-file by the watcher. A server whose user never presses `t` never
reads a file for it.

The first query on a very large repository pays one sequential pass over the
markdown; it reports `scan_in_progress` while the repository scan is still
running, and the panel shows partial results in the meantime rather than making
you wait.

## See Also

- [Task syntax](./#tasks) — markers, annotations and date formats
- [In-Browser Editing](../modes/editing/) — enabling writes
- [Keyboard Shortcuts](../reference/keyboard-shortcuts/)
- [Configuration Reference](../reference/configuration/#task-settings)
