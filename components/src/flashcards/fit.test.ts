import { describe, expect, it } from 'vitest'
import { MAX_FONT_PX, MIN_FONT_PX, fitFontSize } from './fit.js'

describe('fitFontSize', () => {
  it('finds the largest size that fits', () => {
    for (const limit of [16, 17, 23, 40, 71]) {
      expect(fitFontSize((px) => px <= limit)).toBe(limit)
    }
  })

  it('returns the maximum when everything fits, without searching', () => {
    let calls = 0
    expect(fitFontSize(() => (calls++, true))).toBe(MAX_FONT_PX)
    expect(calls).toBe(1)
  })

  it('falls back to the minimum when nothing fits (the face scrolls)', () => {
    expect(fitFontSize(() => false)).toBe(MIN_FONT_PX)
  })

  it('needs only a handful of layouts', () => {
    let calls = 0
    fitFontSize((px) => (calls++, px <= 30))
    expect(calls).toBeLessThanOrEqual(8)
  })
})
