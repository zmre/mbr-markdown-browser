import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'

/**
 * KaTeX and asciimath2tex are fetched from /.mbr/ at runtime, which happy-dom
 * cannot do. The loader is mocked so each test decides when (and whether) a
 * script "arrives", and idle tasks are queued so a test decides when the
 * render pass runs.
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

import { mathSettled } from './mbr-katex.ts'
import { MATH_STATE_ATTR } from './math-render.ts'

/** A KaTeX stand-in: writes a `.katex` span so the tests can see a render. */
function stubKatex(): { render: ReturnType<typeof vi.fn> } {
  const katex = {
    render: vi.fn((tex: string, el: Element) => {
      el.innerHTML = `<span class="katex">${tex}</span>`
    }),
  }
  vi.stubGlobal('katex', katex)
  return katex
}

/** A promise with its resolve/reject handles, for a script that arrives late. */
function deferred(): { promise: Promise<void>; resolve: () => void; reject: (e: Error) => void } {
  let resolve!: () => void
  let reject!: (e: Error) => void
  const promise = new Promise<void>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

async function flush(): Promise<void> {
  for (let i = 0; i < 4; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}

async function mount(): Promise<HTMLElement> {
  const el = document.createElement('mbr-katex')
  document.body.appendChild(el)
  await flush()
  return el
}

function runIdleTasks(): void {
  const tasks = mocks.idleTasks.splice(0)
  tasks.forEach((task) => task())
}

/** Tracks whether `mathSettled()` (asked now) has resolved. */
function watchSettled(): { readonly settled: boolean } {
  const state = { settled: false }
  void mathSettled().then(() => {
    state.settled = true
  })
  return state
}

describe('<mbr-katex>', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
    mocks.idleTasks.length = 0
    mocks.loadScript.mockImplementation(() => Promise.resolve())
  })

  afterEach(() => {
    document.body.innerHTML = ''
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
    mocks.loadScript.mockClear()
  })

  describe('mathSettled', () => {
    it('resolves at once on a page without math, loading nothing', async () => {
      document.body.innerHTML = '<main><p>no math</p></main>'
      await mount()
      await expect(mathSettled()).resolves.toBeUndefined()
      expect(mocks.loadScript).not.toHaveBeenCalled()
    })

    it('resolves only once KaTeX has loaded and the idle render has run', async () => {
      document.body.innerHTML = '<main><div class="math math-display">x^2</div></main>'
      const katex = stubKatex()
      const script = deferred()
      mocks.loadScript.mockImplementation(() => script.promise)

      await mount()
      const watch = watchSettled()
      await flush()
      expect(watch.settled).toBe(false)

      script.resolve()
      await flush()
      // Loaded, but the render pass is still queued for idle time.
      expect(watch.settled).toBe(false)
      expect(katex.render).not.toHaveBeenCalled()

      runIdleTasks()
      await flush()
      expect(watch.settled).toBe(true)
      expect(katex.render).toHaveBeenCalledOnce()
    })

    it('resolves, not rejects, when KaTeX fails to load', async () => {
      document.body.innerHTML = '<main><span class="math math-inline">x</span></main>'
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      mocks.loadScript.mockImplementation(() => Promise.reject(new Error('404')))

      await mount()
      await expect(mathSettled()).resolves.toBeUndefined()
      expect(warn).toHaveBeenCalled()
      expect(mocks.idleTasks).toHaveLength(0)
    })
  })

  describe('AsciiMath', () => {
    it('loads the converter only when an AsciiMath block exists', async () => {
      document.body.innerHTML = '<main><div class="math math-display">x</div></main>'
      stubKatex()
      await mount()
      runIdleTasks()
      await mathSettled()
      expect(mocks.loadScript.mock.calls.map(([src]) => src)).toEqual(['/.mbr/katex.min.js'])
    })

    it('fetches it in parallel with KaTeX and renders through it', async () => {
      document.body.innerHTML = '<main><div class="math math-display math-asciimath">x^2</div></main>'
      const katex = stubKatex()
      vi.stubGlobal(
        'AsciiMathParser',
        class {
          parse(s: string): string {
            return `TEX(${s})`
          }
        }
      )
      await mount()
      expect(mocks.loadScript.mock.calls.map(([src]) => src)).toEqual([
        '/.mbr/katex.min.js',
        '/.mbr/asciimath2tex.min.js',
      ])
      runIdleTasks()
      await mathSettled()
      expect(katex.render.mock.calls[0][0]).toBe('TEX(x^2)')
    })

    it('leaves the source and still settles when the converter fails to load', async () => {
      document.body.innerHTML = '<main><div class="math math-display math-asciimath">x^2</div></main>'
      const katex = stubKatex()
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      mocks.loadScript.mockImplementation((src: string) =>
        src.includes('asciimath2tex') ? Promise.reject(new Error('404')) : Promise.resolve()
      )
      await mount()
      runIdleTasks()
      await mathSettled()

      const block = document.querySelector('.math-asciimath')!
      expect(katex.render).not.toHaveBeenCalled()
      expect(block.textContent).toBe('x^2')
      expect(block.getAttribute(MATH_STATE_ATTR)).toBe('source')
      expect(warn).toHaveBeenCalled()
    })
  })

  it('renders clones made while KaTeX was loading, not the detached originals', async () => {
    // What <mbr-slides> does when its bounded wait runs out: the page's math
    // is replaced by deep clones before KaTeX arrives.
    document.body.innerHTML = '<main id="wrapper"><div class="math math-display">x^2</div></main>'
    const katex = stubKatex()
    const script = deferred()
    mocks.loadScript.mockImplementation(() => script.promise)
    await mount()

    const wrapper = document.getElementById('wrapper')!
    const original = wrapper.querySelector('.math')!
    const deck = document.createElement('div')
    deck.className = 'reveal'
    deck.append(original.cloneNode(true))
    wrapper.replaceChildren(deck)

    script.resolve()
    await flush()
    runIdleTasks()
    await mathSettled()

    expect(katex.render).toHaveBeenCalledOnce()
    expect(document.querySelector('.reveal .math .katex')).toBeTruthy()
    expect(original.querySelector('.katex')).toBeNull()
  })
})
