import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'

/**
 * Reveal.js is fetched from /.mbr/ at runtime, which happy-dom cannot do, and
 * scheduleIdleTask defers work past the end of the test. Mock the loader so the
 * DOM transformation runs synchronously against a stub Reveal global.
 */
const mocks = vi.hoisted(() => ({
  loadScript: vi.fn(() => Promise.resolve()),
  loadCss: vi.fn(() => Promise.resolve()),
}))

vi.mock('./dynamic-loader.ts', () => ({
  waitForDom: () => Promise.resolve(),
  loadScript: mocks.loadScript,
  loadCss: mocks.loadCss,
  getMbrAssetBase: () => '/.mbr/',
  scheduleIdleTask: (task: () => void) => task(),
}))

import type { MbrSlidesElement } from './mbr-slides.ts'
import './mbr-slides.ts'

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
