/**
 * Pure helpers behind the search panel's note-type options and folder picker.
 * Chunk-side (`mbr-search-extras.min.js`): nothing here is needed until the
 * search modal opens, so none of it is paid for on page load.
 */

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
 * The `type:` facet token for a note type. A value with whitespace is quoted —
 * `parse_query` (src/search.rs `query_tokens`) keeps `key:"two words"` whole.
 * A literal `"` cannot be expressed inside that quoting, so it is dropped; the
 * facet is a substring match, so the remaining text still selects the type.
 */
export function typeFacetToken(type: string): string {
  const value = type.replace(/"/g, '').trim()
  return /\s/.test(value) ? `type:"${value}"` : `type:${value}`
}

/** Matches an existing `type:` facet token, quoted or bare. */
const TYPE_TOKEN = /(^|\s)type:("[^"]*"?|\S*)/gi

/**
 * `query` with any existing `type:` facet replaced by the one for `type`, so
 * choosing a second type swaps rather than stacking two facets (which would
 * have to both match and select nothing).
 */
export function withTypeFacet(query: string, type: string): string {
  const rest = query.replace(TYPE_TOKEN, ' ').replace(/\s+/g, ' ').trim()
  const token = typeFacetToken(type)
  return rest ? `${rest} ${token}` : token
}
