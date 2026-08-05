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
  /** 1-based source line, and the `#mbr-task-<line>` deep-link target. */
  line: number
  /** Display indent level for subtasks. */
  depth: number
  status: TaskStatus
  priority: TaskPriority
  /** Display text with annotations stripped. */
  text: string
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
