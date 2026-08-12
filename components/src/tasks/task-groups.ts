/**
 * Turns a `/.mbr/tasks` response into the list the results pane renders, and
 * that list into the flat row sequence the keyboard walks.
 *
 * Both steps are pure functions of the response plus the collapse set, which is
 * what makes the keyboard behaviour testable without a DOM.
 *
 * # Why calendar mode needs a synthetic group
 *
 * The server emits one group per upcoming *date* (`upcoming:2026-08-20`), and
 * TASKS_SPEC.md wants a progress bar on "Upcoming" as a whole but **not** on the
 * individual dates under it. There is no aggregate group on the wire, so this
 * module synthesizes one and sums the per-date counts. That sum is safe: the
 * date buckets partition the upcoming tasks, so no task is counted twice.
 *
 * Overdue keeps no progress at all (the server sends `0/0` — a backlog of
 * missed deadlines has no meaningful denominator), and neither does the
 * no-due-date bucket.
 */
import type { TaskGroup, TaskHit, TaskMode, TaskQueryResponse } from './types.js'

/**
 * Collapse key of the synthesized "Upcoming" section header.
 *
 * Cannot collide with a server key: calendar buckets are spelled
 * `upcoming:<date>`, never bare `upcoming` (see `task_query::bucket_key`).
 */
export const UPCOMING_SECTION_KEY = 'upcoming'

/** Prefix of the per-date calendar buckets. */
const UPCOMING_PREFIX = 'upcoming:'

/** One heading in the results pane, with the tasks rendered under it. */
export interface DisplayGroup {
  /** Collapse-state key; unique within a render. */
  key: string
  /** Heading text. */
  label: string
  /** Smaller muted line under the heading (the note's folder in category mode). */
  sublabel: string
  /** Page the heading links to, or `null` for a calendar bucket. */
  urlPath: string | null
  /** ISO date this group covers, when it has one. */
  date: string | null
  done: number
  total: number
  /** Whether to draw the `x/y` + progress bar (see the module docs). */
  showProgress: boolean
  /** `0` for a top-level heading, `1` for a date nested under Upcoming. */
  level: 0 | 1
  /** Collapse key of the section that hides this group, if any. */
  parentKey: string | null
  /** Empty for the synthesized Upcoming header, which owns no tasks itself. */
  tasks: TaskHit[]
}

/** A rendered line: either a heading or one task under it. */
export type TaskRow =
  | { kind: 'group'; groupIndex: number }
  | { kind: 'task'; groupIndex: number; taskIndex: number }

/**
 * Project a response into display groups, in render order.
 *
 * Category mode is 1:1 with the server's groups, except that the page the user
 * opened the panel from is pinned first. Calendar mode keeps the server's
 * bucket order (Overdue → Today → Tomorrow → dates → No due date) and splices
 * the synthesized Upcoming header in front of the first date bucket — a date is
 * not a file, so there is nothing to pin.
 */
export function buildDisplayGroups(
  response: TaskQueryResponse | null,
  mode: TaskMode,
  currentPath: string | null = null
): DisplayGroup[] {
  const groups = response?.groups ?? []
  if (mode === 'category') {
    return pinCurrentFile(
      groups.map((group) => fromServerGroup(group, true, 0, null)),
      currentPath
    )
  }

  const out: DisplayGroup[] = []
  let upcomingHeaderIndex = -1

  for (const group of groups) {
    if (group.key.startsWith(UPCOMING_PREFIX)) {
      if (upcomingHeaderIndex < 0) {
        upcomingHeaderIndex = out.length
        out.push({
          key: UPCOMING_SECTION_KEY,
          label: 'Upcoming',
          sublabel: '',
          urlPath: null,
          date: null,
          done: 0,
          total: 0,
          showProgress: true,
          level: 0,
          parentKey: null,
          tasks: [],
        })
      }
      const header = out[upcomingHeaderIndex]
      header.done += group.done
      header.total += group.total
      // Individual dates get no bar of their own, per TASKS_SPEC.md.
      out.push(fromServerGroup(group, false, 1, UPCOMING_SECTION_KEY))
      continue
    }
    // Overdue is sent as 0/0 and no-due-date is not a deadline, so neither gets
    // a bar; Today and Tomorrow do.
    const showProgress = group.key === 'today' || group.key === 'tomorrow'
    out.push(fromServerGroup(group, showProgress, 0, null))
  }

  return out
}

