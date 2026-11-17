import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

/**
 * The loaders define custom elements, which cannot be defined twice, and the
 * module-load tests re-import the trigger; stand them in with their settle
 * functions only. The dynamic loader would fetch over the network.
 */
const mocks = vi.hoisted(() => ({
  mermaid: vi.fn(() => Promise.resolve()),
  math: vi.fn(() => Promise.resolve()),
  hljs: vi.fn(() => Promise.resolve()),
  loadCss: vi.fn((_href: string) => Promise.resolve()),
}))

vi.mock('./mbr-mermaid.ts', () => ({ mermaidSettled: mocks.mermaid }))
vi.mock('./mbr-katex.ts', () => ({ mathSettled: mocks.math }))
vi.mock('./mbr-hljs.ts', () => ({ hljsSettled: mocks.hljs }))
vi.mock('./mbr-editor.ts', () => ({ EDITOR_STATE_EVENT: 'mbr-editor-state' }))
vi.mock('./dynamic-loader.ts', () => ({
  waitForDom: () => Promise.resolve(),
  loadCss: mocks.loadCss,
  getMbrAssetBase: () => '/.mbr/',
  scheduleIdleTask: (task: () => void) => setTimeout(task, 0),
}))

import {
  EXPORT_PAGE_STYLE_ID,
  EXPORT_READY_GLOBAL,
  IPC_EDITOR_STATE_PREFIX,
  IPC_EXPORT_FAILED_PREFIX,
  IPC_EXPORT_READY_PREFIX,
} from './export/contract.ts'
import * as exportChunk from './export/index.ts'
import { setExportChunkImporter } from './export-chunk.ts'
import { EXPORT_FLAG_GLOBAL } from './export-mode.ts'
import { exportDeps, installEditorStateRelay, resetExportReadyForTests, startExportReady } from './export-ready.ts'

declare const __MBR_PDF_EXPORT_RS__: string | null

const FLAG = { links: 'strip', paperWidthIn: 8.5, paperHeightIn: 11, margin: '0.6in 0.5in' }

/** wry's `window.ipc`, recording what the page posts. */
function stubIpc(): string[] {
  const posted: string[] = []
  vi.stubGlobal('ipc', { postMessage: (m: string) => posted.push(m) })
  return posted
}

/** Let the run's promise chains finish. */
async function flush(): Promise<void> {
  for (let i = 0; i < 6; i++) await new Promise((resolve) => setTimeout(resolve, 0))
}

/** The chunk, served from source rather than fetched. */
const importer = vi.fn(() => Promise.resolve(exportChunk))

beforeEach(() => {
  document.head.innerHTML = ''
  document.body.innerHTML = ''
  resetExportReadyForTests()
  setExportChunkImporter(importer)
})

afterEach(() => {
  vi.unstubAllGlobals()
  vi.clearAllMocks()
  delete window.__MBR_EXPORT__
  delete window.__MBR_CONFIG__
  resetExportReadyForTests()
  setExportChunkImporter(null)
})

describe('UNIT startExportReady', () => {
  it('loads the chunk with this page`s services and posts ready exactly once', async () => {
    const posted = stubIpc()
    await expect(startExportReady(FLAG)).resolves.toEqual({ warnings: [] })
    await flush()

    expect(importer).toHaveBeenCalledTimes(1)
    expect(posted).toEqual([`${IPC_EXPORT_READY_PREFIX}{"warnings":[]}`])
    expect(document.getElementById(EXPORT_PAGE_STYLE_ID)).not.toBeNull()
    for (const settled of [mocks.mermaid, mocks.math, mocks.hljs]) expect(settled).toHaveBeenCalled()

    // A second start, or the global, hands back the same run; nothing more is posted.
    const again = startExportReady(FLAG)
    const global = (window as unknown as Record<string, () => Promise<unknown>>)[EXPORT_READY_GLOBAL]
    expect(again).toBe(global())
    await again
    await flush()
    expect(posted).toHaveLength(1)
    expect(importer).toHaveBeenCalledTimes(1)
  })

  it('posts the chunk`s warnings', async () => {
    const posted = stubIpc()
    vi.useFakeTimers()
    mocks.math.mockReturnValueOnce(new Promise(() => {}))
    const run = startExportReady(FLAG)
    await vi.advanceTimersByTimeAsync(15_000)
    const { warnings } = await run
    await vi.advanceTimersByTimeAsync(1)
    vi.useRealTimers()
    expect(warnings).toEqual(['math (KaTeX) not finished after 15 s'])
    expect(posted).toEqual([IPC_EXPORT_READY_PREFIX + JSON.stringify({ warnings })])
  })

  it('posts failed for a malformed flag, and mbrExportReady rejects', async () => {
    const posted = stubIpc()
    const run = startExportReady({ ...FLAG, margin: '}' })
    await expect(window.mbrExportReady!()).rejects.toThrow(/margin/)
    await expect(run).rejects.toThrow(/margin/)
    await flush()
    expect(posted).toHaveLength(1)
    expect(posted[0].startsWith(IPC_EXPORT_FAILED_PREFIX)).toBe(true)
    expect(posted[0]).toContain('margin')
  })

  it('posts failed when the chunk cannot be loaded', async () => {
    const posted = stubIpc()
    setExportChunkImporter(() => Promise.reject(new Error('Failed to fetch mbr-export.min.js')))
    await expect(startExportReady(FLAG)).rejects.toThrow()
    await flush()
    expect(posted).toEqual([`${IPC_EXPORT_FAILED_PREFIX}Failed to fetch mbr-export.min.js`])
  })

  it('caps the failure message', async () => {
    const posted = stubIpc()
    setExportChunkImporter(() => Promise.reject(new Error('x'.repeat(5000))))
    await startExportReady(FLAG).catch(() => {})
    await flush()
    expect(posted[0].length).toBe(IPC_EXPORT_FAILED_PREFIX.length + 300)
  })

  it('never uses requestAnimationFrame', async () => {
    const raf = vi.fn()
    vi.stubGlobal('requestAnimationFrame', raf)
    stubIpc()
    await startExportReady(FLAG)
    await flush()
    expect(raf).not.toHaveBeenCalled()
  })

  it('without wry, still resolves (a plain browser posts nowhere)', async () => {
    await expect(startExportReady(FLAG)).resolves.toEqual({ warnings: [] })
  })

  it('hands the chunk the main bundle`s loaders', () => {
    const deps = exportDeps()
    expect(deps.settled).toEqual({ mermaid: mocks.mermaid, math: mocks.math, hljs: mocks.hljs })
    expect(deps.loadCss).toBe(mocks.loadCss)
    expect(deps.assetBase()).toBe('/.mbr/')
  })
})

