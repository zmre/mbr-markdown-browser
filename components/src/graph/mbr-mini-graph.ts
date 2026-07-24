/**
 * `<mbr-mini-graph>` — a mini force-directed graph of the current note's link
 * neighborhood, shown in the info sidebar, with an expandable modal view.
 *
 * CHUNK ONLY: this element ships in the lazy `mbr-graph.min.js` chunk and must
 * never import stateful main-bundle modules (`shared.ts`, `links-cache.ts`).
 * All services arrive injected as properties from the trigger side
 * (`mbr-info.ts`): `.fetchLinks`, `.isKnownNote`, `.getMeta`, `.resolveHref`.
 *
 * Rendering strategy: Lit `svg` templates build the skeleton (circles/lines/
 * labels); per-tick position updates are applied imperatively on cached
 * element refs — no d3-selection. Node colors encode BFS degree as a
 * sequential single-hue ramp (Pico primary fading toward the background).
 */
import { LitElement, html, svg, css, nothing, type PropertyValues, type TemplateResult } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import {
  forceCenter,
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  type ForceLink,
  type Simulation,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from 'd3-force'
import { DEFAULT_MAX_NODES } from './relationship-graph.js'
import { filterToDepth, type MiniGraph, type MiniGraphNode } from './build.js'
import { expandNeighborhood, type FetchPageLinks } from './bfs.js'
import { DRAG_THRESHOLD_PX, clientPointToSvg, parseViewBox } from './viewport.js'
import { SvgViewportController } from './viewport-controller.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-mini-graph': MbrMiniGraphElement
  }
}

/** Title/description supplied by the injected `getMeta` service. */
export interface NoteMeta {
  title: string
  description?: string
}

interface SimNode extends SimulationNodeDatum {
  id: string
  degree: number
}

type SimLink = SimulationLinkDatum<SimNode>

/** Mini canvas coordinate space (rendered full-width, ~200px tall). */
const MINI_W = 400
const MINI_H = 200
/** Expanded modal coordinate space (pan/zoomable). */
const EXPANDED_W = 800
const EXPANDED_H = 600

/**
 * Force tuning per view. The expanded canvas is 4× wider and 3× taller than the
 * mini one, so it needs a proportionally larger link distance and stronger
 * charge to spread nodes across the available space (otherwise the graph stays
 * clumped in the middle like the mini view). The larger collision radius also
 * reserves room for each node's text label — shown only when expanded — so the
 * labels stop overlapping on densely-linked pages.
 */
const MINI_FORCES = { linkDistance: 30, charge: -40, collidePad: 2 }
const EXPANDED_FORCES = { linkDistance: 70, charge: -200, collidePad: 16 }

/** Depth stepper bounds (mirror the Rust `graph_depth` config range). */
const DEPTH_MIN = 1
const DEPTH_MAX = 5

/** Grace period before hiding the hover card (pointer travel time). */
const HOVER_HIDE_DELAY_MS = 120
/** Gap between a node and its hover card, and the viewport clamp margin. */
const HOVER_GAP_PX = 8

/** Synchronous tick count for the reduced-motion / test static layout. */
const STATIC_TICKS = 150

function nodeRadius(node: { degree: number }): number {
  return node.degree === 0 ? 7 : 4.5
}

function clampDepth(value: number): number {
  const floored = Math.floor(value)
  if (!Number.isFinite(floored)) return DEPTH_MIN
  return Math.max(DEPTH_MIN, Math.min(DEPTH_MAX, floored))
}

/** Fallback display title: the last path segment. */
function lastSegment(path: string): string {
  return path.split('/').filter(Boolean).pop() ?? path
}

/** A resolved (post-initialization) link endpoint. */
function endpoint(value: SimNode | string | number): SimNode | null {
  return typeof value === 'object' ? value : null
}

@customElement('mbr-mini-graph')
export class MbrMiniGraphElement extends LitElement {
  /**
   * The note whose neighborhood is graphed (canonicalized internally).
   * Named `focusPath` because `focus` would shadow `HTMLElement.focus()`
   * (breaking keyboard a11y); the attribute is still `focus`.
   */
  @property({ type: String, attribute: 'focus' })
  focusPath = ''

  /** BFS depth for the mini view; also seeds the expanded depth stepper. */
  @property({ type: Number })
  depth = 2

