/**
 * Pure relationship-graph model shared by the graph visualizations.
 *
 * Builds a de-duplicated typed-relationship graph from the resolved edges
 * exposed in `site.json` (see the "named typed relationships" feature). All
 * functions here are pure and DOM-free so they can be unit-tested directly and
 * safely bundled into any chunk.
 */

// ============================================================================
// site.json data shapes (subset we consume)
// ============================================================================

/** A resolved typed relationship edge, as served in `site.json`/`links.json`. */
export interface SiteRelationship {
  rel_type: string
  predicate: string
  neighbor: string
  neighbor_title: string
  neighbor_raw: string
  resolved: boolean
  direction: 'outgoing' | 'incoming'
  label?: string
  attributes?: Record<string, unknown>
  derived?: boolean
}

/** A `markdown_files` entry from `site.json` (subset). */
export interface SiteNote {
  url_path: string
  frontmatter?: Record<string, unknown>
  relationships?: SiteRelationship[]
}

/** A relation-type descriptor from `site.json`'s `relationship_types`. */
export interface RelationTypeConfig {
  name: string
  symmetric: boolean
  inverse: string | null
  label: string
  label_plural: string
}

// ============================================================================
// links.json data shapes
// ============================================================================

/** An outbound link entry from a page's `links.json`. */
export interface OutboundLink {
  to: string
  text: string
  anchor?: string
  internal: boolean
}

/** An inbound (backlink) entry from a page's `links.json`. */
export interface InboundLink {
  from: string
  text: string
  anchor?: string
}

/** The per-page `links.json` payload. */
export interface PageLinks {
  inbound: InboundLink[]
  outbound: OutboundLink[]
  relationships?: SiteRelationship[]
}

// ============================================================================
// Graph model
// ============================================================================

export type EdgeKind = 'hierarchical' | 'symmetric' | 'directed'

/**
 * A single de-duplicated edge in the relationship graph.
 *
 * - `hierarchical` — an inverse-pair edge (e.g. parent↔child). `from` is the
 *   anchor (parent side) and `to` the role holder (child side), so a top-down
 *   layout renders ancestors above descendants.
 * - `symmetric` — a symmetric edge (e.g. spouse/sibling). `from`/`to` are an
 *   ordered (sorted) unordered pair; drawn as an undirected dotted link.
 * - `directed` — an unknown/plain directed edge. `from` → `to` is subject →
 *   object.
 */
export interface GraphEdge {
  from: string
  to: string
  kind: EdgeKind
  relType: string
  label: string
}

export interface GraphNode {
  urlPath: string
  title: string
  born?: string
  died?: string
  gender?: string
  /** Portrait/avatar path from frontmatter `image`, if any. */
  image?: string
  /** Birth place from frontmatter `born_place`, if any. */
  bornPlace?: string
  isFocus: boolean
}

export interface RelationshipGraph {
  focus: string
  nodes: GraphNode[]
  edges: GraphEdge[]
  /**
   * Hierarchical edges removed to keep the hierarchical subgraph acyclic (see
   * `breakHierarchicalCycles`). Each one is a contradictory parent/child
   * declaration worth surfacing to the author.
   *
   * OPTIONAL on purpose: hand-written `RelationshipGraph` literals (test
   * fixtures, callers building a graph directly) stay valid, and an absent
   * value means "nothing was dropped" — same as an empty array.
   */
  droppedEdges?: GraphEdge[]
}

/** Registry lookups over the `relationship_types` list. */
export interface Registry {
  get(name: string): RelationTypeConfig | undefined
  isSymmetric(name: string): boolean
  inverseOf(name: string): string | null
}

/** Build a case-insensitive registry from `relationship_types`. */
export function buildRegistry(types: RelationTypeConfig[]): Registry {
  const byName = new Map<string, RelationTypeConfig>()
  for (const t of types) {
    if (t && typeof t.name === 'string') byName.set(t.name.toLowerCase(), t)
  }
  return {
    get: (n) => byName.get(n.toLowerCase()),
    isSymmetric: (n) => byName.get(n.toLowerCase())?.symmetric === true,
    inverseOf: (n) => byName.get(n.toLowerCase())?.inverse ?? null,
  }
}

// ============================================================================
// Pure helpers
// ============================================================================

export function capitalize(s: string): string {
  return s ? s.charAt(0).toUpperCase() + s.slice(1) : s
}

