/**
 * Entry point for the lazy `mbr-genealogy.min.js` chunk (built by
 * vite.genealogy.config.ts; loaded on demand by the `<mbr-genealogy>` trigger
 * on `type: person` and `type: organization` pages).
 *
 * `mountGenealogy()` renders the chart-type selector plus the active chart
 * (Family chart, Timeline tree, Org chart, All people, All) into the given
 * container. The opening chart is the persisted choice when it applies to this
 * note, else the first applicable of family → org → all people (see
 * `chooseChartId`); only an explicit selection is persisted.
 *
 * The relationship graph is built HERE, not by the trigger: the trigger only
 * decides whether there is anything to chart (a cheap scan of the focus note),
 * so neither the graph builder nor the contradictory-link notice costs the main
 * bundle anything, and no graph is built for a chart that never scrolls into
 * view.
 *
 * IMPORTANT: nothing in this chunk may import stateful main-bundle modules
 * (shared.ts, graph/links-cache.ts, …) — those hold top-level fetches/caches
 * that would re-run inside the chunk. Everything stateful arrives through the
 * `GenealogyMountInput` object.
 */
import {
  DEFAULT_DEPTH,
  DEFAULT_MAX_NODES,
  buildRegistry,
  buildRelationshipGraph,
  nodeTitle,
  type PageLinks,
  type RelationTypeConfig,
  type SiteNote,
} from '../graph/relationship-graph.js'
import { renderDroppedNotice } from './dropped-notice.js'
import {
  CHART_TYPES,
  injectStylesOnce,
  type GenealogyChartInstance,
  type GenealogyContext,
} from './chart-registry.js'
import {
  chooseChartId,
  createSelector,
  readStoredChartChoice,
  resolveChartId,
  storeChartId,
} from './selector.js'

export type { GenealogyChart, GenealogyChartInstance, GenealogyContext } from './chart-registry.js'

export interface GenealogyController {
  destroy(): void
  /** Switch charts programmatically (same path as the selector). */
  setChartType(id: string): void
  /** The context the charts were given (graph, registry, services). */
  readonly context: GenealogyContext
}

/** What the main-bundle trigger hands the chunk. */
export interface GenealogyMountInput {
  /** `url_path` → site.json note for every known note. */
  notesByPath: Map<string, SiteNote>
  /** site.json's `relationship_types`, as served. */
  relationshipTypes: RelationTypeConfig[]
  /** Canonical url_path of the focused note. */
  focusPath: string
  /** Relationship hops for the family graph (default `DEFAULT_DEPTH`). */
  depth?: number
  /** Node cap for the family graph (default `DEFAULT_MAX_NODES`). */
  maxNodes?: number
  resolveUrl: (path: string) => string
  navigate: (path: string) => void
  graphDepth: number
  loadGraphChunk: () => Promise<boolean>
  fetchPageLinks: (path: string) => Promise<PageLinks | null>
}

/** Build the registry and the focus graph: everything the charts read. */
export function buildGenealogyContext(input: GenealogyMountInput): GenealogyContext {
  const registry = buildRegistry(input.relationshipTypes)
  const graph = buildRelationshipGraph(
    input.focusPath,
    input.notesByPath,
    registry,
    input.depth ?? DEFAULT_DEPTH,
    input.maxNodes ?? DEFAULT_MAX_NODES
  )
  return {
    graph,
    notesByPath: input.notesByPath,
    registry,
    focusPath: graph.focus,
    resolveUrl: input.resolveUrl,
    navigate: input.navigate,
    graphDepth: input.graphDepth,
    loadGraphChunk: input.loadGraphChunk,
    fetchPageLinks: input.fetchPageLinks,
  }
}

export function mountGenealogy(container: HTMLElement, input: GenealogyMountInput): GenealogyController {
  injectStylesOnce(container.getRootNode(), 'mbr-genealogy-base', BASE_CSS)
  const ctx = buildGenealogyContext(input)

  const root = document.createElement('div')
  root.className = 'mbr-genealogy-root'
  const titleOf = (path: string) => {
    const note = ctx.notesByPath.get(path)
    return note ? nodeTitle(note.frontmatter ?? {}, path) : path
  }
  const notice = renderDroppedNotice(ctx.graph.droppedEdges, titleOf, ctx.resolveUrl)
  if (notice) root.appendChild(notice)
  // The selector overlays the chart, not the notice, so both share a wrapper.
  const chartWrap = document.createElement('div')
  chartWrap.className = 'gen-chart-wrap'
  const chartArea = document.createElement('div')
  chartArea.className = 'gen-chart-area'
  chartWrap.appendChild(chartArea)
  root.appendChild(chartWrap)
  container.appendChild(root)

  let activeId = chooseChartId(readStoredChartChoice(), ctx)
  let instance: GenealogyChartInstance | null = null

  const mountActive = (): void => {
    const chart = CHART_TYPES.find((c) => c.id === activeId) ?? CHART_TYPES[0]
    instance = chart.mount(chartArea, ctx)
  }

  const setChartType = (id: string): void => {
    const next = resolveChartId(id)
    if (next === activeId && instance) return
    instance?.destroy()
    instance = null
    chartArea.replaceChildren()
    activeId = next
    storeChartId(next)
    if (selector.value !== next) selector.value = next
    mountActive()
  }

  const selector = createSelector(activeId, setChartType, ctx)
  chartWrap.appendChild(selector)
  mountActive()

  return {
    setChartType,
    context: ctx,
    destroy() {
      instance?.destroy()
      instance = null
      root.remove()
    },
  }
}

