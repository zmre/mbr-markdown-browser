/**
 * Pure construction helpers for the sidebar mini force graph.
 *
 * The mini graph is built by a breadth-first expansion over per-page
 * `links.json` payloads (see `bfs.ts`). Everything in this module is pure and
 * DOM/network-free so it can be unit-tested directly and bundled into the lazy
 * graph chunk without dragging in any stateful module.
 */
import { canonicalizeNotePath, type PageLinks } from './relationship-graph.js'

// ============================================================================
// Model
// ============================================================================

/** A node in the mini graph. `degree` is the BFS level from the focus (0). */
export interface MiniGraphNode {
  id: string
  degree: number
}

/** An undirected edge between two note paths. */
export interface MiniGraphLink {
  source: string
  target: string
}

export interface MiniGraph {
  focus: string
  nodes: MiniGraphNode[]
  links: MiniGraphLink[]
  /** True when the `maxNodes` cap prevented adding at least one node. */
  truncated: boolean
}

/** Stable key for an undirected edge: sorted endpoint pair. */
function edgeKey(a: string, b: string): string {
  return a < b ? `${a}|${b}` : `${b}|${a}`
}

// ============================================================================
// Per-page extraction
// ============================================================================

/**
 * All connected note paths of `self` according to its `links.json`: internal
 * outbound targets, inbound sources, and resolved relationship neighbors.
 * Every path is canonicalized; self-loops are removed and duplicates collapse
 * (first occurrence wins the ordering).
 */
export function neighborsOf(self: string, links: PageLinks): string[] {
  const selfPath = canonicalizeNotePath(self)
  const seen = new Set<string>()
  const neighbors: string[] = []
  const add = (raw: string) => {
    const path = canonicalizeNotePath(raw)
    if (!path || path === selfPath || seen.has(path)) return
    seen.add(path)
    neighbors.push(path)
  }
  for (const link of links.outbound) {
    if (link.internal) add(link.to)
  }
  for (const link of links.inbound) {
    add(link.from)
  }
  for (const rel of links.relationships ?? []) {
    if (rel.resolved && rel.neighbor) add(rel.neighbor)
  }
  return neighbors
}

/**
 * The undirected edges implied by one page's `links.json`: one link from
 * `self` to each of its neighbors, de-duplicated by sorted-pair key (so an
 * outbound link and a backlink to the same note collapse to one edge).
 */
export function edgesOf(self: string, links: PageLinks): MiniGraphLink[] {
  const selfPath = canonicalizeNotePath(self)
  return neighborsOf(selfPath, links).map((neighbor) => ({
    source: selfPath,
    target: neighbor,
  }))
}

// ============================================================================
// Level merging (BFS support)
// ============================================================================

export interface MergeResult {
  graph: MiniGraph
  /** Node ids newly added at this level — the next BFS fetch frontier. */
  frontier: string[]
}

/**
 * Merge one settled BFS level into the graph: for each fetched page, add its
 * neighbors as nodes at `level` (subject to the `maxNodes` cap, which sets the
 * `truncated` flag when hit) and its edges. The `isKnownNote` gate excludes
 * targets that are not notes (media files, dangling links, tag pages); edges
 * are only added between included nodes. Pure: returns a new graph.
 */
export function mergeLevel(
  graph: MiniGraph,
  fetched: ReadonlyArray<{ path: string; links: PageLinks }>,
  level: number,
  maxNodes: number,
  isKnownNote: (path: string) => boolean
): MergeResult {
  const nodes = [...graph.nodes]
  const links = [...graph.links]
  const ids = new Set(nodes.map((n) => n.id))
  const linkKeys = new Set(links.map((l) => edgeKey(l.source, l.target)))
  let truncated = graph.truncated
  const frontier: string[] = []

  for (const { path, links: pageLinks } of fetched) {
    const source = canonicalizeNotePath(path)
    // Only expand pages that are already part of the graph (the frontier).
    if (!ids.has(source)) continue
    for (const neighbor of neighborsOf(source, pageLinks)) {
      if (!isKnownNote(neighbor)) continue
      if (!ids.has(neighbor)) {
        if (nodes.length >= maxNodes) {
          truncated = true
          continue // Edges are only added between included nodes.
        }
        ids.add(neighbor)
        nodes.push({ id: neighbor, degree: level })
        frontier.push(neighbor)
      }
      const key = edgeKey(source, neighbor)
      if (!linkKeys.has(key)) {
        linkKeys.add(key)
        links.push({ source, target: neighbor })
      }
    }
  }

  return { graph: { focus: graph.focus, nodes, links, truncated }, frontier }
}

/**
 * Restrict a graph to nodes within `depth` BFS levels of the focus, keeping
 * only links whose endpoints both survive. Used by the expanded view's depth
 * stepper when stepping DOWN — a pure filter, no refetching.
 */
export function filterToDepth(graph: MiniGraph, depth: number): MiniGraph {
  const nodes = graph.nodes.filter((n) => n.degree <= depth)
  const ids = new Set(nodes.map((n) => n.id))
  const links = graph.links.filter((l) => ids.has(l.source) && ids.has(l.target))
  return { focus: graph.focus, nodes, links, truncated: graph.truncated }
}