/** Extract the first 4-digit year from a date-ish value, if any. */
export function yearOf(value: unknown): string | undefined {
  if (value == null) return undefined
  const match = String(value).match(/\d{4}/)
  return match ? match[0] : undefined
}

/**
 * Normalize a frontmatter `gender` value to a lowercased, trimmed string used
 * as the node's gender tint key. Non-string or empty values yield `undefined`
 * (no tint), so a mistyped value is simply ignored.
 */
export function normalizeGender(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined
  const gender = value.trim().toLowerCase()
  return gender ? gender : undefined
}

/** Trimmed non-empty string from a frontmatter value, else `undefined`. */
function stringOf(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined
  const s = value.trim()
  return s ? s : undefined
}

/** Build a lifespan suffix like "(1925–1999)", "(b. 1950)" or "(d. 2010)". */
export function formatLifespan(born?: string, died?: string): string {
  if (born && died) return `(${born}–${died})`
  if (born) return `(b. ${born})`
  if (died) return `(d. ${died})`
  return ''
}

/** A note's display title from frontmatter, falling back to the path segment. */
export function nodeTitle(fm: Record<string, unknown>, path: string): string {
  const title = fm['title']
  if (typeof title === 'string' && title.trim()) return title.trim()
  const segment = path.split('/').filter(Boolean).pop()
  return segment ?? path
}

/** Full node label: title plus an optional lifespan suffix. */
export function formatNodeLabel(node: GraphNode): string {
  const lifespan = formatLifespan(node.born, node.died)
  return lifespan ? `${node.title} ${lifespan}` : node.title
}

/**
 * Classify one relationship (from `selfPath`'s viewpoint) into a normalized,
 * de-duplicatable graph edge. Returns `null` for edges that should not appear
 * in the graph (unresolved, empty neighbour, or self-loops).
 *
 * The `key` is stable regardless of which endpoint the edge is viewed from, so
 * reciprocal/derived declarations of the same underlying relationship collapse
 * to a single edge.
 */
export function classifyRelationship(
  selfPath: string,
  rel: SiteRelationship,
  registry: Registry
): { edge: GraphEdge; key: string } | null {
  if (!rel.resolved || !rel.neighbor) return null
  const neighbor = rel.neighbor
  if (neighbor === selfPath) return null

  const predicate = (rel.predicate || rel.rel_type).toLowerCase()
  const symmetric = registry.isSymmetric(predicate) || registry.isSymmetric(rel.rel_type)
  const inverse = registry.inverseOf(predicate) ?? registry.inverseOf(rel.rel_type)

  if (symmetric) {
    const [a, b] = [selfPath, neighbor].sort()
    const label = rel.label ?? registry.get(predicate)?.label ?? capitalize(predicate)
    return {
      edge: { from: a, to: b, kind: 'symmetric', relType: predicate, label },
      key: `sym|${predicate}|${a}|${b}`,
    }
  }

  if (inverse) {
    // Inverse pair (e.g. parent/child): canonicalize onto the lexicographically
    // smaller type name so both viewpoints collapse to one edge. `predicate`
    // names the neighbour's role relative to `self`, so the neighbour is the
    // role holder and `self` is the anchor.
    const inv = inverse.toLowerCase()
    const forward = predicate < inv ? predicate : inv
    const roleHolder = predicate === forward ? neighbor : selfPath
    const anchor = predicate === forward ? selfPath : neighbor
    return {
      edge: { from: anchor, to: roleHolder, kind: 'hierarchical', relType: forward, label: rel.label ?? '' },
      key: `hier|${forward}|${anchor}|${roleHolder}`,
    }
  }

  // Unknown/plain directed edge: orient subject → object using `direction`.
  const subject = rel.direction === 'outgoing' ? selfPath : neighbor
  const object = rel.direction === 'outgoing' ? neighbor : selfPath
  const relType = rel.rel_type.toLowerCase()
  const label = rel.label ?? registry.get(rel.rel_type)?.label ?? rel.predicate
  return {
    edge: { from: subject, to: object, kind: 'directed', relType, label },
    key: `dir|${relType}|${subject}|${object}`,
  }
}

