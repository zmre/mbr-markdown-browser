/**
 * Wire types for `POST /.mbr/tasks`.
 *
 * Every declaration here mirrors a Rust type in `src/task_query.rs` (or
 * `src/tasks.rs` for the enums). The Rust structs are `#[serde(default)]` and
 * do NOT set `deny_unknown_fields`, so a misspelled request key is silently
 * ignored by the server and degrades to its default instead of erroring — this
 * file plus the exact-body assertion in `mbr-tasks-panel.test.ts` are the only
 * guards against drift.
 *
 * Naming follows the wire, not TypeScript convention: `url_path`, `due_has_time`
 * and friends are snake_case because serde emits them that way.
 */

/** `tasks::TaskStatus`, serialized lowercase. */
export type TaskStatus = 'open' | 'done' | 'canceled'

/** `tasks::TaskPriority`, serialized lowercase. */
export type TaskPriority = 'normal' | 'high' | 'urgent'

/** `task_query::DueFilter`. Note `none`, not `nodue`. */
export type DueFilter = 'any' | 'overdue' | 'today' | 'tomorrow' | 'upcoming' | 'none'

/** `tasks::TaskKind`, serialized lowercase. See {@link TaskHit.kind}. */
export type TaskKind = 'task' | 'marker'

/**
 * `task_query::IncludeFilter` — which kinds of entry a query wants back.
 *
 * Single-select rather than an array like `statuses`: the three options are
 * mutually exclusive and exhaust the space, so an array would admit two states
 * that mean nothing (empty, and both at once).
 */
export type IncludeFilter = 'all' | 'tasks' | 'markers'

/** `task_query::TaskMode`. */
export type TaskMode = 'category' | 'calendar'

/**
 * `task_query::TaskQuery`.
 *
 * Every field is optional server-side; the panel sends all of them so the body
 * is a complete, assertable description of the visible filter state.
 */
export interface TaskQueryRequest {
  /** Filter text. Bare words match display text or a tag; `#foo` matches tags. */
  q: string
  /** Folder scope including subfolders (e.g. `/docs/`); `null` for everywhere. */
  folder: string | null
  /** Statuses to show. An EMPTY array means "incomplete only" server-side. */
  statuses: TaskStatus[]
  /** Priorities to show. Empty means all. */
  priorities: TaskPriority[]
  /** Due-date filter. */
  due: DueFilter
  /** Checkbox tasks, incomplete markers, or both. */
  include: IncludeFilter
  /** How results are grouped. */
  mode: TaskMode
  /** Cap on returned tasks across all groups. Never affects a count. */
  limit: number
}

/**
 * `task_query::TaskHit` — a `tasks::Task` flattened alongside its page URL.
 *
 * `due`/`done` are naive local datetimes (`2026-08-05T00:00:00`); `moved_to` is
 * a naive date (`2026-08-10`). Neither carries a timezone, so both must be
 * parsed as local time — see `parseNaive` in `task-format.ts`.
 */
export interface TaskHit {
  /**
   * Checkbox or incomplete marker — **first, because it decides how every
   * field below it reads.**
   *
   * A `'marker'` is a read-only pointer at a line somebody left unfinished
   * (`TK`, `TODO`, …; see `tasks::MarkerRule`), and the server pins it flat:
   * `status` is always `'open'`, `priority` always `'normal'`, `tags` empty,
   * and `due`/`done`/`moved_to` all `null`. Its `text` is the **whole source
   * line, verbatim** — the marker word included, and `#tag`/`!!`/`@due(...)`
   * neither parsed nor stripped, because a marker has no annotation grammar.
   * Its deep-link fragment is `#mbr-marker-<line>`, not `#mbr-task-<line>`:
   * the two ids come from different places in the renderer (see `taskHref`).
   *
   * Markers can never be written — `POST /.mbr/task` answers 400 for one —
   * and never move a note's `done`/`total`.
   */
  kind: TaskKind
  /** 1-based source line, and the deep-link target (see {@link kind}). */
  line: number
  /** Display indent level for subtasks. */
  depth: number
  status: TaskStatus
  priority: TaskPriority
  /** Display text with annotations stripped (but see {@link kind}). */
  text: string
  /**
   * Where the marker word sits inside {@link text}, as indices into that
   * string — `null` for a `'task'`.
   *
   * These are UTF-16 code unit offsets, which is exactly what a JavaScript
   * string index is, so `text.slice(marker_start, marker_end)` is the marker
   * word. The server sends them rather than letting the card find the word
   * itself: the marker grammar is markup-aware (a `TODO` inside a code span or
   * a link destination is not one) and its word boundaries are decided per
   * configured alternative, so an `indexOf` here would highlight the wrong
   * word on `Set \`TODO\` in config and TK fix it`. See `tasks::Task`.
   */
  marker_start: number | null
  /** End of the marker word; see {@link marker_start}. */
  marker_end: number | null
  /** Tags without the leading `#`. */
  tags: string[]
  due: string | null
  due_has_time: boolean
  done: string | null
  done_has_time: boolean
  moved_to: string | null
  /** URL of the page containing the task. */
  url_path: string
  /**
   * Source path relative to the repository root, extension included — what
   * `POST /.mbr/task` wants as `path`.
   *
   * Sent by the server rather than derived here, because `url_path` does not
   * determine it: `docs/index.md` is served at `/docs/`, the static-folder
   * overlay hides a directory level, and the extension is gone. See
   * `task_query::TaskHit::path`.
   */
  path: string
}

