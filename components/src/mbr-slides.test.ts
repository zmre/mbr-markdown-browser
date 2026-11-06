import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'

/**
 * Reveal.js is fetched from /.mbr/ at runtime, which happy-dom cannot do, and
 * scheduleIdleTask defers work past the end of the test. Mock the loader so the
 * DOM transformation runs synchronously against a stub Reveal global.
 */
const mocks = vi.hoisted(() => ({
  loadScript: vi.fn(() => Promise.resolve()),
  loadCss: vi.fn(() => Promise.resolve()),
  mermaidSettled: vi.fn(() => Promise.resolve()),
  mathSettled: vi.fn(() => Promise.resolve()),
}))

vi.mock('./dynamic-loader.ts', async (importOriginal) => ({
  // The real bounded wait: the timeout tests drive it with fake timers.
  withTimeout: (await importOriginal<typeof import('./dynamic-loader.ts')>()).withTimeout,
  waitForDom: () => Promise.resolve(),
  loadScript: mocks.loadScript,
  loadCss: mocks.loadCss,
  getMbrAssetBase: () => '/.mbr/',
  scheduleIdleTask: (task: () => void) => task(),
}))

// The real module would register <mbr-mermaid> and load mermaid; the slides
// only need its "diagrams are drawn" promise, which each test controls.
vi.mock('./mbr-mermaid.ts', () => ({
  mermaidSettled: mocks.mermaidSettled,
}))

// Likewise for KaTeX's "math is rendered" promise.
vi.mock('./mbr-katex.ts', () => ({
  mathSettled: mocks.mathSettled,
}))

import type { MbrSlidesElement } from './mbr-slides.ts'
import { SLIDES_STATE_EVENT, groupKanbanColumns, isSlideHash } from './mbr-slides.ts'

interface RevealStub {
  initialize: ReturnType<typeof vi.fn>
  on: ReturnType<typeof vi.fn>
  off: ReturnType<typeof vi.fn>
  getCurrentSlide: ReturnType<typeof vi.fn>
  layout: ReturnType<typeof vi.fn>
  getConfig: ReturnType<typeof vi.fn>
  getIndices: ReturnType<typeof vi.fn>
  addKeyBinding: ReturnType<typeof vi.fn>
  /** Fire every listener registered with `on(type, …)`. */
  emit: (type: string) => void
}

/** Install a fake Reveal global and hand it back for assertions. */
function stubReveal(): RevealStub {
  const listeners = new Map<string, Array<(e: unknown) => void>>()
  const reveal: RevealStub = {
    initialize: vi.fn(() => Promise.resolve()),
    on: vi.fn((type: string, fn: (e: unknown) => void) => {
      listeners.set(type, [...(listeners.get(type) ?? []), fn])
    }),
    off: vi.fn(),
    getCurrentSlide: vi.fn(() => document.querySelector('.slides section')),
    layout: vi.fn(),
    getConfig: vi.fn(() => ({ height: 700 })),
    getIndices: vi.fn(() => ({ h: 0, v: 0 })),
    addKeyBinding: vi.fn(),
    emit: (type) => listeners.get(type)?.forEach((fn) => fn({})),
  }
  vi.stubGlobal('Reveal', reveal)
  return reveal
}

/** The config the deck handed `Reveal.initialize`. */
function revealConfig(reveal: RevealStub): Record<string, unknown> {
  return reveal.initialize.mock.calls[0]![0] as Record<string, unknown>
}

/** Put the page at `hash` (or none) without a navigation. */
function setHash(hash: string): void {
  history.replaceState(null, '', `${location.pathname}${location.search}${hash}`)
}

/** Build a page shaped like templates/index.html in slides mode. */
function setupPage(mainHtml: string): void {
  document.body.className = 'slides'
  document.body.innerHTML = `
    <nav>nav</nav>
    <main id="wrapper" class="container">${mainHtml}</main>
    <footer>footer</footer>
  `
}

/** Let waitForDom/load microtasks and Lit updates settle. */
async function flush(el?: MbrSlidesElement): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0))
    if (el) await el.updateComplete
  }
}

async function mount(): Promise<MbrSlidesElement> {
  const el = document.createElement('mbr-slides')
  document.body.appendChild(el)
  await flush(el)
  return el
}

/** Click the Play button and wait for the transform to finish. */
async function play(el: MbrSlidesElement): Promise<void> {
  const button = el.shadowRoot?.querySelector('button')
  expect(button, 'play button should be rendered').toBeTruthy()
  button?.click()
  await flush(el)
}