  /** Safety cap on graph size. */
  @property({ type: Number, attribute: 'max-nodes' })
  maxNodes = DEFAULT_MAX_NODES

  /**
   * Test/reduced-motion hook: lay out synchronously (stop + tick) instead of
   * animating with the simulation timer (which needs rAF).
   */
  @property({ type: Boolean, attribute: 'static-layout' })
  staticLayout = false

  // Injected services (set as properties by the trigger in the main bundle).
  @property({ attribute: false })
  fetchLinks?: FetchPageLinks

  @property({ attribute: false })
  isKnownNote?: (path: string) => boolean

  @property({ attribute: false })
  getMeta?: (path: string) => NoteMeta | undefined

  @property({ attribute: false })
  resolveHref?: (path: string) => string

  /** Latest BFS output (deepest graph built so far). */
  @state()
  private _fullGraph: MiniGraph | null = null

  /** Whether the expanded modal is open. */
  @state()
  private _expanded = false

  /** Expanded-view depth stepper value (seeded from `depth`). */
  @state()
  private _stepperDepth = 2

  /** Depth the current `_fullGraph` was built at (for step-up vs step-down). */
  private _builtDepth = 0

  // Simulation state -----------------------------------------------------
  private _simulation: Simulation<SimNode, undefined> | null = null
  private _linkForce: ForceLink<SimNode, SimLink> | null = null
  private _simNodes: SimNode[] = []
  /** Accumulates positions across snapshots so re-added nodes keep places. */
  private _simNodeById = new Map<string, SimNode>()
  /** Signature of the last synced (nodes, links, canvas) state. */
  private _syncedSignature = ''

  // Cached DOM refs for imperative per-tick updates ---------------------
  private _circleRefs = new Map<string, SVGCircleElement>()
  private _labelRefs = new Map<string, SVGTextElement>()
  private _lineRefs = new Map<string, SVGLineElement>()

  // Interaction state ----------------------------------------------------
  private _bfsAbort: AbortController | null = null
  private _modalViewport: SvgViewportController | null = null
  private _hoverEnabled = false
  private _hoverHideTimer: number | undefined
  private _drag: {
    id: string
    pointerId: number | undefined
    startX: number
    startY: number
    dragging: boolean
  } | null = null
  private _suppressNextClick = false

  override connectedCallback() {
    super.connectedCallback()
    // Hover cards are desktop-only; on touch they would fight tap-to-navigate.
    const mq = window.matchMedia?.('(hover: hover) and (pointer: fine)')
    this._hoverEnabled = mq?.matches ?? false
  }

  override disconnectedCallback() {
    super.disconnectedCallback()
    this._bfsAbort?.abort()
    this._bfsAbort = null
    this._simulation?.stop()
    this._modalViewport?.destroy()
    this._modalViewport = null
    document.removeEventListener('keydown', this._onEscapeCapture, true)
    document.removeEventListener('pointermove', this._onDocPointerMove)
    document.removeEventListener('pointerup', this._onDocPointerUp)
    document.removeEventListener('pointercancel', this._onDocPointerUp)
    this._cancelHoverHide()
  }

  // =====================================================================
  // BFS orchestration
  // =====================================================================

  override willUpdate(changed: PropertyValues) {
    if (
      changed.has('focusPath') ||
      changed.has('depth') ||
      changed.has('maxNodes') ||
      changed.has('fetchLinks') ||
      changed.has('isKnownNote')
    ) {
      this._restartBfs()
    }
  }

  private _restartBfs(): void {
    if (!this.focusPath || !this.fetchLinks || !this.isKnownNote) return
    this._fullGraph = null
    this._builtDepth = 0
    this._stepperDepth = clampDepth(this.depth)
    this._runBfs(clampDepth(this.depth))
  }

  private _runBfs(depth: number): void {
    const fetchLinks = this.fetchLinks
    const isKnownNote = this.isKnownNote
    if (!this.focusPath || !fetchLinks || !isKnownNote) return

    this._bfsAbort?.abort()
    const controller = new AbortController()
    this._bfsAbort = controller

    void expandNeighborhood({
      focus: this.focusPath,
      depth,
      maxNodes: this.maxNodes,
      fetchLinks,
      isKnownNote,
      signal: controller.signal,
      onUpdate: (graph) => {
        if (controller.signal.aborted) return
        this._fullGraph = graph
      },
    }).then((graph) => {
      if (controller.signal.aborted) return
      if (graph) {
        this._fullGraph = graph
        this._builtDepth = Math.max(this._builtDepth, depth)
      }
    })
  }

