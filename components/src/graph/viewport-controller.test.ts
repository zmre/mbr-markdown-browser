import { describe, it, expect, afterEach } from 'vitest'
import { SvgViewportController } from './viewport-controller.js'
import { parseViewBox, type ViewBox } from './viewport.js'

/**
 * Tests for the controller's viewBox lifecycle, focused on the optional
 * `initialView` ("home") extension: the base viewBox from the SVG stays the
 * zoom-out floor, while the initial and reset() view can be a zoomed-in box.
 * Pointer/wheel gesture math is covered by the pure viewport.ts tests.
 */

const BASE_ATTR = '0 0 1000 200'
const INITIAL: ViewBox = { x: 400, y: 50, w: 200, h: 40 }

let cleanup: Array<() => void> = []

function setup(initialView?: ViewBox) {
  const canvas = document.createElement('div')
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg') as SVGSVGElement
  svg.setAttribute('viewBox', BASE_ATTR)
  canvas.appendChild(svg)
  document.body.appendChild(canvas)
  const controller = new SvgViewportController(
    canvas,
    svg,
    initialView ? { initialView } : undefined
  )
  cleanup.push(() => {
    controller.destroy()
    canvas.remove()
  })
  return { canvas, svg, controller }
}

const vbOf = (svg: SVGSVGElement): ViewBox => {
  const vb = parseViewBox(svg.getAttribute('viewBox'))
  expect(vb).not.toBeNull()
  return vb!
}

afterEach(() => {
  cleanup.forEach((fn) => fn())
  cleanup = []
})

describe('UNIT SvgViewportController', () => {
  it('keeps the SVG viewBox as-is without an initialView (backward compatible)', () => {
    const { svg, controller } = setup()
    expect(svg.getAttribute('viewBox')).toBe(BASE_ATTR)
    controller.reset()
    expect(svg.getAttribute('viewBox')).toBe(BASE_ATTR)
    // Base is also the zoom-out floor: zooming out from fit changes nothing.
    controller.zoomOut()
    expect(vbOf(svg).w).toBeCloseTo(1000, 5)
  })

  it('starts at the provided initialView', () => {
    const { svg } = setup(INITIAL)
    expect(vbOf(svg)).toEqual(INITIAL)
  })

  it('reset() returns to the initialView, not the raw fit', () => {
    const { svg, controller } = setup(INITIAL)
    controller.zoomIn()
    expect(vbOf(svg).w).toBeLessThan(INITIAL.w)
    controller.reset()
    expect(vbOf(svg)).toEqual(INITIAL)
  })

  it('still allows manual zoom-out from the initialView all the way to fit', () => {
    const { svg, controller } = setup(INITIAL)
    for (let i = 0; i < 20; i++) controller.zoomOut()
    const vb = vbOf(svg)
    // Clamped at the BASE (fit) size — not at the initial view's size.
    expect(vb.w).toBeCloseTo(1000, 5)
    expect(vb.h).toBeCloseTo(200, 5)
  })

  it('caps zoom-in relative to the base, independent of the initialView', () => {
    const { svg, controller } = setup(INITIAL)
    for (let i = 0; i < 30; i++) controller.zoomIn()
    // MAX_SCALE is 8× the base width (1000/8), not 8× the initial view.
    expect(vbOf(svg).w).toBeCloseTo(1000 / 8, 5)
  })
})
