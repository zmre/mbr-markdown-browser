/**
 * Org chart view: renders the pure `org-layout.ts` output as an SVG via
 * lit-html, with pan/zoom from the shared `SvgViewportController` and
 * click/keyboard navigation on the cards. A "+N more" card navigates to the
 * person whose hidden reports it stands for, where they are drawn in full.
 */
import { html, render, svg, nothing, type TemplateResult } from 'lit'
import { SvgViewportController } from '../graph/viewport-controller.js'
import { nodeTitle } from '../graph/relationship-graph.js'
import { computeInitialViewBox } from './timeline-layout.js'
import {
  ORG_CARD_H,
  buildOrgTree,
  computeOrgLayout,
  hasWorkHierarchy,
  type OrgCard,
  type OrgLayout,
} from './org-layout.js'
import {
  injectStylesOnce,
  type GenealogyChart,
  type GenealogyChartInstance,
  type GenealogyContext,
} from './chart-registry.js'

/** Same readability thresholds as the timeline (52px card, 13px title). */
const MIN_READABLE_CARD_PX = 34
const TARGET_READABLE_CARD_PX = 46

const TITLE_CHARS = 22
const SUBTITLE_CHARS = 28

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text
}

function cardClass(card: OrgCard): string {
  const classes = ['org-card', `org-${card.kind}`]
  if (card.isFocus) classes.push('org-focus')
  return classes.join(' ')
}

/** Building glyph for organization cards (16×16, stroked). */
function buildingIcon(x: number, y: number): TemplateResult {
  return svg`
    <g class="org-icon" transform="translate(${x} ${y})" aria-hidden="true">
      <rect x="2" y="1.5" width="9" height="13" rx="1"></rect>
      <path d="M11 6.5h3v8h-3M4.5 4.5h1M7.5 4.5h1M4.5 7.5h1M7.5 7.5h1M4.5 10.5h1M7.5 10.5h1"></path>
    </g>
  `
}

function cardTemplate(
  card: OrgCard,
  titleOf: (path: string) => string,
  onActivate: (card: OrgCard) => void,
  onKeydown: (e: KeyboardEvent, card: OrgCard) => void
): TemplateResult {
  const left = card.x - card.w / 2
  const top = card.y - card.h / 2
  const also = (card.also ?? []).map(titleOf)
  const tooltip =
    card.kind === 'more'
      ? `${card.moreCount} more — open ${titleOf(card.target)} to see them`
      : [card.title, card.jobTitle, also.length ? `Also reports to ${also.join(', ')}` : '']
          .filter(Boolean)
          .join('\n')
  const label = card.kind === 'more' ? `Show ${card.moreCount} more under ${titleOf(card.target)}` : `Go to ${card.title}`

  if (card.kind === 'more') {
    return svg`
      <g class="${cardClass(card)}" role="link" tabindex="0" aria-label="${label}"
        @click=${() => onActivate(card)} @keydown=${(e: KeyboardEvent) => onKeydown(e, card)}>
        <title>${tooltip}</title>
        <rect x="${left}" y="${top}" width="${card.w}" height="${card.h}" rx="${card.h / 2}"></rect>
        <text class="org-more-text" x="${card.x}" y="${card.y + 4}" text-anchor="middle">${card.title}</text>
      </g>
    `
  }

  const isOrg = card.kind === 'organization'
  const textX = left + (isOrg ? 34 : 16)
  const hasSub = Boolean(card.jobTitle)
  return svg`
    <g class="${cardClass(card)}" role="link" tabindex="0" aria-label="${label}"
      @click=${() => onActivate(card)} @keydown=${(e: KeyboardEvent) => onKeydown(e, card)}>
      <title>${tooltip}</title>
      <rect class="org-card-bg" x="${left}" y="${top}" width="${card.w}" height="${card.h}" rx="8"></rect>
      <rect class="org-accent" x="${left}" y="${top}" width="5" height="${card.h}" rx="2.5"></rect>
      ${isOrg ? buildingIcon(left + 12, card.y - 8) : nothing}
      <text class="org-title" x="${textX}" y="${hasSub ? top + 22 : card.y + 4.5}">
        ${truncate(card.title, isOrg ? TITLE_CHARS - 2 : TITLE_CHARS)}
      </text>
      ${hasSub
        ? svg`<text class="org-subtitle" x="${textX}" y="${top + 39}">${truncate(card.jobTitle ?? '', SUBTITLE_CHARS)}</text>`
        : nothing}
      ${also.length > 0
        ? svg`<text class="org-also" x="${left + card.w - 8}" y="${top + 14}" text-anchor="end" aria-hidden="true">+${also.length}</text>`
        : nothing}
    </g>
  `
}

function chartTemplate(
  layout: OrgLayout,
  titleOf: (path: string) => string,
  handlers: {
    onActivate: (card: OrgCard) => void
    onKeydown: (e: KeyboardEvent, card: OrgCard) => void
    onZoomIn: () => void
    onZoomOut: () => void
    onReset: () => void
  }
): TemplateResult {
  return html`
    <svg
      viewBox="0 0 ${layout.width} ${layout.height}"
      preserveAspectRatio="xMidYMid meet"
      aria-label="Organization chart"
    >
      <g class="org-groups" aria-hidden="true">
        ${layout.groups.map(
          (g) => svg`<rect class="org-group" x="${g.x}" y="${g.y}" width="${g.w}" height="${g.h}" rx="10"></rect>`
        )}
      </g>
      <g class="org-links" aria-hidden="true">
        ${layout.links.map((link) => svg`<path class="org-link org-link-${link.kind}" d="${link.d}"></path>`)}
      </g>
      <g class="org-group-labels" aria-hidden="true">
        ${layout.groups.map(
          (g) => svg`<text class="org-group-label" x="${g.x + 12}" y="${g.y + 16}">${truncate(g.label, Math.max(6, Math.floor((g.w - 24) / 6.6)))}</text>`
        )}
      </g>
      <g class="org-cards">
        ${layout.cards.map((card) => cardTemplate(card, titleOf, handlers.onActivate, handlers.onKeydown))}
      </g>
    </svg>
    <div class="rel-graph-controls">
      <button type="button" aria-label="Zoom in" title="Zoom in" @click=${handlers.onZoomIn}>+</button>
      <button type="button" aria-label="Zoom out" title="Zoom out" @click=${handlers.onZoomOut}>−</button>
      <button type="button" aria-label="Reset view" title="Reset view" @click=${handlers.onReset}>⤢</button>
    </div>
  `
}

