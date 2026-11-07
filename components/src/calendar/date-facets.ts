/**
 * Search date facets in the browser (design/calendar.md §6): `date:`,
 * `created:`, `modified:` and `when:` with a date-expression value.
 *
 * The server applies them itself (`crates/mbr-core/src/search.rs`). A static
 * build searches with Pagefind, which has no facets, so there the search modal
 * strips the date facets from the query (`splitDateFacets`) and filters by
 * date here, against `site.json` (`matchDateFacets`, `filterByDate`) — the
 * same rules, with the dates in the reader's time zone.
 *
 * Pure, with a type-only import surface apart from the two calendar modules:
 * safe for the main bundle and any chunk.
 */

import { evaluateDateExpr, rangeContains, type DateRange } from './date-expr.js'
import { localDateOf, noteDate, parseDateKey, type DateKey, type NoteCalendar } from './note-date.js'

/** The facet keys with date semantics (matched case-insensitively). */
export type DateFacetKey = 'date' | 'created' | 'modified' | 'when'

const DATE_FACET_KEYS: ReadonlySet<string> = new Set<DateFacetKey>(['date', 'created', 'modified', 'when'])

/** One date facet of a query, resolved against today. */
export interface DateFacet {
  readonly key: DateFacetKey
  readonly range: DateRange
}

/** Which of a file's dates a result matched on (the server's `DateField`). */
export type DateField = 'date' | 'created' | 'modified'

/** The server's `matched_date` on a search result. */
export interface MatchedDate {
  readonly date: DateKey
  readonly field: DateField
}

/** The `site.json` markdown-entry fields date matching reads. */
export interface DatedFile {
  readonly url_path: string
  /** Birthtime (else mtime), unix seconds. */
  readonly created?: number
  /** Modification time, unix seconds. */
  readonly modified?: number
  readonly calendar?: NoteCalendar | null
}

/**
 * A query split on whitespace, except inside a quoted facet value: the mirror
 * of `query_tokens` in `crates/mbr-core/src/search.rs`.
 */
export function queryTokens(q: string): string[] {
  const tokens: string[] = []
  let rest = q.trimStart()
  const nextSpace = (s: string): number => {
    const i = s.search(/\s/)
    return i < 0 ? s.length : i
  }
  while (rest.length > 0) {
    let end = nextSpace(rest)
    const open = rest.slice(0, end).indexOf(':"')
    if (open > 0) {
      const key = rest.slice(0, open)
      const valueStart = open + 2
      const close = rest.indexOf('"', valueStart)
      if (!/[:"]/.test(key) && close >= 0) {
        const after = close + 1
        end = after + nextSpace(rest.slice(after))
      }
    }
    tokens.push(rest.slice(0, end))
    rest = rest.slice(end).trimStart()
  }
  return tokens
}

/**
 * The `key` and unquoted `value` of a facet token, or `null` for a plain term
 * (the mirror of `parse_query`'s facet rule: no `://`, a colon neither first
 * nor last, both halves non-empty).
 */
export function facetOf(token: string): { key: string; value: string } | null {
  const colon = token.indexOf(':')
  if (colon <= 0 || colon === token.length - 1 || token.includes('://')) return null
  const key = token.slice(0, colon)
  let value = token.slice(colon + 1)
  if (value.startsWith('"')) value = value.endsWith('"') && value.length > 1 ? value.slice(1, -1) : value.slice(1)
  return value ? { key, value } : null
}

/** A query with its date facets taken out. */
export interface SplitQuery {
  /** Every other token, as typed (what Pagefind is given). */
  readonly text: string
  readonly facets: readonly DateFacet[]
}

/**
 * Separates `query`'s date facets — a date key with a value that is a date
 * expression, resolved against `today` — from everything else. A date key
 * whose value is not an expression stays in `text`, as on the server, where
 * it is a substring frontmatter match.
 */
export function splitDateFacets(query: string, today: DateKey): SplitQuery {
  const text: string[] = []
  const facets: DateFacet[] = []
  for (const token of queryTokens(query)) {
    const facet = facetOf(token)
    const key = facet?.key.toLowerCase()
    const range = facet && key && DATE_FACET_KEYS.has(key) ? evaluateDateExpr(facet.value, today) : null
    if (range) facets.push({ key: key as DateFacetKey, range })
    else text.push(token)
  }
  return { text: text.join(' '), facets }
}

/**
 * Every facet against one note: `null` when one fails, else the date the
 * first facet matched on (the same rule as the server's `DateFacet::matches`:
 * `when:` is the note date, else modified). Call with at least one facet.
 */
export function matchDateFacets(file: DatedFile, facets: readonly DateFacet[]): MatchedDate | null {
  let first: MatchedDate | null = null
  for (const facet of facets) {
    const matched = matchOne(file, facet)
    if (!matched) return null
    first ??= matched
  }
  return first
}

function matchOne(file: DatedFile, facet: DateFacet): MatchedDate | null {
  const check = (date: DateKey | null, field: DateField): MatchedDate | null =>
    date !== null && rangeContains(facet.range, date) ? { date, field } : null
  switch (facet.key) {
    case 'date':
      return check(noteDate(file), 'date')
    case 'created':
      return check(localDateOf(file.created), 'created')
    case 'modified':
      return check(localDateOf(file.modified), 'modified')
    case 'when':
      return check(noteDate(file), 'date') ?? check(localDateOf(file.modified), 'modified')
  }
}

/** One file that passed the date facets, with the date it matched on. */
export interface DatedHit<F extends DatedFile> {
  readonly file: F
  readonly matched: MatchedDate
}

/**
 * The files that pass every facet, newest matched date first, ties by URL —
 * the server's order for a date-only query. One pass, then one sort of the
 * hits.
 */
export function filterByDate<F extends DatedFile>(
  files: readonly F[],
  facets: readonly DateFacet[]
): DatedHit<F>[] {
  const hits: DatedHit<F>[] = []
  for (const file of files) {
    const matched = matchDateFacets(file, facets)
    if (matched) hits.push({ file, matched })
  }
  return hits.sort((a, b) =>
    a.matched.date !== b.matched.date
      ? a.matched.date < b.matched.date
        ? 1
        : -1
      : a.file.url_path < b.file.url_path
        ? -1
        : a.file.url_path > b.file.url_path
          ? 1
          : 0
  )
}

const FIELD_PREFIX: Readonly<Record<DateField, string>> = { date: '', created: 'created ', modified: 'modified ' }

/**
 * The result label for a matched date: the reader's locale, medium style
 * (`Oct 31, 2026`), prefixed `created`/`modified` when it is not the note
 * date.
 */
export function formatMatchedDate(matched: MatchedDate, locale?: string): string {
  const parts = parseDateKey(matched.date)
  if (!parts) return FIELD_PREFIX[matched.field] + matched.date
  const when = new Date(Date.UTC(parts.year, parts.month - 1, parts.day))
  const text = new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeZone: 'UTC' }).format(when)
  return FIELD_PREFIX[matched.field] + text
}
