import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import './mbr-fuzzy-nav.js'
import './mbr-info.js'
import { setGraphChunkImporter } from './mbr-info.js'
import type { PageLinks } from './graph/relationship-graph.js'

/**
 * `<mbr-fuzzy-nav>` loads its links through the shared `links.json` cache in
 * `graph/links-cache.ts`, the same entry point `<mbr-info>` and the mini graph
 * use. These tests pin the two properties that motivated the switch: exactly
 * one network request per page no matter how many consumers open, and a 404
 * (link tracking off) that stays distinguishable from a request failure.
 *
 * NOTE: the cache is module-level and persists across tests in this file, so
 * every test navigates to a UNIQUE path.
 */

const EMPTY_LINKS: PageLinks = { inbound: [], outbound: [] }

function okResponse(payload: PageLinks): Partial<Response> {
  return { ok: true, status: 200, json: async () => payload }
}

function statusResponse(status: number): Partial<Response> {
  return { ok: false, status, json: async () => ({}) }
}

/** Let queued microtasks and element updates settle. */
async function flush(...elements: HTMLElement[]): Promise<void> {
  for (let i = 0; i < 5; i++) {
    for (const el of elements) {
      await (el as HTMLElement & { updateComplete?: Promise<unknown> }).updateComplete
    }
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}

describe('MbrFuzzyNavElement links loading', () => {
  let nav: HTMLElement
  let fetchMock: ReturnType<typeof vi.fn>

  beforeEach(() => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    window.headings = []
    // The info panel eagerly imports the lazy graph chunk once links resolve;
    // stub the import seam so no runtime URL import is attempted.
    setGraphChunkImporter(() => Promise.resolve({}))
    fetchMock = vi.fn().mockResolvedValue(okResponse(EMPTY_LINKS))
    vi.stubGlobal('fetch', fetchMock)
    nav = document.createElement('mbr-fuzzy-nav')
    document.body.appendChild(nav)
  })

  afterEach(() => {
    nav.remove()
    vi.unstubAllGlobals()
    setGraphChunkImporter(() => Promise.reject(new Error('unset test importer')))
    window.__MBR_CONFIG__ = undefined
  })

  /** links.json requests only (site.json/media.json use the setup mock). */
  function linksRequests(): string[] {
    return fetchMock.mock.calls
      .map((args) => String(args[0]))
      .filter((url) => url.endsWith('links.json'))
  }

  it('fetches links.json once when the info panel and fuzzy nav both open', async () => {
    window.history.pushState({}, '', '/notes/shared-fetch/')
    const info = document.createElement('mbr-info')
    document.body.appendChild(info)

    try {
      ;(info as unknown as { _open(): void })._open()
      ;(nav as unknown as { open(): void }).open()
      await flush(info, nav)

      expect(linksRequests()).toEqual(['/notes/shared-fetch/links.json'])
      expect((nav as unknown as { _links: PageLinks | null })._links).toEqual(EMPTY_LINKS)
    } finally {
      info.remove()
    }
  })

  it('resolves the canonical path for a static build deployed in a subdirectory', async () => {
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../../' }
    window.history.pushState({}, '', '/deployed-at/notes/sub dir/')

    ;(nav as unknown as { open(): void }).open()
    await flush(nav)

    // Canonical path (deployment prefix stripped) + per-segment encoding;
    // the old local fetch used the raw pathname and no encoding.
    expect(linksRequests()).toEqual(['../../notes/sub%20dir/links.json'])
  })

  it('treats a 404 as "link tracking disabled": empty results, no error', async () => {
    window.history.pushState({}, '', '/notes/no-tracking/')
    fetchMock.mockResolvedValue(statusResponse(404))

    const el = nav as unknown as { open(): void; _links: PageLinks | null; _linksError: string | null }
    el.open()
    await flush(nav)

    expect(el._links).toEqual(EMPTY_LINKS)
    expect(el._linksError).toBeNull()
    const root = (nav as unknown as { shadowRoot: ShadowRoot }).shadowRoot
    expect(root.querySelector('.error-state')).toBeNull()
    expect(root.querySelector('.empty-state')?.textContent).toContain('No outbound links')
  })

  it('surfaces a network failure as an error state and retries on reopen', async () => {
    window.history.pushState({}, '', '/notes/network-down/')
    fetchMock.mockRejectedValueOnce(new Error('network down'))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})

    try {
      const el = nav as unknown as {
        open(): void
        close(): void
        _links: PageLinks | null
        _linksError: string | null
      }
      el.open()
      await flush(nav)

      expect(el._linksError).toBe('network down')
      const root = (nav as unknown as { shadowRoot: ShadowRoot }).shadowRoot
      expect(root.querySelector('.error-state')?.textContent).toContain('network down')

      // The failure was not cached locally, so reopening retries and recovers.
      fetchMock.mockResolvedValue(okResponse({ inbound: [], outbound: [] }))
      el.close()
      await flush(nav)
      el.open()
      await flush(nav)

      expect(el._linksError).toBeNull()
      expect(linksRequests()).toHaveLength(2)
    } finally {
      warn.mockRestore()
    }
  })
})

describe('MbrFuzzyNavElement link sanitization', () => {
  let nav: HTMLElement

  beforeEach(() => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    window.headings = []
    nav = document.createElement('mbr-fuzzy-nav')
    document.body.appendChild(nav)
  })

  afterEach(() => {
    nav.remove()
    window.__MBR_CONFIG__ = undefined
  })

  async function openWithOutbound(outbound: PageLinks['outbound']): Promise<ShadowRoot> {
    const el = nav as unknown as {
      _isOpen: boolean
      _links: PageLinks
      _linksCache: PageLinks
      requestUpdate(): void
      updateComplete: Promise<unknown>
      shadowRoot: ShadowRoot
    }
    el._links = { inbound: [], outbound }
    el._linksCache = el._links
    el._isOpen = true
    el.requestUpdate()
    await el.updateComplete
    return el.shadowRoot
  }

  it('neutralizes script-capable destinations but keeps ordinary links', async () => {
    const root = await openWithOutbound([
      { to: 'javascript:alert(1)', text: 'external evil', internal: false },
      { to: 'JaVaScRiPt:alert(2)', text: 'internal evil', internal: true },
      { to: 'https://example.com/ok', text: 'external ok', internal: false },
      { to: '/docs/guide/', text: 'internal ok', internal: true },
    ])

    const hrefs = Array.from(root.querySelectorAll('a.result-item')).map((a) =>
      a.getAttribute('href')
    )
    expect(hrefs).toEqual(['#', '#', 'https://example.com/ok', '/docs/guide/'])
  })
})
