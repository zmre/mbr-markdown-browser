import { describe, expect, it } from 'vitest'
import {
  CONCENTRIC_DEFAULT_SIZE,
  MIN_GAP,
  NO_CHANGE,
  PLACEMENT,
  RETIRE_AFTER_EASY,
  cardsLeft,
  challengingQuota,
  displaySize,
  isMasteredState,
  placementIndex,
  rateConcentric,
  resizeConcentric,
  setConcentricEligible,
  startConcentric,
  type ConcentricState,
  type Rng,
} from './concentric.js'
import { RATINGS, type Rating } from './history.js'
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

/**
 * A state laid out by hand: `stack` front first, the pool the rest of
 * `eligible` (document order). Lets a test start from exactly the situation it
 * is about rather than steering a seeded session into it.
 */
function layout(
  stack: number[],
  options: {
    eligible?: number[]
    latest?: (Rating | null)[]
    session?: [number, Rating][]
    streaks?: [number, number][]
    retired?: number[]
    threshold?: number
    target?: number
  } = {}
): ConcentricState {
  const eligible = options.eligible ?? range(Math.max(...stack) + 1)
  const inPlay = new Set(stack)
  return {
    stack,
    pool: eligible.filter((i) => !inPlay.has(i)),
    eligible,
    target: options.target ?? stack.length,
    threshold: options.threshold ?? 0.7,
    latest: options.latest ?? eligible.map(() => null),
    seeded: (options.latest ?? []).some((r) => r !== null),
    sessionRatings: new Map(options.session ?? []),
    easyStreaks: new Map(options.streaks ?? []),
    retired: new Set(options.retired ?? []),
    mastered: false,
    lastChange: NO_CHANGE,
  }
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

const sorted = (xs: Iterable<number>) => Array.from(xs).sort((a, b) => a - b)

function invariant(s: ConcentricState) {
  expect(sorted([...s.stack, ...s.pool])).toEqual(sorted(s.eligible))
  expect(new Set(s.stack).size).toBe(s.stack.length)
  expect(new Set(s.pool).size).toBe(s.pool.length)
  const pool = new Set(s.pool)
  for (const card of s.retired) expect(pool.has(card)).toBe(true)
  for (const [card, streak] of s.easyStreaks) {
    expect(s.eligible).toContain(card)
    expect(streak).toBeGreaterThan(0)
    expect(streak).toBeLessThan(RETIRE_AFTER_EASY)
  }
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
    expect(s.lastChange).toEqual(NO_CHANGE)
    expect(s.retired.size).toBe(0)
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

  it('keeps the rest of a seeded start random, not all challenging', () => {
    // Plenty of struggled and new cards: a strict priority order would fill
    // all six with them. The start keeps most of the stack likely known.
    const latest: (Rating | null)[] = Array(40).fill('easy')
    for (let i = 0; i < 20; i++) latest[i] = i % 2 === 0 ? 'again' : null
    let sawKnown = false
    for (let seed = 1; seed <= 20; seed++) {
      const s = start(latest, { seed })
      if (s.stack.some((i) => latest[i] === 'easy')) sawKnown = true
    }
    expect(sawKnown).toBe(true)
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
    expect(sorted(s.stack)).toEqual([1, 3, 5])
  })
})

