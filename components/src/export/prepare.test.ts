import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { EXPORT_CSS_FILE, EXPORT_PAGE_STYLE_ID, UNLINKED_CLASS } from './contract.ts'
import {
  applyLinkPolicy,
  forceEagerImages,
  linkAction,
  pageRule,
  parseExportOptions,
  prepareForExport,
  type ExportDeps,
  type LinkPolicy,
} from './prepare.ts'

/** The services the main-bundle trigger injects, as spies. */
const mocks = {
  mermaid: vi.fn(() => Promise.resolve()),
  math: vi.fn(() => Promise.resolve()),
  hljs: vi.fn(() => Promise.resolve()),
  loadCss: vi.fn((_href: string) => Promise.resolve()),
}

const PAGE = new URL('http://127.0.0.1:5611/notes/page/?v=1')
const STRIP: LinkPolicy = { kind: 'strip' }
const BASE: LinkPolicy = { kind: 'base', base: new URL('https://notes.example.com/site/') }

const FLAG = { links: 'strip', paperWidthIn: 8.5, paperHeightIn: 11, margin: '0.6in 0.5in' }

function deps(overrides: Partial<ExportDeps> = {}): ExportDeps {
  return {
    doc: document,
    pageUrl: () => PAGE,
    waitForDom: () => Promise.resolve(),
    loadCss: mocks.loadCss,
    assetBase: () => '/.mbr/',
    settled: { mermaid: mocks.mermaid, math: mocks.math, hljs: mocks.hljs },
    ...overrides,
  }
}

/** An image whose load state the test controls. */
function image(src: string, state: 'loading' | 'loaded' | 'broken' = 'loaded'): HTMLImageElement {
  const img = document.createElement('img')
  img.setAttribute('src', src)
  Object.defineProperty(img, 'complete', { configurable: true, get: () => state !== 'loading' })
  Object.defineProperty(img, 'naturalWidth', { configurable: true, get: () => (state === 'loaded' ? 10 : 0) })
  document.body.appendChild(img)
  return img
}

beforeEach(() => {
  document.head.innerHTML = ''
  document.body.innerHTML = ''
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
  vi.clearAllMocks()
  for (const m of Object.values(mocks)) m.mockImplementation(() => Promise.resolve())
})

describe('UNIT parseExportOptions', () => {
  it('accepts the contract shape, strip and base', () => {
    expect(parseExportOptions(FLAG)).toEqual({
      links: { kind: 'strip' },
      paperWidthIn: 8.5,
      paperHeightIn: 11,
      margin: '0.6in 0.5in',
    })
    const based = parseExportOptions({ ...FLAG, links: { base: 'https://notes.example.com/' } })
    expect(based.links).toEqual({ kind: 'base', base: new URL('https://notes.example.com/') })
  })

  it.each([
    ['not an object', null],
    ['unknown link policy', { ...FLAG, links: 'keep' }],
    ['base without trailing slash', { ...FLAG, links: { base: 'https://x.example/a' } }],
    ['base with a script scheme', { ...FLAG, links: { base: 'javascript:alert(1)/' } }],
    ['zero width', { ...FLAG, paperWidthIn: 0 }],
    ['string height', { ...FLAG, paperHeightIn: '11' }],
    ['margin escaping its rule', { ...FLAG, margin: '0 } body { display: none' }],
    ['margin closing the style', { ...FLAG, margin: '</style>' }],
  ])('refuses %s', (_name, raw) => {
    expect(() => parseExportOptions(raw)).toThrow()
  })

  it('writes the @page rule from the options', () => {
    expect(pageRule(parseExportOptions({ ...FLAG, paperWidthIn: 11.69, paperHeightIn: 8.27 }))).toBe(
      '@page { size: 11.69in 8.27in; margin: 0.6in 0.5in }'
    )
  })
})

