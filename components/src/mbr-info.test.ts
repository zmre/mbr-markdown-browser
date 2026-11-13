import { describe, it, expect, vi, beforeAll, beforeEach, afterEach } from 'vitest'

/**
 * site.json arrival is driven manually in this file so the late-resolution
 * case can be tested: the real `subscribeSiteNav` fires once, at module import
 * time, and cannot be replayed. Everything else in `shared.js` is the real
 * implementation, so exported function identities still match.
 */
type SiteNavTestState = { isLoading: boolean; data: unknown; error: string | null }

const { siteNavListeners, siteNavStateRef, emitSiteNav } = vi.hoisted(() => {
  const listeners = new Set<(state: SiteNavTestState) => void>()
  const stateRef = {
    current: { isLoading: true, data: null, error: null } as SiteNavTestState,
  }
  return {
    siteNavListeners: listeners,
    siteNavStateRef: stateRef,
    emitSiteNav: (data: unknown) => {
      stateRef.current = { isLoading: false, data, error: null }
      listeners.forEach((cb) => cb(stateRef.current))
    },
  }
})

vi.mock('./shared.js', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./shared.js')>()
  return {
    ...actual,
    subscribeSiteNav: (callback: (state: SiteNavTestState) => void) => {
      siteNavListeners.add(callback)
      callback(siteNavStateRef.current)
      return () => siteNavListeners.delete(callback)
    },
  }
})

import './mbr-info.js'
import { GRAPH_MODAL_CLOSE_EVENT, hasGraphableLinks, isEmptyMetadataValue, setGraphChunkImporter } from './mbr-info.js'
import { fetchPageLinks } from './graph/links-cache.js'
import { resolveUrl } from './shared.js'

/**
 * Tests for the mbr-info relationships section. The relationship data normally
 * arrives via links.json; here we set the component's state directly and assert
 * on the rendered grouping, links, attributes, and unresolved handling.
 */
describe('MbrInfoElement relationships', () => {
  let element: HTMLElement

  beforeEach(() => {
    window.frontmatter = { type: 'person' }
    element = document.createElement('mbr-info')
    document.body.appendChild(element)
  })

  afterEach(() => {
    element.remove()
  })

  async function openWithRelationships(relationships: unknown[]): Promise<void> {
    const el = element as any
    el._isOpen = true
    el._links = { inbound: [], outbound: [], relationships }
    el.requestUpdate()
    await el.updateComplete
  }

  it('renders a Relationships section grouped by predicate', async () => {
    await openWithRelationships([
      {
        rel_type: 'parent', predicate: 'parent', neighbor: '/people/dad/',
        neighbor_title: 'Dad', neighbor_raw: '[[Dad]]', resolved: true,
        direction: 'outgoing', attributes: {}, derived: false,
      },
      {
        rel_type: 'spouse', predicate: 'spouse', neighbor: '/people/mary/',
        neighbor_title: 'Mary', neighbor_raw: '[[Mary]]', resolved: true,
        direction: 'outgoing', attributes: { married: '1925', divorced: '1940' }, derived: false,
      },
    ])

    const root = (element as any).shadowRoot as ShadowRoot
    const text = root.textContent || ''
    expect(text).toContain('Relationships')

    // Group headers use the fallback pluralization (no registry in this test).
    const groupTitles = Array.from(root.querySelectorAll('.link-group-title')).map(
      (n) => n.textContent || ''
    )
    expect(groupTitles.some((t) => t.startsWith('Parents'))).toBe(true)
    expect(groupTitles.some((t) => t.startsWith('Spouses'))).toBe(true)

    // Resolved neighbors are rendered as links.
    const links = Array.from(root.querySelectorAll('a.link-url')).map((a) => a.textContent?.trim())
    expect(links).toContain('Dad')
    expect(links).toContain('Mary')

    // Edge attributes are shown.
    const attrs = Array.from(root.querySelectorAll('.rel-attrs')).map((n) => n.textContent || '')
    expect(attrs.some((a) => a.includes('married 1925') && a.includes('divorced 1940'))).toBe(true)
  })

  it('renders unresolved neighbors as plain text, not links', async () => {
    await openWithRelationships([
      {
        rel_type: 'spouse', predicate: 'spouse', neighbor: '',
        neighbor_title: 'Jane Ghost', neighbor_raw: '[[Jane Ghost]]', resolved: false,
        direction: 'outgoing', attributes: {}, derived: false,
      },
    ])

    const root = (element as any).shadowRoot as ShadowRoot
    const unresolved = root.querySelector('.rel-unresolved')
    expect(unresolved).not.toBeNull()
    expect(unresolved?.textContent?.trim()).toBe('Jane Ghost')
    // No anchor should have been produced for the unresolved edge.
    expect(root.querySelector('a.link-url')).toBeNull()
  })

  it('omits the Relationships section when there are none', async () => {
    await openWithRelationships([])

    const root = (element as any).shadowRoot as ShadowRoot
    const text = root.textContent || ''
    expect(text).not.toContain('Relationships')
  })
})

