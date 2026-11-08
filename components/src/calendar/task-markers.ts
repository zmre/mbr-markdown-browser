/**
 * The companion's task markers (design/calendar.md §8.3): a
 * {@link TaskMarkerSource} over `POST /.mbr/tasks` `mode: "grid"`.
 *
 * MAIN BUNDLE, and stateful (a per-range cache for the page's life), which
 * is why it lives here and reaches the calendar chunk's companion only as a
 * property. `<mbr-calendar>` builds one only where the task endpoint exists
 * (server/GUI with `tasks_enabled`): a static build never makes the request.
 *
 * A file change (`task-toggle.ts`'s `FILE_CHANGED_EVENT`: this page's task
 * writes, and every change the live-reload socket hears) makes the
 * cached counts stale: {@link TaskMarkers.invalidate} drops them and hands
 * back a *new* source function, whose new identity is what makes the
 * companion ask again.
 */
import type { DateKey } from './note-date.js'
import { todayKey } from './note-date.js'
import type { DayTaskCount, TaskMarkerSource } from './types.js'
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
  /** Forgets every cached range; returns the new {@link source}. */
  invalidate(): TaskMarkerSource
}

/**
 * Builds the companion's marker source. Counts are cached per requested
 * range (the companion asks for the six weeks of the month on screen), and a
 * failed request is not cached, so the next month change retries it.
 */
export function createTaskMarkers(
  endpoint: string,
  fetchImpl: typeof fetch = (input, init) => fetch(input, init),
  now: () => Date = () => new Date()
): TaskMarkers {
  const cache = new Map<string, Promise<ReadonlyMap<DateKey, DayTaskCount>>>()
  const request = (from: DateKey, to: DateKey): Promise<ReadonlyMap<DateKey, DayTaskCount>> => {
    const key = `${from}..${to}`
    let pending = cache.get(key)
    if (!pending) {
      // `limit: 0`: counts only, no task rows; open tasks are all a marker
      // draws. `today` makes the server's overdue flags the reader's, and
      // those flags are the only source of the overdue colour.
      const today = todayKey(now())
      const body = JSON.stringify({ mode: 'grid', from, to, statuses: ['open'], limit: 0, today })
      pending = fetchImpl(endpoint, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body })
        .then((r) => {
          if (!r.ok) throw new Error(`task markers: HTTP ${r.status}`)
          return r.json() as Promise<GridCounts>
        })
        .then((json) => dayTaskCounts(json))
      const settled = pending
      settled.catch(() => {
        if (cache.get(key) === settled) cache.delete(key)
      })
      cache.set(key, settled)
    }
    return pending
  }
  const markers = {
    source: ((from, to) => request(from, to)) as TaskMarkerSource,
    invalidate(): TaskMarkerSource {
      cache.clear()
      markers.source = (from, to) => request(from, to)
      return markers.source
    },
  }
  return markers
}
