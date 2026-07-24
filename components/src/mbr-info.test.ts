import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import './mbr-info.js'
import { setGraphChunkImporter } from './mbr-info.js'
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
