import { describe, expect, it } from 'vitest'
import {
  CONCENTRIC_DEFAULT_SIZE,
  PLACEMENT,
  challengingQuota,
  displaySize,
  placementIndex,
  rateConcentric,
  resizeConcentric,
  setConcentricEligible,
  startConcentric,
  type ConcentricState,
  type Rng,
} from './concentric.js'
import type { Rating } from './history.js'
import { seededRng } from './test-fixtures.js'

const seeded = seededRng

const range = (n: number) => Array.from({ length: n }, (_, i) => i)

function start(latest: (Rating | null)[], extra: { size?: number; threshold?: number; seed?: number; eligible?: number[] } = {}) {
  return startConcentric({
    latest,
    eligible: extra.eligible ?? range(latest.length),
    size: extra.size,
    threshold: extra.threshold ?? 0.7,
    rng: seeded(extra.seed ?? 1),
  })
}

/** Rate the front card `n` times with `rating` (or a per-turn function). */
function rateMany(state: ConcentricState, n: number, rating: Rating | ((card: number) => Rating), rng: Rng) {
  let s = state
  for (let i = 0; i < n; i++) {
    const r = typeof rating === 'function' ? rating(s.stack[0]) : rating
    s = rateConcentric(s, r, rng)
  }
  return s
}

function invariant(s: ConcentricState) {
  const all = [...s.stack, ...s.pool].sort((a, b) => a - b)
  expect(all).toEqual([...s.eligible].sort((a, b) => a - b))
  expect(new Set(s.stack).size).toBe(s.stack.length)
}

describe('challengingQuota', () => {
  it('is ceil(25% of X)', () => {
    expect(challengingQuota(6)).toBe(2)
    expect(challengingQuota(10)).toBe(3)
    expect(challengingQuota(20)).toBe(5)
    expect(challengingQuota(1)).toBe(1)
  })
})

describe('startConcentric', () => {
  it('picks the default six, all eligible accounted for', () => {
    const s = start(Array(20).fill('good'))
    expect(s.stack).toHaveLength(CONCENTRIC_DEFAULT_SIZE)
    expect(displaySize(s)).toBe(6)
    invariant(s)
  })

  it('caps X at the eligible count but keeps the requested target', () => {
    const s = start(Array(4).fill('good'), { size: 10 })
    expect(s.stack).toHaveLength(4)
    expect(s.target).toBe(10)
    expect(s.pool).toEqual([])
  })

  it.each([
    [6, 2],
    [10, 3],
    [20, 5],
  ])('X=%i seeds %i challenging cards, struggled first', (size, quota) => {
    // 3 struggled cards, 10 new, the rest known.
    const latest: (Rating | null)[] = Array(40).fill('easy')
    latest[3] = 'again'
    latest[17] = 'hard'
    latest[30] = 'again'
    for (let i = 5; i < 15; i++) latest[i] = null
    for (let seed = 1; seed <= 20; seed++) {
      const s = start(latest, { size, seed })
      const struggled = s.stack.filter((i) => latest[i] === 'again' || latest[i] === 'hard')
      const challenging = s.stack.filter((i) => latest[i] !== 'easy')
      expect(challenging.length).toBeGreaterThanOrEqual(quota)
      // Struggled cards fill the quota before any new card does.
      expect(struggled.length).toBe(Math.min(3, quota))
    }
  })

  it('fills the quota with new cards when nothing was struggled with', () => {
    const latest: (Rating | null)[] = Array(12).fill('good')
    latest[0] = null
    latest[11] = null
    const s = start(latest)
    expect(s.stack).toContain(0)
    expect(s.stack).toContain(11)
  })

  it('picks plain random when the note has no history at all', () => {
    const s = start(Array(30).fill(null), { seed: 7 })
    expect(s.seeded).toBe(false)
    expect(s.stack).toHaveLength(6)
    // Different seeds, different stacks: nothing forces a choice.
    const other = start(Array(30).fill(null), { seed: 8 })
    expect(other.stack).not.toEqual(s.stack)
  })

  it('only picks eligible cards', () => {
    const s = start(Array(10).fill('good'), { eligible: [1, 3, 5] })
    expect([...s.stack].sort()).toEqual([1, 3, 5])
  })
})

