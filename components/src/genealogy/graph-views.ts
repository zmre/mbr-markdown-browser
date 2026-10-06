/**
 * "All people" and "All" charts: force-directed graphs drawn by the existing
 * `<mbr-mini-graph>` element in its `inline` mode.
 *
 * The element lives in the separate `mbr-graph.min.js` chunk and is NOT
 * imported here (only its type): bundling it would register the custom element
 * twice and ship d3-force twice. `ctx.loadGraphChunk()` — the main bundle's
 * single-flight loader, shared with the info panel — defines it, then this
 * module creates it by tag name.
 *
 * Data comes from site.json through a SYNTHESIZED `fetchLinks`: each note's
 * resolved `relationships`, no inbound/outbound links, no network. "All"
 * additionally fetches the focus note's own `links.json` (once, only when that
 * chart is chosen) for its plain links; those neighbours are drawn in a muted
 * color and do not expand further.
 */
import type { MbrMiniGraphElement } from '../graph/mbr-mini-graph.js'
import {
  DEFAULT_MAX_NODES,
  nodeTitle,
  type PageLinks,
  type SiteNote,
} from '../graph/relationship-graph.js'
import {
  injectStylesOnce,
  type GenealogyChart,
  type GenealogyChartInstance,
  type GenealogyContext,
} from './chart-registry.js'

/** Upper bound of the mini graph's depth stepper (mirrors `graph_depth`'s range). */
const MAX_GRAPH_DEPTH = 5

/** A note's relationships as a `links.json` payload: no plain links. */
export function relationshipLinks(note: SiteNote | undefined): PageLinks | null {
  if (!note) return null
  return { inbound: [], outbound: [], relationships: note.relationships ?? [] }
}

/** Resolved relationship neighbours of a note that are themselves notes. */
function relationshipNeighbors(path: string, notes: Map<string, SiteNote>): string[] {
  const out: string[] = []
  for (const rel of notes.get(path)?.relationships ?? []) {
    if (rel.resolved && rel.neighbor && rel.neighbor !== path && notes.has(rel.neighbor)) {
      out.push(rel.neighbor)
    }
  }
  return out
}

/**
 * Notes reachable from `focus` through resolved relationships within `depth`
 * hops (the focus included). Used to tell relationship nodes from plain-link
 * nodes on the "All" chart.
 */
export function relationshipReach(focus: string, notes: Map<string, SiteNote>, depth: number): Set<string> {
  const reached = new Set<string>([focus])
  let frontier = [focus]
  for (let d = 0; d < depth && frontier.length > 0; d++) {
    const next: string[] = []
    for (const path of frontier) {
      for (const neighbor of relationshipNeighbors(path, notes)) {
        if (reached.has(neighbor)) continue
        reached.add(neighbor)
        next.push(neighbor)
      }
    }
    frontier = next
  }
  return reached
}

/** True when a resolved relationship joins `a` and `b` (either side declares it). */
export function areRelated(a: string, b: string, notes: Map<string, SiteNote>): boolean {
  return relationshipNeighbors(a, notes).includes(b) || relationshipNeighbors(b, notes).includes(a)
}

/**
 * The `fetchLinks` service for a chart: relationships for every note, plus —
 * for "All" — the focus's real `links.json` merged in. When that fetch yields
 * nothing (link tracking off), the focus falls back to relationships only and
 * `onPlainLinksUnavailable` fires so the chart can say why.
 */
export function chartFetchLinks(
  ctx: Pick<GenealogyContext, 'focusPath' | 'notesByPath' | 'fetchPageLinks'>,
  includePlainLinks: boolean,
  onPlainLinksUnavailable?: () => void
): (path: string) => Promise<PageLinks | null> {
  return async (path) => {
    const synthesized = relationshipLinks(ctx.notesByPath.get(path))
    if (!includePlainLinks || path !== ctx.focusPath) return synthesized
    const real = await ctx.fetchPageLinks(path)
    if (!real) {
      onPlainLinksUnavailable?.()
      return synthesized
    }
    return {
      inbound: real.inbound ?? [],
      outbound: real.outbound ?? [],
      relationships: synthesized?.relationships ?? real.relationships ?? [],
    }
  }
}

function isOrganization(path: string, notes: Map<string, SiteNote>): boolean {
  const type = notes.get(path)?.frontmatter?.['type']
  return typeof type === 'string' && type.trim().toLowerCase() === 'organization'
}

/** Node class for the graph: plain-link neighbours muted, organizations tinted. */
export function graphNodeClass(
  id: string,
  focus: string,
  notes: Map<string, SiteNote>,
  related: Set<string> | null
): string | undefined {
  if (id === focus) return undefined
  if (related && !related.has(id)) return 'node-plain'
  return isOrganization(id, notes) ? 'node-org' : undefined
}

type GraphMode = 'people' | 'all'

function legendItem(swatchClass: string, text: string): HTMLElement {
  const item = document.createElement('span')
  item.className = 'gen-legend-item'
  const swatch = document.createElement('span')
  swatch.className = `gen-legend-swatch ${swatchClass}`
  swatch.setAttribute('aria-hidden', 'true')
  item.append(swatch, document.createTextNode(text))
  return item
}

