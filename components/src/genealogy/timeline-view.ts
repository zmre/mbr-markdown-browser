/**
 * Timeline-tree chart view: renders the pure `timeline-layout.ts` output as an
 * SVG via lit-html templates, with pan/zoom from the shared
 * `SvgViewportController`, `+`/`−`/`⤢` control buttons, and click/keyboard
 * navigation on the person cards.
 */
import { html, render, svg, nothing, type TemplateResult } from 'lit'
import { SvgViewportController } from '../graph/viewport-controller.js'
import {
  AXIS_W,
  CARD_H,
  CARD_W,
  MARGIN,
  MIN_READABLE_CARD_PX,
  TARGET_READABLE_CARD_PX,
  computeInitialViewBox,
  computeTimelineLayout,
  type TimelineLayout,
  type TimelineNode,
} from './timeline-layout.js'
import { formatLifespan } from '../graph/relationship-graph.js'
import {
  injectStylesOnce,
  type GenealogyChart,
  type GenealogyChartInstance,
  type GenealogyContext,
} from './chart-registry.js'

/** Truncate a card title so it fits the fixed card width. */
const MAX_TITLE_CHARS = 20
function truncateTitle(title: string): string {
  return title.length > MAX_TITLE_CHARS ? `${title.slice(0, MAX_TITLE_CHARS - 1)}…` : title
}

function cardClass(node: TimelineNode): string {
  if (node.isFocus) return 'tl-card tl-focus'
  if (node.gender === 'male' || node.gender === 'm' || node.gender === 'man') {
    return 'tl-card tl-male'
  }
  if (node.gender === 'female' || node.gender === 'f' || node.gender === 'woman') {
    return 'tl-card tl-female'
  }
  return 'tl-card'
}

function hoverText(node: TimelineNode): string {
  const lifespan = formatLifespan(node.born, node.died)
  const place = node.bornPlace ? ` — ${node.bornPlace}` : ''
  return `${node.title}${lifespan ? ` ${lifespan}` : ''}${place}`
}

function cardTemplate(
  node: TimelineNode,
  onActivate: (node: TimelineNode) => void,
  onKeydown: (e: KeyboardEvent, node: TimelineNode) => void
): TemplateResult {
  const left = node.x - CARD_W / 2
  const top = node.y - CARD_H / 2
  const lifespan = formatLifespan(node.born, node.died)
  return svg`
    <g
      class="${cardClass(node)}"
      role="link"
      tabindex="0"
      aria-label="Go to ${node.title}"
      @click=${() => onActivate(node)}
      @keydown=${(e: KeyboardEvent) => onKeydown(e, node)}
    >
      <title>${hoverText(node)}</title>
      <rect x="${left}" y="${top}" width="${CARD_W}" height="${CARD_H}" rx="6"></rect>
      <text class="tl-title" x="${node.x}" y="${top + (lifespan ? 14 : 21)}" text-anchor="middle">
        ${truncateTitle(node.title)}
      </text>
      ${
        lifespan
          ? svg`<text class="tl-lifespan" x="${node.x}" y="${top + 27}" text-anchor="middle">${lifespan}</text>`
          : nothing
      }
    </g>
  `
}

function chartTemplate(
  layout: TimelineLayout,
  handlers: {
    onActivate: (node: TimelineNode) => void
    onKeydown: (e: KeyboardEvent, node: TimelineNode) => void
    onZoomIn: () => void
    onZoomOut: () => void
    onReset: () => void
  }
): TemplateResult {
  // Year tick labels are rendered on BOTH sides; the pure layout reserves an
  // `AXIS_W` gutter on each edge when `hasYears`, so nothing clips. Gridlines
  // span the whole content area between the two label columns.
  const leftLabelX = MARGIN + AXIS_W - 10
  const rightLabelX = layout.width - MARGIN - AXIS_W + 10
  return html`
    <svg
      viewBox="0 0 ${layout.width} ${layout.height}"
      preserveAspectRatio="xMidYMid meet"
      aria-label="Family timeline chart"
    >
      ${layout.hasYears
        ? svg`
            <g class="tl-axis" aria-hidden="true">
              ${layout.ticks.map(
                (tick) => svg`
                  <line class="tl-gridline" x1="${leftLabelX + 6}" y1="${tick.y}" x2="${rightLabelX - 6}" y2="${tick.y}"></line>
                  <text class="tl-tick" x="${leftLabelX}" y="${tick.y + 4}" text-anchor="end">${tick.label}</text>
                  <text class="tl-tick" x="${rightLabelX}" y="${tick.y + 4}" text-anchor="start">${tick.label}</text>
                `
              )}
            </g>
          `
        : nothing}
      <g class="tl-links" aria-hidden="true">
        ${layout.links.map((link) => svg`<path class="tl-link tl-link-${link.colorKey}" d="${link.d}"></path>`)}
      </g>
      <g class="tl-marriages" aria-hidden="true">
        ${layout.marriageBars.map(
          (bar) => svg`<line class="tl-marriage" x1="${bar.x1}" y1="${bar.y1}" x2="${bar.x2}" y2="${bar.y2}"></line>`
        )}
      </g>
      <g class="tl-cards">
        ${layout.nodes.map((node) => cardTemplate(node, handlers.onActivate, handlers.onKeydown))}
      </g>
    </svg>
    <div class="rel-graph-controls">
      <button type="button" aria-label="Zoom in" title="Zoom in" @click=${handlers.onZoomIn}>+</button>
      <button type="button" aria-label="Zoom out" title="Zoom out" @click=${handlers.onZoomOut}>−</button>
      <button type="button" aria-label="Reset view" title="Reset view" @click=${handlers.onReset}>⤢</button>
    </div>
  `
}

