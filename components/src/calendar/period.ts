/**
 * Canonical period keys and the civil-date arithmetic under them: the one
 * place the client parses `2026-10-31`, `2026-W44`, `2026-10`, `2026-Q4` and
 * `2026` (`W`/`Q` in either case, as `crates/mbr-core/src/calendar.rs`'s
 * `Period` accepts them).
 *
 * Everything else is built on {@link periodSpan}: the search date grammar
 * (`date-expr.ts`), the grid's ISO weeks and period starts (`month-grid.ts`),
 * the periodic index's kinds (`periodic.ts`) and the overlay's jump picker
 * (`overlay-model.ts`).
 *
 * Pure, no imports: safe for the main bundle and any chunk. Day arithmetic is
 * on integer day numbers (days since 1970-01-01), through `Date`'s UTC
 * calendar but never `Date.UTC`, which reads years 0–99 as 1900–1999.
 */

/** A `YYYY-MM-DD` calendar date (the same alias as `note-date.ts`'s). */
type DateKey = string

export type PeriodKind = 'day' | 'week' | 'month' | 'quarter' | 'year'

/** A period as an inclusive span of day numbers. */
export interface DaySpan {
  readonly start: number
  readonly end: number
}

/** A parsed period: its kind, its canonical key (`W`/`Q` upper case) and its days. */
export interface PeriodSpan {
  readonly kind: PeriodKind
  readonly key: string
  readonly start: DateKey
  readonly end: DateKey
}

const MIN_YEAR = 1000

// ---- Civil-date arithmetic ----

const DAY_MS = 86_400_000

/**
 * Day number (days since 1970-01-01) of a proleptic Gregorian date; `NaN`
 * past `Date`'s ±273 790 years, which callers treat as "out of range"
 * (`date-expr.ts` opens such an end). `setUTCFullYear`, not `Date.UTC`, which
 * reads years 0–99 as 1900–1999.
 */
export function daysFromCivil(year: number, month: number, day: number): number {
  const d = new Date(0)
  d.setUTCFullYear(year, month - 1, day)
  return d.getTime() / DAY_MS
}

/** The date of a day number (`NaN` parts past `Date`'s range). */
export function civilFromDays(days: number): { year: number; month: number; day: number } {
  const d = new Date(days * DAY_MS)
  return { year: d.getUTCFullYear(), month: d.getUTCMonth() + 1, day: d.getUTCDate() }
}

const pad = (n: number, width: number): string => String(n).padStart(width, '0')

/** The date key of a day number. */
export function keyOfDays(days: number): DateKey {
  const { year, month, day } = civilFromDays(days)
  return `${pad(year, 4)}-${pad(month, 2)}-${pad(day, 2)}`
}

/** Days in a month (1–12). */
export function daysInMonth(year: number, month: number): number {
  const next = month === 12 ? daysFromCivil(year + 1, 1, 1) : daysFromCivil(year, month + 1, 1)
  return next - daysFromCivil(year, month, 1)
}

/** Monday = 0 … Sunday = 6. Day 0 (1970-01-01) was a Thursday. */
export function isoWeekday(days: number): number {
  return (((days + 3) % 7) + 7) % 7
}

/** The Monday of ISO week 1 of `isoYear`: the week holding January 4. */
function isoWeekOneMonday(isoYear: number): number {
  const jan4 = daysFromCivil(isoYear, 1, 4)
  return jan4 - isoWeekday(jan4)
}

/** The ISO week key (`2026-W53`) of a day: the week-year is its Thursday's year. */
export function isoWeekKeyOfDays(days: number): string {
  const thursday = days - isoWeekday(days) + 3
  const year = civilFromDays(thursday).year
  return `${pad(year, 4)}-W${pad(Math.floor((thursday - isoWeekOneMonday(year)) / 7) + 1, 2)}`
}

/** Days `[start, start + months)` from the 1st of `month`. */
export function monthSpan(year: number, month: number, months: number): DaySpan {
  const start = daysFromCivil(year, month, 1)
  const endMonth0 = month - 1 + months
  const end = daysFromCivil(year + Math.floor(endMonth0 / 12), (endMonth0 % 12) + 1, 1) - 1
  return { start, end }
}

// ---- Period keys ----

/** `YYYY`, `YYYY-MM`, `YYYY-MM-DD`, `YYYY-Www`, `YYYY-Qn` (`W`/`Q` either case). */
const PERIOD_KEY = /^(\d{4})(?:-(?:(\d\d)(?:-(\d\d))?|[Ww](\d\d)|[Qq]([1-4])))?$/

/**
 * A period key as a kind, canonical key and span of day numbers, or `null`.
 * Years 1000–9999; the date, month and ISO week must exist.
 */
export function parsePeriodDays(s: string): (DaySpan & { readonly kind: PeriodKind; readonly key: string }) | null {
  const m = PERIOD_KEY.exec(s)
  const year = m ? Number(m[1]) : 0
  if (!m || year < MIN_YEAR) return null
  const [, head, mm, dd, ww, q] = m
  if (ww) {
    const week = Number(ww)
    const monday = isoWeekOneMonday(year) + (week - 1) * 7
    // Week 53 exists only in a long year: its Thursday must still be in `year`.
    return week >= 1 && civilFromDays(monday + 3).year === year
      ? { kind: 'week', key: `${head}-W${ww}`, start: monday, end: monday + 6 }
      : null
  }
  if (q) return { kind: 'quarter', key: `${head}-Q${q}`, ...monthSpan(year, Number(q) * 3 - 2, 3) }
  if (!mm) return { kind: 'year', key: s, ...monthSpan(year, 1, 12) }
  const month = Number(mm)
  if (month < 1 || month > 12) return null
  if (!dd) return { kind: 'month', key: s, ...monthSpan(year, month, 1) }
  const day = Number(dd)
  if (day < 1 || day > daysInMonth(year, month)) return null
  const d = daysFromCivil(year, month, day)
  return { kind: 'day', key: s, start: d, end: d }
}

/** A period key's kind, canonical key and first and last day, or `null`. */
export function periodSpan(key: string): PeriodSpan | null {
  const p = parsePeriodDays(key)
  return p ? { kind: p.kind, key: p.key, start: keyOfDays(p.start), end: keyOfDays(p.end) } : null
}

/**
 * The canonical key of each period kind containing period `key` — the mirror
 * of Rust's `Period::containing`: everything is anchored on the period's
 * first day, except that a **week** belongs to its Thursday's month, quarter
 * and year (ISO). So day `2025-12-30` is in `2026-W01` but month `2025-12`,
 * while week `2026-W01` is in month `2026-01`. Kinds larger than `key`'s are
 * the containing periods; smaller ones are those of its first day. `null`
 * for anything that is not a period key.
 */
export function periodKeysContaining(key: string): Readonly<Record<PeriodKind, string>> | null {
  const span = parsePeriodDays(key)
  if (!span) return null
  const anchor = civilFromDays(span.kind === 'week' ? span.start + 3 : span.start)
  const year = pad(anchor.year, 4)
  return {
    day: keyOfDays(span.start),
    week: isoWeekKeyOfDays(span.start),
    month: `${year}-${pad(anchor.month, 2)}`,
    quarter: `${year}-Q${Math.floor((anchor.month - 1) / 3) + 1}`,
    year,
  }
}