function mountForceGraph(container: HTMLElement, ctx: GenealogyContext, mode: GraphMode): GenealogyChartInstance {
  injectStylesOnce(container.getRootNode(), 'mbr-genealogy-graph', GRAPH_CSS)
  const wrap = document.createElement('div')
  wrap.className = 'gen-graph'
  container.appendChild(wrap)

  const status = document.createElement('p')
  status.className = 'gen-empty'
  status.setAttribute('role', 'status')
  status.textContent = 'Loading graph…'
  wrap.appendChild(status)

  let destroyed = false
  const notes = ctx.notesByPath
  // Reach is computed to the stepper's maximum, so stepping deeper in the
  // chart never recolours a relationship node as a plain link.
  const related = mode === 'all' ? relationshipReach(ctx.focusPath, notes, MAX_GRAPH_DEPTH) : null
  const hasOrganizations = (ids: Iterable<string>) => [...ids].some((id) => id !== ctx.focusPath && isOrganization(id, notes))

  const legend = document.createElement('div')
  legend.className = 'gen-legend'
  const unavailable = document.createElement('p')
  unavailable.className = 'gen-graph-note'
  unavailable.hidden = true
  unavailable.textContent = 'Plain links are not shown: link tracking is off for this site.'

  void ctx.loadGraphChunk().then((ready) => {
    if (destroyed) return
    if (!ready || !customElements.get('mbr-mini-graph')) {
      status.textContent = 'The graph could not be loaded.'
      return
    }
    const graph = document.createElement('mbr-mini-graph') as MbrMiniGraphElement
    graph.inline = true
    graph.focusPath = ctx.focusPath
    graph.depth = ctx.graphDepth
    graph.maxNodes = DEFAULT_MAX_NODES
    graph.isKnownNote = (path) => notes.has(path)
    graph.getMeta = (path) => {
      const fm = notes.get(path)?.frontmatter ?? {}
      const description = typeof fm['description'] === 'string' ? fm['description'] : undefined
      return { title: nodeTitle(fm, path), ...(description ? { description } : {}) }
    }
    graph.resolveHref = ctx.resolveUrl
    graph.nodeClass = (id) => graphNodeClass(id, ctx.focusPath, notes, related)
    if (mode === 'all') {
      graph.linkClass = (source, target) => (areRelated(source, target, notes) ? undefined : 'link-plain')
    }
    graph.fetchLinks = chartFetchLinks(ctx, mode === 'all', () => {
      unavailable.hidden = false
    })

    // Legend: only the kinds that can actually appear.
    legend.replaceChildren()
    if (mode === 'all') {
      legend.append(legendItem('swatch-related', 'Relationship'), legendItem('swatch-plain', 'Linked note'))
    }
    if (hasOrganizations(related ?? relationshipReach(ctx.focusPath, notes, MAX_GRAPH_DEPTH))) {
      legend.append(legendItem('swatch-org', 'Organization'))
    }

    status.remove()
    wrap.append(graph, unavailable)
    if (legend.childElementCount > 0) wrap.append(legend)
  })

  return {
    destroy() {
      destroyed = true
      wrap.remove()
    },
  }
}

export const allPeopleChartType: GenealogyChart = {
  id: 'all-people',
  label: 'All people',
  mount: (container, ctx) => mountForceGraph(container, ctx, 'people'),
}

export const allChartType: GenealogyChart = {
  id: 'all',
  label: 'All',
  mount: (container, ctx) => mountForceGraph(container, ctx, 'all'),
}

const GRAPH_CSS = `
.gen-graph {
  position: relative;
  height: 100%;
}

.gen-graph mbr-mini-graph {
  display: block;
  height: 100%;
}

.gen-legend {
  position: absolute;
  left: 0.5rem;
  bottom: 0.5rem;
  display: flex;
  flex-wrap: wrap;
  gap: 0.35rem 0.85rem;
  padding: 0.25rem 0.55rem;
  border: 1px solid var(--pico-muted-border-color, #e0e0e0);
  border-radius: 6px;
  background: var(--pico-background-color, #fff);
  font-size: 0.75rem;
  color: var(--pico-muted-color, #666);
  pointer-events: none;
}

.gen-legend-item {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
}

.gen-legend-swatch {
  width: 0.7rem;
  height: 0.7rem;
  border-radius: 50%;
}

.swatch-related {
  background: color-mix(in oklab, var(--pico-primary, #0172ad) 70%, var(--pico-background-color, #fff));
}

.swatch-plain {
  background: var(--mbr-graph-node-plain, #b8bec8);
}

.swatch-org {
  background: var(--mbr-graph-node-org, #c2410c);
}

.gen-graph-note {
  position: absolute;
  left: 0.5rem;
  bottom: 2.4rem;
  margin: 0;
  font-size: 0.75rem;
  color: var(--pico-muted-color, #666);
}
`
