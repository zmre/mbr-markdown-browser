/**
 * A note's calendar date, and date-only arithmetic.
 *
 * Pure, no imports: safe for the main bundle and for any lazy chunk.
 *
 * `noteDate` is the **only** place the client falls back from the date the
 * server resolved (`site.json`'s `calendar.date`: filename, then frontmatter;
 * see `crates/mbr-core/src/calendar.rs`) to the file's birthtime. Birthtime
 * is turned into a date in the *reader's* time zone, so a static build shows
 * the reader's local date rather than the build machine's.
 *
 * Dates are `YYYY-MM-DD` strings ("date keys") throughout: they compare and
 * sort correctly as strings, and they never carry a time of day, so no
 * arithmetic here can be thrown off by a daylight-saving change (never add
 * 86 400 000 ms to a timestamp to get "tomorrow").
 */

/** A `YYYY-MM-DD` calendar date. */
export type DateKey = string

/** `site.json`'s `calendar` object on a markdown file (omitted when undated). */
export interface NoteCalendar {
  /** The resolved date, `YYYY-MM-DD`. */
  readonly date: DateKey
  /** Where it came from, for "dated by filename" tooltips. */
  readonly source: 'filename' | 'frontmatter'
  /** Canonical period key, present only on a periodic note. */
  readonly period?: string
}

/** The fields of a `site.json` markdown entry `noteDate` reads. */
export interface DatedNote {
  /** Birthtime (else mtime), unix seconds. */
  readonly created?: number
  readonly calendar?: NoteCalendar | null
}

const DATE_KEY = /^(\d{4})-(\d{2})-(\d{2})$/

const pad2 = (n: number): string => String(n).padStart(2, '0')

/** The date key for a year, month (1–12) and day. */
export function dateKey(year: number, month: number, day: number): DateKey {
  return `${String(year).padStart(4, '0')}-${pad2(month)}-${pad2(day)}`
}

/** The local calendar date of `date`, in the reader's time zone. */
export function localDateKey(date: Date): DateKey {
  return dateKey(date.getFullYear(), date.getMonth() + 1, date.getDate())
}

/** Today's date key in the reader's time zone. */
export function todayKey(now: Date = new Date()): DateKey {
  return localDateKey(now)
}

/**
 * The local date of a unix timestamp in seconds, or `null` for a value that
 * is not a finite number.
 */
export function localDateOf(epochSeconds: number | undefined | null): DateKey | null {
  if (typeof epochSeconds !== 'number' || !Number.isFinite(epochSeconds)) return null
  const date = new Date(epochSeconds * 1000)
  return Number.isNaN(date.getTime()) ? null : localDateKey(date)
}

/**
 * A note's date (D1): `calendar.date` (filename, then frontmatter, resolved
 * by the server), else the local date of `created`. `null` only when the
 * entry has neither.
 */
export function noteDate(file: DatedNote): DateKey | null {
  return file.calendar?.date ?? localDateOf(file.created)
}

/** The parts of a valid date key, or `null`. Rejects `2026-02-30`. */
export function parseDateKey(key: string): { year: number; month: number; day: number } | null {
  const m = DATE_KEY.exec(key)
  if (!m) return null
  const year = Number(m[1])
  const month = Number(m[2])
  const day = Number(m[3])
  const probe = new Date(Date.UTC(year, month - 1, day))
  if (probe.getUTCFullYear() !== year || probe.getUTCMonth() !== month - 1 || probe.getUTCDate() !== day) {
    return null
  }
  return { year, month, day }
}

/**
 * `key` plus `days` days (negative goes back), by calendar arithmetic in UTC,
 * where every day has 24 hours. Throws on an invalid key.
 */
export function addDays(key: DateKey, days: number): DateKey {
  const parts = parseDateKey(key)
  if (!parts) throw new RangeError(`not a date key: ${key}`)
  const moved = new Date(Date.UTC(parts.year, parts.month - 1, parts.day + days))
  return dateKey(moved.getUTCFullYear(), moved.getUTCMonth() + 1, moved.getUTCDate())
}
