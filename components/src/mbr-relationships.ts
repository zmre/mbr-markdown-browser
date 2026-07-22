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
import { subscribeSiteNav, getCanonicalPath } from './shared.ts'

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

  const direction = hasHierarchy(graph) ? 'TD' : 'LR'
  const lines: string[] = [`graph ${direction}`]

  for (const node of graph.nodes) {
    const id = ids.get(node.urlPath)!
    lines.push(`  ${id}["${escapeMermaidLabel(formatNodeLabel(node))}"]`)
  }

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
  private _unsubscribeSiteNav?: () => void

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
  }

  override updated(changed: PropertyValues) {
    if ((changed.has('depth') || changed.has('maxNodes')) && this._siteData) {
      void this._rebuild()
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

  override render() {
    if (!this._svg) return nothing
    return html`
      <figure class="rel-graph" role="group" aria-label="${this._heading}">
        <figcaption>${this._heading}</figcaption>
        <div class="rel-graph-canvas">${unsafeHTML(this._svg)}</div>
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

    .rel-graph-canvas {
      overflow-x: auto;
    }

    .rel-graph-canvas svg {
      max-width: 100%;
      height: auto;
      display: block;
      margin: 0 auto;
    }
  `
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-relationships': MbrRelationshipsElement
  }
}