describe('UNIT linkAction', () => {
  it.each([
    ['#section', STRIP, { kind: 'keep' }],
    ['#section', BASE, { kind: 'keep' }],
    ['https://example.com/x', STRIP, { kind: 'keep' }],
    ['http://example.com/x', BASE, { kind: 'keep' }],
    ['mailto:me@example.com', STRIP, { kind: 'keep' }],
    ['mailto:me@example.com', BASE, { kind: 'keep' }],
    ['javascript:alert(1)', STRIP, { kind: 'strip' }],
    ['data:text/html,x', BASE, { kind: 'strip' }],
    ['/other/', STRIP, { kind: 'strip' }],
    ['../sibling/', STRIP, { kind: 'strip' }],
    ['http://127.0.0.1:5611/other/', STRIP, { kind: 'strip' }],
    ['/.mbr/outside/?path=~/x.md', STRIP, { kind: 'strip' }],
    ['/.mbr/outside/?path=~/x.md', BASE, { kind: 'strip' }],
    ['../../.mbr/outside/?path=%2Fetc', BASE, { kind: 'strip' }],
    ['/other/?q=1#top', BASE, { kind: 'rewrite', href: 'https://notes.example.com/site/other/?q=1#top' }],
    ['../sibling/', BASE, { kind: 'rewrite', href: 'https://notes.example.com/site/notes/sibling/' }],
    ['/.mbr/videos/?path=v.mp4', BASE, { kind: 'rewrite', href: 'https://notes.example.com/site/.mbr/videos/?path=v.mp4' }],
    // This very page with a fragment is a same-page link.
    ['/notes/page/?v=1#h', STRIP, { kind: 'rewrite', href: '#h' }],
    ['/notes/page/?v=1#h', BASE, { kind: 'rewrite', href: '#h' }],
    // ...without one, it is a link to a page like any other.
    ['/notes/page/?v=1', STRIP, { kind: 'strip' }],
  ] as const)('%s (%o)', (href, policy, expected) => {
    expect(linkAction(href, PAGE, policy)).toEqual(expected)
  })
})

describe('UNIT applyLinkPolicy', () => {
  it('strips internal links to text, keeps the rest, and is idempotent', () => {
    document.body.innerHTML = `
      <a id="int" href="/other/" target="_blank">internal</a>
      <a id="out" href="/.mbr/outside/?path=~/x">outside</a>
      <a id="frag" href="#h">fragment</a>
      <a id="ext" href="https://example.com/">external</a>
      <a id="mail" href="mailto:me@example.com">mail</a>`
    applyLinkPolicy(document, PAGE, STRIP)
    applyLinkPolicy(document, PAGE, STRIP)
    for (const id of ['int', 'out']) {
      const a = document.getElementById(id)!
      expect(a.hasAttribute('href')).toBe(false)
      expect(a.hasAttribute('target')).toBe(false)
      expect(a.classList.contains(UNLINKED_CLASS)).toBe(true)
      expect(a.textContent).not.toBe('')
    }
    expect(document.getElementById('frag')!.getAttribute('href')).toBe('#h')
    expect(document.getElementById('ext')!.getAttribute('href')).toBe('https://example.com/')
    expect(document.getElementById('mail')!.getAttribute('href')).toBe('mailto:me@example.com')
  })

  it('rebases internal links onto the published site', () => {
    document.body.innerHTML = '<a id="int" href="/a/b/?q#f">x</a><a id="out" href="/.mbr/outside/?path=/tmp">y</a>'
    applyLinkPolicy(document, PAGE, BASE)
    applyLinkPolicy(document, PAGE, BASE)
    expect(document.getElementById('int')!.getAttribute('href')).toBe('https://notes.example.com/site/a/b/?q#f')
    expect(document.getElementById('out')!.hasAttribute('href')).toBe(false)
  })
})

describe('UNIT forceEagerImages', () => {
  it('turns every lazy image eager and leaves the rest', () => {
    document.body.innerHTML = '<img loading="lazy" src="a.png"><img loading="LAZY" src="b.png"><img src="c.png">'
    expect(forceEagerImages(document)).toBe(2)
    const loading = Array.from(document.querySelectorAll('img')).map((img) => img.getAttribute('loading'))
    expect(loading).toEqual(['eager', 'eager', null])
  })
})

