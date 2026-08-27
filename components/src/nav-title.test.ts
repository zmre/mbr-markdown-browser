import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { isClipped, syncTitleTooltip, observeNavTitle } from './nav-title.ts'

/**
 * happy-dom does no layout, so `scrollWidth`/`clientWidth` are both 0 and every
 * title would look un-clipped. These stubs stand in for the measurement, which
 * is why `isClipped` reads those two properties and nothing else.
 */
function measure(el: HTMLElement, scrollWidth: number, clientWidth: number) {
  Object.defineProperty(el, 'scrollWidth', { value: scrollWidth, configurable: true })
  Object.defineProperty(el, 'clientWidth', { value: clientWidth, configurable: true })
}

describe('isClipped', () => {
  let el: HTMLElement

  beforeEach(() => {
    el = document.createElement('strong')
  })

  it('reports content wider than its box', () => {
    measure(el, 400, 200)
    expect(isClipped(el)).toBe(true)
  })

  it('reports content that fits', () => {
    measure(el, 200, 200)
    expect(isClipped(el)).toBe(false)
  })

  // Both values are integers rounded from fractional layout, so an exact fit can
  // report one stray pixel. Tolerating it keeps a tooltip off a title the reader
  // can already read in full.
  it('tolerates a single rounding pixel', () => {
    measure(el, 201, 200)
    expect(isClipped(el)).toBe(false)
    measure(el, 202, 200)
    expect(isClipped(el)).toBe(true)
  })
})

describe('syncTitleTooltip', () => {
  let li: HTMLElement
  let strong: HTMLElement

  beforeEach(() => {
    li = document.createElement('li')
    li.className = 'mbr-nav-title'
    strong = document.createElement('strong')
    strong.textContent = '  A Very Long Document Title  '
    li.appendChild(strong)
    document.body.appendChild(li)
  })

  afterEach(() => {
    li.remove()
  })

  it('adds the tooltip, a tab stop and a downward placement when clipped', () => {
    measure(strong, 500, 120)
    syncTitleTooltip(li)

    // Trimmed: the template puts the Tera expression on its own line.
    expect(li.getAttribute('data-tooltip')).toBe('A Very Long Document Title')
    // Focus is what raises a Pico tooltip on a touch device.
    expect(li.getAttribute('tabindex')).toBe('0')
    // The header is at the top of the viewport; the default placement is above.
    expect(li.getAttribute('data-placement')).toBe('bottom')
  })

  it('adds nothing when the title already fits', () => {
    measure(strong, 120, 120)
    syncTitleTooltip(li)

    expect(li.hasAttribute('data-tooltip')).toBe(false)
    // An untruncated title has nothing to reveal, so it stays out of the tab order.
    expect(li.hasAttribute('tabindex')).toBe(false)
  })

  // The window can be widened again, and a stale tooltip would keep claiming
  // there is hidden text.
  it('removes a tooltip once the title fits again', () => {
    measure(strong, 500, 120)
    syncTitleTooltip(li)
    expect(li.hasAttribute('data-tooltip')).toBe(true)

    measure(strong, 120, 120)
    syncTitleTooltip(li)
    expect(li.hasAttribute('data-tooltip')).toBe(false)
    expect(li.hasAttribute('data-placement')).toBe(false)
  })

  it('is idempotent', () => {
    measure(strong, 500, 120)
    syncTitleTooltip(li)
    const first = li.getAttribute('data-tooltip')
    syncTitleTooltip(li)
    expect(li.getAttribute('data-tooltip')).toBe(first)
  })

  it('does nothing without an inner <strong>', () => {
    const bare = document.createElement('li')
    bare.className = 'mbr-nav-title'
    expect(() => syncTitleTooltip(bare)).not.toThrow()
    expect(bare.hasAttribute('data-tooltip')).toBe(false)
  })

  // An empty title (no `title`, no `current_dir_name`) must not produce a
  // tooltip with nothing in it.
  it('ignores an empty title even if the box reports a clip', () => {
    strong.textContent = ''
    measure(strong, 500, 120)
    syncTitleTooltip(li)
    expect(li.hasAttribute('data-tooltip')).toBe(false)
  })
})

describe('observeNavTitle', () => {
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('syncs on the first call', () => {
    document.body.innerHTML =
      '<li class="mbr-nav-title"><strong>Long</strong></li>'
    const li = document.querySelector<HTMLElement>('.mbr-nav-title')!
    measure(li.querySelector('strong')!, 500, 100)

    const stop = observeNavTitle()
    expect(li.getAttribute('data-tooltip')).toBe('Long')
    stop()
  })

  // A repository that overrides `_nav.html` may not have the element at all.
  it('is a no-op when the title is absent', () => {
    expect(() => observeNavTitle()()).not.toThrow()
  })
})