describe('MbrInfoElement metadata on contact pages', () => {
  afterEach(() => {
    document.querySelectorAll('mbr-info').forEach((el) => el.remove())
  })

  async function metadataText(frontmatter: Record<string, unknown>): Promise<string> {
    window.frontmatter = frontmatter
    const el = document.createElement('mbr-info') as any
    document.body.appendChild(el)
    el._isOpen = true
    el.requestUpdate()
    await el.updateComplete
    return el.shadowRoot.querySelector('.metadata-table')?.textContent ?? ''
  }

  const fields = {
    title: 'Jane Doe',
    'emails.work': 'jane@abc.example',
    'dates.anniversary': '--06-10',
    job_title: 'VP',
    gender: 'female',
  }

  it('leaves card fields to the contact card', async () => {
    const text = await metadataText({ type: 'person', ...fields })
    expect(text).toContain('Jane Doe')
    expect(text).toContain('female')
    expect(text).not.toContain('jane@abc.example')
    expect(text).not.toContain('--06-10')
    expect(text).not.toContain('VP')
  })

  it("never shows the page's internal fields, such as the source fingerprint", async () => {
    const text = await metadataText({ title: 'Note', markdown_source: 'note.md', markdown_source_hash: 'abc123def' })
    expect(text).toContain('Note')
    expect(text).not.toContain('note.md')
    expect(text).not.toContain('abc123def')
  })

  it('never lists edit_enabled, and skips rows with nothing in them', async () => {
    const text = await metadataText({
      type: 'person',
      title: 'Jane Doe',
      edit_enabled: 'true',
      relationships: [],
      aliases: {},
      nickname: '  ',
      middle_name: null,
      draft: false,
      rank: 0,
    })
    expect(text).toContain('Jane Doe')
    expect(text).not.toMatch(/edit enabled/i)
    expect(text).not.toContain('Relationships')
    expect(text).not.toContain('Aliases')
    expect(text).not.toContain('Nickname')
    expect(text).not.toContain('Middle name')
    // false and 0 are values, not blanks.
    expect(text).toContain('Draft')
    expect(text).toContain('Rank')
  })

  it('shows everything on other note types', async () => {
    const text = await metadataText({ type: 'event', ...fields })
    expect(text).toContain('jane@abc.example')
    expect(text).toContain('--06-10')
  })
})

describe('isEmptyMetadataValue', () => {
  it('treats absent, blank and empty containers as empty, and nothing else', () => {
    for (const v of [null, undefined, '', ' \t', [], {}]) expect(isEmptyMetadataValue(v)).toBe(true)
    for (const v of ['x', 0, false, ['a'], { a: 1 }]) expect(isEmptyMetadataValue(v)).toBe(false)
  })
})

