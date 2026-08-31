/**
 * Turning a note's source path into the URL of the page that renders it.
 *
 * The panel lists notes across every file, so it needs a link for files that
 * are not the page in front of you. The server solves the same problem for
 * tasks by sending **both** `path` and `url_path` on every hit, precisely
 * because one cannot be derived from the other in general: `docs/index.md` is
 * served at `/docs/`, the `static_folder` overlay hides a directory level, and
 * the extension is gone. There is no review endpoint to send both, and
 * `site.json`'s `markdown_files` entries carry only `url_path`.
 *
 * So this derives the URL by the ordinary rules and then **verifies it against
 * the site index**, returning `null` when the guess is not a real page. A note
 * on an overlaid or unusual path renders as plain text rather than as a link
 * that 404s — the note itself is never lost, only its link.
 *
 * Pure: the caller supplies the index-file name and the known URL paths.
 */

/** Strip a leading `./` or `/` and collapse `\` to `/`. */
function normalizeSource(file: string): string {
  return file.replace(/\\/g, '/').replace(/^\.?\//, '')
}

/** True for the markdown extensions mbr serves as pages. */
function isMarkdown(file: string): boolean {
  return /\.(md|markdown|mdown|mkd|mkdn|mdwn|text)$/i.test(file)
}

/**
 * The URL path `file` is *probably* served at, by mbr's trailing-slash
 * convention. Not verified — see {@link resolveFileUrlPath}.
 *
 * `index.md` collapses into its directory, which is the one rule that makes
 * this ambiguous in the other direction and the reason the result is checked
 * rather than trusted.
 */
export function deriveUrlPath(file: string, indexFile: string): string | null {
  const path = normalizeSource(file)
  if (path.length === 0 || !isMarkdown(path)) return null

  const segments = path.split('/')
  const last = segments.pop()!

  if (last === indexFile) {
    return segments.length === 0 ? '/' : `/${segments.join('/')}/`
  }

  const stem = last.replace(/\.[^.]+$/, '')
  if (stem.length === 0) return null
  return `/${[...segments, stem].join('/')}/`
}

/**
 * The URL path for `file`, or `null` when no such page is known.
 *
 * `known` is the set of `url_path` values from `site.json`. An empty set means
 * the index has not loaded yet, in which case the derived path is returned
 * unverified — a link that might 404 is better than no link at all while the
 * page is still starting up, and `site.json` is fetched on every page anyway.
 */
export function resolveFileUrlPath(
  file: string,
  indexFile: string,
  known: ReadonlySet<string>
): string | null {
  const derived = deriveUrlPath(file, indexFile)
  if (derived === null) return null
  if (known.size === 0) return derived
  return known.has(derived) ? derived : null
}

/** The `url_path` values from a `site.json` payload, tolerating any shape. */
export function knownUrlPaths(siteData: unknown): Set<string> {
  const files = (siteData as { markdown_files?: unknown })?.markdown_files
  if (!Array.isArray(files)) return new Set()
  const paths = new Set<string>()
  for (const entry of files) {
    const url = (entry as { url_path?: unknown })?.url_path
    if (typeof url === 'string' && url.length > 0) paths.add(url)
  }
  return paths
}

/** The configured index file name from a `site.json` payload. */
export function indexFileOf(siteData: unknown): string {
  const name = (siteData as { index_file?: unknown })?.index_file
  return typeof name === 'string' && name.length > 0 ? name : 'index.md'
}