describe('placement', () => {
  it.each(Object.entries(PLACEMENT) as [Rating, readonly [number, number]][])(
    '%s lands in its fraction of L, raised to its floor',
    (rating, [lo, hi]) => {
      const rng = seeded(3)
      const floor = MIN_GAP[rating]
      for (const L of [1, 2, 3, 4, 5, 10, 37]) {
        const min = Math.min(L, Math.max(floor, Math.round(L * lo)))
        const max = Math.min(L, Math.max(floor, Math.round(L * hi)))
        for (let k = 0; k < 200; k++) {
          const i = placementIndex(rating, L, rng)
          expect(i).toBeGreaterThanOrEqual(min)
          expect(i).toBeLessThanOrEqual(max)
        }
      }
    }
  )

  it('floors Again at three cards in between, Hard at four', () => {
    expect(MIN_GAP.again).toBe(3)
    expect(MIN_GAP.hard).toBe(MIN_GAP.again + 1)
    expect(MIN_GAP.good).toBe(MIN_GAP.hard)
    // An rng pinned at 0 asks for the earliest slot every time.
    expect(placementIndex('again', 10, () => 0)).toBe(3) // 20% of 10 = 2, raised
    expect(placementIndex('again', 30, () => 0)).toBe(6) // 20% of 30 is past the floor
    expect(placementIndex('hard', 5, () => 0)).toBe(4) // 40% of 5 = 2, raised
    expect(placementIndex('hard', 20, () => 0)).toBe(8)
  })

  it('sends a card to the very back when the stack is smaller than its floor', () => {
    for (const L of [1, 2, 3]) expect(placementIndex('again', L, () => 0)).toBe(L)
    for (const L of [1, 2, 3, 4]) expect(placementIndex('hard', L, () => 0)).toBe(L)
    for (const L of [1, 2, 3, 4]) expect(placementIndex('good', L, () => 0)).toBe(L)
  })

  it('never brings Good back before Hard’s earliest slot', () => {
    // 75% of 4 or 5 would be index 3 or 4: the floor lifts Good to Hard's.
    expect(placementIndex('good', 5, () => 0)).toBe(4)
    expect(placementIndex('good', 20, () => 0)).toBe(15)
    const rng = seeded(11)
    for (const L of [1, 2, 3, 4, 5, 6, 8, 12]) {
      const hardMin = Math.min(L, Math.max(MIN_GAP.hard, Math.round(L * PLACEMENT.hard[0])))
      for (let k = 0; k < 100; k++) expect(placementIndex('good', L, rng)).toBeGreaterThanOrEqual(hardMin)
    }
  })

  it('puts Easy at the very end and clamps an empty rest to 0', () => {
    expect(placementIndex('easy', 9, () => 0)).toBe(9)
    expect(placementIndex('good', 0, () => 0.5)).toBe(0)
    expect(placementIndex('again', 0, () => 0.5)).toBe(0)
  })

  it('moves the rated card from the front to its slot', () => {
    const s = start(Array(10).fill('good'), { size: 6 })
    const [front, ...rest] = s.stack
    const next = rateConcentric(s, 'easy', seeded(1))
    expect(next.stack).toEqual([...rest, front])
  })

  it('shows three other cards before an Again card returns', () => {
    const s = layout(range(10), { threshold: 1 })
    const next = rateConcentric(s, 'again', () => 0)
    expect(next.stack.slice(0, 4)).toEqual([1, 2, 3, 0])
  })

  it('shows four other cards before a Hard card returns', () => {
    const s = layout(range(10), { threshold: 1 })
    const next = rateConcentric(s, 'hard', () => 0)
    expect(next.stack.slice(0, 5)).toEqual([1, 2, 3, 4, 0])
  })
})

