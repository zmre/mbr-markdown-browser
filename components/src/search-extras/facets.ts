/**
 * Pure helpers behind the search panel's note-type options and folder picker.
 * Chunk-side (`mbr-search-extras.min.js`): nothing here is needed until the
 * search modal opens, so none of it is paid for on page load.
 *
 * A query is read with `calendar/date-facets.ts`'s `queryTokens` / `facetOf`
 * (pure; the mirror of the server's `query_tokens`), so `when:` inside a
 * quoted value (`title:"a when:today"`) is part of that value here too.
 */
import { facetOf, queryTokens } from '../calendar/date-facets.js'

/** Case-insensitive, locale-independent ordering for folder and type lists. */
function compareFolded(a: string, b: string): number {
  const x = a.toLowerCase()
  const y = b.toLowerCase()
  return x < y ? -1 : x > y ? 1 : a < b ? -1 : a > b ? 1 : 0
}

/**
 * Every folder that holds at least one note, derived from site.json `url_path`s
 * (no endpoint needed). A note's own URL is not a folder — `/people/john/` is
 * John's page — so only *proper* ancestors count; a note that also has notes
 * beneath it contributes its URL through those children. The root is omitted:
 * scoping to `/` is the same as searching everywhere.
 */
export function foldersFromUrlPaths(paths: readonly string[]): string[] {
  const folders = new Set<string>()
  for (const path of paths) {
    if (typeof path !== 'string') continue
    const segments = path.split('/').filter(Boolean)
    let prefix = '/'
    for (const segment of segments.slice(0, -1)) {
      prefix += `${segment}/`
      folders.add(prefix)
    }
  }
  return [...folders].sort(compareFolded)
}

/** One distinct frontmatter `type`, with how many notes carry it. */
export interface NoteTypeCount {
  type: string
  count: number
}

type SiteFile = { url_path?: unknown; frontmatter?: Record<string, unknown> | null }

/**
 * Distinct frontmatter `type` values across site.json, case-insensitively
 * merged (the server's facet match is case-insensitive, so `Person` and
 * `person` select the same notes) under the first spelling seen.
 */
export function noteTypesFromSite(files: ReadonlyArray<SiteFile> | null | undefined): NoteTypeCount[] {
  const byKey = new Map<string, NoteTypeCount>()
  for (const file of files ?? []) {
    const raw = file?.frontmatter?.['type']
    if (typeof raw !== 'string') continue
    const type = raw.trim()
    if (!type) continue
    const key = type.toLowerCase()
    const entry = byKey.get(key)
    if (entry) entry.count += 1
    else byKey.set(key, { type, count: 1 })
  }
  return [...byKey.values()].sort((a, b) => compareFolded(a.type, b.type))
}

/** Folders and note types for the search panel, from one site.json payload. */
export function deriveSearchFacets(data: { markdown_files?: unknown } | null | undefined): {
  folders: string[]
  types: NoteTypeCount[]
} {
  const files = Array.isArray(data?.markdown_files) ? (data.markdown_files as SiteFile[]) : []
  return {
    folders: foldersFromUrlPaths(
      files.map((f) => f?.url_path).filter((p): p is string => typeof p === 'string')
    ),
    types: noteTypesFromSite(files),
  }
}

/**
 * The `key:value` facet token. A value with whitespace is quoted —
 * `parse_query` (src/search.rs `query_tokens`) keeps `key:"two words"` whole.
 * A literal `"` cannot be expressed inside that quoting, so it is dropped; a
 * frontmatter facet is a substring match, so the remaining text still selects
 * the value.
 */