function mountOrgChart(container: HTMLElement, ctx: GenealogyContext): GenealogyChartInstance {
  injectStylesOnce(container.getRootNode(), 'mbr-genealogy-org', ORG_CSS)

  const canvas = document.createElement('div')
  canvas.className = 'org-canvas'
  container.appendChild(canvas)

  const tree = buildOrgTree(ctx.focusPath, ctx.notesByPath, ctx.registry)
  if (!tree) {
    const empty = document.createElement('p')
    empty.className = 'gen-empty'
    empty.textContent = 'No reporting or employment relationships to chart.'
    canvas.appendChild(empty)
    return { destroy: () => canvas.remove() }
  }

  const layout = computeOrgLayout(tree)
  const titleOf = (path: string) => nodeTitle(ctx.notesByPath.get(path)?.frontmatter ?? {}, path)
  let controller: SvgViewportController | null = null
  const go = (card: OrgCard) => {
    if (card.target !== ctx.focusPath) ctx.navigate(card.target)
  }
  const onActivate = (card: OrgCard) => {
    // A pan that started on a card must not navigate (see timeline-view).
    if (controller?.consumeDragFlag()) return
    go(card)
  }
  const onKeydown = (e: KeyboardEvent, card: OrgCard) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      go(card)
    }
  }

  render(
    chartTemplate(layout, titleOf, {
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
    const rect = canvas.getBoundingClientRect()
    const focus = layout.cards.find((c) => c.isFocus)
    const initialView = computeInitialViewBox({
      contentWidth: layout.width,
      contentHeight: layout.height,
      canvasWidth: rect.width,
      canvasHeight: rect.height,
      cardH: ORG_CARD_H,
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

export const orgChartType: GenealogyChart = {
  id: 'org-chart',
  label: 'Org chart',
  isApplicable: (ctx) => hasWorkHierarchy(ctx.focusPath, ctx.notesByPath, ctx.registry),
  mount: mountOrgChart,
}

/**
 * Org chart styles. Colors come from Pico variables (so every theme and both
 * color schemes work) plus the shared `--mbr-gen-focus*` props from the base
 * stylesheet; `--mbr-org-*` are defined there too, with dark overrides.
 */
const ORG_CSS = `
.org-canvas {
  position: relative;
  height: 100%;
  overflow: hidden;
  cursor: grab;
  touch-action: none;
  border-radius: 4px;
}

.org-canvas svg {
  width: 100%;
  height: 100%;
  display: block;
}

.org-group {
  fill: var(--mbr-org-group-fill);
  stroke: var(--mbr-org-group-stroke);
  stroke-width: 1;
}

.org-group-label {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  fill: var(--pico-muted-color, #6b7280);
}

.org-link {
  fill: none;
  stroke: var(--mbr-org-link);
  stroke-width: 1.5;
  stroke-linejoin: round;
}

.org-link-assistant {
  stroke-dasharray: 2 4;
  stroke-linecap: round;
}

.org-link-secondary {
  stroke-dasharray: 6 5;
  opacity: 0.7;
}

.org-card {
  cursor: pointer;
}

.org-card .org-card-bg {
  fill: var(--pico-card-background-color, #fff);
  stroke: var(--mbr-org-card-stroke);
  stroke-width: 1;
  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.12));
}

.org-card:hover .org-card-bg {
  stroke: var(--pico-primary, #0172ad);
}

.org-card:focus-visible {
  outline: none;
}

.org-card:focus-visible .org-card-bg,
.org-more:focus-visible rect {
  stroke: var(--pico-primary, #0172ad);
  stroke-width: 2.5;
}

.org-accent {
  fill: var(--mbr-org-accent);
}

.org-organization .org-card-bg {
  fill: var(--mbr-org-org-fill);
}

.org-organization .org-accent {
  fill: var(--mbr-org-org-accent);
}

.org-icon rect,
.org-icon path {
  fill: none;
  stroke: var(--mbr-org-org-accent);
  stroke-width: 1.3;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.org-focus .org-card-bg {
  fill: var(--mbr-gen-focus-fill, #ffe0b2);
  stroke: var(--mbr-gen-focus, #e65100);
  stroke-width: 2;
}

.org-focus .org-accent {
  fill: var(--mbr-gen-focus, #e65100);
}

.org-title {
  font-size: 13px;
  font-weight: 600;
  fill: var(--pico-color, #1f2937);
}

.org-subtitle {
  font-size: 11px;
  fill: var(--pico-muted-color, #6b7280);
}

.org-also {
  font-size: 10px;
  font-weight: 600;
  fill: var(--pico-muted-color, #6b7280);
}

.org-more rect {
  fill: transparent;
  stroke: var(--mbr-org-card-stroke);
  stroke-width: 1.25;
  stroke-dasharray: 4 3;
}

.org-more:hover rect {
  stroke: var(--pico-primary, #0172ad);
}

.org-more-text {
  font-size: 12px;
  font-weight: 600;
  fill: var(--pico-muted-color, #6b7280);
}
`
