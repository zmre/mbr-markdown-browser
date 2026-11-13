/**
 * The pure half of the task panel's Week and Month views (design/calendar.md
 * §8.2), in the tasks chunk.
 *
 * The grids themselves are the calendar chunk's `<mbr-calendar-month-grid>`
 * and `<mbr-calendar-week-grid>`, which the panel creates **by tag name**
 * after the injected `loadCalendarChunk()`: nothing here may import a grid
 * element (a second `customElements.define` throws). What this
 * module turns the `mode: "grid"` response into is plain data the grids read:
 * `GridEntry` lists and `DayMarks`. Ranges, titles and the key map are
 * `calendar/month-grid.ts`'s (pure, imported directly).
 */
import type { DateKey } from '../calendar/note-date.js'
import { gridMonthRange, monthOf, weekRange, type DayRange, type Weekday } from '../calendar/month-grid.js'
import type { DayMarks, GridEntry } from '../calendar/types.js'
import type { DueView, GridDay, TaskGridResponse, TaskHit, TaskStatus } from './types.js'
import { taskDisplayText } from './task-text.js'

/** A grid view: "By Due" minus its list. */
export type GridView = Exclude<DueView, 'list'>

/** Below this width the Month grid's cells are too small: it shows as Week. */
export const NARROW_QUERY = '(max-width: 600px)'

/**
 * Identity of one task across a re-query: path + line, not an array index —
 * a toggle re-queries, and a task it completed may leave the view.
 */
export function taskKey(hit: TaskHit): string {
  return `${hit.path}:${hit.line}`
}

/** The view drawn for a chosen one: Month falls back to Week on a narrow screen. */
export function effectiveView(view: DueView, narrow: boolean): DueView {
  return view === 'month' && narrow ? 'week' : view
}

/**
 * The inclusive days a view of `focus` shows, which is exactly the range the
 * grid request asks for: the week containing `focus` from `weekStart`, or the
 * six week rows of its month (42 days, inside the server's 62-day cap).
 */
export function gridRange(view: GridView, focus: DateKey, weekStart: Weekday): DayRange {
  return view === 'week' ? weekRange(focus, weekStart) : gridMonthRange(monthOf(focus), weekStart)
}

/** The response's day, if it has one. */
export function gridDay(response: TaskGridResponse | null, date: DateKey): GridDay | undefined {
  return response?.days[date]
}

/**
 * A day's task rows as grid entries: a checkbox glyph in the task's (possibly
 * optimistic) status, its text, a link to its line, and the note on hover.
 */
export function gridEntries(
  day: GridDay | undefined,
  statusOf: (hit: TaskHit) => TaskStatus,
  href: (hit: TaskHit) => string
): GridEntry[] {
  if (!day) return []
  return day.tasks.map((hit) => ({
    title: taskDisplayText(hit.text),
    href: href(hit),
    status: statusOf(hit),
    // The server sends a time only when the placing date carried one.
    time: hit.time,
    tooltip: `${taskDisplayText(hit.text)} — ${hit.note}`,
  }))
}

/**
 * Rows of `day` that pass the status filter but were not sent: the server
 * shares its row limit out across the days (`task_query::spread_limit`), so a
 * busy day can carry fewer `tasks` than its counts. The counts ignore status
 * and the rows honour it, so the comparison is against the counts of the
 * statuses on screen — never `open + done`, which would call every filtered-
 * out done task "more". Canceled tasks are never placed. An empty `statuses`
 * means open only, as on the server.
 */
export function gridHiddenCount(day: GridDay | undefined, statuses: readonly TaskStatus[]): number {
  if (!day) return 0
  const wanted =
    (statuses.length === 0 || statuses.includes('open') ? day.open : 0) + (statuses.includes('done') ? day.done : 0)
  return Math.max(0, wanted - day.tasks.length)
}

/**
 * A day cell's marks in the panel grid: its open count (for the cell's
 * spoken label), the overdue colour, and the rows the limit dropped (drawn
 * as the cell's `+N`). `notes: 0` because the entries are tasks, not notes,
 * and the label must not count them as "other notes".
 *
 * Undated tasks in a day note are not counted here: the tasks grid plots only
 * dated tasks (D9), and a count without rows to show for it would mislead.
 */
export function gridDayMarks(day: GridDay | undefined, statuses: readonly TaskStatus[]): DayMarks {
  if (!day) return { notes: 0 }
  return { notes: 0, open: day.open, overdue: day.overdue, hidden: gridHiddenCount(day, statuses) }
}

/** Whether a grid response has nothing to show at all. */
export function gridIsEmpty(response: TaskGridResponse): boolean {
  return Object.keys(response.days).length === 0
}