/**
 * Move the group for the page the panel was opened from to the front, leaving
 * every other group in the server's order.
 *
 * Matched on the tasks' source `path`, never on the group key: that key is the
 * file's `url_path`, which a source path does not determine — `docs/index.md`
 * is served at `/docs/` and the static-folder overlay hides a directory level
 * (see `task_query::TaskHit::path`). One task is enough, since a category group
 * is one file.
 *
 * A splice rather than a sort, so the reorder is stable.
 */
function pinCurrentFile(groups: DisplayGroup[], currentPath: string | null): DisplayGroup[] {
  if (currentPath === null) return groups
  const index = groups.findIndex((group) => group.tasks.some((task) => task.path === currentPath))
  if (index <= 0) return groups
  groups.unshift(...groups.splice(index, 1))
  return groups
}

function fromServerGroup(
  group: TaskGroup,
  showProgress: boolean,
  level: 0 | 1,
  parentKey: string | null
): DisplayGroup {
  return {
    key: group.key,
    label: group.label,
    sublabel: group.sublabel,
    urlPath: group.url_path,
    date: group.date,
    done: group.done,
    total: group.total,
    showProgress,
    level,
    parentKey,
    tasks: group.tasks ?? [],
  }
}

/**
 * Flatten display groups into the sequence the keyboard walks.
 *
 * A collapsed group contributes its heading but none of its tasks, so `↓`/`↑`
 * skip straight past it. A group whose *parent section* is collapsed
 * contributes nothing at all — collapsing "Upcoming" hides its dates too.
 */
export function buildRows(groups: DisplayGroup[], collapsed: ReadonlySet<string>): TaskRow[] {
  const rows: TaskRow[] = []
  groups.forEach((group, groupIndex) => {
    if (group.parentKey !== null && collapsed.has(group.parentKey)) return
    rows.push({ kind: 'group', groupIndex })
    if (collapsed.has(group.key)) return
    group.tasks.forEach((_, taskIndex) => {
      rows.push({ kind: 'task', groupIndex, taskIndex })
    })
  })
  return rows
}

/**
 * The row index for a group heading, or `-1` when the heading is not visible.
 * Used to park focus on a heading after `←` collapses it.
 */
export function groupRowIndex(rows: readonly TaskRow[], groupIndex: number): number {
  return rows.findIndex((row) => row.kind === 'group' && row.groupIndex === groupIndex)
}

/** The task a row points at, or `null` for a heading row. */
export function taskAt(groups: readonly DisplayGroup[], row: TaskRow | undefined): TaskHit | null {
  if (!row || row.kind !== 'task') return null
  return groups[row.groupIndex]?.tasks[row.taskIndex] ?? null
}

/**
 * Deep link to a hit: its page plus the fragment the renderer gave that line.
 *
 * The prefix follows `hit.kind` because the two ids are emitted by different
 * code on the server and are not interchangeable: a checkbox gets
 * `#mbr-task-<line>` from `html.rs::task_checkbox_html`, while a marker gets
 * `#mbr-marker-<line>` from the highlight span `markdown.rs` wraps around the
 * marker word. Asking for the wrong one lands on a fragment that does not
 * exist, which scrolls nowhere and flashes nothing.
 *
 * The single implementation is the point: every caller that navigates to a hit
 * routes through here, so click and `Enter` cannot disagree with the rendered
 * `href`.
 */
export function taskHref(hit: TaskHit, resolveHref: (path: string) => string): string {
  const prefix = hit.kind === 'marker' ? 'mbr-marker' : 'mbr-task'
  return `${resolveHref(hit.url_path)}#${prefix}-${hit.line}`
}
