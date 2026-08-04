# Tasks Feature — Implementation Plan

Plan for [TASKS_SPEC.md](TASKS_SPEC.md). Decisions marked **[REVIEW]** are ones I made
that go slightly beyond the spec — veto them here before we start.

## Design decisions (confirmed)

| Decision | Choice |
|--------------------------|-------------------------------------------------------------------------|
| Index strategy | Lazy in-memory index, built on first use, kept fresh by the notify watcher |
| Toggle write | New line-patch endpoint `POST /.mbr/task` with per-line optimistic concurrency |
| Annotation rendering | `@due` / `@done` / `#tag` / `!!` render as chips + pills + dot in documents too |
| Toggle gating | Requires `edit_enabled` (`--edit` / `MBR_EDIT_ENABLED`) |

## Non-goals (this pass)

- No `TODO:`/`FIXME:` regex scanning of source files (spec defers it).
- No parent/child task rollup — nested tasks are independent, per spec.
- No task creation, editing of text, or reordering. Toggle status only.
- No full `[>] moved` semantics beyond "treat as canceled, record the date".

---

## 1. Syntax

### Task line grammar

```
^[ \t]*(?:[-*+]|\d+[.)])[ \t]+\[(?<marker>[ xX>-])\][ \t]+(?<text>.*)$
```

| Marker | Status                   |
|--------|--------------------------|
| ` `    | `Open`                   |
| `x` `X`| `Done`                   |
| `-`    | `Canceled`               |
| `>`    | `Canceled` + `moved_to`  |

### Annotations (parsed out of `text`, stripped from the display string)

| Syntax                  | Meaning | Rule |
|-------------------------|---------|------|
| `@due(<dt>)`            | Due     | `<dt>` = `YYYY-MM-DD`, optional ` HH:MM` (24h) or ` HH:MM AM/PM` |
| `@done(<dt>)`           | Completed | same datetime grammar |
| `#tag`                  | Tag     | `[A-Za-z0-9_-]+`, must follow start-of-line or whitespace (so `page.md#anchor` is not a tag) |
| `!!` / `!!!`            | High / Urgent | whitespace-delimited on both sides (or EOL), so `wow!!` is not a priority. `!!!` wins |
| `> YYYY-MM-DD` (trailing)| Moved-to date | recorded, stripped |
| `< YYYY-MM-DD` (trailing)| Moved-from date | stripped, not surfaced |

Dates are naive/local — no timezone conversion, no UTC round-trip. `@due(2026-08-05)`
with no time sorts as start-of-day and is "overdue" only after that day ends.
Assume local timezone.

**[REVIEW] Spec typo:** the spec writes `@done(YYYY-DD-MM ...)` but its own examples are
`@due(2026-08-05)`. Reading it as **YYYY-MM-DD**.

---

## 2. Rust

### 2.1 New module `src/tasks.rs` (pure parsing + index + query)

```rust
pub enum TaskStatus { Open, Done, Canceled }
pub enum TaskPriority { Normal, High, Urgent }

pub struct Task {
    pub line: u32,              // 1-based source line
    pub depth: u8,              // indent level, for subtask display
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub text: String,           // display text, annotations stripped
    pub tags: Vec<String>,
    pub due: Option<NaiveDateTime>,
    pub due_has_time: bool,
    pub done: Option<NaiveDateTime>,
    pub moved_to: Option<NaiveDate>,
}

pub struct FileTasks {
    pub url_path: String,
    pub raw_path: PathBuf,
    pub title: Option<String>,
    pub tasks: Vec<Task>,
    pub open: u32, pub done_count: u32,   // canceled excluded from both
}
```

