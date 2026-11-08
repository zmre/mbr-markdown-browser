/**
 * Month-grid arithmetic: week starts, the six-row month grid, the ranges and
 * titles of a month / week view, ISO week labels, month keys, localized date
 * and clock labels, and **the** calendar keyboard map.
 *
 * Pure, importing only the other pure calendar modules (`note-date.ts`,
 * `period.ts`): the one place grid math lives, for the companion's mini
 * month, the calendar overlay and the task panel's grids (tasks chunk, which
 * imports this module directly). Everything works on `YYYY-MM-DD` date keys
 * and UTC calendar arithmetic, so no daylight-saving change can move a cell.
 */
import type { WeekStart } from './config.js'
import { addDays, dateKey, dayNumber, parseDateKey, type DateKey } from './note-date.js'
import { daysInMonth, isoWeekKeyOfDays, isoWeekday, periodSpan } from './period.js'

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
/** Year and month (1–12) `delta` months after `year`-`month`. */
function shiftMonth(year: number, month: number, delta: number): { year: number; month: number } {
  const total = year * 12 + month - 1 + delta
  const y = Math.floor(total / 12)
  return { year: y, month: total - y * 12 + 1 }
}

/** `month` moved by `delta` months. Throws on an invalid key. */
export function addMonths(month: MonthKey, delta: number): MonthKey {
  const parts = parseMonthKey(month)
  if (!parts) throw new RangeError(`not a month key: ${month}`)
  const moved = shiftMonth(parts.year, parts.month, delta)
  return monthKey(moved.year, moved.month)
}

/**
 * `date` moved by `delta` months, the day clamped to the target month's
 * length (Jan 31 + 1 month = Feb 28/29), as PageDown does in every calendar.
 */
export function addMonthsToDate(date: DateKey, delta: number): DateKey {
  const parts = parseDateKey(date)
  if (!parts) throw new RangeError(`not a date key: ${date}`)
  const { year, month } = shiftMonth(parts.year, parts.month, delta)
  return dateKey(year, month, Math.min(parts.day, daysInMonth(year, month)))
}

/** The weekday of a date key. Throws on an invalid key. */
export function weekdayOf(date: DateKey): Weekday {
  return ((isoWeekday(dayNumber(date)) + 1) % 7) as Weekday
}

/** The ISO 8601 week key of a date (`2026-W53`). Throws on an invalid key. */
export function isoWeekOf(date: DateKey): string {
  return isoWeekKeyOfDays(dayNumber(date))
}

/** The Monday an ISO week (`2026-W44`, `w` either case) starts on, or `null` for another key. */
export function isoWeekStart(key: string): DateKey | null {
  const span = periodSpan(key)
  return span?.kind === 'week' ? span.start : null
}

/**
 * The first day of a period key (`2026-10-31`, `2026-W44`, `2026-10`,
 * `2026-Q4`, `2026`), or `null` for anything else.
 */
export function periodStart(key: string): DateKey | null {
  return periodSpan(key)?.start ?? null
}

/** The week-number label of an ISO week key: `2026-W05` → `5`. */
export function weekNumberLabel(isoWeek: string): string {
  return String(Number(isoWeek.slice(6)))
}

/** The first day of the `weekStart` week holding `date`. */
export function weekStartOf(date: DateKey, weekStart: Weekday): DateKey {
  return addDays(date, -((weekdayOf(date) - weekStart + 7) % 7))
}

/** An inclusive range of date keys. */
export interface DayRange {
  readonly from: DateKey
  readonly to: DateKey
}

/** The `weekStart` week holding `date`. */
export function weekRange(date: DateKey, weekStart: Weekday): DayRange {
  const from = weekStartOf(date, weekStart)
  return { from, to: addDays(from, 6) }
}

