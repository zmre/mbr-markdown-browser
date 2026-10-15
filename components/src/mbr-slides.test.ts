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
}))

vi.mock('./dynamic-loader.ts', () => ({
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

import type { MbrSlidesElement } from './mbr-slides.ts'
import { SLIDES_STATE_EVENT } from './mbr-slides.ts'

interface RevealStub {
  initialize: ReturnType<typeof vi.fn>
  on: ReturnType<typeof vi.fn>
  off: ReturnType<typeof vi.fn>
  getCurrentSlide: ReturnType<typeof vi.fn>
}

/** Install a fake Reveal global and hand it back for assertions. */
function stubReveal(): RevealStub {
  const reveal: RevealStub = {
    initialize: vi.fn(() => Promise.resolve()),
    on: vi.fn(),
    off: vi.fn(),
    getCurrentSlide: vi.fn(() => document.querySelector('.slides section')),
  }
  vi.stubGlobal('Reveal', reveal)
  return reveal
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

  describe('diagrams', () => {
    beforeEach(() => {
      stubReveal()
    })

    afterEach(() => {
      mocks.mermaidSettled.mockImplementation(() => Promise.resolve())
      vi.useRealTimers()
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
    vi.spyOn(window.location, 'reload').mockImplementation(() => {
      reload(states.slice())
    })
    setupPage('<section id="s1"><p>one</p></section>')
    stubReveal()
    element = await mount()
    element.open()
    await flush(element)
    document.querySelector<HTMLButtonElement>('button.mbr-slides-exit')!.click()
    expect(reload).toHaveBeenCalledWith([true, false])
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
    // Above the (hidden) header's stacking context.
    expect(Number(exitButton()!.style.zIndex)).toBeGreaterThan(999)

    element.remove()
    expect(exitButton()).toBeNull()
  })

  it('adds no exit button to a desktop presentation', async () => {
    narrow(false)
    stubReveal()
    setupPage('<section id="s1"><p>one</p></section>')
    element = await mount()
    await play(element)
    expect(exitButton()).toBeNull()
  })
})