/** Build a `url_path` → note lookup from raw `site.json` data. */
export function notesByPathFromSite(data: { markdown_files?: SiteNote[] } | null): Map<string, SiteNote> {
  const map = new Map<string, SiteNote>()
  const files = data?.markdown_files
  if (Array.isArray(files)) {
    for (const file of files) {
      if (file && typeof file.url_path === 'string') map.set(file.url_path, file)
    }
  }
  return map
}

export const DEFAULT_DEPTH = 3
export const MAX_DEPTH = 6
export const DEFAULT_MAX_NODES = 80

/**
 * Relationship types excluded from the graph entirely. Sibling links clutter the
 * family tree and are redundant: siblings share parents, so a sibling that is a
 * co-child of an in-graph parent still appears via the parent→child edges (with
 * the correct generation). Only a sibling reachable *solely* through a sibling
 * link drops out — which is intended.
 */
export const EXCLUDED_REL_TYPES = new Set(['sibling'])

/**
 * True when a relationship should participate in the graph (node expansion,
 * edges, and generations). Matches the excluded set against both the lowercased
 * `predicate` and `rel_type` so either spelling is caught.
 */
export function isGraphRelationship(rel: SiteRelationship): boolean {
  return (
    !EXCLUDED_REL_TYPES.has((rel.predicate || '').toLowerCase()) &&
    !EXCLUDED_REL_TYPES.has((rel.rel_type || '').toLowerCase())
  )
}

/**
 * Normalize a note path to the canonical trailing-slash form used by
 * `site.json` `url_path` keys.
 *
 * In server mode, markdown is served at non-trailing-slash URLs in place (200,
 * no redirect), so `getCanonicalPath()` can return a slashless path (e.g.
 * `/people/george`) while every `url_path` ends in `/` (e.g. `/people/george/`).
 * Returns `p` unchanged when empty or already slash-terminated, else appends a
 * trailing `/`.
 */
export function canonicalizeNotePath(p: string): string {
  if (!p || p.endsWith('/')) return p
  return `${p}/`
}

/** Code-unit string comparison (locale-independent, unlike `localeCompare`). */
function compareStrings(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0
}

/**
 * Make the `hierarchical` subgraph of `edges` acyclic by dropping back edges.
 * Non-hierarchical edges (symmetric, directed) are returned untouched — cycles
 * there are harmless and often meaningful.
 *
 * WHY THIS EXISTS: contradictory frontmatter — note A declaring B as its parent
 * while note B declares A as its parent — does NOT collapse during
 * de-duplication. `classifyRelationship` canonicalizes each declaration onto the
 * smaller type name and swaps the endpoints, so the two produce the distinct
 * keys `hier|child|B|A` and `hier|child|A|B` and both survive. family-chart's
 * `calculateTree()` hands such data straight to `d3.hierarchy()`, which has no
 * cycle detection and allocates a fresh node per step forever, pinning the
 * browser's main thread. Two people in a loop are enough, and the library's
 * ancestry/progeny depth limits do not help because trimming happens only after
 * the hierarchy has been fully materialized.
 *
 * ALGORITHM: one iterative depth-first search (explicit stack — recursion could
 * exceed the JS stack on a deep lineage) over the hierarchical edges only. An
 * edge whose target is currently ON the DFS stack is a back edge: it closes a
 * cycle and is dropped. Tree, forward, and cross edges are kept. Removing every
 * back edge found by a DFS always leaves a DAG, so a single pass suffices.
 * O(V + E).
 *
 * DETERMINISM: hierarchical edges are sorted by `(relType, from, to)` before
 * traversal and DFS roots are visited in the order `[focus, ...sorted paths]`.
 * The result therefore never depends on `Map` insertion order, and the edges
 * reachable from the focus are explored first, so those closest to the focus
 * survive as tree edges.
 *
 * Kept edges are returned in their original input order, so callers that rely
 * on edge ordering see no change when there is nothing to drop.
 */