describe('MbrInfoElement graph section', () => {
  let element: HTMLElement

  beforeEach(() => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    // A frontmatter title makes the Metadata section render, so the
    // "graph comes first" assertion has a second section to compare against.
    window.frontmatter = { title: 'Test Note' }
    element = document.createElement('mbr-info')
    document.body.appendChild(element)
  })

  afterEach(() => {
    element.remove()
    window.__MBR_CONFIG__ = undefined
  })

  async function renderPanel(state: Record<string, unknown>): Promise<ShadowRoot> {
    const el = element as any
    el._isOpen = true
    Object.assign(el, state)
    el.requestUpdate()
    await el.updateComplete
    return el.shadowRoot as ShadowRoot
  }

  it('renders no graph section when links.json is unavailable (null links)', async () => {
    const root = await renderPanel({
      _links: null,
      _linksUnavailable: true,
      _graphReady: true,
    })
    expect(root.querySelector('mbr-mini-graph')).toBeNull()
  })

  it('renders no graph section before the chunk has loaded', async () => {
    const root = await renderPanel({
      _links: { inbound: [], outbound: [] },
      _graphReady: false,
    })
    expect(root.querySelector('mbr-mini-graph')).toBeNull()
  })

  it('renders the mini graph first in the panel with injected services', async () => {
    const root = await renderPanel({
      _links: { inbound: [], outbound: [] },
      _graphReady: true,
    })
    const graph = root.querySelector('mbr-mini-graph') as any
    expect(graph).not.toBeNull()

    // Injected service bindings (properties, not attributes).
    expect(graph.fetchLinks).toBe(fetchPageLinks)
    expect(graph.resolveHref).toBe(resolveUrl)
    expect(typeof graph.isKnownNote).toBe('function')
    expect(typeof graph.getMeta).toBe('function')
    expect(graph.depth).toBe(2) // getGraphDepth default
    expect(graph.maxNodes).toBe(80)
    expect(graph.focusPath).toBe(window.location.pathname)

    // The graph section comes FIRST: before the metadata/relationships/etc.
    const content = root.querySelector('.info-panel-content') as HTMLElement
    const sections = content.querySelectorAll('mbr-mini-graph, details.info-section')
    expect(sections.length).toBeGreaterThan(1)
    expect(sections[0]?.tagName.toLowerCase()).toBe('mbr-mini-graph')
  })

  it('loads the chunk through the overridable import seam on open', async () => {
    const importer = vi.fn().mockResolvedValue({})
    setGraphChunkImporter(importer)
    try {
      // The global fetch mock (test-setup) returns ok JSON, so links resolve
      // non-null and the chunk import is triggered.
      const el = element as any
      el._open()
      for (let i = 0; i < 5; i++) {
        await el.updateComplete
        await new Promise((resolve) => setTimeout(resolve, 0))
      }
      expect(importer).toHaveBeenCalledTimes(1)
      expect(el._graphReady).toBe(true)
      const root = el.shadowRoot as ShadowRoot
      expect(root.querySelector('mbr-mini-graph')).not.toBeNull()

      // Reopening does not import the chunk again (module-level promise).
      el._close()
      await el.updateComplete
      el._open()
      await el.updateComplete
      expect(importer).toHaveBeenCalledTimes(1)
    } finally {
      setGraphChunkImporter(() => Promise.reject(new Error('unset test importer')))
    }
  })

  it('shows no graph (but keeps other sections) when the chunk import fails', async () => {
    const importer = vi.fn().mockRejectedValue(new Error('offline'))
    setGraphChunkImporter(importer)
    try {
      const el = element as any
      el._open()
      for (let i = 0; i < 5; i++) {
        await el.updateComplete
        await new Promise((resolve) => setTimeout(resolve, 0))
      }
      expect(importer).toHaveBeenCalledTimes(1)
      expect(el._graphReady).toBe(false)
      const root = el.shadowRoot as ShadowRoot
      expect(root.querySelector('mbr-mini-graph')).toBeNull()
    } finally {
      setGraphChunkImporter(() => Promise.reject(new Error('unset test importer')))
    }
  })
})

