/**
 * Relationship graph component (`<mbr-relationships>`).
 *
 * Renders a mermaid graph / family tree of a note's typed relationships,
 * traversing outward from the focused note through the resolved edges exposed
 * in `site.json` (see the "named typed relationships" feature). It reuses the
 * already-embedded mermaid pipeline (no new dependencies): it generates mermaid
 * `graph` source from the collected neighbourhood and renders it itself via
 * `mermaid.render()` so it never races with the page-wide `<mbr-mermaid>`
 * scanner (that component only looks for `.mermaid` blocks, which this one does
 * not produce).
 *
 * The graph-building and mermaid-source generation are implemented as pure,
 * exported functions so they can be unit-tested without a DOM or mermaid.
 */
import { LitElement, html, css, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { unsafeHTML } from 'lit/directives/unsafe-html.js'
import { waitForDom, loadScript, getMbrAssetBase } from './dynamic-loader.ts'
import { subscribeSiteNav, getCanonicalPath, resolveUrl } from './shared.ts'

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
  isFocus: boolean
}

export interface RelationshipGraph {
  focus: string
  nodes: GraphNode[]
  edges: GraphEdge[]
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

/** Build a lifespan suffix like "(1925–1999)", "(b. 1950)" or "(d. 2010)". */
export function formatLifespan(born?: string, died?: string): string {
  if (born && died) return `(${born}–${died})`
  if (born) return `(b. ${born})`
  if (died) return `(d. ${died})`
  return ''
}

function nodeTitle(fm: Record<string, unknown>, path: string): string {
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
 * Escape a node label for the mermaid quoted-string form (`id["..."]`).
 * Double/curly quotes become the mermaid `#quot;` entity and newlines are
 * collapsed so the diagram source stays on one line per statement.
 */
export function escapeMermaidLabel(text: string): string {
  return text
    .replace(/[\r\n]+/g, ' ')
    .replace(/["“”]/g, '#quot;')
    .trim()
}

/**
 * Sanitize an edge label to a subset that is safe as an *unquoted* mermaid link
 * label (letters, digits, spaces, hyphens, underscores). Anything else is
 * dropped so a stray quote/pipe cannot break the diagram.
 */
export function sanitizeEdgeLabel(text: string): string {
  return text
    .replace(/[^\w \-]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
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

const DEFAULT_DEPTH = 3
const MAX_DEPTH = 6
const DEFAULT_MAX_NODES = 80

/**
 * Relationship types excluded from the graph entirely. Sibling links clutter the
 * family tree and are redundant: siblings share parents, so a sibling that is a
 * co-child of an in-graph parent still appears via the parent→child edges (with
 * the correct generation). Only a sibling reachable *solely* through a sibling
 * link drops out — which is intended.
 */
const EXCLUDED_REL_TYPES = new Set(['sibling'])

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

/**
 * Build a de-duplicated relationship graph around `focusPath`.
 *
 * Nodes are collected breadth-first up to `depth` hops from the focus (capped
 * at `maxNodes` for performance); every relationship among the collected nodes
 * is then added as an edge, de-duplicated by canonical key. Unresolved edges,
 * self-loops, and edges to notes outside the collected set are skipped.
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
    return { focus, nodes: [], edges: [] }
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

  return { focus, nodes, edges: [...edges.values()] }
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

/**
 * Extract the mermaid graph node id (e.g. `n3`) from a rendered SVG node
 * element's `id` attribute. Mermaid v11 flowcharts use `flowchart-<nodeId>-<n>`;
 * a trailing `n\d+` is accepted as a fallback. Returns `null` when no id is
 * present. Pure/exported for unit testing.
 */
export function mermaidNodeId(elId: string): string | null {
  if (!elId) return null
  const primary = elId.match(/^flowchart-(n\d+)-\d+$/)
  if (primary) return primary[1]
  const fallback = elId.match(/(n\d+)(?:-\d+)?$/)
  return fallback ? fallback[1] : null
}

/**
 * When true, the hierarchical layout wraps each generation's nodes in a
 * `subgraph ... direction LR` block so a generation renders as a horizontal row.
 *
 * CAVEAT: mermaid/dagre often IGNORES a subgraph's `direction` once edges cross
 * subgraph boundaries — which family-tree (parent→child) edges always do. If the
 * layout regresses, flip this to `false` to emit a plain `graph TD` with flat
 * node declarations (a one-line revert); all other output is identical.
 */
const USE_GENERATION_SUBGRAPHS = true

/**
 * Generate mermaid `graph` source for a relationship graph.
 *
 * Orientation: top-down (`graph TD`) when any hierarchical edge is present (so
 * genealogy renders as a family tree, ancestors above descendants); otherwise
 * left-to-right (`graph LR`) for purely symmetric/directed neighbourhoods.
 * Symmetric edges are drawn as undirected dotted links, hierarchical edges as
 * plain arrows (parent → child), and unknown directed edges as labelled arrows.
 * Nodes are tinted by `gender` (when set), and the focused node is highlighted
 * via a `classDef` applied last so its highlight wins over the gender tint.
 */

/**
 * Per-gender node styling. Only the genders in this palette are tinted; any
 * other value (or none) keeps mermaid's default node styling. `color:#111`
 * keeps labels readable against the soft fills in both light and dark themes.
 */
const GENDER_CLASSES: Record<string, { className: string; classDef: string }> = {
  female: { className: 'genderFemale', classDef: 'classDef genderFemale fill:#f8d7e3,stroke:#c2185b,color:#111;' },
  male: { className: 'genderMale', classDef: 'classDef genderMale fill:#d7e3f8,stroke:#1565c0,color:#111;' },
}

export function generateMermaidSource(graph: RelationshipGraph): string {
  const ids = new Map<string, string>()
  graph.nodes.forEach((node, index) => ids.set(node.urlPath, `n${index}`))

  const hierarchical = hasHierarchy(graph)
  const direction = hierarchical ? 'TD' : 'LR'
  const lines: string[] = [`graph ${direction}`]

  const nodeDecl = (node: GraphNode, indent: string): string =>
    `${indent}${ids.get(node.urlPath)!}["${escapeMermaidLabel(formatNodeLabel(node))}"]`

  if (hierarchical && USE_GENERATION_SUBGRAPHS) {
    // Group nodes into per-generation LR subgraphs, oldest generation first.
    const gens = computeGenerations(graph)
    const byGen = new Map<number, GraphNode[]>()
    for (const node of graph.nodes) {
      const g = gens.get(node.urlPath) ?? 0
      const bucket = byGen.get(g)
      if (bucket) bucket.push(node)
      else byGen.set(g, [node])
    }
    for (const g of [...byGen.keys()].sort((a, b) => a - b)) {
      lines.push(`  subgraph gen${g} [" "]`)
      lines.push('    direction LR')
      for (const node of byGen.get(g)!) lines.push(nodeDecl(node, '    '))
      lines.push('  end')
    }
  } else {
    for (const node of graph.nodes) lines.push(nodeDecl(node, '  '))
  }

  // Edges are emitted AFTER any subgraphs; mermaid resolves the node ids across
  // subgraph boundaries.
  for (const edge of graph.edges) {
    const a = ids.get(edge.from)
    const b = ids.get(edge.to)
    if (!a || !b) continue
    const label = sanitizeEdgeLabel(edge.label)
    if (edge.kind === 'symmetric') {
      lines.push(label ? `  ${a} -. ${label} .- ${b}` : `  ${a} -.- ${b}`)
    } else {
      lines.push(label ? `  ${a} -->|${label}| ${b}` : `  ${a} --> ${b}`)
    }
  }

  // Gender tints: declare only the classes actually used, then assign them.
  // Emitted before the focus block so the focus class (applied last) wins.
  const genderAssignments: string[] = []
  const usedGenders = new Set<string>()
  for (const node of graph.nodes) {
    const style = node.gender ? GENDER_CLASSES[node.gender] : undefined
    if (!style) continue
    const id = ids.get(node.urlPath)
    if (!id) continue
    usedGenders.add(node.gender!)
    genderAssignments.push(`  class ${id} ${style.className};`)
  }
  for (const gender of Object.keys(GENDER_CLASSES)) {
    if (usedGenders.has(gender)) lines.push(`  ${GENDER_CLASSES[gender].classDef}`)
  }
  lines.push(...genderAssignments)

  const focusId = ids.get(graph.focus)
  if (focusId) {
    lines.push('  classDef focus fill:#ffe0b2,stroke:#e65100,stroke-width:2px,color:#111;')
    lines.push(`  class ${focusId} focus;`)
  }

  return lines.join('\n')
}

// ============================================================================
// Viewport (zoom / pan) math — pure & unit-tested
// ============================================================================

/** An SVG `viewBox` as separate numbers. */
export interface ViewBox {
  x: number
  y: number
  w: number
  h: number
}

/** Wheel zoom sensitivity: `factor = exp(-deltaY * sens)`. */
const ZOOM_WHEEL_SENS = 0.001
/** Per-click zoom step for the +/- buttons. */
const ZOOM_BUTTON_FACTOR = 1.3
/**
 * Scale bounds relative to the initial "fit" viewBox. `minScale = 1` means the
 * fit is the most zoomed-OUT state (you cannot zoom out past fit); `maxScale`
 * caps zoom-in at 8×.
 */
const MIN_SCALE = 1
const MAX_SCALE = 8
/** Pointer travel (px) above which a gesture is a pan/drag, not a click. */
const DRAG_THRESHOLD_PX = 4

/** Parse an SVG `viewBox` attribute (`"x y w h"`). Returns null when invalid. */
export function parseViewBox(attr: string | null): ViewBox | null {
  if (!attr) return null
  const parts = attr.trim().split(/[\s,]+/).map(Number)
  if (parts.length !== 4 || parts.some((n) => !Number.isFinite(n))) return null
  const [x, y, w, h] = parts
  if (w <= 0 || h <= 0) return null
  return { x, y, w, h }
}

/** Serialize a viewBox to the `"x y w h"` attribute form. */
export function formatViewBox(vb: ViewBox): string {
  return `${vb.x} ${vb.y} ${vb.w} ${vb.h}`
}

/**
 * Clamp a desired viewBox width to the range implied by the scale bounds:
 * width is `baseW / scale`, so a larger scale ⇒ smaller width (more zoomed in).
 */
export function clampViewBoxScale(
  desiredW: number,
  baseW: number,
  minScale: number,
  maxScale: number
): number {
  const minW = baseW / maxScale // most zoomed in → smallest width
  const maxW = baseW / minScale // most zoomed out → largest width
  return Math.min(maxW, Math.max(minW, desiredW))
}

/**
 * Zoom a viewBox by `factor` (>1 zooms in) while keeping `point` (in SVG-user
 * coordinates) fixed on screen. The zoom is uniform (aspect preserved) and the
 * resulting width is clamped to the configured scale bounds.
 */
export function zoomViewBoxAtPoint(
  vb: ViewBox,
  factor: number,
  point: { x: number; y: number },
  opts: { minScale: number; maxScale: number; baseW: number }
): ViewBox {
  if (!(factor > 0) || vb.w <= 0 || vb.h <= 0) return vb
  const aspect = vb.h / vb.w
  const newW = clampViewBoxScale(vb.w / factor, opts.baseW, opts.minScale, opts.maxScale)
  const newH = newW * aspect
  // Preserve the point's fractional position within the viewBox so it stays put.
  const relX = (point.x - vb.x) / vb.w
  const relY = (point.y - vb.y) / vb.h
  return { x: point.x - relX * newW, y: point.y - relY * newH, w: newW, h: newH }
}

/** Translate a viewBox by a delta expressed in SVG-user units. */
export function panViewBox(vb: ViewBox, dxUser: number, dyUser: number): ViewBox {
  return { x: vb.x - dxUser, y: vb.y - dyUser, w: vb.w, h: vb.h }
}

/**
 * Map a client (screen) point to SVG-user coordinates using the canvas rect and
 * current viewBox. Uses a straightforward fractional mapping across the rect;
 * this is exact when the SVG fills the canvas and a good approximation under
 * `preserveAspectRatio` letterboxing (sufficient for cursor-centered zoom).
 */
export function clientPointToSvg(
  clientX: number,
  clientY: number,
  rect: { left: number; top: number; width: number; height: number },
  vb: ViewBox
): { x: number; y: number } {
  const fx = rect.width > 0 ? (clientX - rect.left) / rect.width : 0
  const fy = rect.height > 0 ? (clientY - rect.top) / rect.height : 0
  return { x: vb.x + fx * vb.w, y: vb.y + fy * vb.h }
}

// ============================================================================
// Mermaid runtime glue
// ============================================================================

interface MermaidApi {
  initialize: (config: { startOnLoad: boolean; theme: string }) => void
  render: (id: string, text: string) => Promise<{ svg: string; bindFunctions?: (el: Element) => void }>
}

interface WindowWithMermaid extends Window {
  mermaid?: MermaidApi
}

// ============================================================================
// Lit element
// ============================================================================

/**
 * `<mbr-relationships>` — renders a relationship graph for the current note.
 * Renders nothing until site data loads and only when the focused note has at
 * least one resolved relationship. `depth` and `max-nodes` are configurable.
 */
@customElement('mbr-relationships')
export class MbrRelationshipsElement extends LitElement {
  /** How many relationship hops to expand outward from the focused note. */
  @property({ type: Number })
  depth = DEFAULT_DEPTH

  /** Safety cap on graph size for very large repositories. */
  @property({ type: Number, attribute: 'max-nodes' })
  maxNodes = DEFAULT_MAX_NODES

  @state()
  private _svg = ''

  @state()
  private _graph: RelationshipGraph | null = null

  private _siteData: { markdown_files?: SiteNote[]; relationship_types?: RelationTypeConfig[] } | null = null
  private _source = ''
  /** Mermaid node id (`n0`, `n1`, …) → the note's `url_path`, for click nav. */
  private _nodeIdToPath: Map<string, string> = new Map()
  private _unsubscribeSiteNav?: () => void

  // Zoom/pan viewport state -------------------------------------------------
  /** The live SVG element and its fit ("base") + current viewBox. */
  private _svgEl: SVGSVGElement | null = null
  private _baseViewBox: ViewBox | null = null
  private _viewBox: ViewBox | null = null
  /** Aborts the canvas wheel/pointer listeners; the canvas we bound them to. */
  private _viewportListeners?: AbortController
  private _boundCanvas: HTMLElement | null = null
  /** Single-pointer pan gesture state. */
  private _panPointerId: number | null = null
  private _panStartClient: { x: number; y: number } | null = null
  private _panStartVB: ViewBox | null = null
  private _panMoved = 0
  private _panActive = false
  /** Set when a gesture dragged past threshold, so the node click is skipped. */
  private _wasDragging = false
  /** Active pointers (for two-finger pinch-zoom) and last pinch distance. */
  private _activePointers: Map<number, { x: number; y: number }> = new Map()
  private _pinchPrevDist: number | null = null

  override connectedCallback() {
    super.connectedCallback()
    waitForDom().then(() => {
      this._unsubscribeSiteNav = subscribeSiteNav((state) => {
        if (state.data && state.data !== this._siteData) {
          this._siteData = state.data
          void this._rebuild()
        }
      })
    })
  }

  override disconnectedCallback() {
    super.disconnectedCallback()
    this._unsubscribeSiteNav?.()
    this._viewportListeners?.abort()
  }

  override updated(changed: PropertyValues) {
    if ((changed.has('depth') || changed.has('maxNodes')) && this._siteData) {
      void this._rebuild()
    }
    // After each render that produced a new SVG: normalize the viewport (re-fit
    // to the fresh graph) and (re)bind node click-navigation.
    if (changed.has('_svg') && this._svg) {
      this._setupViewport()
      this._bindNodeLinks()
    }
  }

  private get _heading(): string {
    return this._graph && hasHierarchy(this._graph) ? 'Family tree' : 'Relationships'
  }

  private async _rebuild() {
    const data = this._siteData
    if (!data) return

    const notes = notesByPathFromSite(data)
    const focusPath = getCanonicalPath()
    const types = Array.isArray(data.relationship_types) ? data.relationship_types : []
    const registry = buildRegistry(types)
    const graph = buildRelationshipGraph(focusPath, notes, registry, this.depth, this.maxNodes)
    // Node ids mirror `generateMermaidSource`'s `n${index}` assignment order.
    this._nodeIdToPath = new Map(graph.nodes.map((nd, i) => [`n${i}`, nd.urlPath]))

    const source = graph.edges.length > 0 ? generateMermaidSource(graph) : ''

    // Skip re-rendering identical source (e.g. redundant lifecycle updates).
    if (source === this._source) {
      this._graph = graph
      return
    }
    this._source = source
    this._graph = graph

    if (!source) {
      this._svg = ''
      return
    }

    try {
      await loadScript(`${getMbrAssetBase()}mermaid.min.js`)
      const mermaid = (window as unknown as WindowWithMermaid).mermaid
      if (!mermaid) return
      const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches
      mermaid.initialize({ startOnLoad: false, theme: prefersDark ? 'dark' : 'default' })
      const renderId = `mbr-rel-${Math.random().toString(36).slice(2)}`
      const { svg } = await mermaid.render(renderId, source)
      this._svg = svg
    } catch (err) {
      console.warn('[mbr-relationships] Failed to render graph:', err)
      this._svg = ''
    }
  }

  /**
   * After each mermaid render, make graph nodes act as SAME-TAB links to their
   * note. Chosen approach: bind handlers manually in the shadow DOM rather than
   * mermaid-native `click` directives — that keeps navigation same-tab and
   * avoids downgrading mermaid's `securityLevel` to `'loose'`. Idempotent per
   * SVG via a data flag; the focus/self node is linked too (harmless).
   */
  private _bindNodeLinks(): void {
    const root = this.shadowRoot
    if (!root) return
    const nodeEls = root.querySelectorAll<SVGGElement>('.rel-graph-canvas g.node')
    nodeEls.forEach((g) => {
      if (g.dataset.mbrLinked === '1') return
      const nodeId = mermaidNodeId(g.id)
      if (!nodeId) return
      const urlPath = this._nodeIdToPath.get(nodeId)
      if (!urlPath) return
      g.dataset.mbrLinked = '1'

      const target = resolveUrl(urlPath)
      const node = this._graph?.nodes.find((n) => n.urlPath === urlPath)
      const label = node ? node.title : urlPath
      g.style.cursor = 'pointer'
      g.setAttribute('role', 'link')
      g.setAttribute('tabindex', '0')
      g.setAttribute('aria-label', `Go to ${label}`)

      g.addEventListener('click', () => {
        // A pan that started on this node must not navigate; consume the flag so
        // the next genuine click works again.
        if (this._wasDragging) {
          this._wasDragging = false
          return
        }
        window.location.assign(target)
      })
      g.addEventListener('keydown', (e: KeyboardEvent) => {
        // Keyboard activation always navigates (never a drag).
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          window.location.assign(target)
        }
      })
    })
  }

  // Viewport (zoom/pan) wiring -----------------------------------------------

  private get _scaleOpts(): { minScale: number; maxScale: number; baseW: number } {
    return { minScale: MIN_SCALE, maxScale: MAX_SCALE, baseW: this._baseViewBox?.w ?? 1 }
  }

  /**
   * Normalize the freshly-rendered mermaid SVG for a bounded, zoomable viewport:
   * strip mermaid's inline sizing, make it fill the fixed-height canvas, read
   * the fit viewBox (falling back to the content bbox), and reset the current
   * viewBox to it. Interaction listeners are bound once per canvas element.
   */
  private _setupViewport(): void {
    const root = this.shadowRoot
    if (!root) return
    const canvas = root.querySelector<HTMLElement>('.rel-graph-canvas')
    const svg = canvas?.querySelector('svg') ?? null
    if (!canvas || !(svg instanceof SVGSVGElement)) return
    this._svgEl = svg

    // Strip mermaid's inline sizing so the SVG fills the fixed-height canvas.
    svg.style.maxWidth = 'none'
    svg.style.width = '100%'
    svg.style.height = '100%'
    svg.style.display = 'block'
    svg.removeAttribute('width')
    svg.removeAttribute('height')

    // Re-fit: a fresh graph resets both the base and current viewBox.
    const base = parseViewBox(svg.getAttribute('viewBox')) ?? this._bboxViewBox(svg)
    if (base) {
      this._baseViewBox = base
      this._viewBox = { ...base }
      this._applyViewBox()
    }

    // Bind pointer/wheel listeners once to the (stable) canvas element.
    if (this._boundCanvas !== canvas) {
      this._viewportListeners?.abort()
      this._viewportListeners = new AbortController()
      this._boundCanvas = canvas
      this._bindViewportListeners(canvas, this._viewportListeners.signal)
    }
  }

  /** Fallback fit box from the rendered content when no `viewBox` is present. */
  private _bboxViewBox(svg: SVGSVGElement): ViewBox | null {
    try {
      const b = svg.getBBox()
      if (b.width > 0 && b.height > 0) return { x: b.x, y: b.y, w: b.width, h: b.height }
    } catch {
      // getBBox throws if the element is not yet rendered; ignore.
    }
    return null
  }

  private _applyViewBox(): void {
    if (this._svgEl && this._viewBox) {
      this._svgEl.setAttribute('viewBox', formatViewBox(this._viewBox))
    }
  }

  private _bindViewportListeners(canvas: HTMLElement, signal: AbortSignal): void {
    const opts = { signal }
    canvas.addEventListener('wheel', (e) => this._onWheel(e, canvas), { signal, passive: false })
    canvas.addEventListener('pointerdown', (e) => this._onPointerDown(e, canvas), opts)
    canvas.addEventListener('pointermove', (e) => this._onPointerMove(e, canvas), opts)
    const end = (e: PointerEvent) => this._onPointerUp(e, canvas)
    canvas.addEventListener('pointerup', end, opts)
    canvas.addEventListener('pointercancel', end, opts)
    canvas.addEventListener(
      'dblclick',
      (e) => {
        if ((e.target as Element | null)?.closest('.rel-graph-controls')) return
        this._resetView()
      },
      opts
    )
  }

  private _onWheel(e: WheelEvent, canvas: HTMLElement): void {
    if (!this._viewBox || !this._baseViewBox) return
    // Prevent the page from scrolling; also covers macOS trackpad pinch, which
    // arrives here as a wheel event with `ctrlKey` set.
    e.preventDefault()
    const rect = canvas.getBoundingClientRect()
    const point = clientPointToSvg(e.clientX, e.clientY, rect, this._viewBox)
    const factor = Math.exp(-e.deltaY * ZOOM_WHEEL_SENS)
    this._viewBox = zoomViewBoxAtPoint(this._viewBox, factor, point, this._scaleOpts)
    this._applyViewBox()
  }

  private _onPointerDown(e: PointerEvent, canvas: HTMLElement): void {
    if ((e.target as Element | null)?.closest('.rel-graph-controls')) return
    if (!this._viewBox) return
    this._activePointers.set(e.pointerId, { x: e.clientX, y: e.clientY })
    if (this._activePointers.size >= 2) {
      // Entering a pinch: abandon any in-progress single-pointer pan.
      this._endPan(canvas)
      this._pinchPrevDist = null
      return
    }
    // Tentative pan. Pointer capture is deferred until movement passes the drag
    // threshold, so a genuine click still reaches the node and navigates.
    this._panPointerId = e.pointerId
    this._panStartClient = { x: e.clientX, y: e.clientY }
    this._panStartVB = { ...this._viewBox }
    this._panMoved = 0
    this._panActive = false
    this._wasDragging = false
  }

  private _onPointerMove(e: PointerEvent, canvas: HTMLElement): void {
    if (this._activePointers.has(e.pointerId)) {
      this._activePointers.set(e.pointerId, { x: e.clientX, y: e.clientY })
    }
    if (this._activePointers.size >= 2) {
      this._handlePinch(canvas)
      return
    }
    if (this._panPointerId !== e.pointerId || !this._panStartVB || !this._panStartClient) return

    const rect = canvas.getBoundingClientRect()
    const dxClient = e.clientX - this._panStartClient.x
    const dyClient = e.clientY - this._panStartClient.y
    this._panMoved = Math.max(this._panMoved, Math.hypot(dxClient, dyClient))
    if (!this._panActive) {
      if (this._panMoved <= DRAG_THRESHOLD_PX) return
      this._panActive = true
      try {
        canvas.setPointerCapture(e.pointerId)
      } catch {
        // Capture can fail if the pointer already ended; harmless.
      }
      canvas.style.cursor = 'grabbing'
    }
    const dxUser = rect.width > 0 ? dxClient * (this._panStartVB.w / rect.width) : 0
    const dyUser = rect.height > 0 ? dyClient * (this._panStartVB.h / rect.height) : 0
    this._viewBox = panViewBox(this._panStartVB, dxUser, dyUser)
    this._applyViewBox()
  }

  private _onPointerUp(e: PointerEvent, canvas: HTMLElement): void {
    this._activePointers.delete(e.pointerId)
    if (this._panPointerId === e.pointerId) {
      // Only a real drag suppresses the ensuing node click.
      this._wasDragging = this._panActive
      this._endPan(canvas, e.pointerId)
    }
    if (this._activePointers.size < 2) this._pinchPrevDist = null
  }

  private _endPan(canvas: HTMLElement, pointerId?: number): void {
    if (this._panActive) {
      if (pointerId != null) {
        try {
          canvas.releasePointerCapture(pointerId)
        } catch {
          // Already released; ignore.
        }
      }
      canvas.style.cursor = 'grab'
    }
    this._panActive = false
    this._panPointerId = null
    this._panStartClient = null
    this._panStartVB = null
    this._panMoved = 0
  }

  private _handlePinch(canvas: HTMLElement): void {
    if (!this._viewBox || !this._baseViewBox) return
    const pts = [...this._activePointers.values()]
    if (pts.length < 2) return
    const [a, b] = pts
    const dist = Math.hypot(a.x - b.x, a.y - b.y)
    const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }
    if (this._pinchPrevDist != null && this._pinchPrevDist > 0 && dist > 0) {
      const factor = dist / this._pinchPrevDist
      const rect = canvas.getBoundingClientRect()
      const point = clientPointToSvg(mid.x, mid.y, rect, this._viewBox)
      this._viewBox = zoomViewBoxAtPoint(this._viewBox, factor, point, this._scaleOpts)
      this._applyViewBox()
    }
    this._pinchPrevDist = dist
  }

  private _zoomByButton(factor: number): void {
    if (!this._viewBox || !this._baseViewBox) return
    const center = {
      x: this._viewBox.x + this._viewBox.w / 2,
      y: this._viewBox.y + this._viewBox.h / 2,
    }
    this._viewBox = zoomViewBoxAtPoint(this._viewBox, factor, center, this._scaleOpts)
    this._applyViewBox()
  }

  private _resetView(): void {
    if (!this._baseViewBox) return
    this._viewBox = { ...this._baseViewBox }
    this._applyViewBox()
  }

  override render() {
    if (!this._svg) return nothing
    return html`
      <figure class="rel-graph" role="group" aria-label="${this._heading}">
        <figcaption>${this._heading}</figcaption>
        <div class="rel-graph-canvas">
          ${unsafeHTML(this._svg)}
          <div class="rel-graph-controls">
            <button
              type="button"
              aria-label="Zoom in"
              title="Zoom in"
              @click=${() => this._zoomByButton(ZOOM_BUTTON_FACTOR)}
            >
              +
            </button>
            <button
              type="button"
              aria-label="Zoom out"
              title="Zoom out"
              @click=${() => this._zoomByButton(1 / ZOOM_BUTTON_FACTOR)}
            >
              −
            </button>
            <button
              type="button"
              aria-label="Reset view"
              title="Reset view"
              @click=${() => this._resetView()}
            >
              ⤢
            </button>
          </div>
        </div>
      </figure>
    `
  }

  static override styles = css`
    :host {
      display: block;
    }

    .rel-graph {
      max-width: 1024px;
      margin: 2rem auto;
      padding: 1rem 1.25rem 1.25rem;
      border: 1px solid var(--pico-muted-border-color, #e0e0e0);
      border-radius: 8px;
      background: var(--pico-card-background-color, transparent);
    }

    .rel-graph figcaption {
      font-weight: 600;
      margin-bottom: 0.75rem;
      color: var(--pico-color, #333);
    }

    /* Bounded viewport: fixed height stops mermaid from auto-shrinking the SVG,
       and pan/zoom happen via viewBox manipulation inside this window. */
    .rel-graph-canvas {
      position: relative;
      height: min(70vh, 640px);
      overflow: hidden;
      cursor: grab;
      touch-action: none;
      border-radius: 4px;
    }

    .rel-graph-canvas svg {
      width: 100%;
      height: 100%;
      display: block;
    }

    .rel-graph-canvas g.node {
      cursor: pointer;
    }

    .rel-graph-controls {
      position: absolute;
      top: 0.5rem;
      right: 0.5rem;
      display: flex;
      flex-direction: column;
      gap: 0.25rem;
      z-index: 2;
    }

    .rel-graph-controls button {
      width: 2rem;
      height: 2rem;
      padding: 0;
      display: flex;
      align-items: center;
      justify-content: center;
      font-size: 1.1rem;
      line-height: 1;
      cursor: pointer;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
      opacity: 0.85;
    }

    .rel-graph-controls button:hover {
      opacity: 1;
    }
  `
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-relationships': MbrRelationshipsElement
  }
}