export function facetToken(key: string, value: string): string {
  const clean = value.replace(/"/g, '').trim()
  return /\s/.test(clean) ? `${key}:"${clean}"` : `${key}:${clean}`
}

/** The `type:` facet token for a note type. */
export function typeFacetToken(type: string): string {
  return facetToken('type', type)
}

/**
 * Whether `token` is a `key:` facet (key ASCII case-insensitive), or the bare
 * `key:` of one still being typed.
 */
function isFacetToken(token: string, key: string): boolean {
  const lower = key.toLowerCase()
  return facetOf(token)?.key.toLowerCase() === lower || token.toLowerCase() === `${lower}:`
}

/**
 * `query` with every existing `key:` facet replaced by one for `value` (or
 * just removed, for `null` or an empty value), so choosing a second value
 * swaps rather than stacking two facets (which would both have to match).
 */
export function withFacet(query: string, key: string, value: string | null): string {
  const rest = queryTokens(query)
    .filter((token) => !isFacetToken(token, key))
    .join(' ')
  if (value === null || !value.trim()) return rest
  const token = facetToken(key, value)
  return rest ? `${rest} ${token}` : token
}

/** {@link withFacet} for the note-type selector. */
export function withTypeFacet(query: string, type: string): string {
  return withFacet(query, 'type', type)
}

/** The value of the last `key:` facet in `query`, unquoted, or `null`. */
export function facetValue(query: string, key: string): string | null {
  const lower = key.toLowerCase()
  let value: string | null = null
  for (const token of queryTokens(query)) {
    const facet = facetOf(token)
    if (facet && facet.key.toLowerCase() === lower) value = facet.value
  }
  return value
}

// ---- The search modal's "When" select (design/calendar.md §6.3) ----

/** One `when:` preset; `value` is what goes after `when:`. */
export interface WhenPreset {
  readonly label: string
  readonly value: string
}

/** The select's value for "Any time" (no `when:` facet). */
export const WHEN_ANY = ''

/** The select's value for "Custom…" (a hand-made range). Not an expression. */
export const WHEN_CUSTOM = 'custom'

/** The presets, in menu order, between "Any time" and "Custom…". */
export const WHEN_PRESETS: readonly WhenPreset[] = [
  { label: 'Today', value: 'today' },
  { label: 'Last 7 days', value: '-7d' },
  { label: 'Last 30 days', value: '-30d' },
  { label: 'Last 3 months', value: '-3m' },
  { label: 'Last 12 months', value: '-12m' },
  { label: 'This year', value: 'this-year' },
]

/**
 * What the When select shows for `query`: {@link WHEN_ANY} with no `when:`
 * facet, the preset's value when the (hand-typed or chosen) facet equals one
 * (ASCII case-insensitive, as the grammar is), else {@link WHEN_CUSTOM}.
 */
export function whenSelectValue(query: string): string {
  const value = facetValue(query, 'when')
  if (value === null) return WHEN_ANY
  const lower = value.toLowerCase()
  return WHEN_PRESETS.find((p) => p.value === lower)?.value ?? WHEN_CUSTOM
}

const DAY = /^\d{4}-\d{2}-\d{2}$/

/**
 * The two date inputs of "Custom…" for `query`'s `when:` facet: the ends of a
 * `YYYY-MM-DD..YYYY-MM-DD` range (either may be open), both ends for a single
 * day, and empty for anything else (a hand-typed `when:2026-W44` is still
 * applied; the inputs just cannot show it).
 */
export function whenCustomRange(query: string): { from: string; to: string } {
  const value = facetValue(query, 'when') ?? ''
  if (DAY.test(value)) return { from: value, to: value }
  const dots = value.indexOf('..')
  if (dots < 0) return { from: '', to: '' }
  const from = value.slice(0, dots)
  const to = value.slice(dots + 2)
  return {
    from: DAY.test(from) ? from : '',
    to: DAY.test(to) ? to : '',
  }
}

/**
 * The `when:` value for a custom range from two `<input type="date">` values
 * (`YYYY-MM-DD` or empty): `A..B`, `A..`, `..B`, or `null` when both are
 * empty (no facet).
 */
export function whenCustomValue(from: string, to: string): string | null {
  return from || to ? `${from}..${to}` : null
}
