/**
 * A note's calendar date, and date-only arithmetic.
 *
 * Pure, importing only `period.ts`'s day-number arithmetic: safe for the main
 * bundle and for any lazy chunk.
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

import { daysFromCivil, daysInMonth, keyOfDays } from './period.js'

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

/** The local calendar date of `now` (default: today), in the reader's time zone. */
export function todayKey(now: Date = new Date()): DateKey {
  return dateKey(now.getFullYear(), now.getMonth() + 1, now.getDate())
}

/**
 * The local date of a unix timestamp in seconds, or `null` for a value that
 * is not a finite number.
 */
export function localDateOf(epochSeconds: number | undefined | null): DateKey | null {
  if (typeof epochSeconds !== 'number' || !Number.isFinite(epochSeconds)) return null
  const date = new Date(epochSeconds * 1000)
  return Number.isNaN(date.getTime()) ? null : todayKey(date)
}

/**
 * The unix-second bounds `[from, to)` of the reader's local days `first` to
 * `last` (inclusive; `null` = open): `localDateOf(t)` lies in the range
 * exactly when `from <= t < to`. Lets a scan over thousands of timestamps
 * compare numbers and build a date key only for a hit. A day whose midnight
 * does not exist locally starts at its first instant, as `Date` resolves it.
 */
export function localDayBounds(first: DateKey | null, last: DateKey | null): { from: number; to: number } {
  const startOf = (key: DateKey, plusDays: number): number => {
    const [y, m, d] = key.split('-').map(Number)
    const date = new Date(y, m - 1, d + plusDays)
    date.setFullYear(y, m - 1, d + plusDays) // years 0–99 are not 1900–1999
    return date.getTime() / 1000
  }
  return {
    from: first === null ? -Infinity : startOf(first, 0),
    to: last === null ? Infinity : startOf(last, 1),
  }
}

/**
 * A note's date (D1): `calendar.date` (filename, then frontmatter, resolved
 * by the server), else the local date of `created`. `null` only when the
 * entry has neither.
 */
export function noteDate(file: DatedNote): DateKey | null {
  return file.calendar?.date ?? localDateOf(file.created)
}

/**
 * The day a note sits on in a calendar: {@link noteDate}, except that a week,
 * month, quarter or year note sits on none — its date is only its period's
 * first day, and it is reached through the week-number and period links.
 * A day note sits on its own day. The one rule behind the companion's day
 * buckets and the overlay's "Note date" plot.
 */
export function noteDayOf(file: DatedNote): DateKey | null {
  const period = file.calendar?.period
  return period && !DATE_KEY.test(period) ? null : noteDate(file)
}

/** The parts of a valid date key, or `null`. Rejects `2026-02-30`. */
export function parseDateKey(key: string): { year: number; month: number; day: number } | null {
  const m = DATE_KEY.exec(key)
  if (!m) return null
  const year = Number(m[1])
  const month = Number(m[2])
  const day = Number(m[3])
  return month >= 1 && month <= 12 && day >= 1 && day <= daysInMonth(year, month) ? { year, month, day } : null
}

/** The day number (days since 1970-01-01) of a date key. Throws on an invalid key. */
export function dayNumber(key: DateKey): number {
  const parts = parseDateKey(key)
  if (!parts) throw new RangeError(`not a date key: ${key}`)
  return daysFromCivil(parts.year, parts.month, parts.day)
}

/**
 * `key` plus `days` days (negative goes back), by calendar arithmetic on day
 * numbers, where every day has 24 hours. Throws on an invalid key.
 */
export function addDays(key: DateKey, days: number): DateKey {
  return keyOfDays(dayNumber(key) + days)
}
