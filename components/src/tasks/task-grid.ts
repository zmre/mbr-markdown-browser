/**
 * The pure half of the task panel's Week and Month views (design/calendar.md
 * §8.2), in the tasks chunk.
 *
 * The grids themselves are the calendar chunk's `<mbr-calendar-month-grid>`
 * and `<mbr-calendar-week-grid>`, which the panel creates **by tag name**
 * after the injected `loadCalendarChunk()`: nothing here may import a grid
 * element or `overlay-model.ts` as a value (a second `customElements.define`
 * throws, and the calendar chunk is already over its size budget). What this
 * module turns the `mode: "grid"` response into is plain data the grids read:
 * `GridEntry` lists and `DayMarks`.
 */
import { addDays, type DateKey } from '../calendar/note-date.js'
import { addMonthsToDate, formatDate, monthGrid, monthOf, weekdayOf, type Weekday } from '../calendar/month-grid.js'
import type { DayMarks, GridEntry } from '../calendar/types.js'
import type { DueView, GridDay, TaskGridResponse, TaskHit, TaskStatus } from './types.js'

/** Text that starts with a clock time, as the server's `leading_minutes` reads one. */
const LEADING_TIME = /^\d{1,2}[:.]\d{2}/

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
export function gridRange(view: GridView, focus: DateKey, weekStart: Weekday): { from: DateKey; to: DateKey } {
  if (view === 'week') {
    const from = addDays(focus, -((weekdayOf(focus) - weekStart + 7) % 7))
    return { from, to: addDays(from, 6) }
  }
  const weeks = monthGrid(monthOf(focus), weekStart).weeks
  return { from: weeks[0].days[0], to: weeks[weeks.length - 1].days[6] }
}

/** `focus` moved one page: a week, or a calendar month (clamped to its length). */
export function stepGrid(view: GridView, focus: DateKey, delta: 1 | -1): DateKey {
  return view === 'week' ? addDays(focus, 7 * delta) : addMonthsToDate(focus, delta)
}

/** The grid header: "October 2026", or "Oct 26 – Nov 1, 2026" for a week. */
export function gridTitle(view: GridView, focus: DateKey, weekStart: Weekday, locale: string): string {
  if (view === 'month') return formatDate(`${monthOf(focus)}-01`, locale, { month: 'long', year: 'numeric' })
  const { from, to } = gridRange('week', focus, weekStart)
  return `${formatDate(from, locale, { month: 'short', day: 'numeric' })} – ${formatDate(to, locale, { month: 'short', day: 'numeric', year: 'numeric' })}`
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
    key: taskKey(hit),
    title: hit.text,
    href: href(hit),
    status: statusOf(hit),
    // A time the text already starts with (`09:30 call`) is not repeated.
    time: hit.time && !LEADING_TIME.test(hit.text) ? hit.time : undefined,
    tooltip: `${hit.text} — ${hit.note}`,
  }))
}

/**
 * A day cell's marks in the panel grid: its open count (for the cell's
 * spoken label) and the overdue colour. `notes: 0` because the entries are
 * tasks, not notes, and the label must not count them as "other notes".
 *
 * Undated tasks in a day note are not counted here: the tasks grid plots only
 * dated tasks (D9), and a count without rows to show for it would mislead.
 */
export function gridDayMarks(day: GridDay | undefined): DayMarks {
  if (!day) return { notes: 0 }
  return { notes: 0, open: day.open, overdue: day.overdue }
}

/** Whether a grid response has nothing to show at all. */
export function gridIsEmpty(response: TaskGridResponse): boolean {
  return Object.keys(response.days).length === 0
}