describe('INTEGRATION module load', () => {
  it('does nothing without the flag: no chunk, no global, no message', async () => {
    vi.resetModules()
    const posted = stubIpc()
    const loader = await import('./export-chunk.ts')
    const fresh = vi.fn(() => Promise.resolve(exportChunk))
    loader.setExportChunkImporter(fresh)
    await import('./export-ready.ts')
    await flush()
    expect(fresh).not.toHaveBeenCalled()
    expect(posted).toEqual([])
    expect(window.mbrExportReady).toBeUndefined()
  })

  it('starts itself when Rust set the flag', async () => {
    vi.resetModules()
    const posted = stubIpc()
    window.__MBR_EXPORT__ = FLAG
    const loader = await import('./export-chunk.ts')
    loader.setExportChunkImporter(() => Promise.resolve(exportChunk))
    const mod = await import('./export-ready.ts')
    await expect(window.mbrExportReady!()).resolves.toEqual({ warnings: [] })
    await flush()
    expect(posted).toEqual([`${IPC_EXPORT_READY_PREFIX}{"warnings":[]}`])
    mod.resetExportReadyForTests()
  })
})

describe('UNIT installEditorStateRelay', () => {
  const editorState = (open: boolean) =>
    document.dispatchEvent(new CustomEvent('mbr-editor-state', { detail: { open } }))

  it('relays open and closed in GUI windows', () => {
    window.__MBR_CONFIG__ = { guiMode: true } as Window['__MBR_CONFIG__']
    const posted = stubIpc()
    const uninstall = installEditorStateRelay()
    editorState(true)
    editorState(false)
    uninstall()
    editorState(true)
    expect(posted).toEqual([`${IPC_EDITOR_STATE_PREFIX}open`, `${IPC_EDITOR_STATE_PREFIX}closed`])
  })

  it('stays silent outside the GUI', () => {
    window.__MBR_CONFIG__ = { guiMode: false } as Window['__MBR_CONFIG__']
    const posted = stubIpc()
    const uninstall = installEditorStateRelay()
    editorState(true)
    uninstall()
    expect(posted).toEqual([])
  })

  it('tolerates a GUI page without window.ipc', () => {
    window.__MBR_CONFIG__ = { guiMode: true } as Window['__MBR_CONFIG__']
    const uninstall = installEditorStateRelay()
    expect(() => editorState(true)).not.toThrow()
    uninstall()
  })
})

describe('CONTRACT names shared with src/pdf_export/mod.rs', () => {
  const names = {
    flag: EXPORT_FLAG_GLOBAL,
    ready: EXPORT_READY_GLOBAL,
    readyPrefix: IPC_EXPORT_READY_PREFIX,
    failedPrefix: IPC_EXPORT_FAILED_PREFIX,
    editorPrefix: IPC_EDITOR_STATE_PREFIX,
    pageStyle: EXPORT_PAGE_STYLE_ID,
  }

  it('match design/pdf-export.md', () => {
    expect(names).toEqual({
      flag: '__MBR_EXPORT__',
      ready: 'mbrExportReady',
      readyPrefix: 'mbr:export-ready:',
      failedPrefix: 'mbr:export-failed:',
      editorPrefix: 'mbr:editor-state:',
      pageStyle: 'mbr-export-page',
    })
  })

  const rust = __MBR_PDF_EXPORT_RS__
  // Each is a quoted `&str` literal there (`pub const … = "…";`).
  it.each(Object.entries(names))('%s is a string literal in the Rust half', (_key, name) => {
    expect(rust, 'src/pdf_export/mod.rs is missing').not.toBeNull()
    expect(rust).toContain(JSON.stringify(name))
  })
})
