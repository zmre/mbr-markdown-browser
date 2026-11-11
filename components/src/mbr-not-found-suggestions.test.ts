import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  path: '/projects/gamma/',
  site: { index_file: 'index.md', markdown_files: [] as unknown[] } as unknown,
  media: { other_files: [] as unknown[] } as unknown,
  mediaLoads: 0,
  static: false,
}))

vi.mock('./shared.js', () => ({
  getCanonicalPath: () => mocks.path,
  get siteNav() {
    return Promise.resolve(mocks.site)
  },
  loadMediaNav: () => {
    mocks.mediaLoads++
    return Promise.resolve(mocks.media)
  },
  // Static mode under a deployment prefix: root-anchored, like 404.html's basePath.
  resolveUrl: (p: string) => (mocks.static ? `/my-repo${p}` : p),
}))

import { setNotFoundChunkImporter } from './not-found-chunk.js'
import './mbr-not-found-suggestions.js'

const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

async function mount(): Promise<HTMLElement> {
  document.body.innerHTML = '<article><mbr-not-found-suggestions></mbr-not-found-suggestions></article>'
  // The chunk is a real dynamic import; make sure it has resolved first.
  await import('./not-found/index.js')
  for (let i = 0; i < 10; i++) await settle()
  return document.querySelector('mbr-not-found-suggestions')!
}

const md = (raw: string, url: string, title?: string) => ({
  raw_path: raw,
  url_path: url,
  frontmatter: title ? { title } : {},
})

describe('<mbr-not-found-suggestions>', () => {
  beforeEach(() => {
    setNotFoundChunkImporter(() => import('./not-found/index.js'))
    mocks.path = '/projects/gamma/'
    mocks.static = false
    mocks.mediaLoads = 0
    mocks.media = { other_files: [] }
  })
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('lists where a moved note went', async () => {
    mocks.site = {
      index_file: 'index.md',
      markdown_files: [md('archive/gamma.md', '/archive/gamma/', 'Gamma'), md('projects/beta.md', '/projects/beta/')],
    }
    const el = await mount()
    const links = [...el.querySelectorAll('a')]
    expect(links.map((a) => a.getAttribute('href'))).toEqual(['/archive/gamma/'])
    expect(links[0].textContent).toBe('Gamma')
    expect(el.querySelector('.mbr-not-found-where')?.textContent).toBe('/archive/')
    expect(el.textContent).toContain('Maybe you were looking for')
    // A note target never needs media.json.
    expect(mocks.mediaLoads).toBe(0)
  })

  it('links through resolveUrl (static builds under a prefix)', async () => {
    mocks.static = true
    mocks.site = { index_file: 'index.md', markdown_files: [md('archive/gamma.md', '/archive/gamma/')] }
    const el = await mount()
    expect(el.querySelector('a')?.getAttribute('href')).toBe('/my-repo/archive/gamma/')
  })

  it('falls back to the notes left in the folder', async () => {
    mocks.site = {
      index_file: 'index.md',
      markdown_files: [md('projects/index.md', '/projects/', 'Projects'), md('projects/beta.md', '/projects/beta/', 'Beta')],
    }
    const el = await mount()
    expect(el.textContent).toContain('Its folder is still here')
    expect([...el.querySelectorAll('a')].map((a) => a.getAttribute('href'))).toEqual(['/projects/', '/projects/beta/'])
  })

  it('stays empty when there is nothing to suggest', async () => {
    mocks.path = '/gone/entirely/'
    mocks.site = { index_file: 'index.md', markdown_files: [md('a.md', '/a/')] }
    const el = await mount()
    expect(el.childNodes).toHaveLength(0)
  })

  it('reads media.json only for a URL naming a file', async () => {
    mocks.path = '/docs/report.pdf'
    mocks.site = { index_file: 'index.md', markdown_files: [] }
    mocks.media = { other_files: [{ url_path: '/archive/report.pdf' }] }
    const el = await mount()
    expect(mocks.mediaLoads).toBe(1)
    expect(el.querySelector('a')?.getAttribute('href')).toBe('/archive/report.pdf')
  })

  it('stays empty when the chunk fails to load', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    setNotFoundChunkImporter(() => Promise.reject(new Error('offline')))
    mocks.site = { index_file: 'index.md', markdown_files: [md('archive/gamma.md', '/archive/gamma/')] }
    const el = await mount()
    expect(el.childNodes).toHaveLength(0)
    warn.mockRestore()
  })
})

/** The chunk's own sources, read raw (calendar-chunk.test.ts's approach). */
const CHUNK_SOURCES = import.meta.glob('./not-found/*.ts', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

describe('mbr-not-found.min.js chunk boundary', () => {
  it('imports no main-bundle module', () => {
    const files = Object.entries(CHUNK_SOURCES).filter(([name]) => !name.endsWith('.test.ts'))
    expect(files.length).toBeGreaterThan(0)
    for (const [file, source] of files) {
      const imports = [...source.matchAll(/^\s*(?:import|export)\s+(?!type\b)[^'"]*from\s+['"]([^'"]+)['"]/gm)].map(
        (m) => m[1],
      )
      expect(imports.filter((spec) => !spec.startsWith('./')), file).toEqual([])
    }
  })
})