describe('UNIT MbrSlidesElement', () => {
  let element: MbrSlidesElement | null = null

  beforeEach(() => {
    document.body.className = ''
    document.body.innerHTML = ''
  })

  afterEach(() => {
    element?.remove()
    element = null
    document.body.className = ''
    document.body.innerHTML = ''
    setHash('')
    vi.unstubAllGlobals()
    vi.clearAllMocks()
  })

  describe('play button', () => {
    it('renders only on slides documents', async () => {
      setupPage('<section><p>one</p></section>')
      element = await mount()
      expect(element.shadowRoot?.querySelector('button')).toBeTruthy()
    })

    it('renders nothing on ordinary pages', async () => {
      document.body.innerHTML = '<main id="wrapper"><p>hi</p></main>'
      element = await mount()
      expect(element.shadowRoot?.querySelector('button')).toBeNull()
    })

    it('disappears once the presentation is playing', async () => {
      setupPage('<section><p>one</p></section>')
      stubReveal()
      element = await mount()
      await play(element)
      expect(element.shadowRoot?.querySelector('button')).toBeNull()
    })
  })

  describe('DOM transformation', () => {
    beforeEach(() => {
      stubReveal()
    })

    it('wraps top-level sections in .reveal > .slides and hides chrome', async () => {
      setupPage(`
        <section id="s1"><p>one</p></section>
        <hr />
        <section id="s2"><p>two</p></section>
      `)
      element = await mount()
      await play(element)

      const slides = document.querySelectorAll('#wrapper .reveal > .slides > section')
      expect(slides.length).toBe(2)
      expect(document.querySelectorAll('#wrapper hr').length).toBe(0)
      expect(document.querySelector('nav')?.getAttribute('style')).toBe('display: none')
      expect(document.querySelector('footer')?.getAttribute('style')).toBe('display: none')
      expect(document.body.classList.contains('slides')).toBe(false)
      expect(document.body.classList.contains('slides-container')).toBe(true)
      expect(document.body.classList.contains('reveal-viewport')).toBe(true)
    })

    it('does not repeat a nested section as its own slide', async () => {
      // A `---` inside a blockquote or list item makes the renderer emit a
      // section nested inside the previous one.
      setupPage(`
        <section id="outer">
          <blockquote><section id="inner"><p>nested</p></section></blockquote>
        </section>
        <section id="last"><p>last</p></section>
      `)
      element = await mount()
      await play(element)

      const slides = document.querySelectorAll('#wrapper .reveal > .slides > section')
      expect(Array.from(slides).map((s) => s.id)).toEqual(['outer', 'last'])
      // The nested section survives exactly once, inside its parent slide.
      expect(document.querySelectorAll('#inner').length).toBe(1)
      expect(document.querySelector('#outer #inner')).toBeTruthy()
    })

    describe('leading empty section (a plain `---` at the start of the note)', () => {
      /** Ids (or `''`) of the deck's top-level slides, in order. */
      async function slideIdsFor(mainHtml: string): Promise<string[]> {
        setupPage(mainHtml)
        element = await mount()
        await play(element)
        return Array.from(document.querySelectorAll('#wrapper .reveal > .slides > section')).map(
          (s) => s.id
        )
      }

      it('skips the empty section the renderer emits before the first rule', async () => {
        // Exactly the renderer's shape for a note opening with `---`.
        const ids = await slideIdsFor(
          '<section>\n</section>\n<hr />\n<section id="a"><h1>A</h1></section>\n<hr />\n<section id="b"><p>B</p></section>'
        )
        expect(ids).toEqual(['a', 'b'])
      })

      it('makes the first real slide the one Reveal numbers 0', async () => {
        await slideIdsFor('<section></section><hr /><section id="a"><p>A</p></section>')
        const first = document.querySelector('#wrapper .reveal > .slides > section')
        expect(first?.id).toBe('a')
      })

      it('skips a first section holding only whitespace and comments', async () => {
        const ids = await slideIdsFor(
          '<section>\n  <!-- a comment -->\n  \n</section><hr /><section id="a"><p>A</p></section>'
        )
        expect(ids).toEqual(['a'])
      })

      it('ignores mbr bookkeeping attributes when judging emptiness', async () => {
        const ids = await slideIdsFor(
          '<section data-mbr-line="3"></section><hr /><section id="a"><p>A</p></section>'
        )
        expect(ids).toEqual(['a'])
      })

      it.each([
        ['an id', '<section id="intro"></section>', 'intro'],
        ['a class', '<section class="title"></section>', ''],
        ['a data attribute', '<section data-background-color="black"></section>', ''],
      ])('keeps an empty first section with %s', async (_label, first, firstId) => {
        const ids = await slideIdsFor(`${first}<hr /><section id="a"><p>A</p></section>`)
        expect(ids).toEqual([firstId, 'a'])
      })

      it('keeps a first section with content', async () => {
        const ids = await slideIdsFor(
          '<section id="a"><p>A</p></section><hr /><section id="b"><p>B</p></section>'
        )
        expect(ids).toEqual(['a', 'b'])
      })

      it('keeps an empty section in the middle (an authored blank slide)', async () => {
        const ids = await slideIdsFor(
          '<section id="a"><p>A</p></section><hr /><section>\n</section><hr /><section id="b"><p>B</p></section>'
        )
        expect(ids).toEqual(['a', '', 'b'])
      })

      it('drops only the first of two leading empty sections', async () => {
        const ids = await slideIdsFor(
          '<section></section><hr /><section></section><hr /><section id="a"><p>A</p></section>'
        )
        expect(ids).toEqual(['', 'a'])
      })

      it('keeps a lone empty section, so the deck is never empty', async () => {
        const ids = await slideIdsFor('<section>\n</section>')
        expect(ids).toEqual([''])
      })

      it('gives the deck background to the remaining slides, starting with the first real one', async () => {
        vi.stubGlobal('__MBR_CONFIG__', { slides: { background_image: '/bg.png' } })
        await slideIdsFor('<section></section><hr /><section id="a"><p>A</p></section>')
        const slides = document.querySelectorAll('#wrapper .reveal > .slides > section')
        expect(slides.length).toBe(1)
        expect(slides[0]!.getAttribute('data-background-image')).toBe('/bg.png')
      })
    })

    it('turns a triple blockquote into an aside.notes', async () => {
      setupPage(`
        <section id="s1">
          <p>one</p>
          <blockquote><blockquote><blockquote><p>speaker note</p></blockquote></blockquote></blockquote>
        </section>
      `)
      element = await mount()
      await play(element)

      const notes = document.querySelectorAll('#wrapper aside.notes')
      expect(notes.length).toBe(1)
      expect(notes[0].textContent?.trim()).toBe('speaker note')
      expect(document.querySelectorAll('#wrapper blockquote').length).toBe(0)
    })

    it('leaves ordinary blockquotes alone', async () => {
      setupPage('<section id="s1"><blockquote><p>quote</p></blockquote></section>')
      element = await mount()
      await play(element)

      expect(document.querySelectorAll('#wrapper aside.notes').length).toBe(0)
      expect(document.querySelectorAll('#wrapper blockquote').length).toBe(1)
    })

    it('initializes Reveal once even if play is triggered twice', async () => {
      setupPage('<section id="s1"><p>one</p></section>')
      const reveal = stubReveal()
      element = await mount()
      await play(element)

      // Second trigger via the keyboard shortcut, after the button is gone.
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'p' }))
      await flush(element)

      expect(reveal.initialize).toHaveBeenCalledTimes(1)
      expect(document.querySelectorAll('#wrapper .reveal').length).toBe(1)
    })

    it('keeps the scaled slide view on narrow screens (no Reveal scroll view)', async () => {
      // Reveal 6's default `scrollActivationWidth` (435) turns every upright
      // phone into its scroll view, which restructures the slides out from
      // under a deck's own CSS. Only a number enables it.
      setupPage('<section id="s1"><p>one</p></section>')
      const reveal = stubReveal()
      element = await mount()
      await play(element)

      expect(reveal.initialize).toHaveBeenCalledTimes(1)
      const config = reveal.initialize.mock.calls[0][0] as Record<string, unknown>
      expect(config).toHaveProperty('scrollActivationWidth', null)
      expect(config.view).toBeUndefined()
    })
  })

  describe('deck settings', () => {
    afterEach(() => {
      delete window.__MBR_CONFIG__
    })

    it('passes the same Reveal options as before when nothing is configured', async () => {
      setupPage('<section><p>one</p></section>')
      const reveal = stubReveal()
      element = await mount()
      await play(element)

      const config = revealConfig(reveal)
      expect(config).toMatchObject({
        transition: 'slide',
        controls: true,
        progress: true,
        hash: true,
        history: false,
        center: true,
      })
      expect(config).not.toHaveProperty('slideNumber')
      expect(config).not.toHaveProperty('transitionSpeed')
      expect(config).not.toHaveProperty('backgroundTransition')
    })

    it('applies config, then frontmatter, and keeps the fixed settings', async () => {
      window.__MBR_CONFIG__ = {
        serverMode: true,
        guiMode: false,
        slides: { transition: 'fade', controls: false, background_image: '/bg/config.jpg' },
        slidesPage: { transition: 'zoom', slide_number: 'c/t', background_image: '../page.jpg' },
      }
      setupPage(
        '<section><p>one</p></section>' +
          '<section data-background-color="#000"><p>two</p></section>' +
          '<section><p>three</p></section>'
      )
      const reveal = stubReveal()
      element = await mount()
      await play(element)

      expect(revealConfig(reveal)).toMatchObject({
        transition: 'zoom',
        controls: false,
        progress: true,
        slideNumber: 'c/t',
        hash: true,
        history: false,
      })
      const slides = Array.from(document.querySelectorAll('.reveal .slides > section'))
      expect(slides.map((s) => s.getAttribute('data-background-image'))).toEqual([
        '../page.jpg',
        null,
        '../page.jpg',
      ])
    })
  })

  describe('diagrams', () => {
    beforeEach(() => {
      stubReveal()
    })

    afterEach(() => {
      mocks.mermaidSettled.mockImplementation(() => Promise.resolve())
      mocks.mathSettled.mockImplementation(() => Promise.resolve())
      vi.useRealTimers()
    })

    it('clones the slides only once the math is rendered', async () => {
      setupPage('<section><div class="math math-display">x^2</div></section>')
      let rendered!: () => void
      mocks.mathSettled.mockImplementation(
        () =>
          new Promise<void>((resolve) => {
            rendered = resolve
          })
      )
      element = await mount()
      await play(element)

      // Still waiting: cloning now would copy raw TeX.
      expect(document.querySelector('#wrapper .reveal')).toBeNull()

      // KaTeX renders into the page's original element.
      document.querySelector('.math')!.innerHTML = '<span class="katex" id="typeset"></span>'
      rendered()
      await flush(element)

      expect(document.querySelector('#wrapper .reveal .slides section .katex#typeset')).toBeTruthy()
    })

    it('waits for diagrams and math under one bound', async () => {
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
      setupPage('<section><pre class="mermaid">graph TD; A--&gt;B</pre><span class="math math-inline">x</span></section>')
      // Diagrams finish at once; math never does.
      mocks.mathSettled.mockImplementation(() => new Promise<void>(() => {}))
      const el = document.createElement('mbr-slides')
      document.body.appendChild(el)
      element = el
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete

      el.open()
      await vi.advanceTimersByTimeAsync(7000)
      expect(document.querySelector('#wrapper .reveal')).toBeNull()

      await vi.advanceTimersByTimeAsync(1500)
      expect(document.querySelector('#wrapper .reveal .slides section .math')).toBeTruthy()
    })

    it('clones the slides only once the diagrams are drawn', async () => {
      setupPage('<section><pre class="mermaid">graph TD; A--&gt;B</pre></section>')
      let drawn!: () => void
      mocks.mermaidSettled.mockImplementation(
        () =>
          new Promise<void>((resolve) => {
            drawn = resolve
          })
      )
      element = await mount()
      await play(element)

      // Still waiting: the page is untouched, so nothing raw gets cloned.
      expect(document.querySelector('#wrapper .reveal')).toBeNull()
      expect(element.shadowRoot?.querySelector('button')?.disabled).toBe(true)

      // Mermaid finishes, drawing into the page's original block.
      document.querySelector('pre.mermaid')!.innerHTML = '<svg id="drawn"></svg>'
      drawn()
      await flush(element)

      expect(document.querySelector('#wrapper .reveal .slides section svg#drawn')).toBeTruthy()
    })

    it('plays anyway when the diagrams never finish', async () => {
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
      setupPage('<section><pre class="mermaid">graph TD; A--&gt;B</pre></section>')
      mocks.mermaidSettled.mockImplementation(() => new Promise<void>(() => {}))
      const el = document.createElement('mbr-slides')
      document.body.appendChild(el)
      element = el
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete

      el.open()
      await vi.advanceTimersByTimeAsync(7000)
      expect(document.querySelector('#wrapper .reveal')).toBeNull()

      await vi.advanceTimersByTimeAsync(1500)
      expect(document.querySelector('#wrapper .reveal .slides section pre.mermaid')).toBeTruthy()
    })
  })

  describe('keyboard shortcut', () => {
    it('starts the presentation on "p"', async () => {
      setupPage('<section id="s1"><p>one</p></section>')
      stubReveal()
      element = await mount()

      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'p' }))
      await flush(element)

      expect(document.querySelector('#wrapper .reveal')).toBeTruthy()
    })

    it('ignores "p" typed into an input', async () => {
      setupPage('<section id="s1"><p>one</p></section><input id="field" />')
      stubReveal()
      element = await mount()

      const field = document.querySelector<HTMLInputElement>('#field')!
      field.dispatchEvent(new KeyboardEvent('keydown', { key: 'p', bubbles: true, composed: true }))
      await flush(element)

      expect(document.querySelector('#wrapper .reveal')).toBeNull()
    })

    it('unregisters the handler when the element is removed', async () => {
      setupPage('<section id="s1"><p>one</p></section>')
      stubReveal()
      element = await mount()
      element.remove()
      element = null

      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'p' }))
      await flush()

      expect(document.querySelector('#wrapper .reveal')).toBeNull()
    })

    it('registers no handler when removed before the DOM settles', async () => {
      setupPage('<section id="s1"><p>one</p></section>')
      stubReveal()
      // connectedCallback defers to a microtask; disconnect before it runs.
      const el = document.createElement('mbr-slides')
      document.body.appendChild(el)
      el.remove()
      await flush()

      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'p' }))
      await flush()

      expect(mocks.loadCss).not.toHaveBeenCalled()
      expect(document.querySelector('#wrapper .reveal')).toBeNull()
      expect(document.body.classList.contains('slides')).toBe(true)
    })
  })

  describe('load failures', () => {
    it('restores the body class when Reveal never shows up', async () => {
      setupPage('<section id="s1"><p>one</p></section>')
      // No Reveal global: the loader resolves but the script defined nothing.
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      element = await mount()

      await play(element)
      // The component polls for Reveal for up to ~500ms before giving up.
      await new Promise((resolve) => setTimeout(resolve, 700))
      await element.updateComplete

      expect(document.body.classList.contains('slides')).toBe(true)
      expect(document.body.classList.contains('slides-container')).toBe(false)
      expect(document.querySelector('#wrapper .reveal')).toBeNull()
      // The page is still usable: the button comes back.
      expect(element.shadowRoot?.querySelector('button')).toBeTruthy()
      consoleError.mockRestore()
    })

    it('restores the body class when an asset fails to load', async () => {
      setupPage('<section id="s1"><p>one</p></section>')
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      mocks.loadCss.mockRejectedValueOnce(new Error('boom'))
      element = await mount()

      await play(element)

      expect(document.body.classList.contains('slides')).toBe(true)
      expect(document.body.classList.contains('slides-container')).toBe(false)
      consoleError.mockRestore()
    })
  })
})

