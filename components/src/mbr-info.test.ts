import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import './mbr-info.js'

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