  // =====================================================================
  // Displayed graph + simulation sync
  // =====================================================================

  private _activeDepth(): number {
    return this._expanded ? this._stepperDepth : clampDepth(this.depth)
  }

  private _displayedGraph(): MiniGraph | null {
    if (!this._fullGraph) return null
    return filterToDepth(this._fullGraph, this._activeDepth())
  }

  private _bounds(): { w: number; h: number } {
    return this._expanded ? { w: EXPANDED_W, h: EXPANDED_H } : { w: MINI_W, h: MINI_H }
  }

  private _isStatic(): boolean {
    if (this.staticLayout) return true
    return window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches ?? false
  }

  private _ensureSimulation(): Simulation<SimNode, undefined> {
    if (this._simulation) return this._simulation
    const link = forceLink<SimNode, SimLink>([])
      .id((d) => d.id)
      .distance(MINI_FORCES.linkDistance)
    const sim = forceSimulation<SimNode>([])
      .force('link', link)
      .force('charge', forceManyBody<SimNode>().strength(MINI_FORCES.charge))
      .force('center', forceCenter<SimNode>(MINI_W / 2, MINI_H / 2))
      .force('collide', forceCollide<SimNode>((d) => nodeRadius(d) + MINI_FORCES.collidePad))
      .force('x', forceX<SimNode>(MINI_W / 2).strength(0.04))
      .force('y', forceY<SimNode>(MINI_H / 2).strength(0.04))
    sim.stop() // We control (re)starts explicitly.
    sim.on('tick', () => {
      this._clampPositions()
      this._applyPositions()
    })
    this._simulation = sim
    this._linkForce = link
    return sim
  }

  /** Seed a new node at its discovering parent's position, plus jitter. */
  private _seedPosition(id: string, graph: MiniGraph): { x: number; y: number } {
    const { w, h } = this._bounds()
    const jitter = () => (Math.random() - 0.5) * 16
    for (const link of graph.links) {
      const other = link.source === id ? link.target : link.target === id ? link.source : null
      if (!other) continue
      const parent = this._simNodeById.get(other)
      if (parent && parent.x != null && parent.y != null) {
        return { x: parent.x + jitter(), y: parent.y + jitter() }
      }
    }
    return { x: w / 2 + jitter(), y: h / 2 + jitter() }
  }

  /**
   * Sync the running simulation with the displayed graph: existing nodes keep
   * their positions, new nodes are seeded near their discovering parent and
   * fed into the RUNNING simulation with a reheat (`alpha(0.5).restart()`),
   * or laid out synchronously in static mode.
   */
  private _syncSimulation(displayed: MiniGraph): void {
    const { w, h } = this._bounds()
    const signature = [
      this._expanded ? 'x' : 'm',
      displayed.nodes.map((n) => n.id).join(','),
      displayed.links.map((l) => `${l.source}|${l.target}`).join(','),
    ].join('#')
    if (signature === this._syncedSignature) return
    this._syncedSignature = signature

    const sim = this._ensureSimulation()
    const nodes: SimNode[] = displayed.nodes.map((node) => {
      const existing = this._simNodeById.get(node.id)
      if (existing) {
        existing.degree = node.degree
        return existing
      }
      const seed = this._seedPosition(node.id, displayed)
      const created: SimNode = { id: node.id, degree: node.degree, x: seed.x, y: seed.y }
      this._simNodeById.set(node.id, created)
      return created
    })
    this._simNodes = nodes

    // Re-target the centering forces at the active canvas, and scale the link /
    // charge / collision forces to the active view: the expanded canvas needs
    // more spread so nodes fill the larger space and their labels stop
    // overlapping, instead of staying clumped like the mini view.
    const forces = this._expanded ? EXPANDED_FORCES : MINI_FORCES
    this._linkForce?.distance(forces.linkDistance)
    sim.force('charge', forceManyBody<SimNode>().strength(forces.charge))
    sim.force('collide', forceCollide<SimNode>((d) => nodeRadius(d) + forces.collidePad))
    sim.force('center', forceCenter<SimNode>(w / 2, h / 2))
    sim.force('x', forceX<SimNode>(w / 2).strength(0.04))
    sim.force('y', forceY<SimNode>(h / 2).strength(0.04))

    sim.nodes(nodes)
    // Fresh link copies each sync: d3 mutates source/target into node refs.
    this._linkForce?.links(displayed.links.map((l) => ({ source: l.source, target: l.target })))

    if (this._isStatic()) {
      sim.stop()
      sim.alpha(1)
      sim.tick(STATIC_TICKS)
      this._clampPositions()
      this._applyPositions()
    } else {
      sim.alpha(0.5).restart()
    }
  }

