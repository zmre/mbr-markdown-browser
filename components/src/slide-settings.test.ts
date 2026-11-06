import { describe, it, expect, vi, afterEach } from 'vitest'
import {
  DEFAULT_SLIDE_SETTINGS,
  applyDefaultBackground,
  deckSettings,
  readSlideSettings,
  revealConfig,
} from './slide-settings.ts'

/** mbr's fixed settings, as `mbr-slides.ts` passes them. */
const FIXED = { hash: true, history: false, center: true, scrollActivationWidth: null }

describe('UNIT slide-settings', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('defaults', () => {
    it('equal what mbr hardcoded before the settings existed', () => {
      expect(revealConfig(deckSettings({}, {}), FIXED)).toEqual({
        transition: 'slide',
        controls: true,
        progress: true,
        hash: true,
        history: false,
        center: true,
        scrollActivationWidth: null,
      })
    })

    it('apply when the page carries no settings at all (an old custom _head.html)', () => {
      expect(deckSettings(undefined, undefined)).toEqual(DEFAULT_SLIDE_SETTINGS)
    })
  })

  describe('merge', () => {
    it('lets config override the defaults', () => {
      const config = revealConfig(
        deckSettings({ transition: 'fade', controls: false, transition_speed: 'slow' }, {}),
        FIXED
      )
      expect(config).toMatchObject({
        transition: 'fade',
        controls: false,
        progress: true,
        transitionSpeed: 'slow',
      })
    })

    it('lets frontmatter override config key by key', () => {
      const settings = deckSettings(
        { transition: 'fade', progress: false, slide_number: true },
        { transition: 'zoom', background_transition: 'concave' }
      )
      expect(revealConfig(settings, FIXED)).toMatchObject({
        transition: 'zoom',
        progress: false,
        slideNumber: true,
        backgroundTransition: 'concave',
        controls: true,
      })
    })

    it('maps every Reveal option to its camelCase name', () => {
      const config = revealConfig(
        deckSettings(
          {},
          {
            transition: 'none',
            transition_speed: 'fast',
            background_transition: 'fade',
            slide_number: 'h/v',
            controls: false,
            progress: false,
          }
        ),
        {}
      )
      expect(config).toEqual({
        transition: 'none',
        transitionSpeed: 'fast',
        backgroundTransition: 'fade',
        slideNumber: 'h/v',
        controls: false,
        progress: false,
      })
    })

    it('never passes the background settings to Reveal', () => {
      const config = revealConfig(
        deckSettings({ background_image: 'a.jpg', background_size: 'contain' }, {}),
        {}
      )
      expect(Object.keys(config).some((k) => k.startsWith('background') && k !== 'backgroundTransition')).toBe(
        false
      )
    })

    it('cannot override the fixed settings', () => {
      const config = revealConfig(
        { ...deckSettings({}, {}), ...({ hash: false, history: true } as object) },
        FIXED
      )
      expect(config.hash).toBe(true)
      expect(config.history).toBe(false)
      // And a setting named like a fixed one is not even read.
      expect(readSlideSettings({ hash: false, center: false }, 'x')).toEqual({})
    })
  })

  describe('validation', () => {
    it('ignores invalid frontmatter values and falls back to config', () => {
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      const settings = deckSettings(
        { transition: 'fade', slide_number: 'c' },
        { transition: 'spin', slide_number: 'x/y', controls: 'yes', background_opacity: 2 }
      )
      expect(settings.transition).toBe('fade')
      expect(settings.slide_number).toBe('c')
      expect(settings.controls).toBe(true)
      expect(settings.background_opacity).toBeUndefined()
      expect(warn).toHaveBeenCalledTimes(4)
    })

    it('ignores a non-object source', () => {
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      expect(readSlideSettings('fade', 'slides')).toEqual({})
      expect(readSlideSettings([1], 'slides')).toEqual({})
      expect(warn).toHaveBeenCalledTimes(2)
    })

    it('accepts every slide_number Reveal knows', () => {
      for (const value of [true, false, 'h.v', 'h/v', 'c', 'c/t']) {
        expect(readSlideSettings({ slide_number: value }, 'x')).toEqual({ slide_number: value })
      }
    })
  })

  describe('applyDefaultBackground', () => {
    function deck(html: string): Element {
      const slides = document.createElement('div')
      slides.className = 'slides'
      slides.innerHTML = html
      return slides
    }

    it('gives bare top-level slides the image, size and opacity', () => {
      const slides = deck('<section>a</section><section><section>v1</section></section>')
      const count = applyDefaultBackground(slides, {
        background_image: '../bg.jpg',
        background_size: 'contain',
        background_opacity: 0.5,
      })
      expect(count).toBe(2)
      const [first, stack] = Array.from(slides.children)
      expect(first.getAttribute('data-background-image')).toBe('../bg.jpg')
      expect(first.getAttribute('data-background-size')).toBe('contain')
      expect(first.getAttribute('data-background-opacity')).toBe('0.5')
      expect(stack.getAttribute('data-background-image')).toBe('../bg.jpg')
      // Nested (vertical) slides are left to their stack.
      expect(stack.querySelector('section')!.hasAttribute('data-background-image')).toBe(false)
    })

    it('leaves a slide with any background of its own alone', () => {
      const own = [
        'data-background="#336"',
        'data-background-image="mine.png"',
        'data-background-color="black"',
        'data-background-gradient="linear-gradient(red, blue)"',
        'data-background-video="v.mp4"',
        'data-background-iframe="https://example.com"',
      ]
      const slides = deck(own.map((attr) => `<section ${attr}>x</section>`).join(''))
      expect(applyDefaultBackground(slides, { background_image: 'bg.jpg' })).toBe(0)
      expect(slides.querySelector('[data-background-image="bg.jpg"]')).toBeNull()
      expect(slides.querySelector('[data-background-image="mine.png"]')).not.toBeNull()
    })

    it('still applies to a slide that only sets a background transition or size', () => {
      const slides = deck('<section data-background-transition="zoom">x</section>')
      expect(applyDefaultBackground(slides, { background_image: 'bg.jpg' })).toBe(1)
    })

    it('does nothing without an image', () => {
      const slides = deck('<section>a</section>')
      expect(applyDefaultBackground(slides, { background_size: 'contain' })).toBe(0)
      expect(slides.firstElementChild!.attributes.length).toBe(0)
    })
  })
})