export function breakHierarchicalCycles(
  edges: GraphEdge[],
  focus: string
): { edges: GraphEdge[]; droppedEdges: GraphEdge[] } {
  const hierarchical = edges.filter((e) => e.kind === 'hierarchical')
  if (hierarchical.length === 0) return { edges, droppedEdges: [] }

  const sorted = [...hierarchical].sort(
    (a, b) =>
      compareStrings(a.relType, b.relType) ||
      compareStrings(a.from, b.from) ||
      compareStrings(a.to, b.to)
  )

  const outgoing = new Map<string, GraphEdge[]>()
  const paths = new Set<string>()
  for (const edge of sorted) {
    paths.add(edge.from)
    paths.add(edge.to)
    const list = outgoing.get(edge.from)
    if (list) list.push(edge)
    else outgoing.set(edge.from, [edge])
  }

  // DFS colouring: ON_STACK = grey (an ancestor of the current node),
  // FINISHED = black (fully explored).
  const ON_STACK = 1
  const FINISHED = 2
  const state = new Map<string, typeof ON_STACK | typeof FINISHED>()
  const droppedEdges: GraphEdge[] = []

  for (const root of [focus, ...[...paths].sort(compareStrings)]) {
    if (!paths.has(root) || state.has(root)) continue
    state.set(root, ON_STACK)
    // Each frame remembers how far through its node's outgoing edges we are.
    const stack: Array<{ node: string; next: number }> = [{ node: root, next: 0 }]
    while (stack.length > 0) {
      const frame = stack[stack.length - 1]
      const out = outgoing.get(frame.node)
      if (!out || frame.next >= out.length) {
        state.set(frame.node, FINISHED)
        stack.pop()
        continue
      }
      const edge = out[frame.next++]
      const target = state.get(edge.to)
      if (target === ON_STACK) {
        // Back edge (a self-loop counts): dropping it breaks the cycle.
        droppedEdges.push(edge)
        continue
      }
      // Already fully explored → a forward/cross edge, which cannot close a
      // cycle; keep it but do not descend again.
      if (target === FINISHED) continue
      state.set(edge.to, ON_STACK)
      stack.push({ node: edge.to, next: 0 })
    }
  }

  if (droppedEdges.length === 0) return { edges, droppedEdges }
  const dropped = new Set(droppedEdges)
  return { edges: edges.filter((edge) => !dropped.has(edge)), droppedEdges }
}

/**
 * Build a de-duplicated relationship graph around `focusPath`.
 *
 * Nodes are collected breadth-first up to `depth` hops from the focus (capped
 * at `maxNodes` for performance); every relationship among the collected nodes
 * is then added as an edge, de-duplicated by canonical key. Unresolved edges,
 * self-loops, and edges to notes outside the collected set are skipped.
 *
 * INVARIANT: the `hierarchical` subgraph of the returned `edges` is guaranteed
 * ACYCLIC. Contradictory parent/child declarations that would otherwise form a
 * loop are removed and reported in `droppedEdges`; see
 * `breakHierarchicalCycles` for why an unbroken loop hangs the browser.
 * Non-hierarchical edges are never dropped.
 */
export function buildRelationshipGraph(
  focusPath: string,
  notesByPath: Map<string, SiteNote>,
  registry: Registry,
  depth: number = DEFAULT_DEPTH,
  maxNodes: number = DEFAULT_MAX_NODES
): RelationshipGraph {
  // Normalize the focus to the canonical trailing-slash form so slashless
  // server-mode URLs (e.g. `/people/george`) match `site.json`'s `url_path`
  // keys. Neighbors already come canonical, so only the focus needs this.
  const focus = canonicalizeNotePath(focusPath)

  if (!notesByPath.has(focus)) {
    return { focus, nodes: [], edges: [], droppedEdges: [] }
  }

  const clampedDepth = Math.max(1, Math.min(Math.floor(depth) || DEFAULT_DEPTH, MAX_DEPTH))
  const cap = Math.max(1, Math.floor(maxNodes) || DEFAULT_MAX_NODES)

  // Phase 1: breadth-first node collection.
  const included = new Set<string>([focus])
  let frontier: string[] = [focus]
  for (let d = 0; d < clampedDepth && frontier.length > 0 && included.size < cap; d++) {
    const next: string[] = []
    for (const path of frontier) {
      const note = notesByPath.get(path)
      if (!note?.relationships) continue
      for (const rel of note.relationships) {
        if (!isGraphRelationship(rel)) continue
        if (!rel.resolved || !rel.neighbor || rel.neighbor === path) continue
        const neighbor = rel.neighbor
        if (!notesByPath.has(neighbor) || included.has(neighbor)) continue
        if (included.size >= cap) break
        included.add(neighbor)
        next.push(neighbor)
      }
    }
    frontier = next
  }

  // Phase 2: build node objects.
  const nodes: GraphNode[] = [...included].map((path) => {
    const fm = notesByPath.get(path)?.frontmatter ?? {}
    return {
      urlPath: path,
      title: nodeTitle(fm, path),
      born: yearOf(fm['born']),
      died: yearOf(fm['died']),
      gender: normalizeGender(fm['gender']),
      image: stringOf(fm['image']),
      bornPlace: stringOf(fm['born_place']),
      isFocus: path === focus,
    }
  })

  // Phase 3: collect every edge among included nodes, de-duplicated.
  const edges = new Map<string, GraphEdge>()
  for (const path of included) {
    const note = notesByPath.get(path)
    if (!note?.relationships) continue
    for (const rel of note.relationships) {
      if (!isGraphRelationship(rel)) continue
      const classified = classifyRelationship(path, rel, registry)
      if (!classified) continue
      const { edge, key } = classified
      if (!included.has(edge.from) || !included.has(edge.to)) continue
      if (!edges.has(key)) edges.set(key, edge)
    }
  }

  // Phase 4: enforce the acyclic-hierarchy invariant. Must run here rather than
  // in each consumer: family-chart hangs the tab on a parent/child loop, and
  // reporting the dropped edges alongside the graph is what lets the UI point
  // the author at the contradictory notes.
  const { edges: acyclic, droppedEdges } = breakHierarchicalCycles([...edges.values()], focus)

  return { focus, nodes, edges: acyclic, droppedEdges }
}

