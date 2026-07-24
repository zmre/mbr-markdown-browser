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

const cache = new Map<string, Promise<PageLinks | null>>()

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
 * Fetch a page's `links.json`, de-duplicated per canonical path. Never
 * rejects: resolves `null` when the payload is unavailable.
 */
export const fetchPageLinks: FetchPageLinks = (path) => {
  const key = canonicalizeNotePath(path)
  const cached = cache.get(key)
  if (cached) return cached

  const promise = (async (): Promise<PageLinks | null> => {
    try {
      const response = await fetch(linksJsonUrl(key))
      if (response.status === 404) {
        // Link tracking disabled for this page: a permanent null (kept cached).
        return null
      }
      if (!response.ok) {
        throw new Error(`links.json failed: ${response.status}`)
      }
      return (await response.json()) as PageLinks
    } catch {
      // Transient failure: evict so a later call can retry.
      cache.delete(key)
      return null
    }
  })()
  cache.set(key, promise)
  return promise
}