Pure functions (unit + proptest coverage):
- `parse_task_line(&str) -> Option<ParsedTask>` — the whole grammar above.
- `scan_source_tasks(&str) -> Vec<ParsedTask>` — line scan that **skips fenced and
  indented code blocks and HTML blocks**, so `- [ ]` inside a ``` fence is not a task.
- `strip_annotations(&str) -> (String, Annotations)`.

Index (`papaya::HashMap<PathBuf, Arc<FileTasks>>`, only files that actually contain tasks):
- `TaskIndex::ensure_built(&Repo)` — one pass, `tokio::task::spawn_blocking`, sequential
  file reads mirroring `search.rs`'s deliberate no-rayon rule. Idempotent; concurrent
  callers await a single build via a `tokio::sync::OnceCell`.
- `TaskIndex::invalidate_file(path, ChangeEventType)` — hooked into the existing watcher
  reconciliation loop at `src/server.rs:1930-1970`, right next to `repo.invalidate_file`.
- `TaskIndex::query(&TaskQuery) -> TaskQueryResponse` — filter + group + count.

Perf guardrails: only markdown files; files read once with a `Vec<u8>` reuse buffer;
`limit` caps the returned task count (default 500) while group totals are computed over
the unfiltered set. Index build for a 10k-file repo is one sequential read pass — if
benchmarks show it is slow we chunk it, but it never blocks a request beyond the first.

### 2.2 Source-line → rendered-checkbox mapping

Needed so the popup can jump to a task and so in-document checkboxes know what to patch.

**RESOLVED (phase 0):** `TextMergeWithOffset` exists in pulldown-cmark 0.13.4
(`src/utils.rs:61`), and `Parser::into_offset_iter()` at `src/parse.rs:1434` yields
`(Event, Range<usize>)`. So:

- `collect_events_and_headings` switches from `TextMergeStream::new(Parser::new_ext(..))`
  to `TextMergeWithOffset::new(Parser::new_ext(..).into_offset_iter())`, capturing the byte
  range of each `Event::TaskListMarker` and converting it to a line number via a
  precomputed newline index. Correct by construction — no counter, no drift risk.
- The extra `Range<usize>` is dropped for every other event, so the change is confined to
  the iterator construction plus one match arm.

### 2.3 Render changes (`src/markdown.rs`, `src/html.rs`)

In `process_event`, replace `Event::TaskListMarker(_)` with `Event::Html` emitting:

```html
<input type="checkbox" class="mbr-task-check" data-mbr-task-line="42"
       data-mbr-task-state="open" disabled>
```

plus wrap the item's inline content so annotations render as UI:

```html
<span class="mbr-task-pri mbr-task-pri-urgent"></span>
<span class="mbr-task-tag">#work</span>
<span class="mbr-task-due mbr-task-overdue">Aug 5</span>
```

- The existing `[-]` canceled hack at `src/markdown.rs:1708` is **replaced** by this path,
  so canceled items finally get a real class instead of a bare `<s>`.
- `[>]` gains handling for the first time.
- `disabled` is dropped and `id="mbr-task-42"` added only when `edit_enabled` — so static
  builds and read-only servers emit inert checkboxes.
- The golden fixture `tests/fixtures/render/task-lists.expected.html` gets regenerated;
  the diff is the review artifact for this step.

**[REVIEW]** Inline chips/pills render in **all** modes (including static builds), because
they are a markdown extension, not "tasks machinery". Only the popup, the index, the
endpoints and clickable checkboxes are server/GUI-only, per the spec's Applicability
section. Say the word if you want static builds to keep showing raw `@due(...)`.

### 2.4 Endpoints (`src/server.rs`, server/GUI only)

**`POST /.mbr/tasks`** — query. Gated on `tasks_enabled`.

```jsonc
// request
{ "q": "report #work", "folder": "/docs/", "statuses": ["open"],
  "priorities": [], "due": "any|overdue|today|tomorrow|upcoming|none",
  "mode": "category|calendar", "limit": 500 }

// response
{ "groups": [ { "key": "/docs/notes/", "label": "Weekly notes", "sublabel": "docs/notes",
                "url_path": "/docs/notes/", "done": 3, "total": 7,
                "tasks": [ /* Task + url_path + line */ ] } ],
  "folders": [ { "path": "/docs/", "count": 12 } ],
  "total_matches": 42, "duration_ms": 3, "scan_in_progress": false }
```

Grouping and the x/y counts are computed server-side so the "counts include filtered-out
tasks" rule has one authoritative implementation. `q` semantics: bare words AND-match the
display text (case-insensitive substring), `#foo` matches tags.

**`POST /.mbr/task`** — toggle. Gated on `edit_enabled` + `check_edit_access` (reuses the
existing CSRF header, same-origin, DNS-rebind and token checks unchanged).

