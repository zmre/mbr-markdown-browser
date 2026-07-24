/**
 * Pure SVG viewport (zoom / pan) math shared by the graph visualizations.
 *
 * All functions operate on plain `ViewBox` values and have no DOM dependencies,
 * so they are unit-testable and safe to bundle into any chunk. The stateful
 * event wiring lives in `viewport-controller.ts`.
 */

/** An SVG `viewBox` as separate numbers. */
export interface ViewBox {
  x: number
  y: number
  w: number
  h: number
}

/** Wheel zoom sensitivity: `factor = exp(-deltaY * sens)`. */
export const ZOOM_WHEEL_SENS = 0.001
/** Per-click zoom step for the +/- buttons. */
export const ZOOM_BUTTON_FACTOR = 1.3
/**
 * Scale bounds relative to the initial "fit" viewBox. `minScale = 1` means the
 * fit is the most zoomed-OUT state (you cannot zoom out past fit); `maxScale`
 * caps zoom-in at 8×.
 */
export const MIN_SCALE = 1
export const MAX_SCALE = 8
/** Pointer travel (px) above which a gesture is a pan/drag, not a click. */
export const DRAG_THRESHOLD_PX = 4

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
