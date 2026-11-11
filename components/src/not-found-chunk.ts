/**
 * Single-flight loader for the lazy `mbr-not-found.min.js` chunk (the "where
 * did it go" matcher and its 404-page renderer, `not-found/index.ts`).
 *
 * MAIN BUNDLE. `calendar-chunk.ts`'s pattern: the URL is computed against the
 * asset base, so it works in server mode, in static builds at any depth and
 * on a static `404.html` (whose base is root-anchored, see `base_url`).
 */
import { getMbrAssetBase } from './dynamic-loader.js'

/** What the chunk exports. */
export type NotFoundChunk = typeof import('./not-found/index.js')

const defaultImporter = (): Promise<unknown> => {
  const url = new URL(getMbrAssetBase() + 'components/mbr-not-found.min.js', document.baseURI).href
  return import(/* @vite-ignore */ url)
}

let importNotFoundChunk: () => Promise<unknown> = defaultImporter

/** Shared once-per-page promise; resolves `null` when the import failed. */
let notFoundChunkPromise: Promise<NotFoundChunk | null> | null = null

/** Test hook: replace the chunk importer (module-level seam). */
export function setNotFoundChunkImporter(importer: () => Promise<unknown>): void {
  importNotFoundChunk = importer
  notFoundChunkPromise = null
}

/**
 * Loads the chunk once per page. Never rejects: resolves `null` when the
 * import failed, and callers then simply show no suggestions.
 */
export function loadNotFoundChunk(): Promise<NotFoundChunk | null> {
  if (!notFoundChunkPromise) {
    notFoundChunkPromise = importNotFoundChunk()
      .then((module) => module as NotFoundChunk)
      .catch((err: unknown) => {
        console.warn('Failed to load the not-found chunk:', err)
        return null
      })
  }
  return notFoundChunkPromise
}
