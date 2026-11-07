import { describe, expect, it } from 'vitest'
import { DEFAULT_DURATION, drawnEnd, layoutAgenda, visibleRange } from './agenda-layout.ts'

const hm = (h: number, m = 0): number => h * 60 + m

describe('visibleRange', () => {
  it('shows the working day for an empty or small agenda', () => {
    expect(visibleRange([])).toEqual({ from: hm(7), to: hm(19) })
    expect(visibleRange([{ start: hm(10), end: hm(11) }])).toEqual({ from: hm(7), to: hm(19) })
  })

  it('widens to early and late entries with half an hour of margin, snapped to hours', () => {
    expect(visibleRange([{ start: hm(6, 45) }, { start: hm(21), end: hm(21, 40) }])).toEqual({
      from: hm(6),
      to: hm(23),
    })
  })

  it('stays inside the day', () => {
    expect(visibleRange([{ start: hm(0, 10) }, { start: hm(23, 50) }])).toEqual({ from: 0, to: hm(24) })
  })
})

describe('layoutAgenda', () => {
  const place = (items: { start: number; end?: number }[]) =>
    layoutAgenda(items).map((b) => [b.item.start, b.column, b.columns, b.open])

  it('gives non-overlapping entries the full width', () => {
    expect(place([{ start: hm(9), end: hm(10) }, { start: hm(10), end: hm(11) }])).toEqual([
      [hm(9), 0, 1, false],
      [hm(10), 0, 1, false],
    ])
  })

  it('puts overlapping entries side by side, greedy by start then end', () => {
    expect(
      place([
        { start: hm(9), end: hm(11) },
        { start: hm(9, 30), end: hm(10) },
        { start: hm(10), end: hm(10, 30) },
        { start: hm(10, 15), end: hm(12) },
      ])
    ).toEqual([
      [hm(9), 0, 3, false],
      [hm(9, 30), 1, 3, false],
      [hm(10), 1, 3, false],
      [hm(10, 15), 2, 3, false],
    ])
  })

  it('draws an entry without an end as a default-length open block', () => {
    const [block] = layoutAgenda([{ start: hm(9) }])
    expect(block.open).toBe(true)
    expect(block.end - block.start).toBe(DEFAULT_DURATION)
    // Two open entries half an hour apart do not overlap…
    expect(place([{ start: hm(9) }, { start: hm(9, 30) }]).map((p) => p[2])).toEqual([1, 1])
    // …but ten minutes apart they do.
    expect(place([{ start: hm(9) }, { start: hm(9, 10) }]).map((p) => p[2])).toEqual([2, 2])
  })

  it('sorts its output by start and keeps clusters independent', () => {
    expect(
      place([
        { start: hm(14), end: hm(15) },
        { start: hm(9), end: hm(10) },
        { start: hm(9), end: hm(9, 30) },
      ])
    ).toEqual([
      [hm(9), 0, 2, false],
      [hm(9), 1, 2, false],
      [hm(14), 0, 1, false],
    ])
  })

  it('caps a late open block at midnight', () => {
    expect(drawnEnd({ start: hm(23, 50) })).toBe(hm(24))
  })
})
