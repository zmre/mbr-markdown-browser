/**
 * The agenda of another day's note: fetched as HTML, parsed with `DOMParser`
 * and read by the same parser as the live page (`agenda-parse.ts`), so static
 * builds and the server need no new endpoint.
 *
 * Module state is one per-page cache keyed by URL; this lives in the lazy
 * chunk only, so there is exactly one copy. A failed fetch is not cached, so
 * a later click retries.
 */
import { agendaEntries, type AgendaEntry } from './agenda-parse.js'

/** Fetches a page's HTML (injected by the trigger; stubbed in tests). */
export type FetchPageHtml = (url: string) => Promise<string>

const cache = new Map<string, Promise<AgendaEntry[]>>()

/** The note's rendered body in a parsed document. */
function noteBody(doc: Document): ParentNode {
  return doc.querySelector('main#wrapper') ?? doc.querySelector('main') ?? doc.body
}

/** Agenda entries of an HTML page (pure apart from `DOMParser`). */
export function agendaFromHtml(html: string): AgendaEntry[] {
  const doc = new DOMParser().parseFromString(html, 'text/html')
  return agendaEntries(noteBody(doc))
}

/** The agenda of the page at `url`, fetched once per page load. */
export function loadDayAgenda(url: string, fetchPageHtml: FetchPageHtml): Promise<AgendaEntry[]> {
  let pending = cache.get(url)
  if (!pending) {
    pending = fetchPageHtml(url).then(agendaFromHtml)
    pending.catch(() => cache.delete(url))
    cache.set(url, pending)
  }
  return pending
}

/** Test hook: forget every cached day. */
export function clearDayAgendaCache(): void {
  cache.clear()
}