describe('growth', () => {
  it('folds cards in one at a time until the known share drops below the bar (6/6 → 9)', () => {
    // Five of six already known this session; rating the sixth Good makes 6/6.
    // 6/7 and 6/8 still meet 70%, 6/9 does not.
    const s = layout(range(6), { eligible: range(12), session: [1, 2, 3, 4, 5].map((i) => [i, 'good']) })
    const next = rateConcentric(s, 'good', seeded(2))
    expect(next.stack).toHaveLength(9)
    expect(next.pool).toHaveLength(3)
    expect(next.target).toBe(9)
    expect(next.lastChange).toEqual({ added: 3, retired: 0 })
    invariant(next)
  })

  it('grows as soon as the share is met, mid-way through the stack', () => {
    // 5/6 known meets 70%: +1 (5/7 = 71%), +1 (5/8 = 62.5%) stops.
    const rng = seeded(5)
    let s = start(Array(12).fill('good'), { size: 6 })
    s = rateMany(s, 4, 'good', rng)
    expect(s.stack).toHaveLength(6)
    expect(s.lastChange).toEqual(NO_CHANGE)
    s = rateConcentric(s, 'good', rng)
    expect(s.stack).toHaveLength(8)
    expect(s.target).toBe(8)
    expect(s.lastChange).toEqual({ added: 2, retired: 0 })
    invariant(s)
  })

  it('does not count history: a card is known only once rated Good or Easy this session', () => {
    const rng = seeded(5)
    let s = start(Array(12).fill('easy'), { size: 6 })
    s = rateMany(s, 4, 'good', rng)
    // 4 of 6: the other two are Easy in the history but unrated here.
    expect(s.stack).toHaveLength(6)
  })

  it('counts a card’s latest rating, so a missed card stops counting', () => {
    const s = layout(range(4), { eligible: range(8), threshold: 0.75, session: [[1, 'good'], [2, 'good']] })
    // Front 0 rated Good → 3/4 = 75%: grows.
    expect(rateConcentric(s, 'good', seeded(1)).stack.length).toBeGreaterThan(4)
    // Front 0 rated Again with card 1 since missed → 1/4: no growth.
    const missed = layout(range(4), { eligible: range(8), threshold: 0.75, session: [[1, 'again'], [2, 'good']] })
    expect(rateConcentric(missed, 'good', seeded(1)).stack).toHaveLength(4)
  })

  it('inserts new cards behind the front', () => {
    for (let seed = 1; seed <= 20; seed++) {
      const s = layout(range(6), { eligible: range(12), session: [1, 2, 3, 4, 5].map((i) => [i, 'good']) })
      const next = rateConcentric(s, 'good', seeded(seed))
      expect(next.stack[0]).toBe(1)
    }
  })

  it('picks growth by priority: struggled, never rated, Good, Easy', () => {
    const latest: (Rating | null)[] = [null, null, 'easy', 'good', null, 'hard', 'again']
    // A one-card stack at threshold 0.6: each new card is unrated, so the
    // stack grows from 1/1 to 1/2 and stops — exactly one pick per rating.
    for (let seed = 1; seed <= 10; seed++) {
      const s = layout([0], { eligible: range(7), latest, threshold: 0.6 })
      const next = rateConcentric(s, 'good', seeded(seed))
      expect(next.lastChange.added).toBe(1)
      const [added] = next.stack.filter((i) => i !== 0)
      expect([5, 6]).toContain(added)
    }
    // With the struggled ones gone: the never-rated card, then Good, then Easy.
    const noStruggled = (pool: number[]) => layout([0], { eligible: [0, ...pool], latest, threshold: 0.6 })
    expect(rateConcentric(noStruggled([2, 3, 4]), 'good', seeded(1)).stack).toContain(4)
    expect(rateConcentric(noStruggled([2, 3]), 'good', seeded(1)).stack).toContain(3)
    expect(rateConcentric(noStruggled([2]), 'good', seeded(1)).stack).toContain(2)
  })

  it('never folds retired cards back in by growth', () => {
    const s = layout([0], { eligible: [0, 1], threshold: 0.5, retired: [1] })
    const next = rateConcentric(s, 'good', seeded(1))
    expect(next.stack).toEqual([0])
    expect(next.lastChange).toEqual(NO_CHANGE)
  })

  it('respects a custom threshold', () => {
    const s = () => layout(range(4), { eligible: range(10), session: [[1, 'good']] })
    // 2 of 4 known after this rating.
    expect(rateConcentric({ ...s(), threshold: 0.5 }, 'good', seeded(1)).stack.length).toBeGreaterThan(4)
    expect(rateConcentric({ ...s(), threshold: 0.7 }, 'good', seeded(1)).stack).toHaveLength(4)
  })

  it('stops at what the pool holds', () => {
    const s = layout(range(6), { eligible: range(7), session: [1, 2, 3, 4, 5].map((i) => [i, 'good']) })
    const next = rateConcentric(s, 'good', seeded(1))
    expect(next.stack).toHaveLength(7)
    expect(next.pool).toEqual([])
    expect(next.lastChange).toEqual({ added: 1, retired: 0 })
  })
})

