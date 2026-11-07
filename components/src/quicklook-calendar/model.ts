/**
 * The QuickLook day calendar's decisions (design/calendar.md §4.7), pure: what
 * the preview was told, whether to draw anything, and where.
 *
 * `crates/mbr-ffi/src/quicklook.rs` inlines this bundle only for a note with a
 * date (D1, resolved in Rust) whose source passes the agenda prefilter
 * (`mbr_core::agenda::may_have_agenda`), and writes the payload into
 * `window.__MBR_QUICKLOOK_CALENDAR__` just above it. The entries themselves
 * come from `agenda-parse.ts` over the rendered preview, the same grammar the
 * companion uses; a note the prefilter let through with no entry draws
 * nothing.
 */
import type { AgendaEntry } from '../calendar/agenda-parse.js'
import type { WeekStart } from '../calendar/config.js'
import { parseDateKey, type DateKey } from '../calendar/note-date.js'

/** The global `quicklook.rs` (`DAY_CALENDAR_GLOBAL`) writes the payload to. */
export const QUICKLOOK_CALENDAR_GLOBAL = '__MBR_QUICKLOOK_CALENDAR__'

/** Where the date came from (D1); `birthtime` covers mtime where there is none. */
export type DaySource = 'filename' | 'frontmatter' | 'birthtime'

/** What the preview was told about its note, validated. */
export interface DayCalendarPayload {
  /** The note's date, `YYYY-MM-DD`. */
  readonly date: DateKey
  readonly source: DaySource
  /** `[calendar] week_start`. */
  readonly weekStart: WeekStart
  /** `[calendar] week_numbers`; unset is off (a preview knows no week notes). */
  readonly weekNumbers: boolean
  /** `[calendar] agenda_chips`: paint the time chips in the body. */
  readonly agendaChips: boolean
}

const SOURCES: readonly DaySource[] = ['filename', 'frontmatter', 'birthtime']
const WEEK_STARTS: readonly WeekStart[] = ['locale', 'monday', 'sunday', 'saturday']

/**
 * The payload from its raw (snake_case) global, or `null` when it is missing
 * or carries no valid date. Optional settings default key by key.
 */
export function readPayload(raw: unknown): DayCalendarPayload | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  if (typeof r.date !== 'string' || !parseDateKey(r.date)) return null
  const oneOf = <T extends string>(v: unknown, allowed: readonly T[], fallback: T): T =>
    typeof v === 'string' && (allowed as readonly string[]).includes(v) ? (v as T) : fallback
  return {
    date: r.date,
    source: oneOf(r.source, SOURCES, 'birthtime'),
    weekStart: oneOf(r.week_start, WEEK_STARTS, 'locale'),
    weekNumbers: r.week_numbers === true,
    agendaChips: r.agenda_chips !== false,
  }
}

/** Draw only with a date and at least one agenda entry. */
export function shouldRender(payload: DayCalendarPayload | null, entries: readonly AgendaEntry[]): boolean {
  return payload !== null && entries.length > 0
}

/** A right column beside the note, or a block above it. */
export type Placement = 'column' | 'top'

/** Width of the right column, in CSS pixels. */
export const COLUMN_WIDTH = 280

/**
 * The narrowest panel that takes the column: the column, its gutter and a
 * readable note beside it (~34em of text).
 */
export const COLUMN_MIN_WIDTH = 860

/** Where the calendar goes in a panel `width` CSS pixels wide. */
export function placementFor(width: number): Placement {
  return width >= COLUMN_MIN_WIDTH ? 'column' : 'top'
}
