/**
 * Search date expressions: the TypeScript mirror of
 * `crates/mbr-core/src/date_expr.rs` (design/calendar.md §6.1), used where the
 * browser filters by date itself (static builds, whose Pagefind index has no
 * facets).
 *
 * Both sides are held to one table of cases,
 * `tests/fixtures/date-expr-cases.json`, so they cannot drift. The grammar and
 * its choices (ISO weeks, `-N` counting today, a reversed range swapped, a
 * relative start before year 1000 open-ended) are documented in the Rust
 * module.
 *
 * Pure, with a type-only import: safe for the main bundle and any chunk. Day
 * arithmetic is on integer day numbers (days since 1970-01-01), never on
 * `Date`, whose `Date.UTC` reads years 0–99 as 1900–1999.
 */

import type { DateKey } from './note-date.js'

/** An inclusive range of date keys; `null` is an open end. */
export interface DateRange {
  readonly from: DateKey | null
  readonly to: DateKey | null
}

const MAX_EXPR_LEN = 32
const MAX_COUNT_DIGITS = 6
const MIN_YEAR = 1000
const MAX_YEAR = 9999
const MIN_RELATIVE_YEAR = 1000

/** A period as an inclusive span of day numbers. */
interface Span {
  readonly start: number
  readonly end: number
}

// ---- Civil-date arithmetic (Howard Hinnant's algorithms) ----

/** Day number (days since 1970-01-01) of a proleptic Gregorian date. */
function daysFromCivil(year: number, month: number, day: number): number {
  const y = month <= 2 ? year - 1 : year
  const era = Math.floor(y / 400)
  const yoe = y - era * 400
  const mp = (month + 9) % 12
  const doy = Math.floor((153 * mp + 2) / 5) + day - 1
  const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy
  return era * 146097 + doe - 719468
}

/** The date of a day number. */
function civilFromDays(days: number): { year: number; month: number; day: number } {
  const z = days + 719468
  const era = Math.floor(z / 146097)
  const doe = z - era * 146097
  const yoe = Math.floor((doe - Math.floor(doe / 1460) + Math.floor(doe / 36524) - Math.floor(doe / 146096)) / 365)
  const doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100))
  const mp = Math.floor((5 * doy + 2) / 153)
  const day = doy - Math.floor((153 * mp + 2) / 5) + 1
  const month = mp < 10 ? mp + 3 : mp - 9
  return { year: yoe + era * 400 + (month <= 2 ? 1 : 0), month, day }
}

const pad = (n: number, width: number): string => String(n).padStart(width, '0')

function keyOf(days: number): DateKey {
  const { year, month, day } = civilFromDays(days)
  return `${pad(year, 4)}-${pad(month, 2)}-${pad(day, 2)}`
}

function daysInMonth(year: number, month: number): number {
  const next = month === 12 ? daysFromCivil(year + 1, 1, 1) : daysFromCivil(year, month + 1, 1)
  return next - daysFromCivil(year, month, 1)
}

/** Monday = 0 … Sunday = 6. Day 0 (1970-01-01) was a Thursday. */
function isoWeekday(days: number): number {
  return (((days + 3) % 7) + 7) % 7
}

/** The Monday of ISO week 1 of `isoYear`: the week holding January 4. */
function isoWeekOneMonday(isoYear: number): number {
  const jan4 = daysFromCivil(isoYear, 1, 4)
  return jan4 - isoWeekday(jan4)
}

/** The ISO week-year of a day: the calendar year of its week's Thursday. */
function isoYearOf(days: number): number {
  return civilFromDays(days - isoWeekday(days) + 3).year
}

// ---- Periods ----

const DIGITS = /^[0-9]+$/

const inYearRange = (year: number): boolean => year >= MIN_YEAR && year <= MAX_YEAR

function monthSpan(year: number, month: number, months: number): Span {
  const start = daysFromCivil(year, month, 1)
  const endMonth0 = month - 1 + months
  const end = daysFromCivil(year + Math.floor(endMonth0 / 12), (endMonth0 % 12) + 1, 1) - 1
  return { start, end }
}

/**
 * A period key (`2026`, `2026-10`, `2026-10-31`, `2026-W44`, `2026-Q4`, with
 * `W`/`Q` in either case) as a span of days, or `null`.
 */
function parsePeriod(s: string): Span | null {
  const year = Number(s.slice(0, 4))
  if (!DIGITS.test(s.slice(0, 4)) || !inYearRange(year)) return null
  if (s.length === 4) return monthSpan(year, 1, 12)
  if (s[4] !== '-') return null
  const rest = s.slice(5)
  if (s.length === 10 && s[7] === '-' && DIGITS.test(rest.slice(0, 2)) && DIGITS.test(rest.slice(3))) {
    const month = Number(rest.slice(0, 2))
    const day = Number(rest.slice(3))
    if (month < 1 || month > 12 || day < 1 || day > daysInMonth(year, month)) return null
    const d = daysFromCivil(year, month, day)
    return { start: d, end: d }
  }
  if (s.length !== 7 && s.length !== 8) return null
  const tag = rest[0].toUpperCase()
  if (s.length === 8 && tag === 'W' && DIGITS.test(rest.slice(1))) {
    const week = Number(rest.slice(1))
    const monday = isoWeekOneMonday(year) + (week - 1) * 7
    if (week < 1 || isoYearOf(monday) !== year) return null
    return { start: monday, end: monday + 6 }
  }
  if (s.length === 7 && tag === 'Q' && /^[1-4]$/.test(rest[1])) {
    return monthSpan(year, Number(rest[1]) * 3 - 2, 3)
  }
  if (s.length === 7 && DIGITS.test(rest)) {
    const month = Number(rest)
    return month >= 1 && month <= 12 ? monthSpan(year, month, 1) : null
  }
  return null
}

