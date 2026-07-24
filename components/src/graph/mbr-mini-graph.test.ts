import { describe, it, expect, vi, afterEach } from 'vitest'
import './mbr-mini-graph.ts'
import type { MbrMiniGraphElement, NoteMeta } from './mbr-mini-graph.ts'
import type { PageLinks } from './relationship-graph.ts'

/**
 * Element tests run with the `static-layout` attribute: the d3 simulation is
 * laid out synchronously (stop + tick) so no rAF/timer pumping is needed.
 */

function outLinks(...to: string[]): PageLinks {
  return {
    inbound: [],
    outbound: to.map((t) => ({ to: t, text: t, internal: true })),
  }
}

/** A 4-note chain plus branches: a → (b, c); b → d. */
function chainMap(): Record<string, PageLinks> {
  return {
    '/a/': outLinks('/b/', '/c/'),
    '/b/': outLinks('/d/'),
    '/c/': outLinks(),
    '/d/': outLinks(),
  }
}

/** A 5-deep chain for depth-stepper tests: a → b → c → d → e. */
function deepChainMap(): Record<string, PageLinks> {
  return {
    '/a/': outLinks('/b/'),
    '/b/': outLinks('/c/'),
    '/c/': outLinks('/d/'),
    '/d/': outLinks('/e/'),
    '/e/': outLinks(),
  }
}

interface CreateOptions {
  map?: Record<string, PageLinks>
  focus?: string
  depth?: number
  maxNodes?: number
  meta?: Record<string, NoteMeta>
}

const created: MbrMiniGraphElement[] = []

async function createGraph(options: CreateOptions = {}) {
  const map = options.map ?? chainMap()
  const meta = options.meta ?? {}
  const fetchLinks = vi.fn(async (path: string) => map[path] ?? null)
  const element = document.createElement('mbr-mini-graph')
  element.setAttribute('static-layout', '')
  element.focusPath = options.focus ?? '/a/'
  element.depth = options.depth ?? 2
  if (options.maxNodes !== undefined) element.maxNodes = options.maxNodes
  element.fetchLinks = fetchLinks
  element.isKnownNote = (p) => p in map
  element.getMeta = (p) => meta[p]
  element.resolveHref = (p) => p
  document.body.appendChild(element)
  created.push(element)
  await settle(element)
  return { element, fetchLinks }
}

