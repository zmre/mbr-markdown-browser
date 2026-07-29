/**
 * Shared-promise cache for per-page `links.json` fetches.
 *
 * MAIN BUNDLE ONLY: this module is stateful (module-level cache) and imports
 * `shared.ts` (which fetches site.json at import time), so it must never be
 * imported by the lazy graph chunk. The chunk's `<mbr-mini-graph>` element
 * receives `fetchPageLinks` injected as a property instead.
 *
 * Caching follows the `siteNav` shared-promise pattern in `shared.ts`:
 * concurrent callers for the same canonical path share one in-flight fetch.
 * A 404 resolves `null` and STAYS cached (link tracking is off — permanent for
 * this page load); a network/server error resolves `null` but evicts the cache
 * entry so a later call can retry.
 */
import { resolveUrl } from '../shared.js'
import { canonicalizeNotePath } from './relationship-graph.js'
import type { FetchPageLinks } from './bfs.js'
import type { PageLinks } from './relationship-graph.js'

export type { FetchPageLinks }

/**
 * Outcome of a `links.json` fetch, for callers that need to tell "this page
 * has no links.json" (link tracking disabled — an empty, non-error state) from
 * "the request failed" (worth surfacing to the user and worth retrying).
 * `fetchPageLinks` collapses both into `null`.
 */
export type LinksResult =
  | { status: 'ok'; links: PageLinks }
  | { status: 'unavailable' }
  | { status: 'error'; message: string }

const cache = new Map<string, Promise<LinksResult>>()

/**
 * Build the `links.json` URL for a canonical note path. Paths are stored
 * DECODED (literal spaces etc.), so each segment is percent-encoded for the
 * request; `resolveUrl` handles the static-build base path.
 */
function linksJsonUrl(canonicalPath: string): string {
  const encoded = canonicalPath.split('/').map(encodeURIComponent).join('/')
  return `${resolveUrl(encoded)}links.json`
}

/**
 * Fetch a page's `links.json`, de-duplicated per canonical path, reporting the
 * failure mode. Never rejects. This is the shared entry point: every consumer
 * on a page hits the same in-flight promise, so `links.json` is fetched once.
 */
export function fetchPageLinksResult(path: string): Promise<LinksResult> {
  const key = canonicalizeNotePath(path)
  const cached = cache.get(key)
  if (cached) return cached

  const promise = (async (): Promise<LinksResult> => {
    try {
      const response = await fetch(linksJsonUrl(key))
      if (response.status === 404) {
        // Link tracking disabled for this page: permanent, and kept cached.
        return { status: 'unavailable' }
      }
      if (!response.ok) {
        throw new Error(`links.json failed: ${response.status}`)
      }
      return { status: 'ok', links: (await response.json()) as PageLinks }
    } catch (err) {
      // Transient failure: evict so a later call can retry.
      cache.delete(key)
      return { status: 'error', message: err instanceof Error ? err.message : 'Unknown error' }
    }
  })()
  cache.set(key, promise)
  return promise
}

/**
 * Fetch a page's `links.json`, de-duplicated per canonical path. Never
 * rejects: resolves `null` when the payload is unavailable (404 or failure).
 */
export const fetchPageLinks: FetchPageLinks = (path) =>
  fetchPageLinksResult(path).then((result) => (result.status === 'ok' ? result.links : null))