/**
 * The iOS app hides its native bar while a deck plays (`app-bridge.ts` relays
 * this event), so it must say `true` only once the deck is on screen and
 * `false` whenever it is not.
 */
describe('UNIT MbrSlidesElement playing state event', () => {
  let element: MbrSlidesElement | null = null
  let states: boolean[] = []
  const record = (event: Event): void => {
    states.push((event as CustomEvent<{ playing: boolean }>).detail.playing)
  }

  beforeEach(() => {
    states = []
    document.addEventListener(SLIDES_STATE_EVENT, record)
  })

  afterEach(() => {
    document.removeEventListener(SLIDES_STATE_EVENT, record)
    element?.remove()
    element = null
    document.body.className = ''
    document.body.innerHTML = ''
    setHash('')
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
    vi.clearAllMocks()
  })

  it('is named as the app bridge expects', () => {
    expect(SLIDES_STATE_EVENT).toBe('mbr-slides-state')
  })

  it('announces playing once the deck has replaced the page', async () => {
    setupPage('<section id="s1"><p>one</p></section>')
    stubReveal()
    let deckOnScreen = false
    document.addEventListener(
      SLIDES_STATE_EVENT,
      () => {
        deckOnScreen = document.querySelector('#wrapper .reveal .slides section') !== null
      },
      { once: true }
    )
    element = await mount()
    expect(states).toEqual([])
    await play(element)
    expect(states).toEqual([true])
    expect(deckOnScreen).toBe(true)
  })

  it('announces playing for open() and "p" alike', async () => {
    setupPage('<section id="s1"><p>one</p></section>')
    stubReveal()
    element = await mount()
    element.open()
    await flush(element)
    // A second start is a no-op and says nothing more.
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'p' }))
    await flush(element)
    expect(states).toEqual([true])

    element.remove()
    setupPage('<section id="s1"><p>one</p></section>')
    states = []
    element = await mount()
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'p' }))
    await flush(element)
    expect(states).toEqual([true])
  })

  it('announces not playing when an asset fails to load', async () => {
    setupPage('<section id="s1"><p>one</p></section>')
    vi.spyOn(console, 'error').mockImplementation(() => {})
    mocks.loadCss.mockRejectedValueOnce(new Error('boom'))
    element = await mount()
    await play(element)
    expect(states).toEqual([false])
  })

  it('announces not playing when Reveal never shows up', async () => {
    setupPage('<section id="s1"><p>one</p></section>')
    vi.spyOn(console, 'error').mockImplementation(() => {})
    element = await mount()
    await play(element)
    await new Promise((resolve) => setTimeout(resolve, 700))
    expect(states).toEqual([false])
  })

  it('takes playing back when Reveal fails to initialize', async () => {
    setupPage('<section id="s1"><p>one</p></section>')
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const reveal = stubReveal()
    reveal.initialize.mockImplementation(() => Promise.reject(new Error('no')))
    element = await mount()
    await play(element)
    expect(states).toEqual([true, false])
  })

  it('announces not playing on exit, before the reload', async () => {
    vi.spyOn(window, 'matchMedia').mockReturnValue({
      matches: true,
      addEventListener: () => {},
      removeEventListener: () => {},
    } as unknown as MediaQueryList)
    const reload = vi.fn()
    vi.spyOn(window.location, 'replace').mockImplementation((url) => {
      reload(states.slice(), String(url))
    })
    setupPage('<section id="s1"><p>one</p></section>')
    stubReveal()
    element = await mount()
    element.open()
    await flush(element)
    document.querySelector<HTMLButtonElement>('button.mbr-slides-exit')!.click()
    expect(reload).toHaveBeenCalledWith([true, false], location.pathname + location.search)
    setHash('')
  })
})