```jsonc
// request
{ "path": "docs/guide.md", "line": 42,
  "expected": "- [ ] write the report !!",   // exact current line
  "to": "done|open|canceled" }
// 200 -> { "line": 42, "text": "- [x] write the report !! @done(2026-08-04 14:32)" }
// 409 -> line no longer matches (someone edited the file)
```

Rewrites one line, atomic temp-write + rename (same helper as `save_markdown_handler`),
then broadcasts a `FileChangeEvent::Modified` and invalidates that file in the task index.

**[REVIEW]** Marking done stamps `@done(<now>)`; un-checking removes it. Controlled by
`tasks_stamp_done` (default `true`). This is OmniFocus-ish and not in the spec — easy to
default off if you'd rather.

### 2.5 Config / CLI

| Name | Type | Default | Notes |
|---------------------|--------|---------|--------------------------------------|
| `tasks_enabled` | bool | `true` | Server/GUI only; `--no-tasks` disables |
| `tasks_stamp_done` | bool | `true` | Auto `@done(...)` stamp on completion |

Follows the `--no-X` runtime-feature naming convention. `tasks_enabled` reaches the
frontend through `PageChrome` → `_head.html` → `window.__MBR_CONFIG__.tasksEnabled`
(needed on section/tag/home pages too, so it goes in `page_context.rs`, not just the
markdown handler).

---

## 3. Frontend

### 3.1 New lazy chunk `mbr-tasks.min.js`

Trigger `components/src/mbr-tasks.ts` ships in the main bundle:
- Clipboard icon in the right-hand nav group, `<li><mbr-tasks></mbr-tasks></li>` in
  `_nav.html` gated on `{% if server_mode and tasks_enabled %}`.
- Binds lowercase `t` (verified free — only `Shift+T` is taken by the ToC), guarded by
  `isInputTarget()` + `isModalOpen()`.
- `implements MbrOverlay`; added to `OVERLAY_TAGS` in `overlay.ts`.
- Lazy-imports the chunk on first open using the `mbr-info.ts` seam pattern
  (`setTasksChunkImporter` export so tests can stub it).

Chunk `components/src/tasks/`:

| File | Contents |
|--------------------------|--------------------------------------------------|
| `index.ts` | re-export to force `@customElement` registration |
| `mbr-tasks-panel.ts` | the two-pane overlay |
| `task-card.ts` | one task card |
| `task-groups.ts` | pure client-side helpers (date labels, collapse state) |
| `task-format.ts` | pure date/priority formatting, unit-tested |

Chunk stays free of stateful main-bundle imports; the folder tree and `resolveUrl` are
injected as Lit properties by the trigger, per the existing convention.

### 3.2 Layout

```
┌─ folders ────┬─ [filter field.....................] [⚙ filters] ─┐
│ ▼ 📁 Home 42 │        ( ▤ category | ▦ calendar )                │
│   ▶ docs  12 │  ┌──────────────────────────────────────────────┐ │
│   ▶ notes 30 │  │ Weekly notes            3/7 ▓▓▓░░░░          │ │
│              │  │   docs/notes                                 │ │
│              │  │  ● [ ] write the report  #work  ⌛ Aug 5      │ │
│              │  │    [ ] follow up                             │ │
└──────────────┴──┴──────────────────────────────────────────────┘ │
```

- Left pane reuses `buildFolderTree` output (computed in the main bundle, passed in);
  selecting a folder scopes to it **and its subfolders**.
- Mode tabs centered between filter and results, defaulting to **category**.
- Filter popover: status multi-select (default: incomplete only), priority, due range.
- Group headers collapse on click; x/y + progress bar right-aligned. Category mode counts
  every task in the file; calendar mode counts per date bucket filtered by everything
  except status. Canceled tasks never count toward totals anywhere.
- Calendar mode buckets: Overdue, Today, Tomorrow, Upcoming (per-date subheadings),
  and No due date last. Progress bars on Today/Tomorrow/Upcoming only.

### 3.3 Keyboard

