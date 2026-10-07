import { describe, expect, it } from 'vitest'
import { State } from 'ts-fsrs'
import { parseHistory } from './history.js'
import {
  LEARN_AHEAD_MS,
  dueThisSession,
  formatInterval,
  makeScheduler,
  planSession,
  previewDue,
  rate,
  relativeTime,
  replay,
} from './session.js'
import { seededRng } from './test-fixtures.js'

const scheduler = makeScheduler()
const MINUTE = 60_000
const DAY = 24 * 60 * MINUTE

describe('replay', () => {
  it('is null for a card with no history', () => {
    expect(replay(scheduler, [])).toBeNull()
  })

  it('is deterministic: the same log always gives the same schedule', () => {
    const history = parseHistory([
      '2026-10-01 08:00 - Good',
      '2026-10-01 08:10 - Good',
      '2026-10-03 09:00 - Hard',
      '2026-10-08 19:30 - Easy',
    ])
    const a = replay(scheduler, history)!
    const b = replay(makeScheduler(), history)!
    expect(a.due).toEqual(b.due)
    expect(a.stability).toBe(b.stability)
    expect(a.difficulty).toBe(b.difficulty)
    expect(a.reps).toBe(4)
    expect(a.state).toBe(State.Review)
  })

  it('matches stepping through `rate` one review at a time', () => {
    const history = parseHistory(['2026-10-01 08:00 - Again', '2026-10-01 08:02 - Good'])
    const stepped = rate(scheduler, rate(scheduler, null, 'again', history[0].at), 'good', history[1].at)
    expect(replay(scheduler, history)!.due).toEqual(stepped.due)
  })

  it('treats a Fail line exactly like Again', () => {
    const fail = replay(scheduler, parseHistory(['2026-10-01 08:00 - Fail']))!
    const again = replay(scheduler, parseHistory(['2026-10-01 08:00 - Again']))!
    expect(fail.due).toEqual(again.due)
  })
})

describe('previewDue', () => {
  it('orders the four ratings from soonest to latest', () => {
    const now = new Date(2026, 9, 6, 12)
    for (const card of [null, replay(scheduler, parseHistory(['2026-09-20 08:00 - Good']))]) {
      const due = previewDue(scheduler, card, now)
      expect(due.again.getTime()).toBeLessThanOrEqual(due.hard.getTime())
      expect(due.hard.getTime()).toBeLessThanOrEqual(due.good.getTime())
      expect(due.good.getTime()).toBeLessThan(due.easy.getTime())
    }
  })

  it('uses the default 1m / 10m learning steps for a new card', () => {
    const now = new Date(2026, 9, 6, 12)
    const due = previewDue(scheduler, null, now)
    expect(due.again.getTime() - now.getTime()).toBe(MINUTE)
    expect(due.good.getTime() - now.getTime()).toBe(10 * MINUTE)
  })
})

describe('planSession', () => {
  const now = new Date(2026, 9, 20, 12)
  const long = replay(scheduler, parseHistory(['2026-01-01 08:00 - Easy'])) // long overdue
  const recent = replay(scheduler, parseHistory(['2026-10-18 08:00 - Good'])) // learning, due
  const future = replay(scheduler, parseHistory(['2026-10-19 08:00 - Easy'])) // not yet due

  it('puts due cards first, least retrievable first, then the new cards', () => {
    const plan = planSession(scheduler, [null, recent, long, null, future], now, seededRng(1))
    expect(plan.queue.slice(0, 2)).toEqual([2, 1])
    expect([...plan.queue.slice(2)].sort()).toEqual([0, 3])
    expect([...plan.fresh].sort()).toEqual([0, 3])
    expect(plan.nextDue).toEqual(future!.due)
  })

  it('shuffles new cards rather than keeping document order', () => {
    const states = Array.from({ length: 12 }, () => null)
    const documentOrder = states.map((_, i) => i)
    const plans = [1, 2, 3, 4, 5].map((seed) => planSession(scheduler, states, now, seededRng(seed)).queue)
    for (const queue of plans) expect([...queue].sort((a, b) => a - b)).toEqual(documentOrder)
    // Not document order, and not a fixed permutation either: the first card varies.
    expect(plans.some((queue) => queue.join() !== documentOrder.join())).toBe(true)
    expect(new Set(plans.map((queue) => queue[0])).size).toBeGreaterThan(1)
    // Seeded, so reproducible.
    expect(planSession(scheduler, states, now, seededRng(3)).queue).toEqual(plans[2])
  })

  it('keeps due cards ahead of new ones, and breaks equal retrievability at random', () => {
    // Two copies of one history: identical retrievability and due date.
    const twin = replay(scheduler, parseHistory(['2026-01-01 08:00 - Easy']))
    const states = [null, twin, null, twin, recent]
    const firsts = new Set<number>()
    for (let seed = 1; seed <= 20; seed++) {
      const queue = planSession(scheduler, states, now, seededRng(seed)).queue
      // Long-overdue twins are least retrievable, then `recent`, then the new cards.
      expect([...queue.slice(0, 2)].sort()).toEqual([1, 3])
      expect(queue[2]).toBe(4)
      expect([...queue.slice(3)].sort()).toEqual([0, 2])
      firsts.add(queue[0])
    }
    expect(firsts).toEqual(new Set([1, 3]))
  })

  it('has an empty queue and a next due date when nothing is due', () => {
    const plan = planSession(scheduler, [future], now, seededRng(1))
    expect(plan.queue).toEqual([])
    expect(plan.nextDue).toEqual(future!.due)
  })
})

describe('session helpers', () => {
  it('re-queues only cards due within the learn-ahead window', () => {
    const now = new Date(2026, 9, 6, 12)
    const again = rate(scheduler, null, 'again', now)
    const easy = rate(scheduler, null, 'easy', now)
    expect(dueThisSession(again, now)).toBe(true)
    expect(dueThisSession(easy, now)).toBe(false)
    expect(LEARN_AHEAD_MS).toBe(20 * MINUTE)
  })

  it('formats intervals compactly', () => {
    const t = new Date(2026, 0, 1)
    const at = (ms: number) => new Date(t.getTime() + ms)
    expect(formatInterval(t, at(20_000))).toBe('<1m')
    expect(formatInterval(t, at(10 * MINUTE))).toBe('10m')
    expect(formatInterval(t, at(5 * 60 * MINUTE))).toBe('5h')
    expect(formatInterval(t, at(3 * DAY))).toBe('3d')
    expect(formatInterval(t, at(75 * DAY))).toBe('2.5mo')
    expect(formatInterval(t, at(60 * DAY))).toBe('2mo')
    expect(formatInterval(t, at(438 * DAY))).toBe('1.2y')
  })

  it('describes relative times', () => {
    const now = new Date(2026, 0, 1, 12)
    expect(relativeTime(new Date(now.getTime() + 3 * DAY), now)).toMatch(/3 days/)
  })
})
