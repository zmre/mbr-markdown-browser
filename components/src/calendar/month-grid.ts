/**
 * Month-grid arithmetic: week starts, the six-row month grid, ISO week
 * labels, month keys and the grid's keyboard map.
 *
 * Pure, with type-only imports (plus `note-date.ts`'s pure helpers): the one
 * place grid math lives, for the companion's mini month (phase 2), the
 * calendar overlay's month view (phase 3) and the task panel's grids
 * (phase 6). Everything works on `YYYY-MM-DD` date keys and UTC calendar
 * arithmetic, so no daylight-saving change can move a cell.
 */
import type { WeekStart } from './config.js'
import { addDays, dateKey, parseDateKey, type DateKey } from './note-date.js'

/** A day of the week, JavaScript's numbering: 0 = Sunday … 6 = Saturday. */
export type Weekday = 0 | 1 | 2 | 3 | 4 | 5 | 6

/** A `YYYY-MM` month. */
export type MonthKey = string

/** One row of a month grid. */
export interface GridWeek {
  /** Seven consecutive date keys, starting on the grid's week start. */
  readonly days: readonly DateKey[]
  /**
   * The ISO week of the row's **Thursday** (`2026-W53`). Every 7-day row holds
   * exactly one Thursday, and a Monday-to-Sunday ISO week owns its Thursday,
   * so with any week start the label is unambiguous and matches ISO.
   */
  readonly isoWeek: string
}

/** A month laid out on a fixed six-row grid (42 days). */
export interface MonthGrid {
  readonly month: MonthKey
  readonly weekStart: Weekday
  readonly weeks: readonly GridWeek[]
}

/** Rows in every month grid, so the grid never changes height between months. */
export const GRID_ROWS = 6

const MONTH_KEY = /^(\d{4})-(\d{2})$/

const pad2 = (n: number): string => String(n).padStart(2, '0')

/** The parts of a valid `YYYY-MM` key, or `null`. */
export function parseMonthKey(key: string): { year: number; month: number } | null {
  const m = MONTH_KEY.exec(key)
  if (!m) return null
  const year = Number(m[1])
  const month = Number(m[2])
  return month >= 1 && month <= 12 ? { year, month } : null
}

/** The month key for a year and month (1–12). */
export function monthKey(year: number, month: number): MonthKey {
  return `${String(year).padStart(4, '0')}-${pad2(month)}`
}

/** The month a date key falls in. */
export function monthOf(date: DateKey): MonthKey {
  return date.slice(0, 7)
}

/** `month` moved by `delta` months. Throws on an invalid key. */
export function addMonths(month: MonthKey, delta: number): MonthKey {
  const parts = parseMonthKey(month)
  if (!parts) throw new RangeError(`not a month key: ${month}`)
  const moved = new Date(Date.UTC(parts.year, parts.month - 1 + delta, 1))
  return monthKey(moved.getUTCFullYear(), moved.getUTCMonth() + 1)
}

/** Days in a month. */
function daysInMonth(year: number, month: number): number {
  return new Date(Date.UTC(year, month, 0)).getUTCDate()
}

/**
 * `date` moved by `delta` months, the day clamped to the target month's
 * length (Jan 31 + 1 month = Feb 28/29), as PageDown does in every calendar.
 */
export function addMonthsToDate(date: DateKey, delta: number): DateKey {
  const parts = parseDateKey(date)
  if (!parts) throw new RangeError(`not a date key: ${date}`)
  const target = parseMonthKey(addMonths(monthKey(parts.year, parts.month), delta))!
  return dateKey(target.year, target.month, Math.min(parts.day, daysInMonth(target.year, target.month)))
}

/** The weekday of a date key. Throws on an invalid key. */
export function weekdayOf(date: DateKey): Weekday {
  const parts = parseDateKey(date)
  if (!parts) throw new RangeError(`not a date key: ${date}`)
  return new Date(Date.UTC(parts.year, parts.month - 1, parts.day)).getUTCDay() as Weekday
}

/** The ISO 8601 week of a date: its week-year, week number and key. */
export function isoWeekOf(date: DateKey): { year: number; week: number; key: string } {
  // The ISO week-year is the year of the week's Thursday.
  const thursday = addDays(date, 3 - ((weekdayOf(date) + 6) % 7))
  const year = Number(thursday.slice(0, 4))
  const jan1 = dateKey(year, 1, 1)
  const dayOfYear = Math.round(
    (Date.parse(`${thursday}T00:00:00Z`) - Date.parse(`${jan1}T00:00:00Z`)) / 86_400_000
  )
  const week = Math.floor(dayOfYear / 7) + 1
  return { year, week, key: `${String(year).padStart(4, '0')}-W${pad2(week)}` }
}

/** The Monday an ISO week (`2026-W44`) starts on, or `null` for another key. */
export function isoWeekStart(key: string): DateKey | null {
  const m = /^(\d{4})-W(\d{2})$/.exec(key)
  if (!m) return null
  const year = Number(m[1])
  const week = Number(m[2])
  // Jan 4 is always in week 1.
  const jan4 = dateKey(year, 1, 4)
  const week1Monday = addDays(jan4, -((weekdayOf(jan4) + 6) % 7))
  const start = addDays(week1Monday, (week - 1) * 7)
  return isoWeekOf(start).key === key ? start : null
}