describe('UNIT MbrSlidesElement floating play button', () => {
  let element: MbrSlidesElement | null = null
  const fab = () => element?.shadowRoot?.querySelector<HTMLButtonElement>('.mbr-fab') ?? null
  const exitButton = () => document.querySelector<HTMLButtonElement>('button.mbr-slides-exit')

  /** Report the narrow query as matching (or not), the way a phone would. */
  function narrow(matches: boolean): void {
    vi.spyOn(window, 'matchMedia').mockReturnValue({
      matches,
      addEventListener: () => {},
      removeEventListener: () => {},
    } as unknown as MediaQueryList)
  }

  afterEach(() => {
    element?.remove()
    element = null
    document.documentElement.classList.remove('mbr-embedded')
    document.body.className = ''
    document.body.innerHTML = ''
    setHash('')
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('is absent on a wide browser page', async () => {
    narrow(false)
    setupPage('<section><p>one</p></section>')
    element = await mount()
    expect(element.shadowRoot?.querySelector('.play-slides-btn')).toBeTruthy()
    expect(fab()).toBeNull()
  })

  it('appears on a slides page in a narrow viewport', async () => {
    narrow(true)
    setupPage('<section><p>one</p></section>')
    element = await mount()
    expect(fab()?.getAttribute('aria-label')).toBe('Play slides')
    expect(fab()?.title).toBe('Play slides (P)')
  })

  it('appears at any width in the iOS app', async () => {
    narrow(false)
    document.documentElement.classList.add('mbr-embedded')
    setupPage('<section><p>one</p></section>')
    element = await mount()
    expect(fab()).toBeTruthy()
  })

  it('is absent on a page that is not slides', async () => {
    narrow(true)
    document.body.innerHTML = '<main id="wrapper"><p>hi</p></main>'
    element = await mount()
    expect(fab()).toBeNull()
  })

  it('starts the presentation, then gives way to an exit button', async () => {
    narrow(true)
    stubReveal()
    setupPage('<section id="s1"><p>one</p></section>')
    element = await mount()
    fab()!.click()
    await flush(element)

    expect(document.querySelectorAll('#wrapper .reveal > .slides > section')).toHaveLength(1)
    expect(fab()).toBeNull()
    expect(exitButton()?.getAttribute('aria-label')).toBe('Exit slides')

    element.remove()
    expect(exitButton()).toBeNull()
  })

  it('adds the exit button to a desktop presentation too', async () => {
    narrow(false)
    stubReveal()
    setupPage('<section id="s1"><p>one</p></section>')
    element = await mount()
    await play(element)
    expect(exitButton()?.getAttribute('aria-label')).toBe('Exit slides')
    expect(exitButton()?.title).toBe('Exit slides (Q)')
    // Styled by reveal-slides.css, not inline.
    expect(exitButton()?.getAttribute('style')).toBeNull()
  })

  it('adds no exit button to a speaker-view receiver', async () => {
    narrow(false)
    stubReveal()
    history.replaceState(null, '', `${location.pathname}?receiver`)
    setupPage('<section id="s1"><p>one</p></section>')
    element = await mount()
    await flush(element)
    expect(document.querySelector('#wrapper .reveal')).toBeTruthy()
    expect(exitButton()).toBeNull()
    history.replaceState(null, '', location.pathname)
  })
})

describe('UNIT isSlideHash', () => {
  it('accepts the hashes Reveal writes', () => {
    for (const hash of ['#/0', '#/2', '#/2/1', '#/2/1/0', '#/intro', '#/intro/3']) {
      expect(isSlideHash(hash), hash).toBe(true)
    }
  })

  it('rejects page anchors and empty hashes', () => {
    for (const hash of ['', '#', '#/', '#intro', '#mbr-task-12', '#mbr-review-abc', '#/a b', '#/1/2/3/4']) {
      expect(isSlideHash(hash), hash).toBe(false)
    }
  })
})

describe('UNIT MbrSlidesElement while playing', () => {
  let element: MbrSlidesElement | null = null

  afterEach(() => {
    element?.remove()
    element = null
    document.body.className = ''
    document.body.innerHTML = ''
    setHash('')
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
    vi.clearAllMocks()
    vi.useRealTimers()
  })

  async function playing(mainHtml = '<section id="s1"><p>one</p></section>'): Promise<RevealStub> {
    setupPage(mainHtml)
    const reveal = stubReveal()
    element = await mount()
    element.open()
    await flush(element)
    return reveal
  }

  describe('q', () => {
    it('is a Reveal key binding, so Reveal lists it in its ? help', async () => {
      const reveal = await playing()
      expect(reveal.addKeyBinding).toHaveBeenCalledWith(
        { keyCode: 81, key: 'Q', description: 'Exit slides' },
        expect.any(Function)
      )
    })

    it('leaves by loading the page without the slide hash', async () => {
      const replace = vi.spyOn(window.location, 'replace').mockImplementation(() => {})
      const reveal = await playing()
      setHash('#/3')
      const exit = reveal.addKeyBinding.mock.calls[0]![1] as () => void
      exit()
      expect(replace).toHaveBeenCalledWith(location.pathname + location.search)
    })
  })

  describe('keyboard condition', () => {
    it("keeps Reveal's keys away from fields, shadow-root ones included", async () => {
      const reveal = await playing()
      const condition = revealConfig(reveal).keyboardCondition as (e: KeyboardEvent) => boolean
      expect(typeof condition).toBe('function')

      expect(condition(new KeyboardEvent('keydown', { key: 'g' }))).toBe(true)

      // A textarea inside a shadow root, as the review-note form has: the
      // document sees the host as the target, Reveal's own check sees the
      // host as `activeElement`; only composedPath finds the field.
      const host = document.createElement('div')
      document.body.append(host)
      const field = host.attachShadow({ mode: 'open' }).appendChild(document.createElement('textarea'))
      let seen: boolean | null = null
      document.addEventListener('keydown', (e) => (seen = condition(e)), { once: true })
      field.dispatchEvent(new KeyboardEvent('keydown', { key: 'g', bubbles: true, composed: true }))
      expect(seen).toBe(false)
    })
  })

  describe('slide hash', () => {
    it('starts the presentation on load when the URL carries a slide hash', async () => {
      setHash('#/2')
      setupPage('<section id="s1"><p>one</p></section>')
      stubReveal()
      element = await mount()
      await flush(element)
      expect(document.querySelector('#wrapper .reveal .slides section')).toBeTruthy()
      expect(document.body.classList.contains('slides-container')).toBe(true)
    })

    it('does not start for an ordinary heading anchor', async () => {
      setHash('#one')
      setupPage('<section id="s1"><h2 id="one">one</h2></section>')
      stubReveal()
      element = await mount()
      await flush(element)
      expect(document.querySelector('#wrapper .reveal')).toBeNull()
    })

    it('does not start a page that is not slides', async () => {
      setHash('#/2')
      document.body.innerHTML = '<main id="wrapper"><section><p>hi</p></section></main>'
      stubReveal()
      element = await mount()
      await flush(element)
      expect(document.querySelector('#wrapper .reveal')).toBeNull()
    })

    it('writes #/0 on the first slide, where Reveal writes no hash', async () => {
      await playing()
      expect(location.hash).toBe('#/0')
    })

    it('puts #/0 back after Reveal clears it within its write window', async () => {
      const reveal = await playing()
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
      // Reveal's deferred write for the first slide: the bare path.
      setHash('')
      reveal.emit('slidechanged')
      setHash('')
      vi.advanceTimersByTime(1200)
      expect(location.hash).toBe('#/0')
    })

    it('leaves another slide (or a named one) to Reveal', async () => {
      const reveal = await playing()
      reveal.getIndices.mockReturnValue({ h: 2, v: 0 })
      setHash('')
      reveal.emit('slidechanged')
      expect(location.hash).toBe('')
    })
  })

  describe('fitting', () => {
    it('fits and re-lays out the slide on every slide change and resize', async () => {
      const reveal = await playing()
      const before = reveal.layout.mock.calls.length
      expect(before).toBeGreaterThan(0)
      reveal.emit('slidechanged')
      reveal.emit('resize')
      expect(reveal.layout.mock.calls.length).toBe(before + 2)
    })

    it('re-fits when an image on a slide finishes loading', async () => {
      const reveal = await playing('<section id="s1"><p><img src="a.png" loading="lazy"></p></section>')
      const before = reveal.layout.mock.calls.length
      document.querySelector('#wrapper .reveal img')!.dispatchEvent(new Event('load'))
      expect(reveal.layout.mock.calls.length).toBe(before + 1)
    })

    it('loads every image eagerly, so no slide is centred without its picture', async () => {
      await playing('<section id="s1"><p><img src="a.png" loading="lazy"></p></section><section><img src="b.png" loading="lazy"></section>')
      const images = document.querySelectorAll('#wrapper .reveal img')
      expect(images).toHaveLength(2)
      images.forEach((img) => expect(img.hasAttribute('loading')).toBe(false))
    })
  })

  describe('"p" (start, then stop) regression', () => {
    it('ignores the auto-repeat of a held "p"', async () => {
      setupPage('<section id="s1"><p>one</p></section>')
      stubReveal()
      element = await mount()
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'p', repeat: true }))
      await flush(element)
      expect(document.querySelector('#wrapper .reveal')).toBeNull()
    })

    it('leaves "p" to Reveal (previous slide) once the deck plays', async () => {
      const reveal = await playing()
      const prevented: boolean[] = []
      document.addEventListener('keydown', (e) => prevented.push(e.defaultPrevented))
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'p', cancelable: true }))
      expect(prevented).toEqual([false])
      expect(reveal.initialize).toHaveBeenCalledTimes(1)
    })
  })
})