// ---- Expressions ----

/** A parsed expression, resolved against `today` by `resolveDateExpr`. */
export type DateExpr =
  | { readonly kind: 'span'; readonly from: Span | null; readonly to: Span | null }
  | { readonly kind: 'relative'; readonly resolve: (today: number) => { from: number | null; to: number } }

const RELATIVE_KEYWORDS: ReadonlyMap<string, (today: number) => { from: number; to: number }> = new Map(Object.entries({
  today: (t) => ({ from: t, to: t }),
  yesterday: (t) => ({ from: t - 1, to: t - 1 }),
  'this-week': (t) => weekOf(t),
  'last-week': (t) => weekOf(t - 7),
  'this-month': (t) => {
    const { year, month } = civilFromDays(t)
    const span = monthSpan(year, month, 1)
    return { from: span.start, to: span.end }
  },
  'last-month': (t) => {
    const { year, month } = civilFromDays(t)
    const span = month === 1 ? monthSpan(year - 1, 12, 1) : monthSpan(year, month - 1, 1)
    return { from: span.start, to: span.end }
  },
  'this-year': (t) => {
    const span = monthSpan(civilFromDays(t).year, 1, 12)
    return { from: span.start, to: span.end }
  },
}))

function weekOf(day: number): { from: number; to: number } {
  const monday = day - isoWeekday(day)
  return { from: monday, to: monday + 6 }
}

/** The day after the same date `months` months before `today`, clamped. */
function monthsBackInclusive(today: number, months: number): number {
  const { year, month, day } = civilFromDays(today)
  const total = year * 12 + (month - 1) - months
  const y = Math.floor(total / 12)
  const m = total - y * 12 + 1
  return daysFromCivil(y, m, Math.min(day, daysInMonth(y, m))) + 1
}

function parseRelative(value: string): DateExpr | null {
  const lower = value.toLowerCase()
  const keyword = RELATIVE_KEYWORDS.get(lower)
  if (keyword) return { kind: 'relative', resolve: keyword }
  const m = /^-([0-9]+)([dwmy])$/.exec(lower)
  if (!m || m[1].length > MAX_COUNT_DIGITS) return null
  const count = Number(m[1])
  if (count === 0) return null
  const unit = m[2]
  return {
    kind: 'relative',
    resolve: (today) => {
      const start =
        unit === 'd'
          ? today - (count - 1)
          : unit === 'w'
            ? today - (count * 7 - 1)
            : monthsBackInclusive(today, unit === 'm' ? count : count * 12)
      return { from: civilFromDays(start).year >= MIN_RELATIVE_YEAR ? start : null, to: today }
    },
  }
}

/** Parses a facet value, or `null` when it is not a date expression. */
export function parseDateExpr(value: string): DateExpr | null {
  if (value.length === 0 || value.length > MAX_EXPR_LEN || !/^[\x00-\x7f]*$/.test(value)) return null
  const dots = value.indexOf('..')
  if (dots >= 0) {
    const side = (s: string): Span | null | undefined => (s === '' ? null : (parsePeriod(s) ?? undefined))
    const from = side(value.slice(0, dots))
    const to = side(value.slice(dots + 2))
    if (from === undefined || to === undefined || (from === null && to === null)) return null
    if (from && to && from.start > to.end) return { kind: 'span', from: to, to: from }
    return { kind: 'span', from, to }
  }
  const compare = /^(>=|>|<=|<)(.*)$/.exec(value)
  if (compare) {
    const p = parsePeriod(compare[2])
    if (!p) return null
    switch (compare[1]) {
      case '>=':
        return { kind: 'span', from: p, to: null }
      case '>':
        return { kind: 'span', from: { start: p.end + 1, end: p.end + 1 }, to: null }
      case '<=':
        return { kind: 'span', from: null, to: p }
      default:
        return { kind: 'span', from: null, to: { start: p.start - 1, end: p.start - 1 } }
    }
  }
  const relative = parseRelative(value)
  if (relative) return relative
  const period = parsePeriod(value)
  return period ? { kind: 'span', from: period, to: period } : null
}

/** The dates `expr` names, relative forms resolved against `today`. */
export function resolveDateExpr(expr: DateExpr, today: DateKey): DateRange {
  if (expr.kind === 'span') {
    return {
      from: expr.from ? keyOf(expr.from.start) : null,
      to: expr.to ? keyOf(expr.to.end) : null,
    }
  }
  const [y, m, d] = today.split('-').map(Number)
  const { from, to } = expr.resolve(daysFromCivil(y, m, d))
  return { from: from === null ? null : keyOf(from), to: keyOf(to) }
}

/** Parses `value` and resolves it against `today` in one call. */
export function evaluateDateExpr(value: string, today: DateKey): DateRange | null {
  const expr = parseDateExpr(value)
  return expr ? resolveDateExpr(expr, today) : null
}

/** Whether `date` lies in `range` (date keys compare as strings). */
export function rangeContains(range: DateRange, date: DateKey): boolean {
  return (range.from === null || date >= range.from) && (range.to === null || date <= range.to)
}
