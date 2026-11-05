import { describe, it, expect } from 'vitest'
import { MIN_FONT_SCALE, MIN_IMAGE_SHARE, fitSlide, largestFittingScale, resetSlideFit } from './slide-fit.ts'

describe('UNIT largestFittingScale', () => {
  /** Content `natural` px tall at scale 1, shrinking linearly with the scale. */
  function linear(natural: number): { measure: (s: number) => number; applied: () => number; calls: () => number } {
    let current = 1
    let count = 0
    return {
      measure: (s) => {
        current = s
        count++
        return natural * s
      },
      applied: () => current,
      calls: () => count,
    }
  }

  it('keeps scale 1 for content that fits, measuring once', () => {
    const m = linear(500)
    expect(largestFittingScale(m.measure, 700)).toBe(1)
    expect(m.calls()).toBe(1)
  })

  it('finds the largest scale that fits, to within a percent, and leaves it applied', () => {
    const m = linear(1000)
    const scale = largestFittingScale(m.measure, 700)
    expect(scale).toBeLessThanOrEqual(0.7)
    expect(scale).toBeGreaterThan(0.69)
    expect(m.applied()).toBe(scale)
    expect(1000 * scale).toBeLessThanOrEqual(700)
  })

  it('stops at the minimum when nothing fits', () => {
    const m = linear(5000)
    expect(largestFittingScale(m.measure, 700)).toBe(MIN_FONT_SCALE)
    expect(m.applied()).toBe(MIN_FONT_SCALE)
  })

  it('is cheap: a fixed handful of measurements', () => {
    const m = linear(900)
    largestFittingScale(m.measure, 700)
    expect(m.calls()).toBeLessThanOrEqual(9)
  })
})

/**
 * happy-dom lays nothing out, so the slide's height is modelled: text that
 * scales with the slide's font size, plus each image at its `max-height` or
 * natural height.
 */
function modelSlide(textHeight: number, imageHeights: number[]): HTMLElement {
  const slide = document.createElement('section')
  slide.innerHTML = '<h2>Title</h2>' + imageHeights.map(() => '<p><img src="x.png"></p>').join('')
  const images = Array.from(slide.querySelectorAll('img'))
  images.forEach((img, i) => {
    Object.defineProperty(img, 'offsetHeight', {
      get: () => {
        const cap = parseFloat(img.style.maxHeight)
        return Number.isNaN(cap) ? imageHeights[i]! : Math.min(cap, imageHeights[i]!)
      },
    })
  })
  Object.defineProperty(slide, 'scrollHeight', {
    get: () => {
      const percent = parseFloat(slide.style.fontSize)
      const scale = Number.isNaN(percent) ? 1 : percent / 100
      return textHeight * scale + images.reduce((sum, img) => sum + img.offsetHeight, 0)
    },
  })
  return slide
}

describe('UNIT fitSlide', () => {
  it('leaves a slide that fits untouched', () => {
    const slide = modelSlide(300, [200])
    expect(fitSlide(slide, 700)).toBe(true)
    expect(slide.style.fontSize).toBe('')
    expect(slide.querySelector('img')!.style.maxHeight).toBe('')
  })

  it('caps an image to the room the text leaves, before touching the text', () => {
    const slide = modelSlide(300, [1200])
    expect(fitSlide(slide, 700)).toBe(true)
    expect(slide.querySelector('img')!.style.maxHeight).toBe('400px')
    expect(slide.style.fontSize).toBe('')
  })

  it('never squeezes an image below its share, and shrinks the text instead', () => {
    const slide = modelSlide(650, [1200])
    expect(fitSlide(slide, 700)).toBe(true)
    expect(slide.querySelector('img')!.style.maxHeight).toBe(`${Math.floor(700 * MIN_IMAGE_SHARE)}px`)
    expect(parseFloat(slide.style.fontSize)).toBeLessThan(100)
    expect(slide.scrollHeight).toBeLessThanOrEqual(700)
  })

  it('shrinks text-only slides to fit', () => {
    const slide = modelSlide(1000, [])
    expect(fitSlide(slide, 700)).toBe(true)
    expect(parseFloat(slide.style.fontSize)).toBeLessThanOrEqual(70)
  })

  it('reports a slide that cannot fit even at the smallest text', () => {
    const slide = modelSlide(3000, [])
    expect(fitSlide(slide, 700)).toBe(false)
    expect(slide.style.fontSize).toBe(`${MIN_FONT_SCALE * 100}%`)
  })

  it('starts each fit from the authored slide (a resize can make it fit again)', () => {
    const slide = modelSlide(1000, [900])
    fitSlide(slide, 700)
    resetSlideFit(slide)
    expect(slide.style.fontSize).toBe('')
    expect(slide.querySelector('img')!.style.maxHeight).toBe('')
    expect(slide.querySelector('[data-mbr-fit]')).toBeNull()
    expect(fitSlide(slide, 3000)).toBe(true)
    expect(slide.style.fontSize).toBe('')
  })
})