| Key | Action |
|-----------------|-----------------------------------------------|
| `t` | open / close |
| `Ctrl+n` `Ctrl+p` / `↓` `↑` | move focus between tasks (skipping collapsed groups) |
| `Space` | toggle done ↔ open (needs edit mode) |
| `x` | cancel task (needs edit mode) |
| `Enter` | jump to the task in its file |
| `←` `→` | collapse / expand the focused group |
| `Tab` | switch pane |
| `Ctrl+d` `Ctrl+u` `Ctrl+f` `Ctrl+b` | scroll the active pane |
| `Esc` | close |

Mirrors `mbr-search` / `mbr-fuzzy-nav` so it feels like the rest of the app.
`docs/reference/keyboard-shortcuts.md` and the in-app `SHORTCUTS` table both get entries.

### 3.4 Jump to task

`Enter` navigates to `{url_path}#mbr-task-{line}`. A small handler in the main bundle
resolves that hash on load, scrolls the line into view and flashes a highlight (reusing
`scrollRangeIntoView`'s inset math from `find-in-page.ts`).

### 3.5 In-document checkbox interaction

Only when `editEnabled`: a delegated listener on `main#wrapper` turns clicks on
`.mbr-task-check` into a `POST /.mbr/task`, and `contextmenu` into a cancel. Optimistic
UI update, revert + toast on 409. The line's current text is read back from
`/.mbr/raw` lazily on first interaction to fill `expected`.

---

## 4. Work breakdown

Each phase ends green on `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, and `bun run test`.

| # | Phase | Deliverable |
|---|----------------------------|-------------------------------------------------------------|
| 0 | Offset spike | Confirm `TextMergeWithOffset` in pulldown-cmark 0.13; pick mapping strategy |
| 1 | `src/tasks.rs` parser | Grammar + `scan_source_tasks`; unit + proptest; bench `benches/tasks.rs` |
| 2 | Index + watcher | `TaskIndex`, lazy build, per-file invalidation; integration tests |
| 3 | Query endpoint | `POST /.mbr/tasks`, grouping, counts, folder facets; `server_integration` tests |
| 4 | Render changes | Line-tagged checkboxes, chips/pills/dot, CSS; golden fixture update |
| 5 | Toggle endpoint | `POST /.mbr/task`, line patch, `@done` stamp, 409 path; auth tests |
| 6 | Trigger + chunk plumbing | `mbr-tasks.ts`, vite config, `DEFAULT_FILES`, CI guard, nav icon, `t` key |
| 7 | Panel UI | Two-pane layout, category mode, cards, collapse, keyboard nav |
| 8 | Calendar mode | Date bucketing, per-bucket progress, mode tabs |
| 9 | Toggling + jump-to | Space/x/click toggling, hash jump + flash, optimistic UI |
| 10| Docs + polish | `cli.md`, `configuration.md`, `keyboard-shortcuts.md`, `docs/markdown/index.md`, TODO.md |

Phases 1-5 are Rust and can land as one PR; 6-9 are frontend and can land as a second.

## 5. Registration checklist (easy to miss)

- [ ] `components/vite.tasks.config.ts` + `package.json` build chain
- [ ] `components/src/main.js` exports the **trigger**, never the chunk
- [ ] `src/server.rs` `DEFAULT_FILES` `include_bytes!` entry
- [ ] `src/build.rs` skips the tasks chunk in static output (spec: no tasks in builds)
- [ ] `.github/workflows/ci.yml` `test -f templates/components-js/mbr-tasks.min.js` guard
- [ ] `components/src/overlay.ts` `OVERLAY_TAGS`
- [ ] `components/src/mbr-keys.ts` `SHORTCUTS` table
- [ ] `components/src/shared.ts` `__MBR_CONFIG__` type + accessor
- [ ] `CLAUDE.md` module table + bundle table

## 6. Risks

| Risk | Mitigation |
|-----------------------------------------------|--------------------------------------------|
| Line ↔ checkbox drift (code fences, HTML blocks) | Offset-based mapping; fixture test asserting counts agree |
| `#tag` / `!!` false positives in prose | Whitespace-anchored patterns; proptest that non-task lines are never matched |
| Index memory on a 10k-file repo | Only files with tasks are stored; `Arc<FileTasks>`; measure before optimizing |
| First-open latency on a huge repo | Response carries `scan_in_progress`, panel renders partial results like search does |
| Golden HTML churn | Single regeneration in phase 4, reviewed as a diff |
