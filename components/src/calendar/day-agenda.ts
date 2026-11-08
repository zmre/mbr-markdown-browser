/**
 * The agenda of another day's note: fetched as HTML, parsed with `DOMParser`
 * and read by the same parser as the live page (`agenda-parse.ts`), so static
 * builds and the server need no new endpoint.
 *
 * Module state is one small LRU cache keyed by URL; this lives in the lazy
 * chunk only, so there is exactly one copy. It holds **plain data only**
 * ({@link AgendaSummary}): an `AgendaEntry` keeps its `<li>`, and through it
 * the whole parsed document of that note, so caching entries would pin one
 * DOM per clicked day for the page's life. A failed fetch is not cached, so a
 * later click retries.
 */
import { agendaEntries, type AgendaEntry } from './agenda-parse.js'

/** Fetches a page's HTML (injected by the trigger; stubbed in tests). */
export type FetchPageHtml = (url: string) => Promise<string>

/**
 * What the companion draws for a fetched day, and what a click needs: the
 * `ordinal` opens the note at `#mbr-agenda-<ordinal>`. No DOM references.
 */
export type AgendaSummary = Pick<AgendaEntry, 'start' | 'end' | 'title' | 'details' | 'task' | 'ordinal'>

/** Fetched days kept: a month of back-and-forth stays cached, memory stays flat. */
export const DAY_AGENDA_CACHE_SIZE = 12

/** URL → pending summaries, least recently used first (a `Map` keeps insertion order). */
const cache = new Map<string, Promise<AgendaSummary[]>>()

/** The note's rendered body in a parsed document. */
function noteBody(doc: Document): ParentNode {
  return doc.querySelector('main#wrapper') ?? doc.querySelector('main') ?? doc.body
}

/** Agenda entries of an HTML page (pure apart from `DOMParser`). */
export function agendaFromHtml(html: string): AgendaEntry[] {
  const doc = new DOMParser().parseFromString(html, 'text/html')
  return agendaEntries(noteBody(doc))
}

/** An entry without its DOM (`details` is already a fresh array of strings). */
export function summarizeAgendaEntry({ start, end, title, details, task, ordinal }: AgendaEntry): AgendaSummary {
  return { start, end, title, details, task, ordinal }
}

/**
 * The agenda of the page at `url`, fetched once while it stays among the
 * {@link DAY_AGENDA_CACHE_SIZE} most recently asked-for days.
 */
export function loadDayAgenda(url: string, fetchPageHtml: FetchPageHtml): Promise<AgendaSummary[]> {
  let pending = cache.get(url)
  if (pending) {
    cache.delete(url)
  } else {
    const settled = fetchPageHtml(url).then((html) => agendaFromHtml(html).map(summarizeAgendaEntry))
    settled.catch(() => {
      if (cache.get(url) === settled) cache.delete(url)
    })
    pending = settled
  }
  cache.set(url, pending)
  // One in, at most one out: the first key is the least recently used.
  if (cache.size > DAY_AGENDA_CACHE_SIZE) cache.delete(cache.keys().next().value as string)
  return pending
}

/** Test hook: forget every cached day. */
export function clearDayAgendaCache(): void {
  cache.clear()
}