/** Let the async BFS (all-microtask with stub fetchers) and Lit settle. */
async function settle(element: MbrMiniGraphElement): Promise<void> {
  for (let i = 0; i < 6; i++) {
    await element.updateComplete
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
  await element.updateComplete
}

function shadow(element: MbrMiniGraphElement): ShadowRoot {
  return element.shadowRoot as ShadowRoot
}

function circle(element: MbrMiniGraphElement, id: string): SVGCircleElement | null {
  return shadow(element).querySelector<SVGCircleElement>(`circle[data-id="${id}"]`)
}

function pointerEvent(type: string, init: MouseEventInit = {}): Event {
  const Ctor = (globalThis as { PointerEvent?: typeof MouseEvent }).PointerEvent ?? MouseEvent
  return new Ctor(type, { bubbles: true, composed: true, ...init })
}

afterEach(() => {
  for (const element of created.splice(0)) element.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('mbr-mini-graph rendering', () => {
  it('renders circles with BFS degree classes', async () => {
    const { element } = await createGraph()
    expect(circle(element, '/a/')?.classList.contains('deg-0')).toBe(true)
    expect(circle(element, '/b/')?.classList.contains('deg-1')).toBe(true)
    expect(circle(element, '/c/')?.classList.contains('deg-1')).toBe(true)
    expect(circle(element, '/d/')?.classList.contains('deg-2')).toBe(true)
  })

  it('renders lines for every edge', async () => {
    const { element } = await createGraph()
    const lines = shadow(element).querySelectorAll('line.graph-link')
    expect(lines).toHaveLength(3)
  })

  it('gives the focus node a larger radius than the others', async () => {
    const { element } = await createGraph()
    const focusR = Number(circle(element, '/a/')?.getAttribute('r'))
    const otherR = Number(circle(element, '/b/')?.getAttribute('r'))
    expect(focusR).toBeGreaterThan(otherR)
  })

  it('assigns positions inside the canvas bounds', async () => {
    const { element } = await createGraph()
    const node = circle(element, '/b/')
    const cx = Number(node?.getAttribute('cx'))
    const cy = Number(node?.getAttribute('cy'))
    expect(cx).toBeGreaterThanOrEqual(0)
    expect(cx).toBeLessThanOrEqual(400)
    expect(cy).toBeGreaterThanOrEqual(0)
    expect(cy).toBeLessThanOrEqual(200)
  })

  it('renders nothing with fewer than two nodes', async () => {
    const { element } = await createGraph({ map: { '/solo/': outLinks() }, focus: '/solo/' })
    expect(shadow(element).querySelector('svg')).toBeNull()
  })

  it('renders nothing when the focus has no links.json', async () => {
    const { element } = await createGraph({ map: {}, focus: '/missing/' })
    expect(shadow(element).querySelector('svg')).toBeNull()
  })

  it('shows a truncation indicator when the node cap was hit', async () => {
    const { element } = await createGraph({ maxNodes: 2 })
    expect(shadow(element).querySelector('.truncation-badge')).not.toBeNull()
  })

  it('shows no truncation indicator when nothing was dropped', async () => {
    const { element } = await createGraph()
    expect(shadow(element).querySelector('.truncation-badge')).toBeNull()
  })

  it('exposes nodes as keyboard-reachable links', async () => {
    const { element } = await createGraph({
      meta: { '/b/': { title: 'Note B' } },
    })
    const node = circle(element, '/b/')
    expect(node?.getAttribute('role')).toBe('link')
    expect(node?.getAttribute('tabindex')).toBe('0')
    expect(node?.getAttribute('aria-label')).toBe('Go to Note B')
  })

  it('falls back to the last path segment when no metadata exists', async () => {
    const { element } = await createGraph()
    expect(circle(element, '/b/')?.getAttribute('aria-label')).toBe('Go to b')
  })
})

describe('mbr-mini-graph navigation', () => {
  it('navigates on click via the injected resolveHref', async () => {
    const assign = vi.fn()
    vi.stubGlobal('location', { assign, pathname: '/' })
    const { element } = await createGraph()
    circle(element, '/b/')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    expect(assign).toHaveBeenCalledWith('/b/')
  })

  it('navigates on Enter and Space', async () => {
    const assign = vi.fn()
    vi.stubGlobal('location', { assign, pathname: '/' })
    const { element } = await createGraph()
    circle(element, '/b/')?.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })
    )
    circle(element, '/c/')?.dispatchEvent(
      new KeyboardEvent('keydown', { key: ' ', bubbles: true })
    )
    expect(assign).toHaveBeenCalledTimes(2)
    expect(assign).toHaveBeenNthCalledWith(1, '/b/')
    expect(assign).toHaveBeenNthCalledWith(2, '/c/')
  })

  it('suppresses navigation for a click that follows a drag', async () => {
    const assign = vi.fn()
    vi.stubGlobal('location', { assign, pathname: '/' })
    const { element } = await createGraph()
    const node = circle(element, '/b/') as SVGCircleElement

    // Drag: down on the node, move past the threshold, release.
    node.dispatchEvent(pointerEvent('pointerdown', { clientX: 10, clientY: 10 }))
    document.dispatchEvent(pointerEvent('pointermove', { clientX: 40, clientY: 40 }))
    document.dispatchEvent(pointerEvent('pointerup', { clientX: 40, clientY: 40 }))
    node.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    expect(assign).not.toHaveBeenCalled()

    // The suppression is consumed: the next genuine click navigates.
    node.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    expect(assign).toHaveBeenCalledWith('/b/')
  })

  it('does not suppress navigation after a sub-threshold jiggle', async () => {
    const assign = vi.fn()
    vi.stubGlobal('location', { assign, pathname: '/' })
    const { element } = await createGraph()
    const node = circle(element, '/b/') as SVGCircleElement

    node.dispatchEvent(pointerEvent('pointerdown', { clientX: 10, clientY: 10 }))
    document.dispatchEvent(pointerEvent('pointermove', { clientX: 12, clientY: 11 }))
    document.dispatchEvent(pointerEvent('pointerup', { clientX: 12, clientY: 11 }))
    node.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    expect(assign).toHaveBeenCalledWith('/b/')
  })
})

describe('mbr-mini-graph expanded modal', () => {
  async function openModal(element: MbrMiniGraphElement): Promise<void> {
    const btn = shadow(element).querySelector<HTMLButtonElement>('.expand-btn')
    btn?.click()
    await element.updateComplete
  }

  it('opens from the expand button and closes from the close button', async () => {
    const { element } = await createGraph()
    await openModal(element)
    expect(shadow(element).querySelector('.graph-modal')).not.toBeNull()

    shadow(element).querySelector<HTMLButtonElement>('.graph-modal-close')?.click()
    await element.updateComplete
    expect(shadow(element).querySelector('.graph-modal')).toBeNull()
  })

  it('closes on backdrop (dialog) click', async () => {
    const { element } = await createGraph()
    await openModal(element)
    const dialog = shadow(element).querySelector<HTMLElement>('dialog.graph-modal')
    // A click whose target is the dialog itself is a click on the ::backdrop.
    dialog?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await element.updateComplete
    expect(shadow(element).querySelector('.graph-modal')).toBeNull()
  })

  it('keeps the modal open when its content is clicked', async () => {
    const { element } = await createGraph()
    await openModal(element)
    shadow(element)
      .querySelector<HTMLElement>('.graph-modal-body')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await element.updateComplete
    expect(shadow(element).querySelector('.graph-modal')).not.toBeNull()
  })

  it('shows node labels only in the expanded view', async () => {
    const { element } = await createGraph()
    expect(shadow(element).querySelectorAll('text.node-label')).toHaveLength(0)
    await openModal(element)
    expect(shadow(element).querySelectorAll('text.node-label').length).toBeGreaterThan(0)
  })

  it('Escape closes only the modal, not an outer (drawer) listener', async () => {
    const { element } = await createGraph()
    await openModal(element)

    // Simulates mbr-info's bubbling document-level Escape handler.
    const outerEscape = vi.fn()
    document.addEventListener('keydown', outerEscape)
    try {
      document.body.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })
      )
      await element.updateComplete
      expect(shadow(element).querySelector('.graph-modal')).toBeNull()
      expect(outerEscape).not.toHaveBeenCalled()

      // With the modal closed, Escape reaches the outer listener again.
      document.body.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })
      )
      expect(outerEscape).toHaveBeenCalledTimes(1)
    } finally {
      document.removeEventListener('keydown', outerEscape)
    }
  })
})