describe('UNIT groupKanbanColumns', () => {
  function section(html: string): HTMLElement {
    const el = document.createElement('section')
    el.className = 'kanban'
    el.innerHTML = html
    return el
  }

  it('makes one column per heading and its list', () => {
    const board = section(`
      <h2>Backlog</h2><ul><li>a</li><li>b</li></ul>
      <h2>Doing</h2><ul><li>c</li></ul>
      <h2>Done</h2><ol><li>d</li></ol>
    `)
    groupKanbanColumns(board)
    const columns = board.querySelectorAll(':scope > .mbr-kanban-board > .mbr-kanban-column')
    expect(columns).toHaveLength(3)
    expect([...columns].map((c) => c.querySelector('h2')?.textContent)).toEqual(['Backlog', 'Doing', 'Done'])
    expect(columns[0]!.querySelectorAll(':scope > ul > li')).toHaveLength(2)
    expect(columns[2]!.querySelector(':scope > ol')).toBeTruthy()
  })

  it('breaks after each list, as the page board does (a heading with no list joins the next)', () => {
    const board = section('<h1>Sprint</h1><h2>Todo</h2><ul><li>a</li></ul><h2>Done</h2><ul><li>b</li></ul>')
    groupKanbanColumns(board)
    const columns = board.querySelectorAll('.mbr-kanban-column')
    expect(columns).toHaveLength(2)
    expect(columns[0]!.querySelectorAll('h1, h2')).toHaveLength(2)
  })

  it('leaves speaker notes where Reveal looks for them', () => {
    const board = section('<h2>A</h2><ul><li>a</li></ul><aside class="notes">say this</aside>')
    groupKanbanColumns(board)
    expect(board.querySelector(':scope > aside.notes')?.textContent).toBe('say this')
    expect(board.querySelector('.mbr-kanban-board aside')).toBeNull()
  })

  it('is applied to a kanban slide when the deck starts', async () => {
    setupPage('<section class="kanban"><h2>A</h2><ul><li>a</li></ul><h2>B</h2><ul><li>b</li></ul></section>')
    stubReveal()
    const el = await mount()
    el.open()
    await flush(el)
    expect(document.querySelectorAll('#wrapper .reveal section.kanban .mbr-kanban-column')).toHaveLength(2)
    el.remove()
    document.body.className = ''
    document.body.innerHTML = ''
    setHash('')
    vi.unstubAllGlobals()
  })
})

