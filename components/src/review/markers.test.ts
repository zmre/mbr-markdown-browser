import { describe, expect, it } from 'vitest'
import { passageHitsAt, type HitRect } from './markers.ts'

const box = (left: number, top: number, right: number, bottom: number): HitRect => ({ left, top, right, bottom })

describe('UNIT passageHitsAt', () => {
  // A quote wrapped over two lines, and a second quote on the line below.
  const wrapped = { rects: [box(100, 0, 400, 20), box(0, 24, 150, 44)], value: 'a' }
  const below = { rects: [box(0, 60, 300, 80)], value: 'b' }

  it('finds the passage under the point, and the line box hit', () => {
    expect(passageHitsAt(120, 30, [wrapped, below])).toEqual({ values: ['a'], rect: box(0, 24, 150, 44) })
    expect(passageHitsAt(10, 70, [wrapped, below])).toEqual({ values: ['b'], rect: box(0, 60, 300, 80) })
  })

  it('misses outside every box (the unquoted rest of a wrapped line too)', () => {
    expect(passageHitsAt(50, 10, [wrapped, below])).toEqual({ values: [], rect: null })
    expect(passageHitsAt(350, 50, [wrapped, below])).toEqual({ values: [], rect: null })
  })

  it('bridges the gap between wrapped lines by the slop, not more', () => {
    expect(passageHitsAt(120, 22, [wrapped]).values).toEqual(['a'])
    expect(passageHitsAt(120, 22, [wrapped], 0).values).toEqual([])
    expect(passageHitsAt(120, 52, [wrapped]).values).toEqual([])
  })

  it('lists every note over one passage, in order, each once', () => {
    const same = { rects: [box(0, 24, 150, 44), box(0, 24, 150, 44)], value: 'c' }
    expect(passageHitsAt(10, 30, [wrapped, same]).values).toEqual(['a', 'c'])
  })

  it('does nothing for a page without passages', () => {
    expect(passageHitsAt(0, 0, [])).toEqual({ values: [], rect: null })
  })
})
