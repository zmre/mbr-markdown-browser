/**
 * Search date expressions: the TypeScript mirror of
 * `crates/mbr-core/src/date_expr.rs` (design/calendar.md §6.1), used where the
 * browser filters by date itself (static builds, whose Pagefind index has no
 * facets).
 *
 * Both sides are held to one table of cases,
 * `tests/fixtures/date-expr-cases.json`, so they cannot drift. The grammar and
 * its choices (ISO weeks, `-N` counting today, a reversed range swapped, a
 * relative start before year 1000 open-ended, nothing past 9999-12-31) are
 * documented in the Rust module.
 *
 * Pure, importing only `period.ts` (the one period-key parser and the civil
 * arithmetic): safe for the main bundle and any chunk. Day arithmetic is on
 * integer day numbers, never on `Date`.
 */

import type { DateKey } from './note-date.js'
import { civilFromDays, daysFromCivil, daysInMonth, isoWeekday, keyOfDays, monthSpan, parsePeriodDays, type DaySpan as Span } from './period.js'

/** An inclusive range of date keys; `null` is an open end. */
export interface DateRange {
  readonly from: DateKey | null
  readonly to: DateKey | null
}

const MAX_EXPR_LEN = 32
const MAX_COUNT_DIGITS = 6
const MAX_YEAR = 9999
const MIN_RELATIVE_YEAR = 1000

/**
 * The range that matches nothing: an expression starting past 9999-12-31
 * (`>9999`). `from` after `to`, so `rangeContains` needs no special case; the
 * same constant as Rust's `DateRange::EMPTY`. A five-digit key must never be
 * produced: `'10000-01-01' < '2026-…'` as strings, so it would match every date.
 */
export const EMPTY_RANGE: DateRange = Object.freeze({ from: '9999-12-31', to: '1000-01-01' })

/** Whether no date lies in `range`. */
export const isEmptyRange = (range: DateRange): boolean =>
  range.from !== null && range.to !== null && range.from > range.to

/** Day number of 9999-12-31, the last date an expression can name. */
const LAST_DAY = daysFromCivil(MAX_YEAR, 12, 31)

/**
 * A resolved span of day numbers as date keys: a start past `LAST_DAY` is the
 * empty range, an end past it is clamped to it (Rust's `DateRange::bounded`).
 */
function rangeOfDays(from: number | null, to: number | null): DateRange {
  if (from !== null && from > LAST_DAY) return EMPTY_RANGE
  return { from: from === null ? null : keyOfDays(from), to: to === null ? null : keyOfDays(Math.min(to, LAST_DAY)) }
}

/** A period key as a span of days, or `null` (`period.ts` is the one parser). */
const parsePeriod = (s: string): Span | null => parsePeriodDays(s)

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
    return rangeOfDays(expr.from?.start ?? null, expr.to?.end ?? null)
  }
  const [y, m, d] = today.split('-').map(Number)
  const { from, to } = expr.resolve(daysFromCivil(y, m, d))
  return rangeOfDays(from, to)
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