describe('mbr-mini-graph depth stepper', () => {
  async function openModal(element: MbrMiniGraphElement): Promise<void> {
    shadow(element).querySelector<HTMLButtonElement>('.expand-btn')?.click()
    await element.updateComplete
  }

  function stepperValue(element: MbrMiniGraphElement): string {
    return shadow(element).querySelector('.depth-value')?.textContent?.trim() ?? ''
  }

  function clickStep(element: MbrMiniGraphElement, label: string): void {
    const buttons = shadow(element).querySelectorAll<HTMLButtonElement>('.depth-stepper button')
    for (const btn of buttons) {
      if ((btn.textContent ?? '').trim() === label) btn.click()
    }
  }

  it('seeds from the depth property', async () => {
    const { element } = await createGraph({ map: deepChainMap(), depth: 2 })
    await openModal(element)
    expect(stepperValue(element)).toBe('2')
  })

  it('stepping up resumes the BFS and shows deeper nodes', async () => {
    const { element, fetchLinks } = await createGraph({ map: deepChainMap(), depth: 2 })
    await openModal(element)
    expect(shadow(element).querySelectorAll('circle').length).toBe(3) // a, b, c

    const callsBefore = fetchLinks.mock.calls.length
    clickStep(element, '+')
    await settle(element)
    expect(stepperValue(element)).toBe('3')
    expect(fetchLinks.mock.calls.length).toBeGreaterThan(callsBefore)
    expect(shadow(element).querySelectorAll('circle').length).toBe(4) // + d
  })

  it('stepping down filters without any refetch', async () => {
    const { element, fetchLinks } = await createGraph({ map: deepChainMap(), depth: 2 })
    await openModal(element)

    const callsBefore = fetchLinks.mock.calls.length
    clickStep(element, '−')
    await settle(element)
    expect(stepperValue(element)).toBe('1')
    expect(shadow(element).querySelectorAll('circle').length).toBe(2) // a, b
    expect(fetchLinks.mock.calls.length).toBe(callsBefore)
  })

  it('clamps to the 1–5 range', async () => {
    const { element } = await createGraph({ map: deepChainMap(), depth: 2 })
    await openModal(element)

    clickStep(element, '−')
    await settle(element)
    clickStep(element, '−')
    await settle(element)
    expect(stepperValue(element)).toBe('1')

    for (let i = 0; i < 8; i++) {
      clickStep(element, '+')
      await settle(element)
    }
    expect(stepperValue(element)).toBe('5')
  })
})

describe('mbr-mini-graph hover card', () => {
  function mockHoverCapable(matches: boolean): void {
    vi.spyOn(window, 'matchMedia').mockReturnValue({ matches } as MediaQueryList)
  }

  it('shows title and description on hover when hover-capable', async () => {
    mockHoverCapable(true)
    const { element } = await createGraph({
      meta: { '/b/': { title: 'Note B', description: 'All about B' } },
    })
    circle(element, '/b/')?.dispatchEvent(new MouseEvent('mouseenter'))

    const card = shadow(element).querySelector<HTMLElement>('.hover-card') as HTMLElement
    expect(card.style.display).toBe('block')
    expect(card.textContent).toContain('Note B')
    expect(card.textContent).toContain('All about B')
  })

  it('falls back to the path segment title without metadata', async () => {
    mockHoverCapable(true)
    const { element } = await createGraph()
    circle(element, '/b/')?.dispatchEvent(new MouseEvent('mouseenter'))
    const card = shadow(element).querySelector<HTMLElement>('.hover-card') as HTMLElement
    expect(card.style.display).toBe('block')
    expect(card.textContent).toContain('b')
    expect(card.querySelector('.hover-desc')).toBeNull()
  })

  it('is gated off on non-hover (touch) devices', async () => {
    mockHoverCapable(false)
    const { element } = await createGraph({
      meta: { '/b/': { title: 'Note B' } },
    })
    circle(element, '/b/')?.dispatchEvent(new MouseEvent('mouseenter'))
    const card = shadow(element).querySelector<HTMLElement>('.hover-card') as HTMLElement
    expect(card.style.display).toBe('none')
  })
})