/**
 * Base styles: root sizing, the selector overlay, and the shared gender /
 * focus color custom properties (with dark-mode overrides) consumed by both
 * chart views.
 */
const BASE_CSS = `
.mbr-genealogy-root {
  position: relative;
  height: 100%;
  display: flex;
  flex-direction: column;
  --mbr-gen-male: #1565c0;
  --mbr-gen-female: #c2185b;
  --mbr-gen-male-fill: #d7e3f8;
  --mbr-gen-female-fill: #f8d7e3;
  --mbr-gen-focus: #e65100;
  --mbr-gen-focus-fill: #ffe0b2;
  /* Org chart + graph charts. Derived from Pico variables, so they follow
     every Pico theme and both color schemes without overrides. */
  --mbr-org-link: color-mix(in srgb, var(--pico-muted-color, #6b7280) 60%, transparent);
  --mbr-org-card-stroke: var(--pico-muted-border-color, #d1d5db);
  --mbr-org-accent: color-mix(in srgb, var(--pico-primary, #0172ad) 55%, var(--pico-card-background-color, #fff));
  --mbr-org-org-accent: var(--pico-primary, #0172ad);
  --mbr-org-org-fill: color-mix(in srgb, var(--pico-primary, #0172ad) 9%, var(--pico-card-background-color, #fff));
  --mbr-org-group-fill: color-mix(in srgb, var(--pico-primary, #0172ad) 4%, transparent);
  --mbr-org-group-stroke: color-mix(in srgb, var(--pico-primary, #0172ad) 24%, transparent);
  --mbr-graph-node-org: #c2410c;
  --mbr-graph-node-plain: color-mix(in srgb, var(--pico-muted-color, #6b7280) 45%, var(--pico-background-color, #fff));
}

@media only screen and (prefers-color-scheme: dark) {
  .mbr-genealogy-root {
    --mbr-gen-male: #64b5f6;
    --mbr-gen-female: #f48fb1;
    --mbr-gen-male-fill: rgba(100, 181, 246, 0.22);
    --mbr-gen-female-fill: rgba(244, 143, 177, 0.22);
    --mbr-gen-focus: #ffb74d;
    --mbr-gen-focus-fill: rgba(255, 183, 77, 0.25);
    --mbr-graph-node-org: #fb923c;
  }
}

.gen-chart-wrap {
  position: relative;
  flex: 1;
  min-height: 0;
}

.gen-chart-area {
  height: 100%;
}

/* Contradictory-link notice. Small and muted so it informs without competing
   with the chart; it sits inside the fixed-height canvas, so the chart shrinks
   instead of the page shifting, and it scrolls rather than crowd the chart out. */
.gen-notice {
  flex: none;
  max-height: 40%;
  overflow-y: auto;
  margin-bottom: 0.75rem;
  font-size: 0.85rem;
  line-height: 1.4;
  color: var(--pico-muted-color, #666);
}

.gen-notice p {
  margin: 0;
}

.gen-notice ul {
  margin: 0.35rem 0;
  padding-left: 1.25rem;
}

.gen-notice li {
  margin: 0.1rem 0;
}

.gen-chart-select {
  position: absolute;
  top: 0.5rem;
  left: 0.5rem;
  z-index: 3;
  padding: 0.15rem 1.4rem 0.15rem 0.5rem;
  font-size: 0.8rem;
  line-height: 1.2;
  border: 1px solid var(--pico-muted-border-color, #ccc);
  border-radius: 4px;
  background: var(--pico-background-color, #fff);
  color: var(--pico-color, #333);
  opacity: 0.9;
  cursor: pointer;
}

.gen-chart-select:hover,
.gen-chart-select:focus-visible {
  opacity: 1;
}

/* "Nothing to draw" message for a chart chosen on a note it does not fit. */
.gen-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  margin: 0;
  color: var(--pico-muted-color, #6b7280);
  font-size: 0.9rem;
}

/* Zoom controls shared by every pan/zoom chart view. */
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
