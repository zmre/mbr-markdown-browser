import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

/**
 * The loader fetches scripts and defers the highlight pass to idle time;
 * neither happens under happy-dom, so both are stood in for. `idleTasks`
 * holds the deferred passes until a test runs them.
 */
const mocks = vi.hoisted(() => ({
  loadScript: vi.fn((_src: string) => Promise.resolve()),
  loadCss: vi.fn((_href: string) => Promise.resolve()),
  idleTasks: [] as Array<() => void>,
}))

vi.mock('./dynamic-loader.ts', () => ({
  waitForDom: () => Promise.resolve(),
  loadScript: mocks.loadScript,
  loadCss: mocks.loadCss,
  getMbrAssetBase: () => '/.mbr/',
  scheduleIdleTask: (task: () => void) => {
    mocks.idleTasks.push(task)
  },
}))

import { hljsSettled } from './mbr-hljs.ts'

function stubHljs(): { highlightAll: ReturnType<typeof vi.fn> } {
  const hljs = { highlightAll: vi.fn() }
  vi.stubGlobal('hljs', hljs)
  return hljs
}

async function flush(): Promise<void> {
  for (let i = 0; i < 4; i++) await new Promise((resolve) => setTimeout(resolve, 0))
}

async function mount(): Promise<HTMLElement> {
  const el = document.createElement('mbr-hljs')
  document.body.appendChild(el)
  await flush()
  return el
}

/** Whether `promise` has settled by now. */
async function isSettled(promise: Promise<unknown>): Promise<boolean> {
  let settled = false
  void promise.then(() => (settled = true))
  await flush()
  return settled
}

const CODE = '<pre><code class="language-rust">fn main() {}</code></pre>'

describe('UNIT MbrHljsElement / hljsSettled', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    mocks.idleTasks.length = 0
  })

  afterEach(() => {
    document.body.innerHTML = ''
    delete window.__MBR_EXPORT__
    vi.unstubAllGlobals()
    vi.clearAllMocks()
  })

  it('loads nothing for a page without code', async () => {
    await mount()
    expect(mocks.loadScript).not.toHaveBeenCalled()
    await expect(hljsSettled()).resolves.toBeUndefined()
  })

  it('settles only after the idle-time highlight pass has run', async () => {
    document.body.innerHTML = CODE
    const hljs = stubHljs()
    await mount()

    expect(mocks.loadScript).toHaveBeenCalledWith('/.mbr/hljs.lang.rust.js')
    const settled = hljsSettled()
    expect(await isSettled(settled)).toBe(false)
    expect(mocks.idleTasks).toHaveLength(1)

    mocks.idleTasks[0]()
    expect(hljs.highlightAll).toHaveBeenCalledTimes(1)
    expect(await isSettled(settled)).toBe(true)
  })

  it('in a PDF export, highlights at once instead of at idle', async () => {
    window.__MBR_EXPORT__ = { links: 'strip', paperWidthIn: 8.5, paperHeightIn: 11, margin: '0.5in' }
    document.body.innerHTML = CODE
    const hljs = stubHljs()
    await mount()
    await hljsSettled()

    expect(mocks.idleTasks).toHaveLength(0)
    expect(hljs.highlightAll).toHaveBeenCalledTimes(1)
  })

  it('settles, not rejects, when hljs fails to load', async () => {
    document.body.innerHTML = CODE
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    mocks.loadScript.mockImplementationOnce(() => Promise.reject(new Error('offline')))
    await mount()
    await expect(hljsSettled()).resolves.toBeUndefined()
  })
})
