/**
 * Unit tests for the pure SVG viewport (zoom / pan) math in `viewport.ts`.
 */
import { describe, it, expect } from 'vitest'
import {
  parseViewBox,
  formatViewBox,
  clampViewBoxScale,
  zoomViewBoxAtPoint,
  panViewBox,
  clientPointToSvg,
  type ViewBox,
} from './viewport.js'

describe('parseViewBox / formatViewBox', () => {
  it('parses a space-separated viewBox', () => {
    expect(parseViewBox('0 0 100 50')).toEqual({ x: 0, y: 0, w: 100, h: 50 })
  })

  it('parses a comma/whitespace-separated viewBox', () => {
    expect(parseViewBox(' 1, 2, 30 , 40 ')).toEqual({ x: 1, y: 2, w: 30, h: 40 })
  })

  it('rejects malformed, wrong-arity, or non-positive-size boxes', () => {
    expect(parseViewBox(null)).toBeNull()
    expect(parseViewBox('')).toBeNull()
    expect(parseViewBox('0 0 100')).toBeNull()
    expect(parseViewBox('0 0 100 nan')).toBeNull()
    expect(parseViewBox('0 0 0 50')).toBeNull()
    expect(parseViewBox('0 0 100 -5')).toBeNull()
  })

  it('round-trips through formatViewBox', () => {
    const vb: ViewBox = { x: 5, y: -3, w: 200, h: 120 }
    expect(formatViewBox(vb)).toBe('5 -3 200 120')
    expect(parseViewBox(formatViewBox(vb))).toEqual(vb)
  })
})

describe('clampViewBoxScale', () => {
  const baseW = 100
  it('clamps zoom-in to maxScale (smallest width)', () => {
    // Requesting an absurdly small width clamps to baseW / maxScale.
    expect(clampViewBoxScale(1, baseW, 1, 8)).toBeCloseTo(100 / 8)
  })

  it('clamps zoom-out to minScale (largest width)', () => {
    // Requesting a huge width clamps to baseW / minScale.
    expect(clampViewBoxScale(10000, baseW, 1, 8)).toBeCloseTo(100 / 1)
  })

  it('passes through a width within bounds', () => {
    expect(clampViewBoxScale(40, baseW, 1, 8)).toBe(40)
  })
})

describe('zoomViewBoxAtPoint', () => {
  const base: ViewBox = { x: 0, y: 0, w: 100, h: 80 }
  const opts = { minScale: 1, maxScale: 8, baseW: 100 }

  it('keeps the zoom point fixed (same fractional position)', () => {
    const point = { x: 25, y: 20 }
    const before = { fx: (point.x - base.x) / base.w, fy: (point.y - base.y) / base.h }
    const out = zoomViewBoxAtPoint(base, 2, point, opts)
    const after = { fx: (point.x - out.x) / out.w, fy: (point.y - out.y) / out.h }
    expect(after.fx).toBeCloseTo(before.fx)
    expect(after.fy).toBeCloseTo(before.fy)
  })

  it('zooms in: a factor > 1 shrinks the viewBox uniformly', () => {
    const out = zoomViewBoxAtPoint(base, 2, { x: 50, y: 40 }, opts)
    expect(out.w).toBeCloseTo(50)
    expect(out.h).toBeCloseTo(40) // aspect preserved
  })

  it('zooms out: a factor < 1 grows the viewBox (bounded by minScale = fit)', () => {
    // Already at fit (scale 1); zooming out cannot exceed the base width.
    const out = zoomViewBoxAtPoint(base, 0.5, { x: 50, y: 40 }, opts)
    expect(out.w).toBeCloseTo(100)
  })

  it('does not zoom in past maxScale', () => {
    const out = zoomViewBoxAtPoint(base, 1000, { x: 50, y: 40 }, opts)
    expect(out.w).toBeCloseTo(100 / 8)
  })

  it('returns the input unchanged for a non-positive factor or empty box', () => {
    expect(zoomViewBoxAtPoint(base, 0, { x: 0, y: 0 }, opts)).toEqual(base)
    expect(zoomViewBoxAtPoint({ x: 0, y: 0, w: 0, h: 0 }, 2, { x: 0, y: 0 }, opts)).toEqual({ x: 0, y: 0, w: 0, h: 0 })
  })
})

describe('panViewBox', () => {
  it('translates the origin by the negated user delta', () => {
    const vb: ViewBox = { x: 10, y: 20, w: 100, h: 80 }
    // Dragging content right (positive dx) moves the viewBox origin left.
    expect(panViewBox(vb, 5, -3)).toEqual({ x: 5, y: 23, w: 100, h: 80 })
  })

  it('preserves width and height', () => {
    const vb: ViewBox = { x: 0, y: 0, w: 100, h: 80 }
    const out = panViewBox(vb, 40, 40)
    expect(out.w).toBe(100)
    expect(out.h).toBe(80)
  })
})

describe('clientPointToSvg', () => {
  const rect = { left: 0, top: 0, width: 200, height: 160 }
  const vb: ViewBox = { x: 0, y: 0, w: 100, h: 80 }

  it('maps the canvas center to the viewBox center', () => {
    expect(clientPointToSvg(100, 80, rect, vb)).toEqual({ x: 50, y: 40 })
  })

  it('maps a corner accounting for the rect offset', () => {
    const offset = { left: 20, top: 10, width: 200, height: 160 }
    expect(clientPointToSvg(20, 10, offset, vb)).toEqual({ x: 0, y: 0 })
    expect(clientPointToSvg(220, 170, offset, vb)).toEqual({ x: 100, y: 80 })
  })

  it('accounts for a non-zero viewBox origin', () => {
    const shifted: ViewBox = { x: 10, y: 5, w: 100, h: 80 }
    expect(clientPointToSvg(0, 0, rect, shifted)).toEqual({ x: 10, y: 5 })
  })
})