declare const __MBR_REVEAL_SLIDES_CSS__: string

/** The declarations of the first rule whose selector list is exactly `selector`. */
function slidesRule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  return new RegExp(`(?:^|})\\s*${escaped}\\s*\\{([^}]*)\\}`, 'm').exec(__MBR_REVEAL_SLIDES_CSS__)?.[1] ?? ''
}

describe('UNIT reveal-slides.css exit button', () => {
  it('sits bottom-left inside the safe area, above the deck', () => {
    const rule = slidesRule('.mbr-slides-exit')
    expect(rule).toMatch(/position:\s*fixed/)
    expect(rule).toMatch(/left:\s*calc\([^;]*env\(safe-area-inset-left/)
    expect(rule).toMatch(/bottom:\s*calc\([^;]*env\(safe-area-inset-bottom/)
    expect(Number(/z-index:\s*(\d+)/.exec(rule)?.[1])).toBeGreaterThan(999)
  })

  it('is nearly transparent until hovered or keyboard-focused', () => {
    expect(Number(/opacity:\s*([\d.]+)/.exec(slidesRule('.mbr-slides-exit'))?.[1])).toBeLessThan(0.3)
    expect(slidesRule('.mbr-slides-exit:hover,\n.mbr-slides-exit:focus-visible')).toMatch(/opacity:\s*1/)
  })

  it('stays visible, at a finger size, where nothing can hover', () => {
    const touch = /@media \(hover: none\)\s*\{\s*\.mbr-slides-exit\s*\{([^}]*)\}/.exec(__MBR_REVEAL_SLIDES_CSS__)?.[1] ?? ''
    expect(touch).toMatch(/width:\s*44px/)
    expect(Number(/opacity:\s*([\d.]+)/.exec(touch)?.[1])).toBeGreaterThanOrEqual(0.7)
  })
})

describe('UNIT reveal-slides.css kanban cards', () => {
  const column = '.reveal .slides section.kanban .mbr-kanban-column'

  it('hangs card contents in theme.css\'s gutter, so tasks align with bullets', () => {
    const nested = slidesRule(`${column} li :is(ul, ol)`)
    expect(nested).toMatch(/padding:\s*0 0 0 var\(--mbr-kanban-gutter, 1\.7em\)/)
    expect(nested).toMatch(/margin:\s*0\.2em 0 0;/)
  })

  it('rules off the first level of a card\'s contents from its title', () => {
    const first = slidesRule(
      `${column} > :is(ul, ol) > li > :is(ul, ol),\n${column} > :is(ul, ol) > li > .mbr-incomplete > :is(ul, ol)`
    )
    expect(first).toMatch(/border-top:\s*1px solid var\(--pico-muted-border-color/)
  })

  it('keeps cards the full width of their column', () => {
    expect(slidesRule(`${column} > :is(ul, ol)`)).toMatch(/margin:\s*0;[\s\S]*padding:\s*0;/)
    expect(slidesRule(`${column} > :is(ul, ol) > li`)).toMatch(/list-style:\s*none/)
  })
})

describe('UNIT reveal-slides.css review notes while playing', () => {
  it('hides the review-note markers (the wash and its hover card stay)', () => {
    expect(slidesRule('body.slides-container .mbr-review-marker')).toMatch(/display:\s*none/)
  })

  it("lifts the review button clear of all four of Reveal's arrows", () => {
    // Reveal's controls are px-sized: the up arrow's top is ~117px above the
    // bottom edge at any width. The button sits at 1rem + the offset.
    const offset = /--mbr-fab-stack-offset:\s*([\d.]+)rem/.exec(slidesRule('body.slides-container'))?.[1]
    expect(16 + Number(offset) * 16).toBeGreaterThanOrEqual(117 + 12)
  })
})
