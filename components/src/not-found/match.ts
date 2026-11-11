/**
 * Where did a missing page go? Pure matching over site.json, shared by the
 * 404 page (`<mbr-not-found-suggestions>`) and the page-errors panel's
 * "Did you mean" links.
 *
 * Lives in the lazy `mbr-not-found.min.js` chunk. Imports nothing: the site
 * index arrives as an argument, so it is testable without a DOM or a fetch.
 *
 * Every path here is a **decoded** canonical site path (`/docs/My Note/`), the
 * form site.json's `url_path` uses — callers decode first (`canonicalPathFor`
 * does on the 404 page).
 */

/** The fields of a site.json `markdown_files` entry this module reads. */
export interface SiteMarkdownFile {
  url_path: string
  raw_path?: string
  frontmatter?: Record<string, unknown> | null
}

/** The fields of a media.json `other_files` entry this module reads. */
export interface SiteOtherFile {
  url_path: string
}

/** The part of site.json this module reads. */
export interface SiteIndex {
  markdown_files?: SiteMarkdownFile[]
  index_file?: string
}

/** One page or file offered as where the missing one went. */
export interface SuggestedPage {
  /** Canonical, decoded site path (`/archive/notes/`, `/files/report.pdf`). */
  url: string
  /** Frontmatter title, else the name in the URL. */
  title: string
  kind: 'note' | 'file'
}

/** The fallback when nothing matches: the missing page's folder still exists. */
export interface FolderFallback {
  url: string
  title: string
  /** Notes directly in the folder, by title, at most {@link MAX_FOLDER_NOTES}. */
  notes: SuggestedPage[]
  /** How many more notes the folder holds than `notes` lists. */
  more: number
}

export interface NotFoundSuggestions {
  matches: SuggestedPage[]
  /** Only when `matches` is empty. */
  folder: FolderFallback | null
}

/** At most this many matches on the 404 page. */
export const MAX_MATCHES = 6
/** At most this many "Did you mean" links per broken link. */
export const MAX_DID_YOU_MEAN = 3
/** At most this many notes listed from the surviving folder. */
export const MAX_FOLDER_NOTES = 12
/**
 * Shortest normalized name a partial (extended / shortened name) match may
 * involve, on either side: below it, `faq` would match every `faq-…` note.
 */
export const MIN_PARTIAL_LENGTH = 4

/**
 * Markdown extensions always stripped from a target name. The repository may
 * configure others; {@link suggestForMissing} adds every extension site.json's
 * `raw_path`s actually use.
 */
const COMMON_MARKDOWN_EXTENSIONS = ['md', 'markdown', 'mdown', 'mkd', 'mkdn', 'mdwn']

/**
 * Lower-cased, with runs of spaces, `-` and `_` read as one separator: `My
 * Note`, `my-note` and `my_note` are the same name.
 */
export function normalizeName(name: string): string {
  return name.toLowerCase().replace(/[\s_-]+/g, ' ').trim()
}

function segmentsOf(path: string): string[] {
  return path.split('/').filter((s) => s.length > 0)
}

/** Canonical form for comparing two site paths: a leading and trailing `/`. */
function pagePath(path: string): string {
  const segs = segmentsOf(path)
  return segs.length === 0 ? '/' : `/${segs.join('/')}/`
}

function extensionOf(name: string): string | null {
  const m = /\.([A-Za-z0-9]{1,10})$/.exec(name)
  return m ? m[1].toLowerCase() : null
}

function stripExtension(name: string): string {
  return name.replace(/\.[A-Za-z0-9]{1,10}$/, '')
}

function fileNameOf(path: string): string {
  const segs = segmentsOf(path)
  return segs.length === 0 ? '' : segs[segs.length - 1]
}

/** `index.md` → `index`, `_index.md` → `_index`. */
function indexStemOf(site: SiteIndex): string {
  return stripExtension(site.index_file || 'index.md').toLowerCase()
}

function markdownExtensionsOf(site: SiteIndex): Set<string> {
  const exts = new Set(COMMON_MARKDOWN_EXTENSIONS)
  for (const file of site.markdown_files ?? []) {
    const ext = file.raw_path ? extensionOf(fileNameOf(file.raw_path)) : null
    if (ext) exts.add(ext)
  }
  return exts
}