/** The six week rows of `month`'s grid (42 days): what a month grid draws. */
export function gridMonthRange(month: MonthKey, weekStart: Weekday): DayRange {
  const from = weekStartOf(`${month}-01`, weekStart)
  return { from, to: addDays(from, GRID_ROWS * 7 - 1) }
}

/** A calendar view: the overlay's three, of which the task grids use two. */
export type CalendarView = 'month' | 'week' | 'list'

/**
 * The days a view of `focus` is about: its `weekStart` week in Week, its
 * calendar month otherwise (the month grid draws more: {@link gridMonthRange}).
 */
export function viewRange(view: CalendarView, focus: DateKey, weekStart: Weekday): DayRange {
  if (view === 'week') return weekRange(focus, weekStart)
  const from = `${monthOf(focus)}-01`
  return { from, to: addDays(addMonthsToDate(from, 1), -1) }
}

/** A view's title: "October 2026", or "Oct 26 – Nov 1, 2026" for a week. */
export function viewTitle(view: CalendarView, focus: DateKey, weekStart: Weekday, locale: string): string {
  if (view !== 'week') return formatDate(`${monthOf(focus)}-01`, locale, MONTH_YEAR)
  const { from, to } = weekRange(focus, weekStart)
  return `${formatDate(from, locale, SHORT_DAY)} – ${formatDate(to, locale, SHORT_DAY_YEAR)}`
}

const MONTH_YEAR: Intl.DateTimeFormatOptions = { month: 'long', year: 'numeric' }
const SHORT_DAY: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' }
const SHORT_DAY_YEAR: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric', year: 'numeric' }

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
  const start = weekStartOf(dateKey(parts.year, parts.month, 1), weekStart)
  const weeks: GridWeek[] = []
  for (let row = 0; row < GRID_ROWS; row++) {
    const days = Array.from({ length: 7 }, (_, i) => addDays(start, row * 7 + i))
    const thursday = days[(4 - weekStart + 7) % 7]
    weeks.push({ days, isoWeek: isoWeekOf(thursday) })
  }
  return { weeks }
}

/** What a key does in a calendar grid (design/calendar.md §5.5). */
export type OverlayCommand =
  | 'left'
  | 'right'
  | 'up'
  | 'down'
  | 'prev'
  | 'next'
  | 'today'
  | 'month'
  | 'week'
  | 'list'
  | 'jump'
  | 'plot'
  | 'filter'
  | 'open'
  | 'escape'
  | 'help'

const KEY_COMMANDS: ReadonlyMap<string, OverlayCommand> = new Map(
  Object.entries({
    ArrowLeft: 'left',
    h: 'left',
    ArrowRight: 'right',
    l: 'right',
    ArrowUp: 'up',
    k: 'up',
    ArrowDown: 'down',
    j: 'down',
    '[': 'prev',
    PageUp: 'prev',
    ']': 'next',
    PageDown: 'next',
    t: 'today',
    m: 'month',
    w: 'week',
    a: 'list',
    g: 'jump',
    p: 'plot',
    '/': 'filter',
    Enter: 'open',
    Escape: 'escape',
    '?': 'help',
  }) as [string, OverlayCommand][]
)

/**
 * **The** calendar key map: the command for an unmodified key, or `null`.
 * The overlay, the task panel's grids and (through {@link moveFocus}) the
 * mini month all read it; modifier keys are the caller's to filter.
 */
export function overlayCommand(key: string): OverlayCommand | null {
  return KEY_COMMANDS.get(key) ?? null
}

/**
 * Where a movement command takes the focused day in Month and Week (List
 * moves between entries instead): arrows by a day or a week, `prev` / `next`
 * by a month (clamped, Jan 31 → Feb 28), or a week in Week.
 */
