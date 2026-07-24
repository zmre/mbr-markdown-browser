/**
 * Level-by-level BFS expansion of a note's link neighborhood.
 *
 * Pure-injectable: the links fetcher and note gate are passed in, so this
 * module has no network or DOM dependencies and is fully unit-testable with a
 * stub fetcher. In production the injected fetcher is the caching
 * `fetchPageLinks` from `links-cache.ts` (main bundle), so re-running the BFS
 * at a deeper depth re-uses every previously fetched level for free.
 */
import { canonicalizeNotePath, DEFAULT_MAX_NODES, type PageLinks } from './relationship-graph.js'
import { mergeLevel, type MiniGraph } from './build.js'

/** Fetches a page's `links.json`; resolves `null` when unavailable. */
export type FetchPageLinks = (path: string) => Promise<PageLinks | null>

/** Default parallel-fetch ceiling per BFS level. */
export const DEFAULT_FETCH_CONCURRENCY = 4

/**
 * Map `fn` over `items` with at most `limit` calls in flight, preserving item
 * order in the result. When `signal` aborts, no NEW calls are started; results
 * of calls that never ran are omitted. `fn` must not reject (a rejection
 * rejects the whole map) — the BFS fetcher resolves `null` on failure.
 */
export async function mapWithConcurrency<T, R>(
  items: readonly T[],
  limit: number,
  fn: (item: T) => Promise<R>,
  signal?: AbortSignal
): Promise<R[]> {
  const settled: Array<{ index: number; value: R }> = []
  let next = 0
  const runWorker = async (): Promise<void> => {
    while (next < items.length && !signal?.aborted) {
      const index = next
      next += 1
      const value = await fn(items[index])
      settled.push({ index, value })
    }
  }
  const workerCount = Math.max(1, Math.min(Math.floor(limit) || 1, items.length))
  await Promise.all(Array.from({ length: workerCount }, () => runWorker()))
  return settled.sort((a, b) => a.index - b.index).map((entry) => entry.value)
}

export interface ExpandOptions {
  /** The note whose neighborhood is expanded (canonicalized internally). */
  focus: string
  /** Number of BFS levels to include (nodes at the final level are leaves). */
  depth: number
  /** Cap on total node count; hitting it sets the graph's `truncated` flag. */
  maxNodes?: number
  /** Parallel-fetch ceiling per level. */
  concurrency?: number
  fetchLinks: FetchPageLinks
  /** Gate excluding non-note link targets (media, tags, dangling links). */
  isKnownNote: (path: string) => boolean
  /** Called with a fresh snapshot after each level settles. */
  onUpdate?: (graph: MiniGraph) => void
  signal?: AbortSignal
}

/**
 * Expand the focus note's neighborhood breadth-first up to `depth` levels.
 *
 * Returns `null` when the focus has no `links.json` (link tracking disabled),
 * so callers can hide the graph entirely. Otherwise returns the accumulated
 * graph — possibly partial when aborted or capped. The final level's frontier
 * is never fetched (those nodes are leaves), which halves the fan-out; a
 * neighbor whose own fetch resolves `null` simply stays a leaf.
 */
export async function expandNeighborhood(options: ExpandOptions): Promise<MiniGraph | null> {
  const {
    depth,
    maxNodes = DEFAULT_MAX_NODES,
    concurrency = DEFAULT_FETCH_CONCURRENCY,
    fetchLinks,
    isKnownNote,
    onUpdate,
    signal,
  } = options
  const focus = canonicalizeNotePath(options.focus)

  const focusLinks = await fetchLinks(focus)
  if (focusLinks === null) return null

  let graph: MiniGraph = {
    focus,
    nodes: [{ id: focus, degree: 0 }],
    links: [],
    truncated: false,
  }
  let fetchedLevel: Array<{ path: string; links: PageLinks }> = [
    { path: focus, links: focusLinks },
  ]

  for (let level = 1; level <= depth; level++) {
    if (signal?.aborted) break
    const merged = mergeLevel(graph, fetchedLevel, level, maxNodes, isKnownNote)
    graph = merged.graph
    onUpdate?.(graph)

    // The final level's nodes are leaves: never fetch their links.
    if (level === depth) break
    if (merged.frontier.length === 0 || graph.nodes.length >= maxNodes) break

    const results = await mapWithConcurrency(
      merged.frontier,
      concurrency,
      async (path) => ({ path, links: await fetchLinks(path) }),
      signal
    )
    fetchedLevel = results.filter(
      (entry): entry is { path: string; links: PageLinks } => entry.links !== null
    )
  }

  return graph
}