/** True when the graph contains at least one hierarchical (tree) edge. */
export function hasHierarchy(graph: RelationshipGraph): boolean {
  return graph.edges.some((e) => e.kind === 'hierarchical')
}

/**
 * Assign each node a generation index for the hierarchical family-tree layout.
 * Lower indices are older generations (emitted first / on top).
 *
 * Relative offsets between adjacent nodes:
 *  - a hierarchical edge `from`→`to` is parent→child, so `child = parent + 1`;
 *  - a symmetric edge (spouse/sibling) — and any non-hierarchical edge — keeps
 *    both endpoints on the SAME generation.
 *
 * The focus is seeded at 0 and generations propagate by BFS; the first value
 * assigned to a node wins, which both guards against cycles and guarantees
 * termination (each node is enqueued at most once). Nodes in components
 * disconnected from the focus are seeded from their own local 0. Finally every
 * value is shifted so the minimum generation is 0, giving clean ascending
 * indices (ancestors first) suitable for subgraph ids.
 */
export function computeGenerations(graph: RelationshipGraph): Map<string, number> {
  const gen = new Map<string, number>()
  const paths = graph.nodes.map((n) => n.urlPath)
  if (paths.length === 0) return gen

  // Adjacency carrying the generation delta from a node to its neighbour.
  // `graph.edges` is already free of excluded (sibling) relationships — they are
  // filtered out in `buildRelationshipGraph` — so generations aren't influenced
  // by sibling links.
  const adj = new Map<string, Array<{ other: string; delta: number }>>()
  for (const p of paths) adj.set(p, [])
  for (const e of graph.edges) {
    const fromAdj = adj.get(e.from)
    const toAdj = adj.get(e.to)
    if (!fromAdj || !toAdj) continue
    const delta = e.kind === 'hierarchical' ? 1 : 0
    fromAdj.push({ other: e.to, delta })
    toAdj.push({ other: e.from, delta: -delta })
  }

  // BFS from the focus first, then from any still-unassigned node (disconnected
  // components). First-assignment-wins is the cycle guard.
  const seeds = [graph.focus, ...paths]
  for (const seed of seeds) {
    if (!adj.has(seed) || gen.has(seed)) continue
    gen.set(seed, 0)
    const queue: string[] = [seed]
    while (queue.length > 0) {
      const cur = queue.shift()!
      const curGen = gen.get(cur)!
      for (const { other, delta } of adj.get(cur)!) {
        if (gen.has(other)) continue
        gen.set(other, curGen + delta)
        queue.push(other)
      }
    }
  }

  // Shift so the minimum generation is 0 (ancestors, seeded negative, become the
  // lowest indices and thus the top rows).
  let min = Infinity
  for (const v of gen.values()) if (v < min) min = v
  if (min !== 0 && Number.isFinite(min)) {
    for (const [k, v] of gen) gen.set(k, v - min)
  }
  return gen
}