export function moveDay(command: OverlayCommand, view: CalendarView, focus: DateKey): DateKey | null {
  switch (command) {
    case 'left':
      return addDays(focus, -1)
    case 'right':
      return addDays(focus, 1)
    case 'up':
      return addDays(focus, -7)
    case 'down':
      return addDays(focus, 7)
    case 'prev':
      return view === 'week' ? addDays(focus, -7) : addMonthsToDate(focus, -1)
    case 'next':
      return view === 'week' ? addDays(focus, 7) : addMonthsToDate(focus, 1)
    default:
      return null
  }
}

/**
 * The mini month's movement keys, on {@link overlayCommand} and
 * {@link moveDay} (Month view): arrows / `h j k l` and PageUp / PageDown.
 * `[` / `]` are left to the page, where they follow the periodic-note bar.
 */
export function moveFocus(key: string, focus: DateKey): DateKey | null {
  const command = key === '[' || key === ']' ? null : overlayCommand(key)
  return command ? moveDay(command, 'month', focus) : null
}

/** A UTC `Date` for a date key, for `Intl` formatting with `timeZone: 'UTC'`. */
export function utcDate(date: DateKey): Date {
  return new Date(`${date}T00:00:00Z`)
}

/** Localized formatters, cached per locale and options. */
const formatters = new Map<string, Intl.DateTimeFormat | null>()

/**
 * A UTC formatter for `locale` and `options`, built once. The cache key
 * names the option fields the calendar uses (cheaper than serializing the
 * object on every cell). An invalid locale falls back to the reader's;
 * `null` when even that throws (then callers format by hand).
 */
function formatter(locale: string, o: Intl.DateTimeFormatOptions): Intl.DateTimeFormat | null {
  const key = `${locale}|${o.dateStyle}|${o.weekday}|${o.year}|${o.month}|${o.day}|${o.hour}`
  let fmt = formatters.get(key)
  if (fmt === undefined) {
    fmt = null
    for (const tag of [locale, undefined]) {
      try {
        fmt = new Intl.DateTimeFormat(tag, { ...o, timeZone: 'UTC' })
        break
      } catch {
        // An invalid tag: try the reader's own; failing that, format by hand.
      }
    }
    formatters.set(key, fmt)
  }
  return fmt
}

/** `date` formatted in `locale` (a date key, read as a calendar date). */
export function formatDate(date: DateKey, locale: string, options: Intl.DateTimeFormatOptions): string {
  return formatter(locale, options)?.format(utcDate(date)) ?? date
}

/** `HH:MM` for minutes after midnight (24-hour; `24:00` for the day's end). */
export function formatMinutes(minutes: number): string {
  return `${String(Math.floor(minutes / 60)).padStart(2, '0')}:${String(minutes % 60).padStart(2, '0')}`
}

const CLOCK: Intl.DateTimeFormatOptions = { hour: 'numeric', minute: '2-digit' }

/**
 * A clock time in the reader's locale (`9:00 AM`, `09:00`); `HH:MM`
 * ({@link formatMinutes}) where `Intl` cannot format at all. The one clock
 * formatter: the companion's timeline, the overlay's entry times and
 * QuickLook's preview.
 */
export function formatClock(minutes: number, locale: string): string {
  const fmt = formatter(locale, CLOCK)
  return fmt ? fmt.format(new Date(Date.UTC(1970, 0, 1, Math.floor(minutes / 60), minutes % 60))) : formatMinutes(minutes)
}

/** An entry's time, or its time range (`9:00 AM–10:00 AM`), localized. */
export function formatSpan(entry: { readonly start: number; readonly end?: number }, locale: string): string {
  const start = formatClock(entry.start, locale)
  return entry.end === undefined ? start : `${start}–${formatClock(entry.end, locale)}`
}

/** Column headers: the weekdays' narrow (or short) names in column order. */
export function weekdayLabels(weekStart: Weekday, locale: string, width: 'narrow' | 'short' = 'narrow'): string[] {
  // 2023-01-01 was a Sunday.
  return weekdayOrder(weekStart).map((d) => formatDate(dateKey(2023, 1, 1 + d), locale, { weekday: width }))
}
