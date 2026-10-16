import { describe, it, expect, afterEach } from 'vitest'
import {
  VVH_PROPERTY,
  VV_TOP_PROPERTY,
  trackVisualViewport,
  visualViewportVars,
  type ViewportSource,
} from './visual-viewport.js'
import './mbr-search.js'
import type { MbrSearchElement } from './mbr-search.js'
import searchSource from './mbr-search.ts?raw'

/** A `VisualViewport` stand-in whose box the test moves. */
class FakeViewport extends EventTarget implements ViewportSource {
  height = 800
  offsetTop = 0
  move(height: number, offsetTop = 0, type: 'resize' | 'scroll' = 'resize'): void {
    this.height = height
    this.offsetTop = offsetTop
    this.dispatchEvent(new Event(type))
  }
}

describe('visualViewportVars', () => {
  it('publishes the visible height and offset in whole pixels', () => {
    expect(visualViewportVars({ height: 451.6, offsetTop: 120.4 })).toEqual({
      [VVH_PROPERTY]: '452px',
      [VV_TOP_PROPERTY]: '120px',
    })
  })

  it('clamps a negative or missing offset to 0 (rubber-band overscroll)', () => {
    expect(visualViewportVars({ height: 600, offsetTop: -30 })?.[VV_TOP_PROPERTY]).toBe('0px')
    expect(visualViewportVars({ height: 600, offsetTop: Number.NaN })?.[VV_TOP_PROPERTY]).toBe('0px')
  })

  it('refuses an unusable height, so the CSS fallbacks stay in force', () => {
    expect(visualViewportVars({ height: 0, offsetTop: 0 })).toBeNull()
    expect(visualViewportVars({ height: -1, offsetTop: 0 })).toBeNull()
    expect(visualViewportVars({ height: Number.POSITIVE_INFINITY, offsetTop: 0 })).toBeNull()
  })
})

describe('trackVisualViewport', () => {
  it('follows resize and scroll, and removes the properties when stopped', () => {
    const target = document.createElement('div')
    const vv = new FakeViewport()
    const stop = trackVisualViewport(target, vv)
    expect(target.style.getPropertyValue(VVH_PROPERTY)).toBe('800px')

    // The keyboard comes up and Safari scrolls the focused field into view.
    vv.move(420, 0)
    vv.move(420, 180, 'scroll')
    expect(target.style.getPropertyValue(VVH_PROPERTY)).toBe('420px')
    expect(target.style.getPropertyValue(VV_TOP_PROPERTY)).toBe('180px')

    stop()
    expect(target.style.getPropertyValue(VVH_PROPERTY)).toBe('')
    expect(target.style.getPropertyValue(VV_TOP_PROPERTY)).toBe('')
    vv.move(300, 0)
    expect(target.style.getPropertyValue(VVH_PROPERTY)).toBe('')
  })

  it('does nothing without a visual viewport', () => {
    const target = document.createElement('div')
    trackVisualViewport(target, null)()
    expect(target.getAttribute('style') ?? '').toBe('')
  })
})

describe('<mbr-search> and the on-screen keyboard', () => {
  let el: MbrSearchElement | undefined
  const original = Object.getOwnPropertyDescriptor(window, 'visualViewport')

  afterEach(() => {
    el?.remove()
    el = undefined
    if (original) Object.defineProperty(window, 'visualViewport', original)
    else delete (window as { visualViewport?: unknown }).visualViewport
  })

  it('tracks the visual viewport only while the modal is open', async () => {
    const vv = new FakeViewport()
    Object.defineProperty(window, 'visualViewport', { configurable: true, value: vv })
    el = document.createElement('mbr-search') as MbrSearchElement
    document.body.appendChild(el)
    await el.updateComplete
    expect(el.style.getPropertyValue(VVH_PROPERTY)).toBe('')

    el.open()
    expect(el.style.getPropertyValue(VVH_PROPERTY)).toBe('800px')
    vv.move(390)
    expect(el.style.getPropertyValue(VVH_PROPERTY)).toBe('390px')

    el.close()
    expect(el.style.getPropertyValue(VVH_PROPERTY)).toBe('')
    vv.move(500)
    expect(el.style.getPropertyValue(VVH_PROPERTY)).toBe('')
  })

  it('stops tracking when removed while open', async () => {
    const vv = new FakeViewport()
    Object.defineProperty(window, 'visualViewport', { configurable: true, value: vv })
    el = document.createElement('mbr-search') as MbrSearchElement
    document.body.appendChild(el)
    el.open()
    el.remove()
    vv.move(300)
    expect(el.style.getPropertyValue(VVH_PROPERTY)).toBe('')
  })

  // happy-dom lays nothing out, so the CSS that consumes the properties is
  // pinned as text.
  it('sizes the backdrop and caps the modal from the visual viewport', () => {
    const backdrop = searchSource.slice(searchSource.indexOf('.modal-backdrop {'))
    const rule = backdrop.slice(0, backdrop.indexOf('}'))
    expect(rule).toContain('top: var(--mbr-vv-top, 0px)')
    expect(rule).toContain('height: var(--mbr-vvh, 100%)')
    const modal = searchSource.slice(searchSource.indexOf('    .modal {'))
    expect(modal.slice(0, modal.indexOf('}'))).toMatch(/max-height: min\(70dvh, calc\(var\(--mbr-vvh, 100dvh\)/)
  })
})