describe('placement', () => {
  it.each(Object.entries(PLACEMENT) as [Rating, readonly [number, number]][])(
    '%s lands in its fraction of L',
    (rating, [lo, hi]) => {
      const rng = seeded(3)
      for (const L of [2, 5, 10, 37]) {
        for (let k = 0; k < 200; k++) {
          const i = placementIndex(rating, L, rng)
          expect(i).toBeGreaterThanOrEqual(Math.max(rating === 'again' ? 1 : 0, Math.round(L * lo)))
          expect(i).toBeLessThanOrEqual(Math.round(L * hi))
          expect(i).toBeLessThanOrEqual(L)
        }
      }
    }
  )

  it('never puts Again straight back on screen when L ≥ 2', () => {
    // An rng pinned at 0 asks for the earliest slot every time.
    expect(placementIndex('again', 2, () => 0)).toBe(1)
    expect(placementIndex('again', 3, () => 0)).toBe(1)
    expect(placementIndex('again', 1, () => 0)).toBe(0)
  })

  it('puts Easy at the very end and clamps an empty rest to 0', () => {
    expect(placementIndex('easy', 9, () => 0)).toBe(9)
    expect(placementIndex('good', 0, () => 0.5)).toBe(0)
  })

  it('moves the rated card from the front to its slot', () => {
    const s = start(Array(10).fill('good'), { size: 6 })
    const [front, ...rest] = s.stack
    const next = rateConcentric(s, 'easy', seeded(1))
    expect(next.stack).toEqual([...rest, front])
  })
})

describe('growth', () => {
  it('grows by two only at the end of a pass that meets the threshold', () => {
    const rng = seeded(5)
    let s = start(Array(12).fill('good'), { size: 6 })
    s = rateMany(s, 5, 'good', rng)
    expect(s.stack).toHaveLength(6)
    s = rateConcentric(s, 'good', rng)
    expect(s.stack).toHaveLength(8)
    expect(s.target).toBe(8)
    expect(s.pool).toHaveLength(4)
    expect(s.passLength).toBe(8)
    invariant(s)
  })

  it('does not grow when the pass falls short (4 of 6 < 70%)', () => {
    const rng = seeded(5)
    let s = start(Array(12).fill('good'), { size: 6 })
    let n = 0
    s = rateMany(s, 6, () => (n++ < 2 ? 'again' : 'good'), rng)
    // The two Again cards may come back within the pass; their *first*
    // rating is what counts, so the pass is 4/6 at best.
    expect(s.stack).toHaveLength(6)
    expect(s.passCount).toBe(0)
  })

  it('counts only each card’s first rating in a pass', () => {
    // A 2-card stack: Again on the front card puts it straight back (L = 1),
    // so the pass's second rating is the same card answered Good. Its first
    // answer was Again: 0 of 1 distinct cards known, no growth.
    const rng = seeded(9)
    let s = start(Array(6).fill('good'), { size: 2 })
    const a = s.stack[0]
    s = rateConcentric(s, 'again', rng)
    expect(s.stack[0]).toBe(a)
    s = rateConcentric(s, 'good', rng)
    expect(s.passCount).toBe(0)
    expect(s.stack).toHaveLength(2)
    // Same stack, both cards' first answers Good: grows.
    let t = start(Array(6).fill('good'), { size: 2 })
    t = rateMany(t, 2, 'good', rng)
    expect(t.stack).toHaveLength(4)
  })

  it('a pass judges distinct cards, not ratings', () => {
    // 1-card stack + a repeat would be one distinct card; with 3 cards where
    // one card is seen twice (Again then Good), firsts are {Again, Good}.
    const s0 = start(Array(6).fill('good'), { size: 3, seed: 2 })
    let s = rateConcentric(s0, 'again', () => 0) // L=2 → index 1
    s = rateConcentric(s, 'good', () => 0.99)
    s = rateConcentric(s, 'good', () => 0.99) // the Again card again
    expect(s.passFirsts.size).toBe(0) // pass reset
    expect(s.stack).toHaveLength(3) // 1 of 2 distinct firsts good < 70%
  })

  it('reports mastery, without growing, once every eligible card is in play', () => {
    const rng = seeded(4)
    let s = start(Array(3).fill('good'), { size: 6 })
    expect(s.pool).toEqual([])
    s = rateMany(s, 2, 'easy', rng)
    expect(s.mastered).toBe(false)
    s = rateConcentric(s, 'easy', rng)
    expect(s.mastered).toBe(true)
    expect(s.stack).toHaveLength(3)
    // Only the rating that completed the pass says so.
    s = rateConcentric(s, 'easy', rng)
    expect(s.mastered).toBe(false)
  })

  it('does not grow with an empty pool when the pass fails', () => {
    const rng = seeded(4)
    let s = start(Array(3).fill('good'), { size: 3 })
    s = rateMany(s, 3, 'again', rng)
    expect(s.stack).toHaveLength(3)
    expect(s.mastered).toBe(false)
  })

  it('grows by what the pool has left when that is less than two', () => {
    const rng = seeded(4)
    let s = start(Array(7).fill('good'), { size: 6 })
    s = rateMany(s, 6, 'good', rng)
    expect(s.stack).toHaveLength(7)
    expect(s.pool).toEqual([])
  })

  it('respects a custom threshold', () => {
    const rng = seeded(4)
    // Easy, Easy (to the back), then Again, Again on two fresh cards: 2 of 4.
    const ratings: Rating[] = ['easy', 'easy', 'again', 'again']
    const run = (threshold: number) => {
      let s = start(Array(10).fill('good'), { size: 4, threshold })
      for (const r of ratings) s = rateConcentric(s, r, rng)
      return s
    }
    expect(run(0.5).stack).toHaveLength(6)
    expect(run(0.7).stack).toHaveLength(4)
  })
})

