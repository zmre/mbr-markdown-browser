import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { EXPORT_PAGE_STYLE_ID, EXPORT_READY_GLOBAL, IPC_EXPORT_READY_PREFIX } from './export/contract.ts'
import * as exportChunk from './export/index.ts'
import {
  assetBaseFromScript,
  bootStandaloneExport,
  domMermaidSettled,
  loadStylesheet,
  standaloneDeps,
} from './export-ready-standalone.ts'
import { resetExportRunForTests, startExportRun } from './export-run.ts'

const FLAG = { links: 'strip', paperWidthIn: 8.5, paperHeightIn: 11, margin: '0.6in 0.5in' }

function stubIpc(): string[] {
  const posted: string[] = []
  vi.stubGlobal('ipc', { postMessage: (m: string) => posted.push(m) })
  return posted
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((resolve) => setTimeout(resolve, 0))
}

/** The chunk from source. */
const importer = vi.fn(() => Promise.resolve(exportChunk))

/**
 * The standalone services, with the stylesheet loader recorded instead of
 * run: happy-dom refuses to load CSS files (an error event).
 */
const cssLoads: string[] = []
const testDeps = (assetBase: string) => ({
  ...standaloneDeps(assetBase),
  loadCss: (href: string) => {
    cssLoads.push(href)
    return Promise.resolve()
  },
})

beforeEach(() => {
  document.head.innerHTML = ''
  document.body.innerHTML = ''
  cssLoads.length = 0
  resetExportRunForTests()
})

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.clearAllMocks()
  vi.useRealTimers()
  delete window.__MBR_EXPORT__
  delete (window as unknown as Record<string, unknown>)['mermaid']
  resetExportRunForTests()
})

describe('UNIT assetBaseFromScript', () => {
  it.each([
    ['http://127.0.0.1:5000/.mbr/components/mbr-export-ready.min.js', 'http://127.0.0.1:5000/.mbr/'],
    ['https://example.com/site/.mbr/components/mbr-export-ready.min.js', 'https://example.com/site/.mbr/'],
    ['http://127.0.0.1:5000/.mbr/components/mbr-export-ready.min.js?v=2', 'http://127.0.0.1:5000/.mbr/'],
    ['http://127.0.0.1:5000/vendor/other.js', '/.mbr/'],
    ['not a url', '/.mbr/'],
    ['', '/.mbr/'],
    [null, '/.mbr/'],
    [undefined, '/.mbr/'],
  ])('%s → %s', (src, expected) => {
    expect(assetBaseFromScript(src)).toBe(expected)
  })
})

describe('UNIT domMermaidSettled', () => {
  it('settles at once when the page loads no mermaid', async () => {
    document.body.innerHTML = '<pre class="mermaid">graph TD</pre>'
    await expect(domMermaidSettled(document, {})).resolves.toBeUndefined()
  })

  it('waits until every diagram is drawn, polling with timers only', async () => {
    vi.useFakeTimers()
    const raf = vi.fn()
    vi.stubGlobal('requestAnimationFrame', raf)
    document.body.innerHTML = '<pre class="mermaid" id="a">graph TD</pre><div class="mermaid" id="b"><svg></svg></div>'
    let settled = false
    void domMermaidSettled(document, { mermaid: {} }, 50).then(() => (settled = true))
    await vi.advanceTimersByTimeAsync(500)
    expect(settled).toBe(false)
    document.getElementById('a')!.setAttribute('data-processed', 'true')
    await vi.advanceTimersByTimeAsync(50)
    expect(settled).toBe(true)
    expect(raf).not.toHaveBeenCalled()
  })
})

describe('UNIT loadStylesheet', () => {
  it('adds the sheet, and rejects when it cannot load (the export warns)', async () => {
    await expect(loadStylesheet('/.mbr/export.css')).rejects.toThrow('failed to load /.mbr/export.css')
    expect(document.querySelectorAll('link[rel="stylesheet"]')).toHaveLength(1)
  })

  it('does not add a sheet the page already links', async () => {
    document.head.innerHTML = '<link rel="stylesheet" href="/.mbr/export.css">'
    await expect(loadStylesheet('/.mbr/export.css')).resolves.toBeUndefined()
    expect(document.querySelectorAll('link[rel="stylesheet"]')).toHaveLength(1)
  })
})

describe('UNIT standaloneDeps', () => {
  it('uses the asset base it was given and settles math and highlighting at once', async () => {
    const deps = standaloneDeps('http://h/.mbr/')
    expect(deps.assetBase()).toBe('http://h/.mbr/')
    await expect(deps.settled.math()).resolves.toBeUndefined()
    await expect(deps.settled.hljs()).resolves.toBeUndefined()
    await expect(deps.settled.mermaid()).resolves.toBeUndefined()
  })
})

describe('INTEGRATION bootStandaloneExport', () => {
  const SRC = 'http://127.0.0.1:5000/.mbr/components/mbr-export-ready.min.js'

  it('does nothing outside an export', async () => {
    const posted = stubIpc()
    await expect(bootStandaloneExport(SRC, importer)).resolves.toBeNull()
    await flush()
    expect(importer).not.toHaveBeenCalled()
    expect(posted).toEqual([])
    expect(window.mbrExportReady).toBeUndefined()
  })

  it('runs the export chunk and posts one ready message, as the main bundle would', async () => {
    const posted = stubIpc()
    window.__MBR_EXPORT__ = FLAG
    await expect(bootStandaloneExport(SRC, importer, testDeps)).resolves.toEqual({ warnings: [] })
    await flush()
    expect(importer).toHaveBeenCalledTimes(1)
    expect(posted).toEqual([`${IPC_EXPORT_READY_PREFIX}{"warnings":[]}`])
    expect(document.getElementById(EXPORT_PAGE_STYLE_ID)?.textContent).toBe(
      '@page { size: 8.5in 11in; margin: 0.6in 0.5in }'
    )
    // export.css from the asset base this script was served from.
    expect(cssLoads).toEqual(['http://127.0.0.1:5000/.mbr/export.css'])
    await expect(window.mbrExportReady!()).resolves.toEqual({ warnings: [] })
  })

  it('stands aside when the main bundle already started the run', async () => {
    const posted = stubIpc()
    window.__MBR_EXPORT__ = FLAG
    const mainRun = Promise.resolve({ warnings: ['from the main bundle'] })
    ;(window as unknown as Record<string, unknown>)[EXPORT_READY_GLOBAL] = () => mainRun
    await expect(bootStandaloneExport(SRC, importer, testDeps)).resolves.toEqual({ warnings: ['from the main bundle'] })
    await flush()
    expect(importer).not.toHaveBeenCalled()
    expect(posted).toEqual([])
  })

  it('leaves `ready = "load"` to Rust`s own script', async () => {
    const posted = stubIpc()
    window.__MBR_EXPORT__ = { ...FLAG, ready: 'load' }
    await expect(bootStandaloneExport(SRC, importer)).resolves.toBeNull()
    await flush()
    expect(importer).not.toHaveBeenCalled()
    expect(posted).toEqual([])
    expect(window.mbrExportReady).toBeUndefined()
  })
})

describe('UNIT startExportRun', () => {
  it('runs once per page, whichever bundle asks', async () => {
    stubIpc()
    const deps = standaloneDeps('/.mbr/')
    const first = startExportRun(FLAG, deps, importer)
    const second = startExportRun(FLAG, deps, importer)
    expect(first).not.toBeNull()
    await expect(second).resolves.toEqual(await first)
    expect(importer).toHaveBeenCalledTimes(1)
  })
})