/**
 * The first day of a canonical period key (`2026-10-31`, `2026-W44`,
 * `2026-10`, `2026-Q4`, `2026`), or `null` for anything else.
 */
export function periodStart(key: string): DateKey | null {
  if (parseDateKey(key)) return key
  if (/^\d{4}-W\d{2}$/.test(key)) return isoWeekStart(key)
  if (parseMonthKey(key)) return `${key}-01`
  const quarter = /^(\d{4})-Q([1-4])$/.exec(key)
  if (quarter) return dateKey(Number(quarter[1]), (Number(quarter[2]) - 1) * 3 + 1, 1)
  if (/^\d{4}$/.test(key)) return `${key}-01-01`
  return null
}

/** The shape of `Intl.Locale`'s week information (ES2024 / the older getter). */
interface WeekInfoLocale {
  getWeekInfo?: () => { firstDay?: number }
  weekInfo?: { firstDay?: number }
}

/**
 * The locale's first day of the week, from `Intl.Locale#getWeekInfo()` (or the
 * older `weekInfo` getter). Where neither exists: Sunday for `en-US`, Monday
 * otherwise (design/calendar.md §3.4).
 */
export function localeWeekStart(locale: string): Weekday {
  try {
    const loc = new Intl.Locale(locale) as Intl.Locale & WeekInfoLocale
    const info = typeof loc.getWeekInfo === 'function' ? loc.getWeekInfo() : loc.weekInfo
    const first = info?.firstDay
    // Intl numbers weekdays 1 (Monday) … 7 (Sunday).
    if (typeof first === 'number' && first >= 1 && first <= 7) return (first % 7) as Weekday
  } catch {
    // An invalid tag: fall through to the fixed rule.
  }
  return /^en-US$/i.test(locale) ? 0 : 1
}

/** The configured `week_start` as a weekday; `locale` asks the reader's locale. */
export function resolveWeekStart(setting: WeekStart, locale: string): Weekday {
  switch (setting) {
    case 'monday':
      return 1
    case 'sunday':
      return 0
    case 'saturday':
      return 6
    case 'locale':
      return localeWeekStart(locale)
  }
}

/** The seven weekdays in column order. */
export function weekdayOrder(weekStart: Weekday): Weekday[] {
  return Array.from({ length: 7 }, (_, i) => ((weekStart + i) % 7) as Weekday)
}

/**
 * `month` as six rows of seven days starting on `weekStart`. The first row
 * holds the 1st; leading and trailing days belong to the neighbouring months.
 * Throws on an invalid month key.
 */
export function monthGrid(month: MonthKey, weekStart: Weekday): MonthGrid {
  const parts = parseMonthKey(month)
  if (!parts) throw new RangeError(`not a month key: ${month}`)
  const first = dateKey(parts.year, parts.month, 1)
  const start = addDays(first, -((weekdayOf(first) - weekStart + 7) % 7))
  const weeks: GridWeek[] = []
  for (let row = 0; row < GRID_ROWS; row++) {
    const days = Array.from({ length: 7 }, (_, i) => addDays(start, row * 7 + i))
    const thursday = days[(4 - weekStart + 7) % 7]
    weeks.push({ days, isoWeek: isoWeekOf(thursday).key })
  }
  return { month, weekStart, weeks }
}

/**
 * The grid keyboard map shared by every calendar grid: where a key moves the
 * focused day, or `null` for a key the grid does not handle. Arrows and
 * `h j k l` move by a day or a week; PageUp / PageDown by a month (clamped).
 * Modifier keys are the caller's to filter.
 */
export function moveFocus(key: string, focus: DateKey): DateKey | null {
  switch (key) {
    case 'ArrowLeft':
    case 'h':
      return addDays(focus, -1)
    case 'ArrowRight':
    case 'l':
      return addDays(focus, 1)
    case 'ArrowUp':
    case 'k':
      return addDays(focus, -7)
    case 'ArrowDown':
    case 'j':
      return addDays(focus, 7)
    case 'PageUp':
      return addMonthsToDate(focus, -1)
    case 'PageDown':
      return addMonthsToDate(focus, 1)
    default:
      return null
  }
}

/** A UTC `Date` for a date key, for `Intl` formatting with `timeZone: 'UTC'`. */
export function utcDate(date: DateKey): Date {
  return new Date(`${date}T00:00:00Z`)
}

/** Localized formatters, cached per locale and options. */
const formatters = new Map<string, Intl.DateTimeFormat>()

/** `date` formatted in `locale` (a date key, read as a calendar date). */
export function formatDate(date: DateKey, locale: string, options: Intl.DateTimeFormatOptions): string {
  const cacheKey = `${locale}|${JSON.stringify(options)}`
  let fmt = formatters.get(cacheKey)
  if (!fmt) {
    try {
      fmt = new Intl.DateTimeFormat(locale, { ...options, timeZone: 'UTC' })
    } catch {
      fmt = new Intl.DateTimeFormat(undefined, { ...options, timeZone: 'UTC' })
    }
    formatters.set(cacheKey, fmt)
  }
  return fmt.format(utcDate(date))
}

/** Column headers: the weekdays' narrow (or short) names in column order. */
export function weekdayLabels(weekStart: Weekday, locale: string, width: 'narrow' | 'short' = 'narrow'): string[] {
  // 2023-01-01 was a Sunday.
  return weekdayOrder(weekStart).map((d) => formatDate(dateKey(2023, 1, 1 + d), locale, { weekday: width }))
}