describe('retirement', () => {
  it(`retires a card after ${RETIRE_AFTER_EASY} Easy ratings in a row`, () => {
    // Two cards, nothing else eligible: Easy alternates them.
    let s = layout([0, 1], { threshold: 1 })
    s = rateConcentric(s, 'easy', seeded(1)) // 0: streak 1
    expect(s.easyStreaks.get(0)).toBe(1)
    s = rateConcentric(s, 'easy', seeded(1)) // 1: streak 1
    expect(s.stack).toEqual([0, 1])
    s = rateConcentric(s, 'easy', seeded(1)) // 0: retired
    expect(s.stack).toEqual([1])
    expect(s.pool).toEqual([0])
    expect(s.retired.has(0)).toBe(true)
    expect(s.easyStreaks.has(0)).toBe(false)
    // Nothing to swap in: the stack shrinks.
    expect(s.lastChange).toEqual({ added: 0, retired: 1 })
    expect(cardsLeft(s)).toBe(0)
    invariant(s)
  })

  it('resets the streak on any other rating', () => {
    let s = layout([0, 1], { threshold: 1 })
    s = rateMany(s, 2, 'easy', seeded(1)) // 0 and 1 at streak 1
    s = rateConcentric(s, 'good', seeded(1)) // 0 reset
    expect(s.easyStreaks.has(0)).toBe(false)
    s = rateConcentric(s, 'easy', seeded(1)) // 1 retires
    expect(s.retired.has(1)).toBe(true)
    expect(s.stack).toEqual([0])
    s = rateConcentric(s, 'easy', seeded(1)) // 0 back at streak 1
    expect(s.stack).toEqual([0])
    expect(s.easyStreaks.get(0)).toBe(1)
    s = rateConcentric(s, 'easy', seeded(1))
    // 0 retires; the only card to swap in is 1, retired earlier, which comes
    // back with its flag cleared.
    expect(s.stack).toEqual([1])
    expect(s.retired).toEqual(new Set([0]))
    expect(s.lastChange).toEqual({ added: 1, retired: 1 })
  })

  it('ignores history: two Easy ratings in this session are needed', () => {
    const s = layout([0, 1], { latest: ['easy', 'easy'], threshold: 1 })
    expect(rateConcentric(s, 'easy', seeded(1)).retired.size).toBe(0)
  })

  it('swaps the best pool card in, behind the new front', () => {
    const latest: (Rating | null)[] = [null, null, null, 'easy', 'good', null, 'hard']
    for (let seed = 1; seed <= 10; seed++) {
      const s = layout([0, 1, 2], { eligible: range(7), latest, streaks: [[0, 1]], threshold: 1 })
      const next = rateConcentric(s, 'easy', seeded(seed))
      expect(next.lastChange).toEqual({ added: 1, retired: 1 })
      expect(next.stack).toHaveLength(3)
      expect(next.stack).toContain(6) // Hard before never-rated
      expect(next.stack[0]).toBe(1)
      expect(next.retired).toEqual(new Set([0]))
      invariant(next)
    }
  })

  it('picks retired cards last, and a returning card starts over', () => {
    const latest: (Rating | null)[] = [null, null, 'again', 'easy']
    // Pool: 2 (retired, though struggled) and 3 (Easy): 3 comes first.
    const s = layout([0, 1], { eligible: range(4), latest, streaks: [[0, 1]], retired: [2], threshold: 1 })
    const next = rateConcentric(s, 'easy', seeded(1))
    expect(next.stack).toContain(3)
    expect(next.retired).toEqual(new Set([0, 2]))
    // Only retired cards left: the swap brings one back, other than the card
    // just put away, and clears its flag and streak.
    const only = layout([0, 1], { eligible: range(3), streaks: [[0, 1], [2, 1]], retired: [2], threshold: 1 })
    const back = rateConcentric(only, 'easy', seeded(1))
    expect(back.stack).toEqual([1, 2])
    expect(back.retired).toEqual(new Set([0]))
    expect(back.easyStreaks.has(2)).toBe(false)
    expect(back.lastChange).toEqual({ added: 1, retired: 1 })
    invariant(back)
  })

  it('retires before growing on the same rating', () => {
    // 0 retires; 1 and 2 are known, 3 swaps in unrated: 2/3 ≥ 60% grows by one
    // (2/4 = 50% stops).
    const s = layout([0, 1, 2], {
      eligible: range(6),
      streaks: [[0, 1]],
      session: [[0, 'easy'], [1, 'good'], [2, 'good']],
      threshold: 0.6,
    })
    const next = rateConcentric(s, 'easy', seeded(4))
    expect(next.lastChange).toEqual({ added: 2, retired: 1 })
    expect(next.stack).toHaveLength(4)
    expect(next.stack).not.toContain(0)
    invariant(next)
  })

  it('handles a stack emptied by retirement', () => {
    let s = layout([0], { threshold: 1 })
    s = rateMany(s, 2, 'easy', seeded(1))
    expect(s.stack).toEqual([])
    expect(s.pool).toEqual([0])
    const again = rateConcentric(s, 'good', seeded(1))
    expect(again.stack).toEqual([])
    expect(again.lastChange).toEqual(NO_CHANGE)
    expect(again.mastered).toBe(false)
    // Resizing back up brings the retired card in again.
    const refilled = resizeConcentric(s, 1, seeded(1))
    expect(refilled.stack).toEqual([0])
    expect(refilled.retired.size).toBe(0)
  })
})

