/**
 * The page-errors panel's "Did you mean" links for broken internal links,
 * from the not-found chunk's matcher over site.json.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  site: {
    index_file: 'index.md',
    markdown_files: [
      { raw_path: 'archive/plan.md', url_path: '/archive/plan/', frontmatter: { title: 'The Plan' } },
      { raw_path: 'docs/exists.md', url_path: '/docs/exists/', frontmatter: {} },
      { raw_path: 'docs/page.md', url_path: '/docs/page/', frontmatter: {} },
    ],
  },
}))

vi.mock('./shared.js', () => ({
  safeDecodePath: (p: string) => decodeURIComponent(p),
  get siteNav() {
    return Promise.resolve(mocks.site)
  },
}))

import { setNotFoundChunkImporter } from './not-found-chunk.js'
import { linkTargetPath, type MbrPageErrorsElement } from './mbr-page-errors.js'

const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('UNIT linkTargetPath', () => {
  it('resolves a rendered href against the page and decodes it', () => {
    expect(linkTargetPath('../plan/', 'http://h/docs/page/')).toBe('/docs/plan/')
    expect(linkTargetPath('/a/My%20Note/', 'http://h/docs/page/')).toBe('/a/My Note/')
    expect(linkTargetPath('sub/x/?q=1#frag', 'http://h/docs/page/')).toBe('/docs/page/sub/x/')
  })
})

describe('<mbr-page-errors> did you mean', () => {
  let el: MbrPageErrorsElement
  const originalFetch = globalThis.fetch
  let imports = 0

  async function mount(errors: unknown[]): Promise<void> {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ page_url: '/docs/page/', errors }),
    }) as unknown as typeof fetch
    el = document.createElement('mbr-page-errors')
    document.body.appendChild(el)
    await settle()
    await el.updateComplete
  }

  async function openPanel(): Promise<void> {
    ;(el as unknown as { _isOpen: boolean })._isOpen = true
    await el.updateComplete
    for (let i = 0; i < 5; i++) await settle()
    await el.updateComplete
  }

  beforeEach(() => {
    imports = 0
    setNotFoundChunkImporter(() => {
      imports++
      return import('./not-found/index.js')
    })
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    window.history.replaceState(null, '', '/docs/page/')
  })

  afterEach(() => {
    document.body.innerHTML = ''
    window.__MBR_CONFIG__ = undefined
    globalThis.fetch = originalFetch
  })

  it('suggests where a missing link target went', async () => {
    await mount([{ type: 'broken_internal_link', target: '../plan/', text: 'plan' }])
    expect(imports).toBe(0)
    await openPanel()
    // The chunk is a real dynamic import: wait for it rather than for ticks.
    const link = await vi.waitFor(() => {
      const a = el.shadowRoot?.querySelector<HTMLAnchorElement>('.did-you-mean a')
      if (!a) throw new Error('no suggestion yet')
      return a
    })
    expect(link.getAttribute('href')).toBe('/archive/plan/')
    expect(link.textContent).toBe('The Plan')
    expect(el.shadowRoot?.querySelector('.did-you-mean')?.textContent?.replace(/\s+/g, ' ')).toContain('Did you mean')
  })

  it('suggests nothing for a target that exists (a missing trailing slash)', async () => {
    await mount([{ type: 'broken_internal_link', target: '../exists', text: 'exists' }])
    await openPanel()
    expect(imports).toBe(1)
    // Let the import and the match finish before asserting the absence.
    await import('./not-found/index.js')
    for (let i = 0; i < 5; i++) await settle()
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('.did-you-mean')).toBeNull()
  })

  it('never loads the chunk without broken links', async () => {
    await mount([{ type: 'unresolved_wikilink', raw: '[[x]]' }])
    await openPanel()
    expect(imports).toBe(0)
  })
})