function mountTimeline(container: HTMLElement, ctx: GenealogyContext): GenealogyChartInstance {
  injectStylesOnce(container.getRootNode(), 'mbr-genealogy-timeline', TIMELINE_CSS)

  const canvas = document.createElement('div')
  canvas.className = 'tl-canvas'
  container.appendChild(canvas)

  let controller: SvgViewportController | null = null
  const layout = computeTimelineLayout(ctx.graph)
  const onActivate = (node: TimelineNode) => {
    // A pan that started on a card must not navigate; consuming the drag flag
    // clears it so the next genuine click works again.
    if (controller?.consumeDragFlag()) return
    ctx.navigate(node.path)
  }
  const onKeydown = (e: KeyboardEvent, node: TimelineNode) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      ctx.navigate(node.path)
    }
  }

  render(
    chartTemplate(layout, {
      onActivate,
      onKeydown,
      onZoomIn: () => controller?.zoomIn(),
      onZoomOut: () => controller?.zoomOut(),
      onReset: () => controller?.reset(),
    }),
    canvas
  )

  const svgEl = canvas.querySelector('svg')
  if (svgEl instanceof SVGSVGElement) {
    // Smart initial view: fit-all when cards stay readable; for very
    // wide/shallow trees start partly zoomed in on the focus person instead.
    // The SVG's own viewBox (fit-all) remains the controller's base, so manual
    // zoom-out to the full tree is always possible; ⤢ returns to this view.
    const rect = canvas.getBoundingClientRect()
    const focus = layout.nodes.find((n) => n.isFocus)
    const initialView = computeInitialViewBox({
      contentWidth: layout.width,
      contentHeight: layout.height,
      canvasWidth: rect.width,
      canvasHeight: rect.height,
      cardH: CARD_H,
      focusX: focus?.x ?? layout.width / 2,
      focusY: focus?.y ?? layout.height / 2,
      minReadablePx: MIN_READABLE_CARD_PX,
      targetPx: TARGET_READABLE_CARD_PX,
    })
    controller = new SvgViewportController(canvas, svgEl, { initialView })
  }

  return {
    destroy() {
      controller?.destroy()
      controller = null
      canvas.remove()
    },
  }
}

export const timelineChartType: GenealogyChart = {
  id: 'timeline',
  label: 'Timeline tree',
  mount: mountTimeline,
}

/**
 * Chart styles. Gender colors come from the shared `--mbr-gen-*` custom
 * properties defined (with dark-mode overrides) in the base stylesheet that
 * `mountGenealogy()` injects.
 */
const TIMELINE_CSS = `
.tl-canvas {
  position: relative;
  height: 100%;
  overflow: hidden;
  cursor: grab;
  touch-action: none;
  border-radius: 4px;
}

.tl-canvas svg {
  width: 100%;
  height: 100%;
  display: block;
}

.tl-gridline {
  stroke: var(--pico-muted-border-color, #e0e0e0);
  stroke-width: 1;
  opacity: 0.55;
}

.tl-tick {
  font-size: 11px;
  fill: var(--pico-muted-color, #777);
}

.tl-link {
  fill: none;
  stroke-width: 1.75;
  opacity: 0.85;
}

.tl-link-male {
  stroke: var(--mbr-gen-male, #1565c0);
}

.tl-link-female {
  stroke: var(--mbr-gen-female, #c2185b);
}

.tl-link-neutral {
  stroke: var(--pico-muted-color, #999);
}

.tl-marriage {
  stroke: var(--pico-muted-color, #999);
  stroke-width: 2;
  stroke-dasharray: 4 3;
  opacity: 0.8;
}

.tl-card {
  cursor: pointer;
}

.tl-card rect {
  fill: var(--pico-card-background-color, #fff);
  stroke: var(--pico-muted-border-color, #bbb);
  stroke-width: 1.5;
}

.tl-card.tl-male rect {
  fill: var(--mbr-gen-male-fill, #d7e3f8);
  stroke: var(--mbr-gen-male, #1565c0);
}

.tl-card.tl-female rect {
  fill: var(--mbr-gen-female-fill, #f8d7e3);
  stroke: var(--mbr-gen-female, #c2185b);
}

.tl-card.tl-focus rect {
  fill: var(--mbr-gen-focus-fill, #ffe0b2);
  stroke: var(--mbr-gen-focus, #e65100);
  stroke-width: 2;
}

.tl-card:focus-visible rect {
  stroke: var(--pico-primary, #0172ad);
  stroke-width: 2.5;
}

.tl-title {
  font-size: 12px;
  font-weight: 600;
  fill: var(--pico-color, #222);
}

.tl-lifespan {
  font-size: 10px;
  fill: var(--pico-muted-color, #666);
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