  private _clampPositions(): void {
    const { w, h } = this._bounds()
    for (const node of this._simNodes) {
      const r = nodeRadius(node)
      if (node.x != null) node.x = Math.max(r, Math.min(w - r, node.x))
      if (node.y != null) node.y = Math.max(r, Math.min(h - r, node.y))
    }
  }

  /** Write current simulation positions onto the cached SVG element refs. */
  private _applyPositions(): void {
    for (const node of this._simNodes) {
      if (node.x == null || node.y == null) continue
      const circle = this._circleRefs.get(node.id)
      if (circle) {
        circle.setAttribute('cx', String(node.x))
        circle.setAttribute('cy', String(node.y))
      }
      const label = this._labelRefs.get(node.id)
      if (label) {
        label.setAttribute('x', String(node.x))
        label.setAttribute('y', String(node.y + nodeRadius(node) + 11))
      }
    }
    const links = this._linkForce?.links() ?? []
    for (const link of links) {
      const source = endpoint(link.source)
      const target = endpoint(link.target)
      if (!source || !target) continue
      const line = this._lineRefs.get(`${source.id}|${target.id}`)
      if (!line || source.x == null || source.y == null || target.x == null || target.y == null) {
        continue
      }
      line.setAttribute('x1', String(source.x))
      line.setAttribute('y1', String(source.y))
      line.setAttribute('x2', String(target.x))
      line.setAttribute('y2', String(target.y))
    }
  }

  /** Re-cache SVG refs after each render (the skeleton is Lit-rendered). */
  private _cacheRefs(): void {
    const root = this.shadowRoot
    if (!root) return
    this._circleRefs = new Map()
    this._labelRefs = new Map()
    this._lineRefs = new Map()
    root.querySelectorAll<SVGCircleElement>('circle[data-id]').forEach((el) => {
      this._circleRefs.set(el.getAttribute('data-id') ?? '', el)
    })
    root.querySelectorAll<SVGTextElement>('text[data-label-for]').forEach((el) => {
      this._labelRefs.set(el.getAttribute('data-label-for') ?? '', el)
    })
    root.querySelectorAll<SVGLineElement>('line[data-source]').forEach((el) => {
      this._lineRefs.set(`${el.getAttribute('data-source')}|${el.getAttribute('data-target')}`, el)
    })
  }

  override updated(changed: PropertyValues) {
    const displayed = this._displayedGraph()
    this._cacheRefs()
    if (displayed && displayed.nodes.length >= 2) {
      this._syncSimulation(displayed)
      this._applyPositions()
    }
    // Wire the expanded canvas's pan/zoom controller when the modal appears.
    if (changed.has('_expanded')) {
      this._modalViewport?.destroy()
      this._modalViewport = null
      if (this._expanded) {
        const dialog = this.shadowRoot?.querySelector<HTMLDialogElement>('dialog.graph-modal')
        if (dialog && !dialog.open) {
          // showModal() promotes the dialog into the top layer, so it floats
          // above the info drawer regardless of the drawer's transform/overflow.
          // Guarded: a webview lacking showModal() still opens non-modally.
          try {
            dialog.showModal()
          } catch {
            dialog.setAttribute('open', '')
          }
        }
        const canvas = this.shadowRoot?.querySelector<HTMLElement>('.graph-modal-canvas')
        const svgEl = canvas?.querySelector('svg')
        if (canvas && svgEl instanceof SVGSVGElement) {
          this._modalViewport = new SvgViewportController(canvas, svgEl)
        }
      }
    }
  }

  // =====================================================================
  // Node interaction: click / keyboard nav, manual drag
  // =====================================================================

