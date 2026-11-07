/**
 * Periodic notes on the client: a per-kind sorted index built from
 * `site.json`, and the lookups the `d` key (and later the companion) needs.
 *
 * Pure, with type-only imports: safe for the main bundle and for any lazy
 * chunk. The stateful loader (memoized off `shared.ts`'s `siteNav`) lives in
 * `periodic-nav.ts`, which a chunk must not import.
 *
 * The server decides which notes are periodic (`calendar.period` in
 * `site.json`, after the `[calendar]` `periods` / `periodic_folders` filters;
 * see `crates/mbr-core/src/calendar.rs`); this module only indexes them, with
 * the server's duplicate rule (the first by `url_path` wins), so the client
 * and the server-rendered bar agree on which note is "the" note for a period.
 *
 * Canonical period keys (`2026-10-31`, `2026-W44`, `2026-10`, `2026-Q4`,
 * `2026`) are fixed-width per kind, so string order is chronological order.
 */

import type { DateKey } from './note-date.js'

export type PeriodKind = 'day' | 'week' | 'month' | 'quarter' | 'year'

/** Every kind, smallest first (the server's order). */
export const PERIOD_KINDS: readonly PeriodKind[] = ['day', 'week', 'month', 'quarter', 'year']

/** One periodic note: its canonical period key and site-absolute URL. */
export interface PeriodicEntry {
  readonly period: string
  readonly url: string
}

/** Periodic notes per kind, sorted by period. */
export type PeriodicIndex = Readonly<Record<PeriodKind, readonly PeriodicEntry[]>>

/** The `site.json` fields `buildPeriodicIndex` reads. */
export interface PeriodicSource {
  readonly url_path: string
  readonly raw_path?: string
  readonly calendar?: { readonly period?: string } | null
}

const KEY_PATTERNS: ReadonlyArray<readonly [PeriodKind, RegExp]> = [
  ['day', /^\d{4}-\d{2}-\d{2}$/],
  ['week', /^\d{4}-W\d{2}$/],
  ['month', /^\d{4}-\d{2}$/],
  ['quarter', /^\d{4}-Q[1-4]$/],
  ['year', /^\d{4}$/],
]

/** The kind of a canonical period key, or `null` for anything else. */
export function periodKindOf(key: string): PeriodKind | null {
  for (const [kind, pattern] of KEY_PATTERNS) {
    if (pattern.test(key)) return kind
  }
  return null
}

const byString = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0)

/**
 * Builds the index in one sort and one pass. Entries without a recognizable
 * `calendar.period` are skipped; of several notes naming one period, the
 * first by `url_path` (then `raw_path`) wins, as on the server.
 */
export function buildPeriodicIndex(files: readonly PeriodicSource[]): PeriodicIndex {
  const periodic = files
    .filter((f) => typeof f.calendar?.period === 'string')
    .slice()
    .sort((a, b) => byString(a.url_path, b.url_path) || byString(a.raw_path ?? '', b.raw_path ?? ''))
  const index: Record<PeriodKind, PeriodicEntry[]> = { day: [], week: [], month: [], quarter: [], year: [] }
  const seen = new Set<string>()
  for (const file of periodic) {
    const period = file.calendar!.period!
    const kind = periodKindOf(period)
    if (!kind || seen.has(period)) continue
    seen.add(period)
    index[kind].push({ period, url: file.url_path })
  }
  for (const kind of PERIOD_KINDS) index[kind].sort((a, b) => byString(a.period, b.period))
  return index
}

const indexCache = new WeakMap<readonly PeriodicSource[], PeriodicIndex>()

/**
 * {@link buildPeriodicIndex}, memoized per `markdown_files` array: the browse
 * sections re-render often, and `site.json` is one array per page.
 */
export function periodicIndexOf(files: readonly PeriodicSource[]): PeriodicIndex {
  let index = indexCache.get(files)
  if (!index) {
    index = buildPeriodicIndex(files)
    indexCache.set(files, index)
  }
  return index
}

/** Index of the first entry whose period is `>= period` (binary search). */
function lowerBound(entries: readonly PeriodicEntry[], period: string): number {
  let lo = 0
  let hi = entries.length
  while (lo < hi) {
    const mid = (lo + hi) >>> 1
    if (entries[mid].period < period) lo = mid + 1
    else hi = mid
  }
  return lo
}

/** The note for exactly `period`, if any. */
export function findPeriod(index: PeriodicIndex, period: string): PeriodicEntry | null {
  const kind = periodKindOf(period)
  if (!kind) return null
  const entries = index[kind]
  const at = entries[lowerBound(entries, period)]
  return at?.period === period ? at : null
}

/**
 * The daily note for `today` (the reader's local date); else the latest one
 * before today; else the earliest one after. `null` when there are no day
 * notes at all.
 */
export function todayDailyNote(index: PeriodicIndex, today: DateKey): PeriodicEntry | null {
  const days = index.day
  if (days.length === 0) return null
  const i = lowerBound(days, today)
  if (days[i]?.period === today) return days[i]
  return i > 0 ? days[i - 1] : days[0]
}

/**
 * The nearest note of the same kind before (`-1`) or after (`1`) `period`,
 * skipping gaps. `period` need not have a note itself.
 */
export function adjacent(index: PeriodicIndex, period: string, delta: 1 | -1): PeriodicEntry | null {
  const kind = periodKindOf(period)
  if (!kind) return null
  const entries = index[kind]
  const i = lowerBound(entries, period)
  if (delta < 0) return i > 0 ? entries[i - 1] : null
  const next = entries[i]?.period === period ? i + 1 : i
  return entries[next] ?? null
}
