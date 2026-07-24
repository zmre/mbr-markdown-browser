/**
 * Stateful zoom/pan/pinch controller for an SVG inside a fixed-size canvas.
 *
 * Mechanical lift of the private viewport methods formerly on the removed
 * mermaid relationships element, so any graph view (timeline chart, mini
 * force graph) can reuse the same interaction wiring:
 * cursor-centered wheel zoom, click-vs-drag panning with a travel threshold,
 * two-finger pinch zoom, double-click reset, and +/−/reset buttons.
 *
 * The controller owns the SVG's `viewBox` from construction until `destroy()`.
 * Construct it once per rendered SVG: the constructor normalizes the SVG's
 * inline sizing, captures the fit ("base") viewBox — falling back to the
 * content bbox — and binds wheel/pointer listeners to the canvas (torn down via
 * an AbortController in `destroy()`). Events originating inside a
 * `.rel-graph-controls` element are ignored so control buttons never start a
 * gesture.
 */
import {
  DRAG_THRESHOLD_PX,
  MAX_SCALE,
  MIN_SCALE,
  ZOOM_BUTTON_FACTOR,
  ZOOM_WHEEL_SENS,
  clientPointToSvg,
  formatViewBox,
  panViewBox,
  parseViewBox,
  zoomViewBoxAtPoint,
  type ViewBox,
} from './viewport.js'

/** Elements matching this selector never start a zoom/pan gesture. */
const CONTROLS_SELECTOR = '.rel-graph-controls'

/** Optional controller behavior overrides. */
export interface SvgViewportOptions {
  /**
   * Start (and reset) at this viewBox instead of the fit ("base") viewBox.
   * The base viewBox still comes from the SVG itself and remains the
   * zoom-out floor (`MIN_SCALE` is relative to it), so the full fit-all view
   * stays reachable by manual zoom-out; only the initial and `reset()` view
   * change. Used for the "smart initial zoom" on very wide/shallow charts.
   */
  initialView?: ViewBox
}

export class SvgViewportController {
  private readonly _svg: SVGSVGElement
  /** The fit ("base") viewBox and the current (zoomed/panned) viewBox. */
  private _baseViewBox: ViewBox | null = null
  private _viewBox: ViewBox | null = null
  /** The view `reset()` returns to: `initialView` when given, else the base. */
  private _homeViewBox: ViewBox | null = null
  /** Aborts the canvas wheel/pointer listeners on `destroy()`. */
  private readonly _listeners = new AbortController()
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

  constructor(canvas: HTMLElement, svg: SVGSVGElement, opts?: SvgViewportOptions) {
    this._svg = svg

    // Strip any inline sizing (e.g. mermaid's) so the SVG fills the
    // fixed-height canvas.
    svg.style.maxWidth = 'none'
    svg.style.width = '100%'
    svg.style.height = '100%'
    svg.style.display = 'block'
    svg.removeAttribute('width')
    svg.removeAttribute('height')

    // Fit: capture the base viewBox, then start at (and reset to) the "home"
    // view — the caller-provided initial view when given, else the base.
    const base = parseViewBox(svg.getAttribute('viewBox')) ?? this._bboxViewBox(svg)
    if (base) {
      this._baseViewBox = base
      this._homeViewBox = { ...(opts?.initialView ?? base) }
      this._viewBox = { ...this._homeViewBox }
      this._applyViewBox()
    }

    this._bindViewportListeners(canvas, this._listeners.signal)
  }

  /** Zoom in one button step, centered on the viewBox center. */
  zoomIn(): void {
    this._zoomByButton(ZOOM_BUTTON_FACTOR)
  }

  /** Zoom out one button step, centered on the viewBox center. */
  zoomOut(): void {
    this._zoomByButton(1 / ZOOM_BUTTON_FACTOR)
  }

  /** Reset to the home viewBox (`initialView` when configured, else fit). */
  reset(): void {
    this._resetView()
  }

  /**
   * Returns whether the last gesture was a drag (pan past the travel
   * threshold) and clears the flag, so a click handler can suppress navigation
   * exactly once after a pan that started on a clickable node.
   */
  consumeDragFlag(): boolean {
    const wasDragging = this._wasDragging
    this._wasDragging = false
    return wasDragging
  }

  /** Remove all listeners. The controller must not be used afterwards. */
  destroy(): void {
    this._listeners.abort()
  }

  // Internal wiring ----------------------------------------------------------

  private get _scaleOpts(): { minScale: number; maxScale: number; baseW: number } {
    return { minScale: MIN_SCALE, maxScale: MAX_SCALE, baseW: this._baseViewBox?.w ?? 1 }
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
    if (this._viewBox) {
      this._svg.setAttribute('viewBox', formatViewBox(this._viewBox))
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
        if ((e.target as Element | null)?.closest(CONTROLS_SELECTOR)) return
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
    if ((e.target as Element | null)?.closest(CONTROLS_SELECTOR)) return
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
    const home = this._homeViewBox ?? this._baseViewBox
    if (!home) return
    this._viewBox = { ...home }
    this._applyViewBox()
  }
}
