---
title: Calendar and Periodic Notes
description: Daily, weekly and monthly notes, how mbr dates a note, and stepping through them
order: 9
---

# Calendar and Periodic Notes

mbr knows two calendar things about your notes, without any setup and without
changing a file:

- **Periodic notes.** A note whose file name *is* a date or a period
  (`2026-10-31.md`, `2026-W44.md`, `2026-10.md`) is a daily, weekly, monthly,
  quarterly or yearly note. Periodic notes get a navigation bar, and the `d`,
  `[` and `]` keys step through them.
- **A note's date.** Every note has a calendar date, taken from its file name,
  its frontmatter, or when the file was created.

Both are computed when mbr scans the repository and are published in
`site.json`, so they work the same in server mode, the GUI and static builds.

## Periodic notes

A note is periodic when its **whole** file name (without the extension) is one
of these forms:

| File name | Example | Kind | Used by |
|-----------|---------|------|---------|
| `YYYY-MM-DD` | `2026-10-31.md` | Day | Obsidian daily notes, Foam, most tools |
| `YYYYMMDD` | `20261031.md` | Day | NotePlan |
| `YYYY_MM_DD` | `2026_10_31.md` | Day | Logseq journals |
| `YYYY-Www` | `2026-W44.md` | Week | Obsidian Periodic Notes, NotePlan (`w` also accepted) |
| `YYYY-MM` | `2026-10.md` | Month | |
| `YYYY-Qn` | `2026-Q4.md` | Quarter | (`q` also accepted) |
| `YYYY` | `2026.md` | Year | |

Rules:

- Digits are ASCII digits, and the year is between 1000 and 9999.
- The date must exist: `2026-02-30.md` is an ordinary note.
- Weeks are [ISO 8601](https://en.wikipedia.org/wiki/ISO_week_date) weeks:
  they start on Monday and belong to the year of their Thursday, so
  `2026-W01` runs from Monday 2025-12-29 to Sunday 2026-01-04. A week number
  must exist in that year (2026 has 53 weeks, 2025 has 52).
- Folders don't matter: `daily/2026-10-31.md`, `journals/2026_10_31.md` and
  `2026-10-31.md` at the root are all day notes. To restrict detection to some
  folders, set [`periodic_folders`](../reference/configuration/#calendar-settings).

A file name that only *starts* with a date, such as `2026-10-31 Acme sync.md` or
Jekyll's `2026-10-31-my-post.md`, is not periodic. It is still dated by its
file name (see below), so a meeting note plots on the right day without
becoming "the" daily note.

**A note named `1984.md`** (a book, say) is a year note under these rules,
wherever it is, and two such notes get a "duplicate period" warning. If that is
not what you want, limit periodic notes to your journal folder with
[`periodic_folders`](../reference/configuration/#calendar-settings)
(`periodic_folders = ["journal"]`), leave `year` out of `periods`, or turn
periodic detection off with `periods = []`: day file names still date their
notes and the calendar views keep working. `enabled = false` turns the whole
calendar off.

### Two notes for one period

If two notes name the same period (`daily/2026-10-31.md` and
`archive/20261031.md`), the first by URL is the periodic note and the other is
left out of periodic navigation. In server and GUI mode the other note shows
a "duplicate periodic note" in its [page problems](../reference/configuration/)
indicator; a static build logs a warning.

## Note dates

Every note's calendar date is resolved in this order; the first that applies
wins:

1. **The file name.** A periodic note is dated to the start of its period (a
   week note to its Monday, a month note to the 1st). Otherwise, a file name
   that starts with `YYYY-MM-DD`, `YYYYMMDD` or `YYYY_MM_DD` followed by `-`,
   `_`, a space, `.` or `T` and more text is dated to that day.
2. **Frontmatter**, by default `date` then `created`
   ([`date_fields`](../reference/configuration/#calendar-settings) changes the
   list). These values are read:
   - `2026-10-31`
   - a timestamp, as Jekyll and Hugo write them: `2026-10-31T21:30:00-07:00`,
     `2026-10-31 21:30:00 -0700`, `2026-10-31T21:30Z`. **The date is taken as
     written and the time zone ignored**: a post written late on the 31st
     belongs to the 31st, even if it was already the 1st in UTC.
   - `2026/10/31`

   A partial date (`2026-10`) or a bare year (`date: 2026`) is skipped, and
   the next field or source is tried.
3. **When the file was created** (its birth time, or its modification time
   where the file system has none), as a date in the **reader's** time zone.
   A static site therefore shows each reader their own local date, not the
   build machine's.

The first two are published in `site.json` as a `calendar` object on the
note's entry; an undated note has none:

```json
{
  "url_path": "/daily/2026-10-31/",
  "calendar": { "date": "2026-10-31", "source": "filename", "period": "2026-10-31" }
}
```

| Field | Meaning |
|-------|---------|
| `date` | The note's date, `YYYY-MM-DD` |
| `source` | `filename` or `frontmatter` |
| `period` | Only on a periodic note: its period (`2026-10-31`, `2026-W44`, `2026-10`, `2026-Q4`, `2026`) |

A periodic-note filter (`periods`, `periodic_folders`) only decides which
notes are periodic. A day file name outside it still dates the note; other
period names outside it (`1984.md` with years turned off) do not.

## Navigating periodic notes

A periodic note opens with a slim bar at the top of the page:

- **‹ previous** and **next ›**: the nearest notes of the same kind. Gaps are
  skipped, so Friday's next is Monday when there are no weekend notes.
- In the middle, the period itself (`Saturday, October 31, 2026`) and links
  **up** to the week, month, quarter and year notes that contain it, when
  those notes exist.

The bar is English and server-rendered, and its links work in static builds.
It is the `_calendar_nav.html` [template partial](../customization/templates/);
a custom `.mbr/index.html` needs `{% include "_calendar_nav.html" %}` inside its
`<main>` to show it.

| Key | Action |
|-----|--------|
| `d` | Today's daily note (by your local date), else the most recent one before today, else the earliest one. A short message says so when there are no daily notes |
| `[` | The previous periodic note of the same kind (follows the bar's ‹ link). Does nothing on other pages |
| `]` | The next periodic note of the same kind |

Turn all of this off with `[calendar] enabled = false`; notes keep their
dates.

In the iOS app, where keys are rare, the ⋯ menu has the same entry points:
**Today's Note** (what `d` does, listed once the vault has a daily note),
**Calendar** (the overlay, as `c`) and **Day Panel** (the companion drawer, as
`C`; not listed while the companion is docked or with `companion = "off"`).

## The daily-note companion

A day note opens with a **companion** column on the right of the page:

- A **mini month**. Days with a daily note are shaded, and up to three dots
  show how many other notes are dated that day. Today has a ring; the day
  being shown is filled. The week-number column (ISO weeks, on when the
  repository has week notes) links to week notes, and the month name to the
  month note. ‹ › page through months and **Today** returns to today.
- The **day agenda**: the shown day's agenda entries (below) as a timeline,
  from 8:00 to 18:00 or wider when the day runs longer. Entries that overlap
  sit side by side; one without an end time is drawn as half an hour with a
  dashed edge. On today a red line marks the current time.
- **Notes from this day**: every other note dated that day (meeting notes,
  journal entries, drafts), grouped by `type` with meetings first, each with
  its folder.

Clicking a day in the mini month shows that day without leaving the page; its
agenda is read from that day's note. Double-click it (or press Enter) to open
the note. Clicking an agenda entry scrolls to it on this page, or opens the
other day's note at that entry.

The column needs room: it docks at 1280px and wider, or 1440px when the file
browser sidebar is docked as well. Below that, and on any other note, the
companion opens as a drawer from the calendar button (bottom right, shown on
periodic notes and on notes with agenda entries) or with `C`.

On a note that is not a day note but has agenda entries (a log of
`07:30 BP 118/76` lines, say), the companion starts with **Times in this
note**: that note's own entries, with no date claimed for them.

| Key | Action |
|-----|--------|
| `C` (Shift+c) | Docked: move into the companion; pressed there, dismiss it (remembered in this browser, per repository, until `C` brings it back). Elsewhere: open or close the drawer |
| Arrows or `h` `j` `k` `l` | Move between days in the mini month |
| PageUp / PageDown | Previous / next month |
| Space | Show the focused day |
| Enter | Open the focused day's note |
| `t` | Today |
| Esc | Leave the companion (a drawer closes) |

### Task markers

With the [task browser](tasks/) available (server and GUI, `tasks_enabled`),
the mini month also marks days with open tasks: a hollow ring, red on a day in
the past that still has an open task **due** that day (a scheduled date is a
plan to start, not a deadline, so a past day of scheduled or undated tasks keeps
a plain ring). A day's count is its open tasks with a due, scheduled or done date
that day (as the [task grids](tasks/#week-and-month-grids) place them), plus the
open tasks with no date at all in that day's daily note. The agenda header then
says how many: "3 tasks due".

The counts are asked for once per month shown, at idle, and kept for the life
of the page; completing a task anywhere on the page refreshes them. A static
build has no task index, so it shows no rings and makes no request.

`[calendar] companion = "on-demand"` never docks it (the drawer and `C` still
work), and `"off"` removes the companion entirely. The ✕ in its header
dismisses the docked column the same way `C` does.

The companion is part of the calendar chunk, which loads only on periodic
notes and notes with agenda entries (or when asked), and works in static
builds. A custom `.mbr/index.html` gets the docked column from the
`mbr-has-companion` class mbr adds to `{{ style }}` on day notes.

## Agenda entries

A **top-level** list item whose text starts with a time or a time range is an
agenda entry, in any note. Nested bullets under it are its details (who,
where), shown under the entry in the companion:

```markdown
## Calendar

* 09:00 - 09:15 Standup
    * Employees, Bob Wall, Patrick Walsh
    * Slack call / Google Hangout
* 10:30 - 11:00 Bob / Patrick 1:1
    * Patrick Walsh, Bob Wall
    * Hangouts
* 14:00 - 15:00 Things we did / Rev team demos / experiment results / kudos
```

The times mbr understands, case-insensitively:

| Form | Examples |
|------|----------|
| 24-hour | `09:00`, `9:00`, `14:30`, `09.00` (Day Planner) |
| 12-hour | `9am`, `9 am`, `9:30pm`, `9:30 p.m.` |
| Words | `noon`, `midnight` |
| Ranges | `09:00 - 10:00`, `9:00–10:00`, `9am to 10:30am`, `5:00pm-6pm` |
| A leading word | `at 3pm Call the bank`, `from 9am to 10am Workshop` |

After the time comes a space, or `:` or a dash and a space, and then the
title: `14:00: Lunch` and `14:00 - Lunch` both work. An end without am/pm takes
the start's (`1pm - 2:30` ends at 14:30); an end at or before the start is
ignored. Task items count too (`- [ ] 10:00 - 10:30 Wake up`), and the
companion shows their checkbox state.

Not entries: `2026 goals`, `10 reasons`, `1:1 with Bob` (minutes need two
digits), `10:30am-ish`, a time with no title, and any list nested in another
list, a quote, a table, a `<details>` or a chat block.

In the note itself each entry's time gets a subtle tint, and on today's day
note the entry in progress and the next one are marked at the left edge,
updated each minute. The tint is drawn without changing the page's text (the
CSS Custom Highlight API), so browsers without that API simply show none.
Turn it off with `[calendar] agenda_chips = false`.

## In QuickLook previews

On macOS, a [QuickLook preview](../modes/quicklook.md#day-calendar) of a note
with agenda entries shows that note's day beside it: the date with its
weekday, a mini month with the day marked (and today, when it is another
day), and the agenda timeline with details and task states, as in the
companion. On today's note the current time is marked. The day is the note's
date as above: its file name, else frontmatter `date`, else `created`, else
when the file was created. The preview reads the repository's `[calendar]`
settings (`date_fields`, `week_start`, `week_numbers`, `agenda_chips`;
`enabled = false` leaves it out). It is read-only and knows only the one file,
so there are no other days, no links and no "Notes from this day".

## The calendar overlay

Press `c` anywhere to see your notes on a calendar. It opens over the page in
one of three views:

- **Month** (the default on a wide screen): six weeks of day cells. A day with
  a daily note is shaded and its number opens the note; the other notes dated
  that day are listed under it, each with a coloured dot for its `type`. When
  a day holds more than its cell can show, the last line becomes **+N**:
  click it (or the day, or press Enter) for the whole day. The week-number
  column links to week notes.
- **Week**: seven columns, each day's notes in full (up to 60; past that a
  **+N** opens the whole day). A note whose date has a time
  (`date: 2026-10-31T14:00`) is listed at that time, after the untimed ones.
- **List** (the default on a phone): the month's notes day by day, up to 50 a
  day, then "+N more". The day list a **+N** opens shows up to 100; type in
  **Filter** to narrow a very busy day (plotting a fresh clone by modified can
  put thousands of notes on one day).

The header pages back and forth with ‹ and ›, **Today** returns to today, and
clicking the title opens a picker: step the year, pick a month, or type
`2026-10`, `2026-10-31`, `2026-W44`, `2026-Q4` or `2026` (`w` and `q` in
either case). Links beside the
title open the month, quarter and year notes (and the week note in Week view)
when they exist. A dot's colour is fixed per type, so `meeting` is the same
colour on every page and in every repository.

**Plot by** chooses which date places a note: its **note date** (see
[Note dates](#note-dates); week, month, quarter and year notes are reached
through their links rather than drawn on their first day), when it was last
**modified**, or when it was **created**. Plotting by modified shows when you
worked on things, long-lived notes included.

**Filter** narrows what is shown as you type. Every word must match the title,
path, `type` or a tag; `#project` matches tags starting with `project`, and
`type:meeting` matches the type exactly. When a month has nothing that matches,
links jump to the nearest month that does. The view, plot-by and filter stay
as you left them while you move between months, and until you leave the page.

What it shows depends on where you opened it:

| Opened from | Notes shown |
|-------------|-------------|
| `c` | All notes. **This folder** narrows to the folder of the page you were on (subfolders included) |
| The calendar button in the file browser's list | The selected folder (with its subfolders), tag or other selection, all of it, not only the page shown |
| **Calendar…** in the file browser | All notes |
| **Show in calendar** in search | The search results. A search of only date words (`when:-30d`) is rebuilt in full; otherwise it is the results listed |

The chip naming the selection has a ✕ that widens it to all notes.

| Key | Action |
|-----|--------|
| Arrows or `h` `j` `k` `l` | Move between days (entries in List) |
| `[` / `]`, PageUp / PageDown | Previous / next month (week in Week view) |
| `t` | Today |
| `m` / `w` / `a` | Month / Week / List view |
| `g` | Open the jump picker |
| `p` | Next plot-by |
| `/` | Filter |
| Enter | Open the day's only note, else list the day |
| `?` | These keys |
| Esc | Close the day list or picker, then the calendar |

The overlay reads only the site index, so it works the same in static builds.

### In the file browser

When the repository has periodic notes, the file browser (`-`) and the docked
sidebar start with a **Calendar** section: **Today**, **This week**, **This
month**, **This quarter** and **This year** link to the current periodic note
of each kind the repository has (or, when there is none yet, to the latest
one, saying so), followed by the last three daily notes and **Calendar…**,
which opens the overlay.

## Searching by date

Search understands dates: `when:-7d` finds notes dated, or changed, in the
last week, `date:2026-10` notes dated in October 2026, and `created:` and
`modified:` read the file's own times. Ranges (`2026-10-01..2026-10-15`),
comparisons (`>=2026-Q3`) and words like `this-week` work too, in server mode
and in static builds, and the search dialog's **When** menu writes the common
ones for you. See [Search by date](../modes/server/#search-by-date).

## Compatibility notes

| Tool | What mbr reads |
|------|----------------|
| Obsidian (core Daily Notes, Periodic Notes) | `YYYY-MM-DD`, `YYYY-Www`, `YYYY-MM`, `YYYY-Qn`, `YYYY` file names |
| NotePlan | `YYYYMMDD`, `YYYY-Www`, `YYYY-MM`, `YYYY-Qn`, `YYYY` file names |
| Logseq | `journals/YYYY_MM_DD.md` |
| Foam | `journal/YYYY-MM-DD.md` |
| Jekyll | `_posts/YYYY-MM-DD-slug.md` (dated, not periodic) and frontmatter `date` |
| Hugo | frontmatter `date` |

Custom formats (`DD-MM-YYYY`, `October 31, 2026`) and Zettelkasten timestamps
(`202610311230 Title`) are not recognized.

See [Calendar Settings](../reference/configuration/#calendar-settings) for every
option.
