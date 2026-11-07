/**
 * The calendar chunk's contract with the main bundle and with later phases:
 * what the trigger injects, and what the grid elements read per day.
 *
 * Types only. The chunk must not import `shared.ts` or any other stateful
 * main-bundle module, so everything stateful arrives through these.
 */
import type { MarkdownFile } from '../sorting.js'
import type { DateKey } from './note-date.js'

/** What one day cell shows besides its number. */
export interface DayMarks {
  /** The day note's site-absolute URL, when the day has one (shaded cell). */
  readonly dailyUrl?: string
  /** Other notes dated that day (up to three dots). */
  readonly notes?: number
  /** Open tasks that day (hollow ring); from {@link TaskMarkerSource}. */
  readonly open?: number
  /** The ring is the overdue colour (a past day with open tasks). */
  readonly overdue?: boolean
}

/** Open-task count for one day (`calendar/task-markers.ts`'s `dayTaskCounts`). */
export interface DayTaskCount {
  /** Open tasks placed on the day plus the day note's open undated ones. */
  readonly open: number
  readonly overdue: boolean
}

/**
 * Per-day open-task counts for an inclusive date range, from
 * `POST /.mbr/tasks` `mode: "grid"` (design/calendar.md §8.3), built by the
 * main bundle's `calendar/task-markers.ts`. `null` where there is no task
 * endpoint (static builds, `tasks_enabled = false`): the companion then draws
 * no rings and makes no request. Called at idle only; a new function
 * identity means the counts went stale (a task write), and the companion
 * asks again.
 */
export type TaskMarkerSource = (from: DateKey, to: DateKey) => Promise<ReadonlyMap<DateKey, DayTaskCount>>

/** `site.json`, as far as the calendar reads it. */
export interface SiteIndex {
  readonly markdown_files?: readonly MarkdownFile[]
}

/** Fired by `<mbr-calendar-companion>` when its ✕ (or Esc in a drawer) asks to go. */
export const COMPANION_CLOSE_EVENT = 'mbr-calendar-close'

/** Where the companion is drawn. */
export type CompanionPlacement = 'docked' | 'drawer'

/**
 * One line in a large calendar grid cell (`<mbr-calendar-month-grid>`,
 * `<mbr-calendar-week-grid>`): a note in the overlay, a task in the phase 6
 * panel. Data only, so a host in another bundle can fill it.
 */
export interface GridEntry {
  /** Stable within a day (the overlay uses the note's `url_path`). */
  readonly key: string
  readonly title: string
  /** Where the entry goes: rendered as a link. Without one, a click fires `mbr-calendar-entry`. */
  readonly href?: string
  /** Type-colour slot 0–7 (`--mbr-cal-type-N`), `null` for a muted dot. */
  readonly color?: number | null
  /** A time label shown before the title (Week view, list rows). */
  readonly time?: string
  /** Drawn as a checkbox glyph: the phase 6 task grids. */
  readonly status?: 'open' | 'done' | 'canceled'
  /** Hover text (folder, full title). */
  readonly tooltip?: string
}

/** What the calendar overlay is opened over (design/calendar.md §5.1). */
export interface CalendarScope {
  /** The scope chip ("All notes", "Folder: meetings", "Search: when:-7d"). */
  readonly label: string
  /** The notes; omitted = every markdown note in `site.json`. */
  readonly files?: readonly MarkdownFile[]
  /** Starts the overlay on this plot-by (search hands over `date`). */
  readonly plotBy?: PlotBy
  /**
   * The page the overlay was opened from with `c` (site-absolute, decoded),
   * which offers the "This folder" toggle.
   */
  readonly pagePath?: string
}

/** Which of a note's dates the overlay plots. */
export type PlotBy = 'date' | 'modified' | 'created'

/** Fired by `<mbr-calendar-overlay>` when it asks to close. */
export const OVERLAY_CLOSE_EVENT = 'mbr-calendar-overlay-close'