/** What the missing URL names. */
interface Target {
  /** Normalized name to look for. */
  name: string
  /** The folders the URL put it in. */
  folder: string[]
  /** The literal file name when the URL names a non-markdown file. */
  fileName: string | null
}

/**
 * Reads the name out of a missing path: the last segment, a markdown
 * extension dropped, and for an index note (`/a/b/index.md`) the folder's
 * name instead. `null` for the site root.
 */
function targetOf(path: string, site: SiteIndex, markdownExts: Set<string>): Target | null {
  const segs = segmentsOf(path)
  if (segs.length === 0) return null
  const last = segs[segs.length - 1]
  const ext = extensionOf(last)
  const isMarkdown = ext !== null && markdownExts.has(ext)
  const stem = isMarkdown ? stripExtension(last) : last

  if (stem.toLowerCase() === indexStemOf(site)) {
    if (segs.length < 2) return null
    return { name: normalizeName(segs[segs.length - 2]), folder: segs.slice(0, -2), fileName: null }
  }
  const name = normalizeName(stem)
  if (!name) return null
  return { name, folder: segs.slice(0, -1), fileName: !isMarkdown && ext ? last : null }
}

function titleOf(file: SiteMarkdownFile): string {
  const title = file.frontmatter?.title
  if (typeof title === 'string' && title.trim()) return title.trim()
  const segs = segmentsOf(file.url_path)
  return segs.length > 0 ? segs[segs.length - 1] : '/'
}

/** The names a note can be linked by: its URL's last segment and its file stem. */
function namesOf(file: SiteMarkdownFile, indexStem: string): string[] {
  const names = new Set<string>()
  const urlName = fileNameOf(file.url_path)
  if (urlName) names.add(normalizeName(urlName))
  if (file.raw_path) {
    const stem = stripExtension(fileNameOf(file.raw_path))
    if (stem && stem.toLowerCase() !== indexStem) names.add(normalizeName(stem))
  }
  names.delete('')
  return [...names]
}

/** `needle` starts a word of `haystack` (`meeting` in `weekly meetings`). */
function startsAWord(haystack: string, needle: string): boolean {
  return haystack.startsWith(needle) || haystack.includes(` ${needle}`)
}

/** 1 = same name, 2 = extended or shortened name, 0 = no match. */
function tierFor(target: string, names: string[]): 0 | 1 | 2 {
  if (names.includes(target)) return 1
  if (target.length < MIN_PARTIAL_LENGTH) return 0
  const partial = names.some(
    (name) =>
      name.length >= MIN_PARTIAL_LENGTH && (startsAWord(name, target) || startsAWord(target, name)),
  )
  return partial ? 2 : 0
}

/** Leading folder segments `url` shares with `folder`. */
function sharedFolders(url: string, folder: string[]): number {
  const segs = segmentsOf(url)
  let n = 0
  while (n < folder.length && n < segs.length && segs[n] === folder[n]) n++
  return n
}

interface Ranked {
  page: SuggestedPage
  tier: number
  shared: number
}

function byRank(a: Ranked, b: Ranked): number {
  if (a.tier !== b.tier) return a.tier - b.tier
  if (a.shared !== b.shared) return b.shared - a.shared
  return a.page.url < b.page.url ? -1 : a.page.url > b.page.url ? 1 : 0
}

/** Whether a page or folder index is published at `path`. */
export function pageExists(path: string, site: SiteIndex): boolean {
  const wanted = pagePath(path)
  return (site.markdown_files ?? []).some((f) => pagePath(f.url_path) === wanted)
}

/**
 * Notes (and, given `otherFiles`, files) that are probably what `path` used
 * to name, best first, at most `limit`:
 *
 * 1. the same name elsewhere (a note's file stem or URL name; a file's exact
 *    file name), compared case-insensitively with spaces, `-` and `_` alike;
 * 2. a name that was extended or shortened (`meeting` ↔ `meeting-notes`), at
 *    a word start, both sides at least {@link MIN_PARTIAL_LENGTH} long — for
 *    notes, and not when the URL names a non-markdown file.
 *
 * Within a tier, the more leading folders a candidate shares with `path` the
 * better, then by URL. `path` itself is never suggested.
 */