describe('MbrInfoElement late site.json arrival', () => {
  let element: HTMLElement

  beforeEach(() => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    window.frontmatter = { title: 'Test Note' }
    // site.json still in flight when the element connects.
    siteNavStateRef.current = { isLoading: true, data: null, error: null }
    element = document.createElement('mbr-info')
    document.body.appendChild(element)
  })

  afterEach(() => {
    element.remove()
    siteNavStateRef.current = { isLoading: true, data: null, error: null }
    window.__MBR_CONFIG__ = undefined
  })

  it('rebuilds the graph lookups when site.json resolves after the panel opened', async () => {
    const el = element as any
    el._isOpen = true
    el._links = { inbound: [], outbound: [] }
    el._graphReady = true
    el.requestUpdate()
    await el.updateComplete

    const graph = () => el.shadowRoot.querySelector('mbr-mini-graph') as any
    const before = graph().isKnownNote
    // No site.json yet: every neighbor is rejected, so the BFS yields nothing.
    expect(before('/notes/one/')).toBe(false)

    emitSiteNav({
      markdown_files: [
        { url_path: '/notes/one/', frontmatter: { title: 'One', description: 'First note' } },
      ],
    })
    await el.updateComplete

    // A NEW function identity is what makes <mbr-mini-graph> restart its BFS
    // (willUpdate keys on `changed.has('isKnownNote')`); reusing the old
    // closure left the graph permanently empty until a close/reopen.
    const after = graph().isKnownNote
    expect(after).not.toBe(before)
    expect(after('/notes/one/')).toBe(true)

    const getMeta = graph().getMeta
    expect(getMeta('/notes/one/')).toEqual({ title: 'One', description: 'First note' })
  })

  it('re-renders the panel when site.json resolves while it is open', async () => {
    const el = element as any
    el._isOpen = true
    el._links = { inbound: [], outbound: [] }
    el._graphReady = true
    el.requestUpdate()
    await el.updateComplete

    const rendersBefore = el.shadowRoot.querySelector('mbr-mini-graph').getMeta
    emitSiteNav({ markdown_files: [{ url_path: '/notes/two/', frontmatter: {} }] })
    // The reactive `_noteMeta` must schedule an update; without it the child
    // keeps the stale bindings forever.
    await el.updateComplete
    expect(el.shadowRoot.querySelector('mbr-mini-graph').getMeta).not.toBe(rendersBefore)
  })
})

describe('MbrInfoElement link sanitization', () => {
  let element: HTMLElement

  beforeEach(() => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    window.frontmatter = {}
    element = document.createElement('mbr-info')
    document.body.appendChild(element)
  })

  afterEach(() => {
    element.remove()
    window.__MBR_CONFIG__ = undefined
  })

  async function openWithOutbound(outbound: unknown[]): Promise<ShadowRoot> {
    const el = element as any
    el._isOpen = true
    el._links = { inbound: [], outbound }
    el.requestUpdate()
    await el.updateComplete
    return el.shadowRoot as ShadowRoot
  }

  it('neutralizes script-capable external destinations', async () => {
    const root = await openWithOutbound([
      { to: 'javascript:alert(1)', text: 'evil', internal: false },
      { to: 'DATA:text/html,<script>alert(1)</script>', text: 'evil data', internal: false },
      { to: 'https://example.com/ok', text: 'fine', internal: false },
    ])

    const hrefs = Array.from(root.querySelectorAll('a.link-url.external')).map((a) =>
      a.getAttribute('href')
    )
    expect(hrefs).toEqual(['#', '#', 'https://example.com/ok'])
  })

  it('neutralizes script-capable internal destinations, anchor included', async () => {
    const root = await openWithOutbound([
      { to: 'java\tscript:alert(1)', text: 'evil', internal: true, anchor: '#x' },
      { to: '/docs/guide/', text: 'fine', internal: true, anchor: '#intro' },
    ])

    const hrefs = Array.from(root.querySelectorAll('a.link-url:not(.external)')).map((a) =>
      a.getAttribute('href')
    )
    expect(hrefs).toEqual(['#', '/docs/guide/#intro'])
  })
})

