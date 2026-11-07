/**
 * Opening the calendar overlay from another component (main bundle): the
 * browse widgets' calendar button and "Calendar" section, and search's
 * "Show in calendar" (design/calendar.md §5.1, §5.6, §6.4).
 *
 * The overlay lives in the calendar chunk and is owned by `<mbr-calendar>`
 * (`_footer.html`, present only when `[calendar] enabled`); callers go
 * through here rather than importing the trigger, so an absent or disabled
 * calendar simply hides their buttons. The scope builders are pure.
 */
import type { MarkdownFile, FolderNode } from '../sorting.js'
import { filterByDate, splitDateFacets } from './date-facets.js'
import type { DateKey } from './note-date.js'
import type { CalendarScope } from './types.js'

/** Whether the overlay can be opened on this page. */
export function calendarOverlayAvailable(root: ParentNode = document): boolean {
  return root.querySelector('mbr-calendar')?.overlayAvailable === true
}

/** Opens the overlay over `scope`; false when there is no calendar here. */
export function openCalendarOverlay(scope: CalendarScope, root: ParentNode = document): boolean {
  const el = root.querySelector('mbr-calendar')
  return typeof el?.openOverlay === 'function' ? el.openOverlay(scope) : false
}

/** Every note under a folder node, its subfolders included (the browse folder scope). */
export function folderNodeFiles(node: FolderNode): MarkdownFile[] {
  const out: MarkdownFile[] = [...node.files]
  for (const child of node.children.values()) out.push(...folderNodeFiles(child))
  return out
}

/** The search session `<mbr-search>` hands over (its `session` getter). */
export interface SearchSession {
  readonly query: string
  readonly results: readonly { readonly url_path: string }[]
}

/**
 * The overlay scope for a search (§6.4), without searching again: a query of
 * date facets only is rebuilt in full from `site.json` (`filterByDate`, the
 * static search's own rule); any other query is the results on screen, looked
 * up in `site.json` by `url_path` (non-markdown results drop out). Plotted by
 * note date.
 */
export function searchScope(session: SearchSession, files: readonly MarkdownFile[], today: DateKey): CalendarScope {
  const label = `Search: ${session.query.trim()}`
  const { text, facets } = splitDateFacets(session.query, today)
  if (facets.length > 0 && !text.trim()) {
    return { label, plotBy: 'date', files: filterByDate(files, facets).map((hit) => hit.file) }
  }
  const byUrl = new Map(files.map((f) => [f.url_path, f]))
  const decode = (p: string): string => {
    try {
      return decodeURIComponent(p)
    } catch {
      return p
    }
  }
  const picked = session.results.flatMap((r) => {
    const file = byUrl.get(r.url_path) ?? byUrl.get(decode(r.url_path))
    return file ? [file] : []
  })
  return { label, plotBy: 'date', files: picked }
}
