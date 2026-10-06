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

/**
 * Which way a hierarchical relation type points. A declaration on note S
 * `{type: T, to: O}` reads "O is S's T"; `up` means O ranks ABOVE S (parent,
 * manager, employer, the person S assists), `down` the reverse.
 */
export type RelationHierarchy = 'up' | 'down'

/** A relation-type descriptor from `site.json`'s `relationship_types`. */
export interface RelationTypeConfig {
  name: string
  symmetric: boolean
  inverse: string | null
  label: string
  label_plural: string
  /** Absent on non-hierarchical types and on registries older than the field. */
  hierarchy?: RelationHierarchy | null
  /** Free-form grouping; the charts use `family` and `work`. */
  category?: string | null
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
  /** The relation type's category (see `Registry.categoryOf`), if any. */
  category?: string
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
  /** Frontmatter `type` (`person`, `organization`, …), lowercased. */
  type?: string
  /** Frontmatter `job_title`, if any. */
  jobTitle?: string
  /** Frontmatter `department`, if any. */
  department?: string
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
  /**
   * The type's hierarchy, or its inverse's flipped when only the inverse
   * declares one. `undefined` = not declared (callers fall back to the legacy
   * lexicographic orientation).
   */
  hierarchyOf(name: string): RelationHierarchy | undefined
  /** The type's category; see {@link BUILTIN_RELATION_CATEGORIES} for the fallback. */
  categoryOf(name: string): string | undefined
  /** True when at least one type declares a category (the modern registry). */
  readonly hasCategories: boolean
}

/**
 * Categories of the built-in relation types, consulted ONLY when the registry
 * declares no categories at all — a site.json written before `category`
 * existed, or a repository's own `relationship_types` that predates it. Without
 * this, such a repository's org chart would be empty and its family chart would
 * draw managers as parents. A custom type absent here stays uncategorised,
 * which keeps the legacy "every hierarchy is a family tree" behaviour for it.
 */
export const BUILTIN_RELATION_CATEGORIES: Readonly<Record<string, string>> = {
  parent: 'family',
  child: 'family',
  spouse: 'family',
  sibling: 'family',
  reports_to: 'work',
  manages: 'work',
  assistant: 'work',
  assists: 'work',
  employer: 'work',
  employee: 'work',
  colleague: 'work',
}

function asHierarchy(value: unknown): RelationHierarchy | undefined {
  return value === 'up' || value === 'down' ? value : undefined
}

function flipHierarchy(h: RelationHierarchy | undefined): RelationHierarchy | undefined {
  return h === 'up' ? 'down' : h === 'down' ? 'up' : undefined
}

function nonEmpty(value: unknown): string | undefined {
  return typeof value === 'string' && value !== '' ? value : undefined
}

/** Build a case-insensitive registry from `relationship_types`. */
export function buildRegistry(types: RelationTypeConfig[]): Registry {
  const byName = new Map<string, RelationTypeConfig>()
  for (const t of types) {
    if (t && typeof t.name === 'string') byName.set(t.name.toLowerCase(), t)
  }
  const hasCategories = [...byName.values()].some((t) => nonEmpty(t.category) !== undefined)
  const get = (n: string) => byName.get(n.toLowerCase())
  return {
    get,
    isSymmetric: (n) => get(n)?.symmetric === true,
    inverseOf: (n) => get(n)?.inverse ?? null,
    hierarchyOf: (n) => {
      const type = get(n)
      const own = asHierarchy(type?.hierarchy)
      if (own || !type?.inverse) return own
      return flipHierarchy(asHierarchy(get(type.inverse)?.hierarchy))
    },
    categoryOf: (n) => {
      if (!hasCategories) return BUILTIN_RELATION_CATEGORIES[n.toLowerCase()]
      const type = get(n)
      return nonEmpty(type?.category) ?? (type?.inverse ? nonEmpty(get(type.inverse)?.category) : undefined)
    },
    hasCategories,
  }
}

// ============================================================================
// Pure helpers
// ============================================================================

export function capitalize(s: string): string {
  return s ? s.charAt(0).toUpperCase() + s.slice(1) : s
}

/** A month-day date with no year: `03-19` or the vCard-style `--03-19`. */
const MONTH_DAY = /^\s*(?:--)?\d{1,2}-\d{1,2}\s*$/

/**
 * Extract the year from a date-ish value: `1927-03-19`, `1927-03`, `1927`, or
 * free text like `c. 1898`. A month-day date (`03-19`, `--03-19`) has no year
 * and yields `undefined` rather than a misread.
 */
export function yearOf(value: unknown): string | undefined {
  if (value == null) return undefined
  const text = String(value)
  if (MONTH_DAY.test(text)) return undefined
  const match = text.match(/\d{4}/)
  return match ? match[0] : undefined
}

/**
 * A date from frontmatter by contract label. site.json flattens `dates:` into
 * dot keys (`dates.birthday`); a page's own `window.frontmatter` may still be
 * nested, so both shapes are read.
 */
function datesEntry(fm: Record<string, unknown>, label: string): unknown {
  const flat = fm[`dates.${label}`]
  if (flat != null && flat !== '') return flat
  const nested = fm['dates']
  if (nested && typeof nested === 'object' && !Array.isArray(nested)) {
    return (nested as Record<string, unknown>)[label]
  }
  return undefined
}

/**
 * Birth and death years. `dates.birthday`/`dates.death` win; the deprecated
 * `born`/`died` are the fallback — the server normalises them into `dates.*`,
 * but an older or cached site.json may not have.
 */
