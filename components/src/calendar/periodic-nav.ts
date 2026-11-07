/**
 * The periodic-note actions behind the `d`, `[` and `]` keys (main bundle).
 *
 * Stateful: it memoizes the periodic index off `shared.ts`'s `siteNav`, built
 * once per page and only when a key first needs it. **A lazy chunk must not
 * import this module** (it would bundle a second `shared.ts`); chunks use the
 * pure `periodic.ts` with data handed to them.
 *
 * `[` / `]` read the server-rendered bar (`_calendar_nav.html`) rather than
 * the index: the link is the source of truth, so key and bar cannot disagree.
 */

import { getCalendarConfig, resolveUrl, siteNav } from '../shared.js'
import { showToast } from '../toast.js'
import { todayKey } from './note-date.js'
import { buildPeriodicIndex, todayDailyNote, type PeriodicIndex, type PeriodicSource } from './periodic.js'

/** Where the files come from; replaced in tests. */
type FilesLoader = () => Promise<readonly PeriodicSource[]>

const defaultLoader: FilesLoader = () =>
  siteNav.then((data: { markdown_files?: PeriodicSource[] } | null) => data?.markdown_files ?? [])

let loadFiles: FilesLoader = defaultLoader
let indexPromise: Promise<PeriodicIndex> | null = null
let navigate: (url: string) => void = (url) => window.location.assign(url)

/** Test seams: the file source and the navigation. `null` restores both. */
export function setPeriodicNavDeps(
  deps: { loadFiles?: FilesLoader; navigate?: (url: string) => void } | null
): void {
  loadFiles = deps?.loadFiles ?? defaultLoader
  navigate = deps?.navigate ?? ((url) => window.location.assign(url))
  indexPromise = null
}

/** The page's periodic index, built once. A failed load is retried next time. */
export function periodicIndex(): Promise<PeriodicIndex> {
  if (!indexPromise) {
    indexPromise = loadFiles()
      .then(buildPeriodicIndex)
      .catch((err: unknown) => {
        indexPromise = null
        throw err
      })
  }
  return indexPromise
}

/** Whether the calendar keys are live on this page. */
export function calendarKeysEnabled(): boolean {
  return getCalendarConfig().enabled
}

/**
 * `d`: today's daily note (the reader's local date), else the most recent one
 * before today, else the earliest after. A toast when there are none.
 */
export async function openTodayDailyNote(today: string = todayKey()): Promise<void> {
  let index: PeriodicIndex
  try {
    index = await periodicIndex()
  } catch {
    showToast('Could not load the site index')
    return
  }
  const entry = todayDailyNote(index, today)
  if (!entry) {
    showToast('No daily notes found')
    return
  }
  navigate(resolveUrl(entry.url))
}

/**
 * `[` / `]`: follows the bar's `rel="prev"` / `rel="next"` link. Returns
 * whether there was one; elsewhere the key is a no-op.
 */
export function followPeriodicLink(rel: 'prev' | 'next', root: ParentNode = document): boolean {
  const link = root.querySelector<HTMLAnchorElement>(`nav.mbr-calendar-nav a[rel="${rel}"]`)
  if (!link) return false
  link.click()
  return true
}