describe('mastery', () => {
  it('is reported by the rating that makes it true, and only that one', () => {
    let s = layout([0, 1, 2], { threshold: 0.7 })
    s = rateConcentric(s, 'good', seeded(1))
    s = rateConcentric(s, 'good', seeded(1))
    expect(s.mastered).toBe(false)
    s = rateConcentric(s, 'good', seeded(1))
    expect(s.mastered).toBe(true)
    expect(isMasteredState(s)).toBe(true)
    expect(s.stack).toHaveLength(3)
    // Keep practising: still mastered, but not reported again.
    s = rateConcentric(s, 'good', seeded(1))
    expect(s.mastered).toBe(false)
    expect(isMasteredState(s)).toBe(true)
  })

  it('is not reached while an unretired card waits in the pool', () => {
    // Threshold 1: the pool card can never be folded in while 0 is unrated.
    const s = layout([0, 1], { eligible: range(3), threshold: 1, session: [[1, 'good']] })
    expect(isMasteredState(rateConcentric(s, 'again', seeded(1)))).toBe(false)
  })

  it('counts retired cards as done', () => {
    // 1 is known; 0 retires with nothing in the pool to swap in.
    const s = layout([0, 1], { eligible: range(2), streaks: [[0, 1]], session: [[1, 'good']], threshold: 1 })
    const next = rateConcentric(s, 'easy', seeded(1))
    expect(next.stack).toEqual([1])
    expect(next.mastered).toBe(true)
  })

  it('is not reached when a card in play is missed', () => {
    let s = layout([0, 1], { threshold: 0.5 })
    s = rateConcentric(s, 'good', seeded(1))
    s = rateConcentric(s, 'again', seeded(1))
    expect(s.mastered).toBe(false)
    expect(isMasteredState(s)).toBe(false)
  })
})

