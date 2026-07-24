/**
 * Entry point for the lazy `mbr-genealogy.min.js` chunk (built by
 * vite.genealogy.config.ts; loaded on demand by the `<mbr-genealogy>` trigger
 * on `type: person` pages).
 *
 * `mountGenealogy()` renders the chart-type selector plus the active chart
 * (family-chart by default, timeline tree as the alternative) into the given
 * container, persisting the selection in localStorage.
 *
 * IMPORTANT: nothing in this chunk may import stateful main-bundle modules
 * (shared.ts, graph/links-cache.ts, …) — those hold top-level fetches/caches
 * that would re-run inside the chunk. Everything stateful arrives through the
 * `GenealogyContext` object.
 */
import {
  CHART_TYPES,
  injectStylesOnce,
  type GenealogyChartInstance,
  type GenealogyContext,
} from './chart-registry.js'
import { createSelector, readStoredChartId, resolveChartId, storeChartId } from './selector.js'

export type { GenealogyChart, GenealogyChartInstance, GenealogyContext } from './chart-registry.js'

export interface GenealogyController {
  destroy(): void
  /** Switch charts programmatically (same path as the selector). */
  setChartType(id: string): void
}

export function mountGenealogy(container: HTMLElement, ctx: GenealogyContext): GenealogyController {
  injectStylesOnce(container.getRootNode(), 'mbr-genealogy-base', BASE_CSS)

  const root = document.createElement('div')
  root.className = 'mbr-genealogy-root'
  const chartArea = document.createElement('div')
  chartArea.className = 'gen-chart-area'
  root.appendChild(chartArea)
  container.appendChild(root)

  let activeId = readStoredChartId()
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

  const selector = createSelector(activeId, setChartType)
  root.appendChild(selector)
  mountActive()

  return {
    setChartType,
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
  --mbr-gen-male: #1565c0;
  --mbr-gen-female: #c2185b;
  --mbr-gen-male-fill: #d7e3f8;
  --mbr-gen-female-fill: #f8d7e3;
  --mbr-gen-focus: #e65100;
  --mbr-gen-focus-fill: #ffe0b2;
}

@media (prefers-color-scheme: dark) {
  .mbr-genealogy-root {
    --mbr-gen-male: #64b5f6;
    --mbr-gen-female: #f48fb1;
    --mbr-gen-male-fill: rgba(100, 181, 246, 0.22);
    --mbr-gen-female-fill: rgba(244, 143, 177, 0.22);
    --mbr-gen-focus: #ffb74d;
    --mbr-gen-focus-fill: rgba(255, 183, 77, 0.25);
  }
}

.gen-chart-area {
  height: 100%;
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
`
