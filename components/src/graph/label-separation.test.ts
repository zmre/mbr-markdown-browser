import { describe, it, expect } from 'vitest'
import { separateLabels, type LabelledPoint } from './label-separation.js'

const label = (x: number, y: number, w = 60): LabelledPoint => ({ x, y, w, h: 13, dy: 12 })

function overlapping(points: LabelledPoint[]): number {
  let n = 0
  for (let i = 0; i < points.length; i++) {
    for (let j = i + 1; j < points.length; j++) {
      const a = points[i]
      const b = points[j]
      if (Math.abs(a.x - b.x) < (a.w + b.w) / 2 && Math.abs(a.y + a.dy - (b.y + b.dy)) < (a.h + b.h) / 2) n++
    }
  }
  return n
}

describe('separateLabels', () => {
  it('leaves non-overlapping labels untouched', () => {
    const points = [label(0, 0), label(200, 0), label(0, 100)]
    const before = points.map((p) => ({ ...p }))
    expect(separateLabels(points)).toBe(0)
    expect(points).toEqual(before)
  })

  it('separates side-by-side labels vertically when that is the cheaper move', () => {
    const points = [label(0, 0), label(30, 4)]
    expect(separateLabels(points)).toBe(0)
    expect(overlapping(points)).toBe(0)
    // Mostly vertical motion: penetration was smaller on y.
    expect(Math.abs(points[0].x)).toBeLessThan(1)
  })

  it('separates identical positions deterministically', () => {
    const a = [label(10, 10), label(10, 10)]
    const b = [label(10, 10), label(10, 10)]
    separateLabels(a)
    separateLabels(b)
    expect(a).toEqual(b)
    expect(overlapping(a)).toBe(0)
  })

  it('leaves no overlap for any random layout (seeded)', () => {
    let seed = 12345
    const rand = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648)
    for (let trial = 0; trial < 200; trial++) {
      const n = 2 + Math.floor(rand() * 79)
      const points = Array.from({ length: n }, () => label(rand() * 300, rand() * 200, 20 + rand() * 120))
      expect(separateLabels(points)).toBe(0)
      expect(overlapping(points)).toBe(0)
    }
  })

  it('untangles a dense cluster', () => {
    const points = Array.from({ length: 40 }, (_, i) => label((i * 37) % 120, (i * 53) % 90, 50 + (i % 5) * 10))
    expect(overlapping(points)).toBeGreaterThan(0)
    expect(separateLabels(points)).toBe(0)
    expect(overlapping(points)).toBe(0)
  })
})