describe('resizeConcentric', () => {
  it('clamps to 1..eligible', () => {
    const s = start(Array(8).fill('good'))
    expect(resizeConcentric(s, 0, seeded(1)).stack).toHaveLength(1)
    expect(resizeConcentric(s, 99, seeded(1)).stack).toHaveLength(8)
    expect(resizeConcentric(s, 99, seeded(1)).target).toBe(8)
  })

  it('grows from the pool, keeps the card on screen, and clears the change', () => {
    const rng = seeded(2)
    let s = start(Array(12).fill('good'))
    s = rateConcentric(s, 'good', rng)
    const front = s.stack[0]
    s = resizeConcentric(s, 9, rng)
    expect(s.stack).toHaveLength(9)
    expect(s.stack[0]).toBe(front)
    expect(s.lastChange).toEqual(NO_CHANGE)
    invariant(s)
  })

  it('raising X picks challenging cards first', () => {
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
      const grown = resizeConcentric(s, 9, seeded(seed))
      // Three more: the two struggled cards, then one of the new ones.
      expect(grown.stack).toContain(12)
      expect(grown.stack).toContain(13)
      expect(grown.stack.filter((i) => latest[i] === null)).toHaveLength(1)
    }
  })

  it('brings retired cards back only when nothing else is left', () => {
    const s = layout([0], { eligible: range(3), retired: [1] })
    const one = resizeConcentric(s, 2, seeded(1))
    expect(one.stack).toContain(2)
    expect(one.retired).toEqual(new Set([1]))
    const both = resizeConcentric(s, 3, seeded(1))
    expect(both.retired.size).toBe(0)
    invariant(both)
  })

  it('shrinks by removing the best-known cards first, never the front', () => {
    const rng = seeded(3)
    let s = start(Array(10).fill('good'), { size: 5, threshold: 1 })
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
  })

  it('prunes retired flags and streaks of cards no longer eligible', () => {
    const s = layout([0, 1], { eligible: range(6), retired: [4, 5], streaks: [[1, 1], [3, 1]] })
    const next = setConcentricEligible(s, [0, 1, 2, 4], seeded(1))
    expect(next.retired).toEqual(new Set([4]))
    expect(next.easyStreaks).toEqual(new Map([[1, 1]]))
    invariant(next)
  })
})

describe('invariants under random play', () => {
  it('stack and pool partition the eligible cards; a rating keeps the next card next', () => {
    for (let seed = 1; seed <= 30; seed++) {
      const rng = seeded(seed)
      const n = 1 + Math.floor(rng() * 25)
      const latest = range(n).map(() => (rng() < 0.4 ? null : RATINGS[Math.floor(rng() * 4)]))
      let s = startConcentric({
        latest,
        eligible: range(n),
        size: 1 + Math.floor(rng() * 8),
        threshold: 0.3 + rng() * 0.7,
        rng,
      })
      invariant(s)
      let wasMastered = isMasteredState(s)
      for (let step = 0; step < 300; step++) {
        const roll = rng()
        if (roll < 0.03) {
          s = resizeConcentric(s, 1 + Math.floor(rng() * (n + 2)), rng)
        } else if (roll < 0.05) {
          s = setConcentricEligible(
            s,
            range(n).filter(() => rng() < 0.7),
            rng
          )
        } else {
          // Easy-heavy, so retirement and swaps happen often.
          const rating: Rating = roll < 0.55 ? 'easy' : RATINGS[Math.floor(rng() * 4)]
          const before = s
          s = rateConcentric(s, rating, rng)
          if (before.stack.length >= 2) expect(s.stack[0]).toBe(before.stack[1])
          expect(s.mastered).toBe(isMasteredState(s) && !wasMastered)
          expect(s.stack.length).toBe(
            before.stack.length + s.lastChange.added - s.lastChange.retired
          )
        }
        wasMastered = isMasteredState(s)
        invariant(s)
      }
    }
  })
})