export function findMatches(
  path: string,
  site: SiteIndex,
  otherFiles: readonly SiteOtherFile[] = [],
  limit = MAX_MATCHES,
): SuggestedPage[] {
  const target = targetOf(path, site, markdownExtensionsOf(site))
  if (!target) return []
  const self = pagePath(path)
  const indexStem = indexStemOf(site)
  const ranked = new Map<string, Ranked>()

  for (const file of site.markdown_files ?? []) {
    const url = file.url_path
    if (pagePath(url) === self || ranked.has(url)) continue
    const tier = tierFor(target.name, namesOf(file, indexStem))
    // A file's name with its extension (`report.pdf`) is not a renamed
    // note's: only an exact match counts there.
    if (tier === 0 || (target.fileName && tier !== 1)) continue
    ranked.set(url, {
      page: { url, title: titleOf(file), kind: 'note' },
      tier,
      shared: sharedFolders(url, target.folder),
    })
  }

  if (target.fileName) {
    const wanted = normalizeName(target.fileName)
    for (const file of otherFiles) {
      const url = file.url_path
      const name = fileNameOf(url)
      if (pagePath(url) === self || ranked.has(url) || normalizeName(name) !== wanted) continue
      ranked.set(url, {
        page: { url, title: name, kind: 'file' },
        tier: 1,
        shared: sharedFolders(url, target.folder),
      })
    }
  }

  return [...ranked.values()]
    .sort(byRank)
    .slice(0, limit)
    .map((r) => r.page)
}

/**
 * The missing page's own folder, when it still exists (some note lives in or
 * under it): the notes directly in it, by title. `null` for a page at the
 * root — the 404 page already links home — or when the folder is gone too;
 * this never walks further up.
 */
export function folderFallback(path: string, site: SiteIndex): FolderFallback | null {
  const target = targetOf(path, site, markdownExtensionsOf(site))
  if (!target || target.folder.length === 0) return null
  const folderUrl = `/${target.folder.join('/')}/`
  const files = site.markdown_files ?? []
  const inFolder = files.filter((f) => pagePath(f.url_path).startsWith(folderUrl))
  if (inFolder.length === 0) return null

  const index = inFolder.find((f) => pagePath(f.url_path) === folderUrl)
  const seen = new Set<string>()
  const notes = inFolder
    .filter((f) => {
      const rest = pagePath(f.url_path).slice(folderUrl.length)
      if (segmentsOf(rest).length !== 1 || seen.has(f.url_path)) return false
      seen.add(f.url_path)
      return true
    })
    .map((f): SuggestedPage => ({ url: f.url_path, title: titleOf(f), kind: 'note' }))
    .sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: 'base' }))

  return {
    url: folderUrl,
    title: index ? titleOf(index) : target.folder[target.folder.length - 1],
    notes: notes.slice(0, MAX_FOLDER_NOTES),
    more: Math.max(0, notes.length - MAX_FOLDER_NOTES),
  }
}

/** Everything the 404 page shows: matches, else the surviving folder. */
export function suggestForMissing(
  path: string,
  site: SiteIndex,
  otherFiles: readonly SiteOtherFile[] = [],
): NotFoundSuggestions {
  const matches = findMatches(path, site, otherFiles)
  return { matches, folder: matches.length > 0 ? null : folderFallback(path, site) }
}

/**
 * Whether `path` names a non-markdown file (`/docs/report.pdf`), i.e. whether
 * media.json's file list is worth fetching to look for it.
 */
export function namesAFile(path: string, site: SiteIndex): boolean {
  return targetOf(path, site, markdownExtensionsOf(site))?.fileName != null
}

/**
 * "Did you mean" for a broken link: matches only, at most
 * {@link MAX_DID_YOU_MEAN}, and nothing when the target exists after all (a
 * link reported for its missing trailing slash, or one that climbs above the
 * root and lands on a real page).
 */
export function didYouMean(path: string, site: SiteIndex, limit = MAX_DID_YOU_MEAN): SuggestedPage[] {
  if (pageExists(path, site)) return []
  return findMatches(path, site, [], limit)
}
