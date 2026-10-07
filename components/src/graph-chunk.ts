/**
 * Single-flight loader for the lazy `mbr-graph.min.js` chunk, which registers
 * `<mbr-mini-graph>` (plus its d3-force copy).
 *
 * MAIN BUNDLE. Shared by the info panel and the person/organization charts so
 * the chunk is imported at most once per page: a second, independent import of
 * the same URL would be deduplicated by the module map anyway, but a chunk that
 * bundled its OWN copy of the element would hit a double `customElements.define`
 * and ship d3-force twice — which is why the genealogy chunk receives this
 * loader as a service instead of importing the element.
 *
 * The URL is computed against the asset base, so it works in server mode and in
 * static builds deployed at any depth.
 */
import { getMbrAssetBase } from './dynamic-loader.js'

const defaultImporter = (): Promise<unknown> => {
  const url = new URL(getMbrAssetBase() + 'components/mbr-graph.min.js', document.baseURI).href
  return import(/* @vite-ignore */ url)
}

let importGraphChunk: () => Promise<unknown> = defaultImporter

/** Shared once-per-page promise for the chunk load; `true` when usable. */
let graphChunkPromise: Promise<boolean> | null = null

/** Test hook: replace the chunk importer (module-level seam). */
export function setGraphChunkImporter(importer: () => Promise<unknown>): void {
  importGraphChunk = importer
  graphChunkPromise = null
}

/**
 * Load the graph chunk once. Never rejects: resolves `false` when the import
 * failed, so callers simply omit the graph.
 */
export function loadGraphChunk(): Promise<boolean> {
  if (!graphChunkPromise) {
    graphChunkPromise = importGraphChunk()
      .then(() => true)
      .catch((err) => {
        console.warn('Failed to load the graph chunk:', err)
        return false
      })
  }
  return graphChunkPromise
}