describe('MbrInfoElement link hrefs in a static build', () => {
  let element: HTMLElement

  beforeEach(() => {
    window.frontmatter = {}
    element = document.createElement('mbr-info')
    document.body.appendChild(element)
  })

  afterEach(() => {
    element.remove()
    window.__MBR_CONFIG__ = undefined
  })

  async function hrefs(config: Window['__MBR_CONFIG__']): Promise<string[]> {
    window.__MBR_CONFIG__ = config
    const el = element as any
    el._isOpen = true
    el._links = {
      inbound: [{ from: '/modes/', text: 'back' }],
      outbound: [{ to: '/modes/gui/', text: 'GUI', internal: true, anchor: '#menu' }],
    }
    el.requestUpdate()
    await el.updateComplete
    return Array.from((el.shadowRoot as ShadowRoot).querySelectorAll('a.link-url')).map(
      (a) => a.getAttribute('href') ?? ''
    )
  }

  it('resolves site-absolute targets against the base path', async () => {
    // Regression: these were written verbatim, so under a deployment prefix
    // `/modes/gui/` left the site.
    expect(await hrefs({ serverMode: false, guiMode: false, basePath: '../../' })).toEqual([
      '../../modes/gui/#menu',
      '../../modes/',
    ])
  })

  it('keeps server-mode targets as they are', async () => {
    expect(await hrefs({ serverMode: true, guiMode: false })).toEqual(['/modes/gui/#menu', '/modes/'])
  })
})

describe('MbrInfoElement sections', () => {
  afterEach(() => {
    document.querySelectorAll('mbr-info').forEach((el) => el.remove())
  })

  it('opens every section by default, links in and out included', async () => {
    window.frontmatter = { title: 'Note' }
    window.headings = [{ level: 2, id: 'a', text: 'A' }]
    const el = document.body.appendChild(document.createElement('mbr-info')) as any
    el._isOpen = true
    el._links = {
      inbound: [{ from: '/b/', text: 'b' }],
      outbound: [{ to: '/c/', text: 'c', internal: true }],
      relationships: [],
    }
    el.requestUpdate()
    await el.updateComplete
    const sections = Array.from((el.shadowRoot as ShadowRoot).querySelectorAll('details.info-section'))
    const titles = sections.map((d) => d.querySelector('summary strong')?.textContent)
    expect(titles).toEqual(expect.arrayContaining(['Metadata', 'Table of Contents', 'Links Out', 'Links In']))
    expect(sections.every((d) => d.hasAttribute('open'))).toBe(true)
  })

  it('opens the loading placeholders too', async () => {
    const el = document.body.appendChild(document.createElement('mbr-info')) as any
    el._isOpen = true
    el._linksLoading = true
    el.requestUpdate()
    await el.updateComplete
    const sections = Array.from((el.shadowRoot as ShadowRoot).querySelectorAll('details.info-section'))
    expect(sections.length).toBeGreaterThanOrEqual(2)
    expect(sections.every((d) => d.hasAttribute('open'))).toBe(true)
  })
})

describe('MbrInfoElement keyboard scrolling', () => {
  afterEach(() => {
    document.querySelectorAll('mbr-info').forEach((el) => el.remove())
  })

  function press(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
    const event = new KeyboardEvent('keydown', { key, bubbles: true, composed: true, cancelable: true, ...init })
    document.body.dispatchEvent(event)
    return event
  }

  it('scrolls the panel, not the page, on j / k / Ctrl+d / Ctrl+u', async () => {
    const el = document.body.appendChild(document.createElement('mbr-info')) as any
    el._isOpen = true
    el._links = { inbound: [], outbound: [] }
    el.requestUpdate()
    await el.updateComplete
    const pane = (el.shadowRoot as ShadowRoot).querySelector('.info-panel') as HTMLElement
    Object.defineProperty(pane, 'clientHeight', { value: 600, configurable: true })
    const paneScroll = vi.fn()
    pane.scrollBy = paneScroll as unknown as typeof pane.scrollBy
    const pageScroll = vi.spyOn(window, 'scrollBy').mockImplementation(() => {})

    try {
      expect(press('j').defaultPrevented).toBe(true)
      press('k')
      press('d', { ctrlKey: true })
      press('u', { ctrlKey: true })
      expect(paneScroll.mock.calls.map((c) => (c[0] as ScrollToOptions).top)).toEqual([40, -40, 300, -300])
      expect(pageScroll).not.toHaveBeenCalled()

      // Closed: the keys are the page's again.
      el._close()
      await el.updateComplete
      paneScroll.mockClear()
      expect(press('j').defaultPrevented).toBe(false)
      expect(paneScroll).not.toHaveBeenCalled()
    } finally {
      pageScroll.mockRestore()
    }
  })

  it('leaves keys typed into a field alone', async () => {
    const el = document.body.appendChild(document.createElement('mbr-info')) as any
    el._isOpen = true
    el.requestUpdate()
    await el.updateComplete
    const input = document.body.appendChild(document.createElement('input'))
    const event = new KeyboardEvent('keydown', { key: 'j', bubbles: true, composed: true, cancelable: true })
    input.dispatchEvent(event)
    expect(event.defaultPrevented).toBe(false)
    input.remove()
  })
})

