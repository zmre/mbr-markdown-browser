---
title: HTTP API
description: "The endpoints mbr's server answers: pages, site.json, search, tasks, edits and live reload"
order: 4
---

# HTTP API

Server and GUI mode serve everything through one local HTTP server. The
front end uses these endpoints, and so can your own scripts. Static builds
have only the files (`site.json` and the pages); every `POST` endpoint is
server-only.

## Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | Home page (root directory listing) |
| `/{path}/` | GET | Markdown page or directory |
| `/{path}` | GET | `301` to the canonical `/{path}/` when it names a markdown page |
| `/.mbr/site.json` | GET | Full site metadata as JSON |
| `/.mbr/media.json` | GET | Every non-markdown file with its metadata; see [Media catalog](#media-catalog) |
| `/.mbr/search` | POST | Search endpoint |
| `/.mbr/tasks` | POST | Task query for the task browser and the calendar's task marks (`404` with `--no-tasks`) |
| `/.mbr/ws/changes` | WS | WebSocket for live reload. Same-origin only. `?source=<note>` (the page's `markdown_source`) adds `content_hash` — the note's current fingerprint, as in `markdown_source_hash` — to frames about that note; frames under the template folder carry `template: true` |
| `/.mbr/outside/?path=…` | GET | Where a link out of the repository leads; see [Outside links](#outside-links) |
| `/.mbr/*` | GET | Static assets (CSS, JS, fonts) |

## Media catalog

`GET /.mbr/media.json` lists the repository's other files (images, video,
audio, PDFs, …), the catalog the media browser shows:

```json
{ "other_files": [ … ], "populated": true }
```

It waits for the initial scan, never for the slower passes after it (the
static folder listing, file sizes and times, video durations, image
dimensions, PDF page counts). While those run it answers at once with what it
has and `"populated": false`; ask again until `"populated": true`, at which
point every entry carries its metadata. The media browser refetches every
2 seconds while it is open. A static build's `.mbr/media.json` has no
`populated` field: it is complete when written.

## Task Queries

`POST /.mbr/tasks` answers the [task browser](../features/tasks/). Every field
is optional: `q`, `folder`, `statuses` (empty = open only), `priorities`,
`due`, `include`, `limit` (default 500), `today` and `mode`. `today` is the
reader's local date (`YYYY-MM-DD`), and it decides what is overdue, due today
and due tomorrow in every mode: the calendar buckets, the `due` filter, a grid
day's `overdue`, `overdue_before_range` and the grid's default `from`. The
browser always sends it, as it does for search, because only the browser knows
the reader's time zone; without it, or with a value that is not a date, the
server's own date is used. `mode` is `category`
(one group per note, the default), `calendar` (one group per due bucket), or
`grid`, which places tasks on the days of a range for the week and month grids
and the calendar companion's day marks:

```json
{ "mode": "grid", "from": "2026-10-26", "to": "2026-11-01", "q": "", "statuses": ["open", "done"] }
```

`from` and `to` are inclusive. `from` defaults to today and `to` to a week
later; reversed ends are swapped, and a range longer than 62 days is cut to 62
days from `from`. In grid mode `include` is always tasks and `due` is ignored.
The answer replaces `groups` with:

```json
{
  "from": "2026-10-26", "to": "2026-11-01",
  "days": {
    "2026-10-31": {
      "tasks": [ { "line": 12, "text": "call Bob", "status": "open", "due": "2026-10-31T09:00:00",
                   "url_path": "/projects/plan/", "path": "projects/plan.md",
                   "placed_by": "due", "time": "09:00", "note": "Plan", "...": "every other task field" } ],
      "open": 3, "done": 2, "overdue": true
    }
  },
  "undated_in_day_notes": { "2026-10-30": 4 },
  "overdue_before_range": 12,
  "folders": [ { "path": "/", "count": 41 } ],
  "total_matches": 41, "duration_ms": 3, "scan_in_progress": false
}
```

A task is placed on its done date when it is ticked and has one, else on its
due date, else on its scheduled date (`placed_by` says which); canceled tasks
and incomplete markers never are. `time` (`HH:MM`) is present only when that
date carries a time of day (`@due(2026-10-31 09:00)`); a time at the start of
the task's text (`09:00 call Bob`) still orders the day's list (timed tasks
first) but is not sent. `open` and `done` count every filter except
the status filter. `limit` caps the rows across all the days without changing
the counts, and is shared out evenly rather than spent in date order: each day
keeps its first rows up to an equal share, and what is left goes to the days
that still have more. A day can therefore list fewer `tasks` than its counts
(the task browser draws the difference as "+N more"); `total_matches` is the
number before the limit.
`overdue` marks a past day (before `today`) that still has an open task placed
by its due date; open tasks placed by their scheduled date never make a day
overdue. `undated_in_day_notes`
counts the open, undated tasks of each daily note in the range, which are never
placed. `overdue_before_range` counts open tasks due before both `from` and
today. While the index is still being built `scan_in_progress` is `true`.

## Static Files and Byte Ranges

Files that are not markdown (images, PDFs, video, audio) are served as they
are, with `Accept-Ranges: bytes`. A `Range` request gets `206 Partial Content`
and a `Content-Range` header, which is what lets a `<video>` element seek
without downloading the whole file:

```
GET /videos/demo.mp4  Range: bytes=0-99    → 206 Content-Range: bytes 0-99/52428800
GET /videos/demo.mp4  Range: bytes=1000-   → 206 (from byte 1000 to the end)
GET /videos/demo.mp4  Range: bytes=-500    → 206 (the last 500 bytes)
GET /videos/demo.mp4  Range: bytes=99999999999-  → 416 Content-Range: bytes */52428800
```

A range that cannot be satisfied, does not parse, or asks for several parts at
once is answered `416 Range Not Satisfiable`.

When mbr runs inside the iOS app, a file the cloud provider has listed but not
downloaded yet is answered `503 Service Unavailable` with a `Retry-After`
header (a note shows a "Downloading" page), and its download is requested.

## Canonical URL Redirects

Markdown pages have exactly one URL: the directory-style one with a trailing
slash (`docs/guide.md` → `/docs/guide/`). Any other spelling gets a `301`:

```
GET /docs/guide      → 301 Location: /docs/guide/
GET /docs/guide.md   → 301 Location: /docs/guide/
GET /docs            → 301 Location: /docs/          (docs/index.md)
GET /docs/index/     → 301 Location: /docs/
GET /docs/guide?x=1  → 301 Location: /docs/guide/?x=1
```

This is not cosmetic. The trailing slash decides the base a browser uses for the
page's own relative links: from `/docs/guide/` a `../other/` href reaches
`/docs/other/`, but from `/docs/guide` it reaches `/other/`. Serving the page in
place at the slashless URL would break every link *on* it, one click later.

Static files (`/images/photo.png`, `/LICENSE`), directory listings and `/.mbr/*`
routes are never redirected.

## Search API

The search endpoint accepts POST requests:

```json
{
  "q": "search query",
  "limit": 50,
  "scope": "all",
  "filetype": "markdown",
  "folder": "/docs",
  "today": "2026-11-06"
}
```

`today` is optional: the reader's local date, which relative date facets
(`when:-7d`) resolve against. Without it (or when it is not a `YYYY-MM-DD`
date) the server's local date is used.

Response:

```json
{
  "query": "search query",
  "total_matches": 42,
  "results": [
    {
      "url_path": "/docs/guide/",
      "title": "Guide",
      "description": "...",
      "tags": "...",
      "score": 95,
      "snippet": "..."
    }
  ],
  "duration_ms": 15
}
```

When the query has a [date facet](../features/search/#search-by-date), each result also carries
`matched_date`, the date it matched the first date facet on, and which of
the file's dates that was:

```json
"matched_date": { "date": "2026-10-31", "field": "date" }
```

`field` is `date` (the note date), `created` or `modified`.

Two fields appear only when they apply: `scan_in_progress: true` while the
initial repository scan is still running (results may be incomplete), and
`not_downloaded: N` when content search had to skip `N` notes whose contents
are not on the device yet (the iOS app with a cloud provider; never on the
desktop).

## Editing endpoints

The write endpoints (`/.mbr/raw`, `/.mbr/edit`, `/.mbr/task`,
`/.mbr/flashcard-review`, file creation, moves and uploads) are documented
with in-browser editing: see
[File-management endpoints](../features/editing/#file-management-endpoints),
[Task toggling](../features/editing/#task-toggling) and
[Flashcard reviews](../features/editing/#flashcard-reviews).

`/.mbr/task` and `/.mbr/flashcard-review` address a note by line. A note with
carriage-return-only line endings cannot be, and both refuse it with `422` and
`{"error": "…", "reason": "lone_carriage_return"}` — distinct from the `409` of
a stale page, since a reload cannot fix it; see
[Carriage-return-only line endings](../features/editing/#carriage-return-only-line-endings).

## Media viewers

`/.mbr/videos/`, `/.mbr/pdfs/`, `/.mbr/audio/` and `/.mbr/images/` take a
`?path=` query; see [Media Viewer Endpoints](cli.md#media-viewer-endpoints).

## Outside links

`/.mbr/outside/?path=<target>` is the page a link to a file outside the
served folder points at (`[x](~/src/app/TODO.md)`, or a `../` chain that
climbs above the root); see
[Links outside the folder](../features/links.md#links-outside-the-folder).
`path` is the target, percent-encoded:

- a `~` link is written as authored: `?path=~/src/app/TODO.md`;
- a `../` chain is written, by the server and GUI, as the absolute path it
  resolves to (`?path=/Users/me/src/app/TODO.md`), and by a static build
  relative to the repository root (`?path=../src/app/TODO.md`), so a
  published site never names a local path.

The link's own `#fragment` and `?query` are not carried. The server answers
with the same page for any `path` and never looks at it: nothing about the
target (whether it exists, what it is) is checked or reported. Static builds
ship it as `.mbr/outside/index.html`, which reads `path` in the browser.