/**
 * One `POST /.mbr/task` write.
 *
 * `expected` is deliberately absent: the caller supplies the *intent*, and the
 * toggler in the main bundle sources the exact current line from `/.mbr/raw`
 * (see `task-toggle.ts`). The panel has no business knowing how that is done,
 * and could not cache it across a panel session if it did.
 */
export interface TaskToggleTarget {
  /** Repo-relative filesystem path, i.e. a {@link TaskHit.path}. */
  path: string
  /** 1-based source line. */
  line: number
  /** Status to write. */
  to: TaskStatus
}

/** Why a toggle failed, at the granularity the UI reacts to. */
export type TaskToggleFailure =
  /** `409`: the line changed on disk. The view is stale and must be refreshed. */
  | 'conflict'
  /** `401`/`403`: editing is off, or the server wants a token we do not have. */
  | 'auth'
  /** Anything else — a bad line, an unreadable file, a network failure. */
  | 'other'

export type TaskToggleOutcome =
  | {
      ok: true
      /**
       * The line's new source text, verbatim
       * (`server.rs::TaskToggleResponse::text`), or absent when the response
       * could not be parsed.
       *
       * The panel has no use for it — it re-queries — but the main bundle does:
       * a write no longer reloads the page, so this is the only thing that can
       * tell the document about a freshly stamped `@done(...)`.
       */
      text?: string
    }
  | { ok: false; kind: TaskToggleFailure; message: string }

/**
 * Writes one task's status.
 *
 * Injected into the panel as a property by `<mbr-tasks>`, like `resolveHref`:
 * the implementation is stateful (a raw-source cache, a live-reload
 * suppression registry) and therefore belongs to the main bundle.
 */
export type TaskToggler = (target: TaskToggleTarget) => Promise<TaskToggleOutcome>

/** `task_query::TaskGroup`. */
export interface TaskGroup {
  /** Unique within a response; the collapse-state key. */
  key: string
  /** Heading text: the note title, or the calendar bucket's name. */
  label: string
  /** Smaller secondary heading: the note's folder; empty in calendar mode. */
  sublabel: string
  /** Page to open when the heading is clicked; `null` for calendar buckets. */
  url_path: string | null
  /** ISO date this group covers; `null` in category mode and for overdue/no-due. */
  date: string | null
  /** Completed tasks counted by this group's rule (see the Rust module docs). */
  done: number
  /** Total tasks counted by this group's rule. */
  total: number
  tasks: TaskHit[]
}

/** `task_query::FolderFacet`: a folder and its cumulative matching-task count. */
export interface FolderFacet {
  /** Folder path with leading and trailing slashes; `/` is the whole repo. */
  path: string
  /** Matching tasks in this folder **and its subfolders**. */
  count: number
}

/** `task_query::TaskQueryResponse`. */
export interface TaskQueryResponse {
  groups: TaskGroup[]
  /** Facets ignore the folder filter, so selecting a folder keeps its siblings. */
  folders: FolderFacet[]
  /** Matching tasks before `limit` was applied. */
  total_matches: number
  duration_ms: number
  /** True while the repository scan is still running: results are partial. */
  scan_in_progress: boolean
}