describe('hasGraphableLinks', () => {
  it('needs an internal link out, a link in or a relationship', () => {
    expect(hasGraphableLinks({ inbound: [], outbound: [] })).toBe(false)
    expect(hasGraphableLinks({ inbound: [], outbound: [{ to: 'https://x', internal: false }] } as never)).toBe(false)
    expect(hasGraphableLinks({ inbound: [], outbound: [{ to: '/a/', internal: true }] } as never)).toBe(true)
    expect(hasGraphableLinks({ inbound: [{ from: '/a/' }], outbound: [] } as never)).toBe(true)
    expect(hasGraphableLinks({ inbound: [], outbound: [], relationships: [{}] } as never)).toBe(true)
  })
})

/**
 * Ctrl+Shift+g. The graph element lives in the lazy chunk; a stub with the
 * same public surface stands in for it (defined here, after every other suite
 * in this file has run against an undefined `<mbr-mini-graph>`).
 */
describe('MbrInfoElement Ctrl+Shift+g full graph', () => {
  class StubGraph extends HTMLElement {
    expanded = false
    openModal(): void {
      this.expanded = true
      if (!this.shadowRoot) this.attachShadow({ mode: 'open' })
      this.shadowRoot!.innerHTML = '<dialog class="graph-modal" open></dialog>'
    }
    closeModal(): void {
      if (!this.expanded) return
      this.expanded = false
      this.shadowRoot!.innerHTML = ''
      this.dispatchEvent(new CustomEvent(GRAPH_MODAL_CLOSE_EVENT))
    }
  }

  const LINKS = { inbound: [{ from: '/b/', text: 'b' }], outbound: [] }

  beforeAll(() => {
    if (!customElements.get('mbr-mini-graph')) customElements.define('mbr-mini-graph', StubGraph)
  })

  afterEach(() => {
    document.querySelectorAll('mbr-info').forEach((el) => el.remove())
    document.querySelectorAll('.mbr-toast').forEach((el) => el.remove())
    document.body.classList.remove('slides-container')
  })

  function pressGraphKey(): KeyboardEvent {
    const event = new KeyboardEvent('keydown', {
      key: 'G', ctrlKey: true, shiftKey: true, bubbles: true, composed: true, cancelable: true,
    })
    document.body.dispatchEvent(event)
    return event
  }

  async function settle(el: any): Promise<void> {
    for (let i = 0; i < 4; i++) {
      await el.updateComplete
      await new Promise((resolve) => setTimeout(resolve, 0))
    }
  }

  it('opens a standalone graph without opening the panel, and drops it on close', async () => {
    const el = document.body.appendChild(document.createElement('mbr-info')) as any
    el._links = LINKS
    el._graphReady = true
    expect(pressGraphKey().defaultPrevented).toBe(true)
    await settle(el)

    expect(el._isOpen).toBe(false)
    const graph = (el.shadowRoot as ShadowRoot).querySelector('mbr-mini-graph.standalone-graph') as StubGraph
    expect(graph).not.toBeNull()
    expect(graph.expanded).toBe(true)
    // It counts as an open overlay while shown, so page shortcuts stand down.
    expect(el.isOpen).toBe(true)

    graph.closeModal()
    await settle(el)
    expect((el.shadowRoot as ShadowRoot).querySelector('mbr-mini-graph')).toBeNull()
    expect(el.isOpen).toBe(false)
  })

  it("expands the open panel's own graph instead of mounting another", async () => {
    const el = document.body.appendChild(document.createElement('mbr-info')) as any
    el._isOpen = true
    el._links = LINKS
    el._graphReady = true
    el.requestUpdate()
    await el.updateComplete
    pressGraphKey()
    await settle(el)
    const graphs = (el.shadowRoot as ShadowRoot).querySelectorAll('mbr-mini-graph')
    expect(graphs).toHaveLength(1)
    expect((graphs[0] as StubGraph).expanded).toBe(true)
    expect(el._isOpen).toBe(true)
  })

  it('does not toggle the panel', async () => {
    const el = document.body.appendChild(document.createElement('mbr-info')) as any
    el._links = LINKS
    el._graphReady = true
    pressGraphKey()
    await settle(el)
    expect(el._isOpen).toBe(false)
  })

  it('says so when link tracking is off', async () => {
    const el = document.body.appendChild(document.createElement('mbr-info')) as any
    el._linksUnavailable = true
    pressGraphKey()
    await settle(el)
    expect((el.shadowRoot as ShadowRoot).querySelector('mbr-mini-graph')).toBeNull()
    expect(document.querySelector('.mbr-toast')?.textContent).toContain('link tracking is off')
  })

  it('says so when the note has nothing to graph', async () => {
    const el = document.body.appendChild(document.createElement('mbr-info')) as any
    el._links = { inbound: [], outbound: [{ to: 'https://example.com', internal: false }] }
    el._graphReady = true
    pressGraphKey()
    await settle(el)
    expect((el.shadowRoot as ShadowRoot).querySelector('mbr-mini-graph')).toBeNull()
    expect(document.querySelector('.mbr-toast')?.textContent).toContain('no links')
  })

  it('stands down while a slide deck plays', async () => {
    document.body.classList.add('slides-container')
    const el = document.body.appendChild(document.createElement('mbr-info')) as any
    el._links = LINKS
    el._graphReady = true
    expect(pressGraphKey().defaultPrevented).toBe(false)
    await settle(el)
    expect((el.shadowRoot as ShadowRoot).querySelector('mbr-mini-graph')).toBeNull()
  })
})

