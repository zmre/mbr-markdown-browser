import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import './mbr-fuzzy-nav.js'
import type { PageLinks } from './graph/relationship-graph.js'

/**
 * Regression: on a static build deployed under a prefix (GitHub Pages'
 * `/mbr-markdown-browser/`), the ROOT page's "links out" list was empty. The
 * canonical path was derived by keeping as many trailing pathname segments as
 * `basePath` has `../`, and the root has none, so the prefix itself came back
 * as the page (`/mbr-markdown-browser/`), links.json was requested from
 * `./mbr-markdown-browser/links.json` (404 = "link tracking off") and the tab
 * showed "No outbound links". Pages below the root found their links.json.
 *
 * Each case drives the real element end to end: the request it makes and the
 * hrefs it renders from the site-absolute `to`/`from` values links.json
 * carries. Its own file because the links cache is module-level and keyed by
 * canonical path, and `/` must be fetched fresh here.
 */

const PREFIX = '/mbr-markdown-browser'

function okResponse(payload: PageLinks): Partial<Response> {
  return { ok: true, status: 200, json: async () => payload }
}

/** Let queued microtasks and element updates settle. */
async function flush(el: HTMLElement): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await (el as HTMLElement & { updateComplete?: Promise<unknown> }).updateComplete
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}

const LINKS: PageLinks = {
  inbound: [{ from: '/modes/', text: 'back' }],
  outbound: [
    { to: '/getting-started/', text: 'Getting Started', internal: true },
    { to: '/modes/editing-frontmatter/', text: 'Properties', internal: true, anchor: '#keys' },
    { to: 'https://example.com/', text: 'Example', internal: false },
  ],
}

describe('MbrFuzzyNavElement in a static build under a path prefix', () => {
  let nav: HTMLElement
  let fetchMock: ReturnType<typeof vi.fn>

  beforeEach(() => {
    window.headings = []
    window.frontmatter = { markdown_source: '/x/' }
    fetchMock = vi.fn().mockResolvedValue(okResponse(LINKS))
    vi.stubGlobal('fetch', fetchMock)
    nav = document.createElement('mbr-fuzzy-nav')
    document.body.appendChild(nav)
  })

  afterEach(() => {
    nav.remove()
    vi.unstubAllGlobals()
    window.__MBR_CONFIG__ = undefined
    window.history.pushState({}, '', '/')
  })

  function linksRequests(): string[] {
    return fetchMock.mock.calls
      .map((args) => String(args[0]))
      .filter((url) => url.endsWith('links.json'))
  }

  async function openTab(tab: 'links-out' | 'links-in'): Promise<string[]> {
    const el = nav as unknown as { open(tab: string): void; shadowRoot: ShadowRoot }
    el.open(tab)
    await flush(nav)
    return Array.from(el.shadowRoot.querySelectorAll('a.result-item')).map(
      (a) => a.getAttribute('href') ?? ''
    )
  }

  // [pathname, basePath (the build's relative_root), links.json request,
  //  prefix the page's own relative hrefs need to reach the site root]
  const cases: Array<[string, string, string, string, string]> = [
    ['root index page', `${PREFIX}/`, '', './links.json', './'],
    ['nested index page', `${PREFIX}/modes/`, '../', '../modes/links.json', '../'],
    ['non-index page', `${PREFIX}/modes/editing/`, '../../', '../../modes/editing/links.json', '../../'],
  ]

  for (const [label, pathname, basePath, request, toRoot] of cases) {
    it(`lists and links the ${label}'s outbound links`, async () => {
      window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath }
      window.history.pushState({}, '', pathname)

      const hrefs = await openTab('links-out')

      expect(linksRequests()).toEqual([request])
      expect(hrefs).toEqual([
        `${toRoot}getting-started/`,
        `${toRoot}modes/editing-frontmatter/#keys`,
        'https://example.com/',
      ])
      // Every href, resolved against the page, lands inside the deployment.
      for (const href of hrefs.slice(0, 2)) {
        expect(new URL(href, `https://host${pathname}`).pathname.startsWith(`${PREFIX}/`)).toBe(true)
      }
    })
  }

  it('links inbound sources relative to the page too', async () => {
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../../' }
    window.history.pushState({}, '', `${PREFIX}/notes/inbound-case/`)

    expect(await openTab('links-in')).toEqual(['../../modes/'])
  })
})
