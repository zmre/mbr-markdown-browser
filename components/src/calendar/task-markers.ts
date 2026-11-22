/**
 * The companion's task markers (design/calendar.md §8.3) and the overlay's
 * task rows: a {@link TaskMarkerSource} and a {@link TaskGridSource} over
 * `POST /.mbr/tasks` `mode: "grid"`, sharing one response cache.
 *
 * MAIN BUNDLE, and stateful (a per-range cache for the page's life), which
 * is why it lives here and reaches the calendar chunk's companion and overlay
 * only as properties. `<mbr-calendar>` builds one only where the task endpoint exists
 * (server/GUI with `tasks_enabled`): a static build never makes the request.
 *
 * A file change (`task-toggle.ts`'s `FILE_CHANGED_EVENT`: this page's task
 * writes, and every change the live-reload socket hears) makes the
 * cached responses stale: {@link TaskMarkers.invalidate} drops them and
 * hands back *new* source functions, whose new identities are what make the
 * companion and the overlay ask again.
 */
import type { DateKey } from './note-date.js'
import { todayKey } from './note-date.js'
import type { DayTaskCount, TaskGridSource, TaskMarkerSource } from './types.js'
import type { GridDay, TaskGridResponse } from '../tasks/types.js'

/**
 * The fields of a `mode: "grid"` response (`tasks/types.ts`) the markers read;
 * `limit: 0` asks for no rows, and an older server may omit either map.
 */
export type GridCounts = Partial<Pick<TaskGridResponse, 'undated_in_day_notes'>> & {
  readonly days?: Readonly<Record<string, Partial<Pick<GridDay, 'open' | 'overdue'>>>>
}

/**
 * Per-day counts from a grid response: the ring is the day's placed open
 * tasks **plus** the open undated tasks of its day note (D9: those count
 * toward markers only). It is red only where the server says the day is
 * overdue: a past day with an open task placed by its **due** date (the
 * request's `today` is the reader's). A past day holding only scheduled,
 * done or undated tasks is not overdue, so nothing here compares dates.
 */
export function dayTaskCounts(body: GridCounts): Map<DateKey, DayTaskCount> {
  const counts = new Map<DateKey, DayTaskCount>()
  const add = (date: DateKey, open: number, overdue: boolean): void => {
    const prev = counts.get(date)
    counts.set(date, { open: (prev?.open ?? 0) + open, overdue: (prev?.overdue ?? false) || overdue })
  }
  for (const [date, day] of Object.entries(body.days ?? {})) add(date, day.open ?? 0, day.overdue === true)
  for (const [date, n] of Object.entries(body.undated_in_day_notes ?? {})) if (n > 0) add(date, n, false)
  for (const [date, c] of counts) if (c.open === 0) counts.delete(date)
  return counts
}

export interface TaskMarkers {
  /** The current source; a new function after every {@link invalidate}. */
  readonly source: TaskMarkerSource
  /**
   * The overlay's task rows (`<mbr-calendar-overlay>`'s `taskSource`), from
   * the same cache; a new function after every {@link invalidate} too.
   */
  readonly grid: TaskGridSource
  /** Forgets every cached response; returns the new {@link source}. */
  invalidate(): TaskMarkerSource
}

/**
 * Task rows the overlay asks for across a 42-day month grid: shared out over
 * the days by the server (`task_query::spread_limit`), so a busy day keeps
 * its share and the rest is a `+N`.
 */
export const OVERLAY_TASK_LIMIT = 500

/**
 * Builds the companion's marker source and the overlay's grid source over one
 * cache of `mode: "grid"` responses, keyed by the exact request body (so the
 * reader's `today` is part of the key). A failed request is not cached, so
 * the next ask retries it.
 */
export function createTaskMarkers(
  endpoint: string,
  fetchImpl: typeof fetch = (input, init) => fetch(input, init),
  now: () => Date = () => new Date()
): TaskMarkers {
  const cache = new Map<string, Promise<unknown>>()
  const post = <T>(fields: Record<string, unknown>): Promise<T> => {
    // `today` makes the server's overdue flags the reader's.
    const body = JSON.stringify({ mode: 'grid', ...fields, today: todayKey(now()) })
    let pending = cache.get(body)
    if (!pending) {
      const settled = fetchImpl(endpoint, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body }).then((r) => {
        if (!r.ok) throw new Error(`task grid: HTTP ${r.status}`)
        return r.json() as Promise<unknown>
      })
      settled.catch(() => {
        if (cache.get(body) === settled) cache.delete(body)
      })
      cache.set(body, settled)
      pending = settled
    }
    return pending as Promise<T>
  }
  // `limit: 0`: counts only, no task rows; open tasks are all a marker draws,
  // and the server's overdue flags are the only source of the overdue colour.
  const markerSource = (): TaskMarkerSource => (from, to) =>
    post<GridCounts>({ from, to, statuses: ['open'], limit: 0 }).then((json) => dayTaskCounts(json))
  const gridSource = (): TaskGridSource => (query) =>
    post<TaskGridResponse>({
      from: query.from,
      to: query.to,
      statuses: [...query.statuses],
      ...(query.folder ? { folder: query.folder } : {}),
      limit: OVERLAY_TASK_LIMIT,
    })
  const markers = {
    source: markerSource(),
    grid: gridSource(),
    invalidate(): TaskMarkerSource {
      cache.clear()
      markers.source = markerSource()
      markers.grid = gridSource()
      return markers.source
    },
  }
  return markers
}