describe('resizeConcentric', () => {
  it('clamps to 1..eligible', () => {
    const s = start(Array(8).fill('good'))
    expect(resizeConcentric(s, 0, seeded(1)).stack).toHaveLength(1)
    expect(resizeConcentric(s, 99, seeded(1)).stack).toHaveLength(8)
    expect(resizeConcentric(s, 99, seeded(1)).target).toBe(8)
  })

  it('grows from the pool, keeps the card on screen, and resets the pass', () => {
    const rng = seeded(2)
    let s = start(Array(12).fill('good'))
    s = rateConcentric(s, 'good', rng)
    const front = s.stack[0]
    s = resizeConcentric(s, 9, rng)
    expect(s.stack).toHaveLength(9)
    expect(s.stack[0]).toBe(front)
    expect(s.passCount).toBe(0)
    expect(s.passLength).toBe(9)
    invariant(s)
  })

  it('raising X tops the challenging cards up to ceil(25% × X)', () => {
    // Six known cards in play, then the pool holds 2 struggled and 3 new.
    const latest: (Rating | null)[] = Array(20).fill('easy')
    latest[12] = 'again'
    latest[13] = 'hard'
    latest[14] = null
    latest[15] = null
    latest[16] = null
    // Start from known cards only (eligible), then widen with the rest.
    let s = start(latest, { size: 6, eligible: range(10) })
    expect(s.stack.every((i) => latest[i] === 'easy')).toBe(true)
    s = setConcentricEligible(s, range(20), seeded(1))
    expect(s.stack).toHaveLength(6) // already at X
    for (let seed = 1; seed <= 10; seed++) {
      const grown = resizeConcentric(s, 16, seeded(seed))
      const challenging = grown.stack.filter((i) => latest[i] !== 'easy')
      expect(challenging.length).toBeGreaterThanOrEqual(4) // ceil(0.25 × 16)
      // Struggled first.
      expect(grown.stack).toContain(12)
      expect(grown.stack).toContain(13)
    }
  })

  it('shrinks by removing the best-known cards first, never the front', () => {
    const rng = seeded(3)
    let s = start(Array(10).fill('good'), { size: 5 })
    const order = s.stack.slice()
    // Rate four cards (front each time): easy, good, hard, again.
    const ratings: Rating[] = ['easy', 'good', 'hard', 'again']
    for (const r of ratings) s = rateConcentric(s, r, () => 0.999)
    const byRating = new Map(ratings.map((r, i) => [r, order[i]]))
    // Two removed: Easy first, then Good.
    const shrunk = resizeConcentric(s, 3, rng)
    expect(shrunk.stack).toHaveLength(3)
    expect(shrunk.stack).not.toContain(byRating.get('easy'))
    expect(shrunk.stack).not.toContain(byRating.get('good'))
    expect(shrunk.pool).toContain(byRating.get('easy'))
    expect(shrunk.stack[0]).toBe(s.stack[0])
    // One more: the unrated card goes before Hard and Again.
    const smaller = resizeConcentric(shrunk, 2, rng)
    expect(smaller.stack).toContain(byRating.get('again'))
    invariant(smaller)
  })
})

describe('setConcentricEligible', () => {
  it('drops ineligible cards and refills toward X from the new pool', () => {
    const rng = seeded(6)
    const s = start(Array(20).fill('good'), { size: 6 })
    const keep = range(20).filter((i) => i % 2 === 0)
    const next = setConcentricEligible(s, keep, rng)
    expect(next.stack).toHaveLength(6)
    expect(next.stack.every((i) => i % 2 === 0)).toBe(true)
    expect(next.target).toBe(6)
    invariant(next)
  })

  it('keeps X as the target when the filter leaves fewer cards', () => {
    const rng = seeded(6)
    const s = start(Array(20).fill('good'), { size: 8 })
    const narrow = setConcentricEligible(s, [1, 2, 3], rng)
    expect(narrow.stack).toHaveLength(3)
    expect(displaySize(narrow)).toBe(3)
    expect(narrow.target).toBe(8)
    const wide = setConcentricEligible(narrow, range(20), rng)
    expect(wide.stack).toHaveLength(8)
    expect(wide.passCount).toBe(0)
  })
})