  private _titleFor(id: string): string {
    return this.getMeta?.(id)?.title ?? lastSegment(id)
  }

  private _navigate(id: string): void {
    const href = this.resolveHref?.(id) ?? id
    window.location.assign(href)
  }

  private _onNodeClick(node: MiniGraphNode): void {
    // A drag that started on this node must not navigate; consuming the flag
    // clears it so the next genuine click works again.
    if (this._suppressNextClick) {
      this._suppressNextClick = false
      return
    }
    this._navigate(node.id)
  }

  private _onNodeKeydown(e: KeyboardEvent, node: MiniGraphNode): void {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      this._navigate(node.id)
    }
  }

  private _onNodePointerDown(e: PointerEvent, node: MiniGraphNode): void {
    if (e.button !== 0) return
    // Keep the modal's pan controller from treating this as a canvas pan.
    e.stopPropagation()
    this._drag = {
      id: node.id,
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      dragging: false,
    }
    document.addEventListener('pointermove', this._onDocPointerMove)
    document.addEventListener('pointerup', this._onDocPointerUp)
    document.addEventListener('pointercancel', this._onDocPointerUp)
  }

  private _onDocPointerMove = (e: PointerEvent): void => {
    const drag = this._drag
    if (!drag || drag.pointerId !== e.pointerId) return
    const travel = Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY)
    if (!drag.dragging) {
      if (travel <= DRAG_THRESHOLD_PX) return
      drag.dragging = true
      this._hideHoverCard()
      if (!this._isStatic()) {
        // Standard d3 drag recipe: reheat while pinned.
        this._ensureSimulation().alphaTarget(0.3).restart()
      }
    }
    const node = this._simNodeById.get(drag.id)
    const svgEl = this.shadowRoot?.querySelector<SVGSVGElement>('svg.graph-svg')
    if (!node || !svgEl) return
    const viewBox = parseViewBox(svgEl.getAttribute('viewBox'))
    if (!viewBox) return
    const rect = svgEl.getBoundingClientRect()
    const point = clientPointToSvg(e.clientX, e.clientY, rect, viewBox)
    node.fx = point.x
    node.fy = point.y
    if (this._isStatic()) {
      node.x = point.x
      node.y = point.y
      this._applyPositions()
    }
  }

  private _onDocPointerUp = (e: PointerEvent): void => {
    const drag = this._drag
    if (!drag || drag.pointerId !== e.pointerId) return
    this._drag = null
    document.removeEventListener('pointermove', this._onDocPointerMove)
    document.removeEventListener('pointerup', this._onDocPointerUp)
    document.removeEventListener('pointercancel', this._onDocPointerUp)
    if (drag.dragging) {
      // Only a real drag suppresses the ensuing click.
      this._suppressNextClick = true
      const node = this._simNodeById.get(drag.id)
      if (node) {
        node.fx = null
        node.fy = null
      }
      if (!this._isStatic()) this._simulation?.alphaTarget(0)
    }
  }

  // =====================================================================
  // Hover card (desktop only)
  // =====================================================================

  private _hoverCard(): HTMLDivElement | null {
    return this.shadowRoot?.querySelector<HTMLDivElement>('.hover-card') ?? null
  }

  private _onNodeHover(e: MouseEvent, node: MiniGraphNode): void {
    if (!this._hoverEnabled || this._drag?.dragging) return
    this._cancelHoverHide()
    const card = this._hoverCard()
    const target = e.currentTarget as Element | null
    if (!card || !target) return

    const meta = this.getMeta?.(node.id)
    const title = document.createElement('div')
    title.className = 'hover-title'
    title.textContent = meta?.title ?? lastSegment(node.id)
    card.replaceChildren(title)
    if (meta?.description) {
      const desc = document.createElement('div')
      desc.className = 'hover-desc'
      desc.textContent = meta.description
      card.appendChild(desc)
    }
    card.style.display = 'block'

    // Position near the node, clamped to the viewport (footnote-card pattern).
    const rect = target.getBoundingClientRect()
    const vw = window.innerWidth
    const vh = window.innerHeight
    card.style.left = '0px'
    card.style.top = '0px'
    const pop = card.getBoundingClientRect()
    let left = rect.left + rect.width / 2 - pop.width / 2
    left = Math.max(HOVER_GAP_PX, Math.min(left, vw - pop.width - HOVER_GAP_PX))
    let top = rect.top - pop.height - HOVER_GAP_PX
    if (top < HOVER_GAP_PX) top = rect.bottom + HOVER_GAP_PX
    top = Math.max(HOVER_GAP_PX, Math.min(top, vh - pop.height - HOVER_GAP_PX))
    card.style.left = `${left}px`
    card.style.top = `${top}px`
  }

  private _scheduleHoverHide(): void {
    this._cancelHoverHide()
    this._hoverHideTimer = window.setTimeout(() => this._hideHoverCard(), HOVER_HIDE_DELAY_MS)
  }

  private _cancelHoverHide(): void {
    if (this._hoverHideTimer !== undefined) {
      clearTimeout(this._hoverHideTimer)
      this._hoverHideTimer = undefined
    }
  }

  private _hideHoverCard(): void {
    this._cancelHoverHide()
    const card = this._hoverCard()
    if (card) card.style.display = 'none'
  }

  // =====================================================================
  // Expanded modal
  // =====================================================================

  private _openModal(): void {
    if (this._expanded) return
    this._expanded = true
    this._hideHoverCard()
    // Capture phase so Escape closes ONLY this modal: mbr-info's own bubbling
    // Escape handler would otherwise also close the surrounding drawer.
    document.addEventListener('keydown', this._onEscapeCapture, true)
  }

  private _closeModal(): void {
    if (!this._expanded) return
    this._expanded = false
    document.removeEventListener('keydown', this._onEscapeCapture, true)
  }

  private _onEscapeCapture = (e: KeyboardEvent): void => {
    if (e.key !== 'Escape' || !this._expanded) return
    e.stopImmediatePropagation()
    e.preventDefault()
    this._closeModal()
  }

  private _stepDepth(delta: number): void {
    const next = clampDepth(this._stepperDepth + delta)
    if (next === this._stepperDepth) return
    this._stepperDepth = next
    // Step UP beyond what's built resumes the BFS (the caching fetcher makes
    // already-fetched levels free). Step DOWN is a pure filter — no refetch.
    if (next > this._builtDepth) this._runBfs(next)
  }

  // =====================================================================
  // Templates
  // =====================================================================

  private _renderNodes(graph: MiniGraph, withLabels: boolean): TemplateResult[] {
    const parts: TemplateResult[] = graph.links.map(
      (link) => svg`<line
        class="graph-link"
        data-source=${link.source}
        data-target=${link.target}
      ></line>`
    )
    for (const node of graph.nodes) {
      parts.push(
        svg`<circle
          class="graph-node deg-${Math.min(node.degree, 5)}"
          data-id=${node.id}
          r=${nodeRadius(node)}
          role="link"
          tabindex="0"
          aria-label=${`Go to ${this._titleFor(node.id)}`}
          @pointerdown=${(e: PointerEvent) => this._onNodePointerDown(e, node)}
          @click=${() => this._onNodeClick(node)}
          @keydown=${(e: KeyboardEvent) => this._onNodeKeydown(e, node)}
          @mouseenter=${(e: MouseEvent) => this._onNodeHover(e, node)}
          @mouseleave=${() => this._scheduleHoverHide()}
        ></circle>`
      )
      if (withLabels) {
        parts.push(
          svg`<text
            class="node-label"
            data-label-for=${node.id}
            text-anchor="middle"
          >${this._titleFor(node.id)}</text>`
        )
      }
    }
    return parts
  }

  private _renderTruncationBadge(graph: MiniGraph): TemplateResult | typeof nothing {
    if (!graph.truncated) return nothing
    return html`<span
      class="truncation-badge"
      title="Graph capped at ${this.maxNodes} notes; some connections are not shown"
      >capped</span
    >`
  }

  private _renderMini(graph: MiniGraph): TemplateResult {
    return html`
      <div class="mini-canvas" aria-label="Link graph">
        <svg
          class="graph-svg"
          viewBox="0 0 ${MINI_W} ${MINI_H}"
          preserveAspectRatio="xMidYMid meet"
          aria-hidden="false"
        >
          ${this._renderNodes(graph, false)}
        </svg>
        <button
          type="button"
          class="expand-btn"
          aria-label="Expand graph"
          title="Expand graph"
          @click=${() => this._openModal()}
        >
          ⤢
        </button>
        ${this._renderTruncationBadge(graph)}
      </div>
    `
  }

  private _renderHoverCard(): TemplateResult {
    return html`
      <div
        class="hover-card"
        role="tooltip"
        style="display:none"
        @mouseenter=${() => this._cancelHoverHide()}
        @mouseleave=${() => this._scheduleHoverHide()}
      ></div>
    `
  }

  private _renderModal(graph: MiniGraph): TemplateResult {
    return html`
      <dialog
        class="graph-modal"
        aria-label="Link graph"
        @click=${(e: MouseEvent) => {
          if (e.target === e.currentTarget) this._closeModal()
        }}
        @cancel=${(e: Event) => {
          e.preventDefault()
          this._closeModal()
        }}
      >
        <div class="graph-modal-body">
          <div class="graph-modal-header">
            <h2>Link graph</h2>
            <div class="depth-stepper rel-graph-controls" role="group" aria-label="Graph depth">
              <button
                type="button"
                aria-label="Decrease depth"
                ?disabled=${this._stepperDepth <= DEPTH_MIN}
                @click=${() => this._stepDepth(-1)}
              >
                −
              </button>
              <span class="depth-value" aria-live="polite">${this._stepperDepth}</span>
              <button
                type="button"
                aria-label="Increase depth"
                ?disabled=${this._stepperDepth >= DEPTH_MAX}
                @click=${() => this._stepDepth(1)}
              >
                +
              </button>
            </div>
            <button
              type="button"
              class="graph-modal-close"
              aria-label="Close graph"
              @click=${() => this._closeModal()}
            >
              &times;
            </button>
          </div>
          <div class="graph-modal-canvas">
            <svg
              class="graph-svg"
              viewBox="0 0 ${EXPANDED_W} ${EXPANDED_H}"
              preserveAspectRatio="xMidYMid meet"
            >
              ${this._renderNodes(graph, true)}
            </svg>
            ${this._renderTruncationBadge(graph)}
          </div>
          ${this._renderHoverCard()}
        </div>
      </dialog>
    `
  }

  override render() {
    const graph = this._displayedGraph()
    if (!graph || graph.nodes.length < 2) return nothing
    // Hover card lives INSIDE the dialog when expanded so it shares the top
    // layer (a card in the normal layer would render behind the modal); in the
    // mini view it sits alongside the canvas.
    return this._expanded
      ? this._renderModal(graph)
      : html`${this._renderMini(graph)}${this._renderHoverCard()}`
  }

  // =====================================================================
  // Styles
  // =====================================================================

  static override styles = css`
    :host {
      display: block;
    }

    .mini-canvas {
      position: relative;
      width: 100%;
      height: 200px;
      margin-bottom: 1.5rem;
      border: 1px solid var(--pico-muted-border-color, #e0e0e0);
      border-radius: 4px;
      overflow: hidden;
    }

    .graph-svg {
      display: block;
      width: 100%;
      height: 100%;
    }

    .graph-link {
      stroke: var(--pico-muted-border-color, #ccc);
      stroke-width: 1;
    }

    .graph-node {
      cursor: pointer;
      stroke: var(--pico-background-color, #fff);
      stroke-width: 1;
    }

    .graph-node:focus-visible {
      outline: none;
      stroke: var(--pico-primary, #0172ad);
      stroke-width: 2;
    }

    /* Degree colors: a sequential single-hue ramp from the theme primary
       toward the background — closer to the focus = stronger. Static hex
       fallbacks (Pico default primary on white) precede each color-mix. */
    .graph-node.deg-0 {
      fill: #0172ad;
      fill: var(--pico-primary, #0172ad);
    }
    .graph-node.deg-1 {
      fill: #4d9cc6;
      fill: color-mix(in oklab, var(--pico-primary, #0172ad) 70%, var(--pico-background-color, #fff));
    }
    .graph-node.deg-2 {
      fill: #80b9d6;
      fill: color-mix(in oklab, var(--pico-primary, #0172ad) 50%, var(--pico-background-color, #fff));
    }
    .graph-node.deg-3 {
      fill: #a4cce2;
      fill: color-mix(in oklab, var(--pico-primary, #0172ad) 36%, var(--pico-background-color, #fff));
    }
    .graph-node.deg-4 {
      fill: #bddaea;
      fill: color-mix(in oklab, var(--pico-primary, #0172ad) 26%, var(--pico-background-color, #fff));
    }
    .graph-node.deg-5 {
      fill: #d1e6f0;
      fill: color-mix(in oklab, var(--pico-primary, #0172ad) 18%, var(--pico-background-color, #fff));
    }

    .node-label {
      font-size: 10px;
      fill: var(--pico-color, #333);
      pointer-events: none;
      user-select: none;
    }

    .expand-btn {
      position: absolute;
      top: 0.25rem;
      right: 0.25rem;
      width: 1.6rem;
      height: 1.6rem;
      padding: 0;
      display: flex;
      align-items: center;
      justify-content: center;
      cursor: pointer;
      border: none;
      border-radius: 4px;
      background: transparent;
      color: var(--pico-muted-color, #666);
      font-size: 0.9rem;
      line-height: 1;
    }

    .expand-btn:hover {
      background: var(--pico-secondary-hover-background, #f0f0f0);
      color: var(--pico-color, #333);
    }

    .truncation-badge {
      position: absolute;
      bottom: 0.25rem;
      left: 0.4rem;
      font-size: 0.7rem;
      color: var(--pico-muted-color, #666);
      background: var(--pico-background-color, #fff);
      border: 1px solid var(--pico-muted-border-color, #e0e0e0);
      border-radius: 3px;
      padding: 0 0.3rem;
    }

    /* Expanded modal: a native <dialog> promoted to the browser top layer via
       showModal(). The top layer is positioned relative to the viewport and
       renders above everything regardless of any ancestor transform / filter /
       overflow — so the modal floats over the whole page instead of being
       trapped and clipped inside the info drawer (whose slide-in transform
       makes it a containing block for fixed descendants). */
    .graph-modal {
      padding: 0;
      border: none;
      width: min(920px, 96vw);
      height: min(85vh, 760px);
      max-width: 96vw;
      max-height: 92vh;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #1a1a1a);
      border-radius: 8px;
      box-shadow: 0 12px 40px rgba(0, 0, 0, 0.35);
      overflow: hidden;
    }

    .graph-modal::backdrop {
      background: rgba(0, 0, 0, 0.5);
    }

    .graph-modal-body {
      display: flex;
      flex-direction: column;
      width: 100%;
      height: 100%;
    }

    .graph-modal-header {
      display: flex;
      align-items: center;
      gap: 1rem;
      padding: 0.6rem 1rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #e0e0e0);
    }

    .graph-modal-header h2 {
      margin: 0;
      font-size: 1rem;
      flex: 1;
    }

    .depth-stepper {
      display: flex;
      align-items: center;
      gap: 0.4rem;
    }

    .depth-stepper button {
      width: 1.6rem;
      height: 1.6rem;
      padding: 0;
      display: flex;
      align-items: center;
      justify-content: center;
      cursor: pointer;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      background: transparent;
      color: var(--pico-color, #333);
      line-height: 1;
    }

    .depth-stepper button:disabled {
      opacity: 0.4;
      cursor: default;
    }

    .depth-value {
      min-width: 1.2rem;
      text-align: center;
      font-variant-numeric: tabular-nums;
    }

    .graph-modal-close {
      background: transparent;
      border: none;
      font-size: 1.4rem;
      cursor: pointer;
      color: inherit;
      line-height: 1;
      padding: 0.25rem 0.5rem;
    }

    .graph-modal-canvas {
      position: relative;
      flex: 1;
      cursor: grab;
      overflow: hidden;
    }

    /* Hover card: single reused popover, viewport-positioned. */
    .hover-card {
      position: fixed;
      z-index: 2100;
      max-width: 280px;
      padding: 0.5rem 0.75rem;
      border: 1px solid var(--pico-muted-border-color, #e0e0e0);
      border-radius: 6px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
      box-shadow: 0 4px 16px rgba(0, 0, 0, 0.2);
      pointer-events: auto;
      font-size: 0.85rem;
    }

    .hover-card .hover-title {
      font-weight: 600;
    }

    .hover-card .hover-desc {
      margin-top: 0.25rem;
      color: var(--pico-muted-color, #666);
    }
  `
}