describe('MbrInfoElement file location (GUI only)', () => {
  const baseMeta = {
    wordCount: 10,
    readingTimeMinutes: 1,
    fleschReadingEase: null,
    fleschKincaidGrade: null,
    filePath: 'notes/a.md',
    modifiedTimestamp: 0,
  }

  afterEach(() => {
    document.querySelectorAll('mbr-info').forEach((el) => el.remove())
    delete window.extendedMeta
  })

  async function openPanel(): Promise<ShadowRoot> {
    window.frontmatter = { title: 'Note' }
    const el = document.body.appendChild(document.createElement('mbr-info')) as any
    el._isOpen = true
    el.requestUpdate()
    await el.updateComplete
    return el.shadowRoot as ShadowRoot
  }

  const rowLabels = (root: ShadowRoot) =>
    Array.from(root.querySelectorAll('th[scope="row"]')).map((th) => th.textContent?.trim())

  it('shows no file button outside the GUI', async () => {
    window.extendedMeta = { ...baseMeta, fileLocation: null }
    const root = await openPanel()
    expect(rowLabels(root)).toContain('File')
    expect(rowLabels(root)).not.toContain('Location')
    expect(root.querySelector('.file-location-action')).toBeNull()
  })

  it('puts the reveal button beside the File label in the GUI, with no Location row', async () => {
    window.extendedMeta = { ...baseMeta, fileLocation: { path: '/Users/me/repo/notes/a.md', reveal: true } }
    const root = await openPanel()
    expect(rowLabels(root)).toContain('File')
    expect(rowLabels(root)).not.toContain('Location')
    const button = root.querySelector<HTMLElement>('th .file-label .file-location-action')
    expect(button?.dataset.action).toBe('reveal')
    // The row still shows the repository-relative path, never the absolute one.
    expect(root.querySelector('code.file-path')?.textContent).toBe('notes/a.md')
    expect(root.textContent).not.toContain('/Users/me')
  })

  it('offers to copy the full path where the GUI cannot reveal', async () => {
    window.extendedMeta = { ...baseMeta, fileLocation: { path: '/home/me/repo/notes/a.md', reveal: false } }
    const root = await openPanel()
    const button = root.querySelector<HTMLElement>('th .file-label .file-location-action')
    expect(button?.dataset.action).toBe('copy')
    expect(button?.getAttribute('aria-label')).toBe('Copy full path')
  })
})