describe('UNIT prepareForExport', () => {
  it('prepares the page: @page rule, export.css, eager images, link policy, every loader awaited', async () => {
    document.body.innerHTML = '<main><a href="/x/">x</a><img loading="lazy" src="a.png"></main>'
    const img = document.querySelector('img')!
    Object.defineProperty(img, 'complete', { get: () => true })
    Object.defineProperty(img, 'naturalWidth', { get: () => 4 })

    await expect(prepareForExport(FLAG, deps())).resolves.toEqual({ warnings: [] })

    expect(document.getElementById(EXPORT_PAGE_STYLE_ID)?.textContent).toBe(
      '@page { size: 8.5in 11in; margin: 0.6in 0.5in }'
    )
    expect(mocks.loadCss).toHaveBeenCalledWith(`/.mbr/${EXPORT_CSS_FILE}`)
    expect(img.getAttribute('loading')).toBe('eager')
    expect(document.querySelector('a')!.hasAttribute('href')).toBe(false)
    for (const settled of [mocks.mermaid, mocks.math, mocks.hljs]) expect(settled).toHaveBeenCalled()
  })

  it('rejects a malformed flag before touching the page', async () => {
    await expect(prepareForExport({ ...FLAG, margin: '}' }, deps())).rejects.toThrow(/margin/)
    expect(document.getElementById(EXPORT_PAGE_STYLE_ID)).toBeNull()
  })

  it('reports failed and stuck images and stalled loaders as warnings, not failure', async () => {
    vi.useFakeTimers()
    image('ok.png', 'loaded')
    image('broken.png', 'broken')
    image('stuck.png', 'loading')
    const late = image('late-error.png', 'loading')
    mocks.mermaid.mockReturnValueOnce(new Promise(() => {}))
    mocks.loadCss.mockImplementationOnce(() => Promise.reject(new Error('404')))

    const run = prepareForExport(FLAG, deps())
    await vi.advanceTimersByTimeAsync(1)
    late.dispatchEvent(new Event('error'))
    await vi.advanceTimersByTimeAsync(15_000)
    const { warnings } = await run

    expect(warnings).toEqual(
      expect.arrayContaining([
        'export stylesheet failed to load: /.mbr/export.css',
        expect.stringMatching(/^image failed to load: \S*\/broken\.png$/),
        expect.stringMatching(/^image failed to load: \S*\/late-error\.png$/),
        expect.stringMatching(/^1 image\(s\) still loading after 10 s: \S*\/stuck\.png$/),
        'diagrams (mermaid) not finished after 15 s',
      ])
    )
    expect(warnings).toHaveLength(5)
  })

  it('takes the caps from its deps', async () => {
    vi.useFakeTimers()
    image('stuck.png', 'loading')
    mocks.hljs.mockReturnValueOnce(new Promise(() => {}))
    const run = prepareForExport(FLAG, deps({ imageCapMs: 2_000, settleCapMs: 3_000 }))
    await vi.advanceTimersByTimeAsync(3_000)
    const { warnings } = await run
    expect(warnings).toEqual([
      expect.stringMatching(/^1 image\(s\) still loading after 2 s: /),
      'code highlighting not finished after 3 s',
    ])
  })

  it('catches links and images added while rendering', async () => {
    let finishDiagram!: () => void
    mocks.mermaid.mockReturnValueOnce(
      new Promise<void>((resolve) => {
        finishDiagram = () => {
          document.body.insertAdjacentHTML('beforeend', '<a id="late" href="/late/">late</a>')
          image('late.png', 'loaded').setAttribute('loading', 'lazy')
          resolve()
        }
      })
    )
    const run = prepareForExport(FLAG, deps())
    await new Promise((resolve) => setTimeout(resolve, 0))
    await new Promise((resolve) => setTimeout(resolve, 0))
    finishDiagram()
    await run
    expect(document.getElementById('late')!.hasAttribute('href')).toBe(false)
    expect(document.querySelector('img')!.getAttribute('loading')).toBe('eager')
  })

  it('never uses requestAnimationFrame', async () => {
    const raf = vi.fn()
    vi.stubGlobal('requestAnimationFrame', raf)
    image('a.png', 'loaded')
    await prepareForExport(FLAG, deps())
    expect(raf).not.toHaveBeenCalled()
  })
})