export function lifeYears(fm: Record<string, unknown>): { born?: string; died?: string } {
  return {
    born: yearOf(datesEntry(fm, 'birthday')) ?? yearOf(fm['born']),
    died: yearOf(datesEntry(fm, 'death')) ?? yearOf(fm['died']),
  }
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
    const category = registry.categoryOf(predicate)
    return {
      edge: { from: a, to: b, kind: 'symmetric', relType: predicate, label, ...(category ? { category } : {}) },
      key: `sym|${predicate}|${a}|${b}`,
    }
  }

  if (inverse) {
    const inv = inverse.toLowerCase()
    const hierarchy = registry.hierarchyOf(predicate)
    // `from` is always the SUPERIOR (parent, manager, employer) and `to` the
    // subordinate, so top-down layouts put superiors above. Both viewpoints of
    // one relationship must canonicalize onto the same name and endpoints so
    // they collapse to one edge.
    let canonical: string
    let superior: string
    let subordinate: string
    if (hierarchy) {
      // The registry says which way the pair points. `predicate` is the
      // neighbour's role relative to `self`: `up` ⇒ the neighbour ranks above.
      // The canonical name is the `down` half (child, manages, employee, …).
      canonical = hierarchy === 'down' ? predicate : inv
      superior = hierarchy === 'up' ? neighbor : selfPath
      subordinate = hierarchy === 'up' ? selfPath : neighbor
    } else {
      // Legacy fallback for registries without `hierarchy`: canonicalize onto
      // the lexicographically smaller name and treat its role holder as the
      // subordinate. Right for parent/child by luck of the alphabet; a custom
      // `manager`/`report` pair came out upside down, which is why `hierarchy`
      // exists.
      canonical = predicate < inv ? predicate : inv
      superior = predicate === canonical ? selfPath : neighbor
      subordinate = predicate === canonical ? neighbor : selfPath
    }
    const category = registry.categoryOf(canonical)
    return {
      edge: {
        from: superior,
        to: subordinate,
        kind: 'hierarchical',
        relType: canonical,
        label: rel.label ?? '',
        ...(category ? { category } : {}),
      },
      key: `hier|${canonical}|${superior}|${subordinate}`,
    }
  }

  // Unknown/plain directed edge: orient subject → object using `direction`.
  const subject = rel.direction === 'outgoing' ? selfPath : neighbor
  const object = rel.direction === 'outgoing' ? neighbor : selfPath
  const relType = rel.rel_type.toLowerCase()
  const label = rel.label ?? registry.get(rel.rel_type)?.label ?? rel.predicate
  const category = registry.categoryOf(relType)
  return {
    edge: { from: subject, to: object, kind: 'directed', relType, label, ...(category ? { category } : {}) },
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

/** A graph node from a note's (simplified) frontmatter. */
export function graphNodeFor(path: string, fm: Record<string, unknown>, isFocus: boolean): GraphNode {
  const { born, died } = lifeYears(fm)
  const type = stringOf(fm['type'])?.toLowerCase()
  const jobTitle = stringOf(fm['job_title'])
  const department = stringOf(fm['department'])
  return {
    urlPath: path,
    title: nodeTitle(fm, path),
    born,
    died,
    gender: normalizeGender(fm['gender']),
    image: stringOf(fm['image']),
    bornPlace: stringOf(fm['born_place']),
    ...(type ? { type } : {}),
    ...(jobTitle ? { jobTitle } : {}),
    ...(department ? { department } : {}),
    isFocus,
  }
}

/**
 * True when an edge belongs on a FAMILY chart (family chart, timeline tree).
 *
 * Contract: only `category === 'family'` edges count. Legacy fallback when the
 * registry declares no categories at all: every edge whose type is not a
 * built-in work type (`BUILTIN_RELATION_CATEGORIES`) — what those charts drew
 * before categories existed, minus managers drawn as parents.
 */
export function isFamilyEdge(edge: GraphEdge, registry: Registry): boolean {
  if (edge.category) return edge.category === 'family'
  return !registry.hasCategories
}

/** True when an edge is part of a work hierarchy (org chart material). */
export function isWorkHierarchyEdge(edge: GraphEdge): boolean {
  return edge.kind === 'hierarchical' && edge.category === 'work'
}

/**
 * The family-only view of a graph: family edges, and the nodes still connected
 * to the focus through them. Pruning matters — a colleague reached through a
 * work edge would otherwise float as an unconnected card on a family chart.
 * `droppedEdges` is narrowed the same way so cycle notices stay on topic.
 */
export function familyGraph(graph: RelationshipGraph, registry: Registry): RelationshipGraph {
  const edges = graph.edges.filter((e) => isFamilyEdge(e, registry))
  const adjacency = new Map<string, string[]>()
  const link = (a: string, b: string) => {
    const list = adjacency.get(a)
    if (list) list.push(b)
    else adjacency.set(a, [b])
  }
  for (const e of edges) {
    link(e.from, e.to)
    link(e.to, e.from)
  }
  const reached = new Set<string>([graph.focus])
  const queue = [graph.focus]
  for (let i = 0; i < queue.length; i++) {
    for (const next of adjacency.get(queue[i]) ?? []) {
      if (reached.has(next)) continue
      reached.add(next)
      queue.push(next)
    }
  }
  return {
    focus: graph.focus,
    nodes: graph.nodes.filter((n) => reached.has(n.urlPath)),
    edges: edges.filter((e) => reached.has(e.from) && reached.has(e.to)),
    droppedEdges: (graph.droppedEdges ?? []).filter((e) => isFamilyEdge(e, registry)),
  }
}

/** True when the graph has at least one edge a family chart would draw. */
export function hasFamilyEdges(graph: RelationshipGraph, registry: Registry): boolean {
  return graph.edges.some((e) => isFamilyEdge(e, registry))
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
  const nodes: GraphNode[] = [...included].map((path) =>
    graphNodeFor(path, notesByPath.get(path)?.frontmatter ?? {}, path === focus)
  )

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
