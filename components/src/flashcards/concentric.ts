/**
 * Concentric mode: a small working stack of cards that grows outward as it is
 * learned. Closely related to Incremental Rehearsal ("folding-in": Tucker
 * 1989; Burns 2004) — a few cards at a time, a failed card comes back soon, a
 * known one goes to the back, new cards are folded in one at a time while the
 * stack is being answered well, and a card answered Easy twice running is put
 * away. `docs/markdown/flashcards.md` explains where each constant below comes
 * from.
 *
 * Pure: no DOM, no clock, and the randomness is injected as `rng` (the shape
 * of `Math.random`), so every decision is reproducible under a seeded rng.
 * Every function returns a new state; nothing is mutated in place.
 *
 * Lazy deck chunk only.
 *
 * # Model
 *
 * - **stack** — the cards in play, front first. The front is the card on
 *   screen; rating it removes it and reinserts it further back (or, when it
 *   is retired, not at all).
 * - **pool** — eligible cards not in play. Some of them may be **retired**:
 *   put away after two Easy ratings in a row, and picked back in last.
 * - **target** — X, the size the user (or growth) asked for. The stack holds
 *   `min(target, eligible)` until retirement thins it: a section filter can
 *   shrink what is eligible without forgetting the size the user chose.
 * - **known** — a card whose latest rating *in this session* is Good or Easy.
 *   A card not yet rated this session is not known, whatever its history says,
 *   so a stack never looks learned before it has been seen.
 *
 * Invariants (pinned by the tests): `stack` and `pool` partition `eligible`,
 * neither holds a duplicate, `retired ⊆ pool`, and a rating changes the front
 * only by moving the rated card away from it.
 */
import type { Rating } from './history.js'
import { shuffle, type Rng } from './random.js'

export type { Rng } from './random.js'

/** Cards in play when a session starts (Cowan 2001: about four chunks, plus slack). */
export const CONCENTRIC_DEFAULT_SIZE = 6

/**
 * Share of the stack seeded with challenging cards (struggled, then never
 * reviewed) when a session starts on a note with history.
 */
export const CHALLENGING_SEED_SHARE = 0.25

/** Consecutive Easy ratings of one card, in this session, that retire it. */
export const RETIRE_AFTER_EASY = 2

/**
 * Where a rated card goes back in, as a fraction range of the rest of the
 * stack's length L: Again soon, Hard about halfway, Good near the back, Easy at
 * the very back. {@link MIN_GAP} puts a floor under the result.
 */
export const PLACEMENT: Readonly<Record<Rating, readonly [number, number]>> = {
  again: [0.2, 0.35],
  hard: [0.4, 0.6],
  good: [0.75, 1],
  easy: [1, 1],
}

/**
 * The fewest cards shown between a rated card and its return: Again comes back
 * no sooner than the 4th card after it (three in between), Hard one card later
 * than that, and Good no sooner than Hard, so a small stack never brings a Good
 * card back before a Hard one could. A stack too small for the floor sends the
 * card to the very back. Easy needs none: it always goes to the back.
 */
export const MIN_GAP: Readonly<Record<Rating, number>> = {
  again: 3,
  hard: 4,
  good: 4,
  easy: 0,
}

/** What a rating did to the stack's membership, for the deck's toast. */
export interface ConcentricChange {
  /** Cards brought into play (growth, or the card swapped in for a retired one). */
  readonly added: number
  /** Cards retired to the pool. */
  readonly retired: number
}

/** A change that changed nothing. */
export const NO_CHANGE: ConcentricChange = Object.freeze({ added: 0, retired: 0 })

/** One concentric session. */
export interface ConcentricState {
  /** Cards in play, front (on screen) first. */
  readonly stack: readonly number[]
  /** Eligible cards not in play, retired ones included. */
  readonly pool: readonly number[]
  /** Every eligible card index, in document order. */
  readonly eligible: readonly number[]
  /** X: the size asked for; the stack holds `min(target, eligible.length)`. */
  readonly target: number
  /** Share of the stack that must be known before a card is folded in. */
  readonly threshold: number
  /**
   * Per card (by index), its latest rating — from the note's history, then
   * this session's — or `null` when it has never been rated.
   */
  readonly latest: readonly (Rating | null)[]
  /**
   * Whether the note had any review history when the session began. Without
   * one every card is new, "challenging" means nothing, and the starting pick
   * is plain random.
   */
  readonly seeded: boolean
  /** The last rating each card got in this session (known-ness, shrinking). */
  readonly sessionRatings: ReadonlyMap<number, Rating>
  /** Per card, its current run of consecutive Easy ratings this session. */
  readonly easyStreaks: ReadonlyMap<number, number>
  /** Pool cards put away by {@link RETIRE_AFTER_EASY}; always a subset of `pool`. */
  readonly retired: ReadonlySet<number>
  /**
   * True only in the state returned by the rating that made the stack
   * mastered: no unretired card left in the pool, and every card in play known
   * (or none in play).
   */
  readonly mastered: boolean
  /** What the last operation added and retired ({@link NO_CHANGE} but for a rating). */
  readonly lastChange: ConcentricChange
}

/** Options for {@link startConcentric}. */
export interface ConcentricOptions {
  /** Latest history rating per card index, `null` for never reviewed. */
  readonly latest: readonly (Rating | null)[]
  /** Card indices that may be shown (the section filter's result). */
  readonly eligible: readonly number[]
  /** Requested X; defaults to {@link CONCENTRIC_DEFAULT_SIZE}. */
  readonly size?: number
  readonly threshold: number
  readonly rng: Rng
}

// ========================================
// Helpers
// ========================================

/** An integer in [lo, hi]. */
function randomInt(lo: number, hi: number, rng: Rng): number {
  return lo + Math.floor(rng() * (hi - lo + 1))
}

/** `ceil(share × size)`, robust to float noise (0.25 × 20 is exactly 5). */
export function challengingQuota(size: number): number {
  return Math.max(0, Math.ceil(size * CHALLENGING_SEED_SHARE - 1e-9))
}

const isStruggled = (rating: Rating | null): boolean => rating === 'again' || rating === 'hard'

const isKnownRating = (rating: Rating | undefined): boolean => rating === 'good' || rating === 'easy'

/** Known: rated Good or Easy last, in this session. */
function isKnown(state: Pick<ConcentricState, 'sessionRatings'>, card: number): boolean {
  return isKnownRating(state.sessionRatings.get(card))
}

/** Rank by latest rating: Again/Hard, then never rated, then Good, then Easy. */
const LATEST_RANK: Readonly<Record<Rating | 'unrated', number>> = {
  again: 0,
  hard: 0,
  unrated: 1,
  good: 2,
  easy: 3,
}

/** Retired cards rank after every other. */
const RETIRED_RANK = 4

type PickContext = Pick<ConcentricState, 'latest' | 'retired'>

/** A pool card's pick rank: lower is picked first. */
function pickRank(state: PickContext, card: number): number {
  if (state.retired.has(card)) return RETIRED_RANK
  return LATEST_RANK[state.latest[card] ?? 'unrated']
}

/**
 * Up to `count` cards from `pool` in pick order — latest Again/Hard, never
 * rated, Good, Easy, retired — random within each rank.
 */
function pickByPriority(pool: readonly number[], count: number, state: PickContext, rng: Rng): number[] {
  if (count <= 0) return []
  // Shuffle first so the stable sort breaks ties randomly.
  return shuffle(pool, rng)
    .sort((a, b) => pickRank(state, a) - pickRank(state, b))
    .slice(0, count)
}

/** One card by {@link pickByPriority}, or `undefined` from an empty pool. */
function pickOne(pool: readonly number[], state: PickContext, rng: Rng): number | undefined {
  if (pool.length === 0) return undefined
  const best = Math.min(...pool.map((i) => pickRank(state, i)))
  const ties = pool.filter((i) => pickRank(state, i) === best)
  return ties[randomInt(0, ties.length - 1, rng)]
}

/**
 * The starting pick: up to `quota` challenging cards first — struggled (latest
 * Again/Hard) before never-reviewed, random within each — then random from the
 * rest. Unlike {@link pickByPriority}, the rest is *not* ordered: a start keeps
 * about three quarters of the stack likely known (the folding-in ratio), where
 * a strict priority order would fill it with every struggled and new card.
 */
function seedPick(
  pool: readonly number[],
  count: number,
  quota: number,
  latest: readonly (Rating | null)[],
  seeded: boolean,
  rng: Rng
): number[] {
  if (count <= 0) return []
  const picked: number[] = []
  if (seeded && quota > 0) {
    const struggled = shuffle(
      pool.filter((i) => isStruggled(latest[i] ?? null)),
      rng
    )
    const fresh = shuffle(
      pool.filter((i) => (latest[i] ?? null) === null),
      rng
    )
    picked.push(...[...struggled, ...fresh].slice(0, Math.min(quota, count)))
  }
  const taken = new Set(picked)
  const rest = shuffle(
    pool.filter((i) => !taken.has(i)),
    rng
  )
  picked.push(...rest.slice(0, count - picked.length))
  return picked
}

/**
 * Insert each of `cards` at a random position of `stack`; with `keepFront`
 * the front card (the one on screen) stays where it is.
 */
function insertRandomly(stack: readonly number[], cards: readonly number[], keepFront: boolean, rng: Rng): number[] {
  const out = stack.slice()
  for (const card of cards) {
    const lo = keepFront && out.length > 0 ? 1 : 0
    out.splice(randomInt(lo, out.length, rng), 0, card)
  }
  return out
}

/** `map` without `keys`, or `map` itself when none of them is in it. */
function without<V>(map: ReadonlyMap<number, V>, keys: Iterable<number>): ReadonlyMap<number, V> {
  const out = new Map(map)
  for (const key of keys) out.delete(key)
  return out.size === map.size ? map : out
}

/** `set` without `keys`, or `set` itself when none of them is in it. */
function withoutAll(set: ReadonlySet<number>, keys: Iterable<number>): ReadonlySet<number> {
  const out = new Set(set)
  for (const key of keys) out.delete(key)
  return out.size === set.size ? set : out
}

/**
 * Move `cards` from the pool into the stack at random positions (behind the
 * front with `keepFront`). A retired card that comes back loses its flag and
 * its Easy streak: it starts over.
 */
function bringIn(state: ConcentricState, cards: readonly number[], keepFront: boolean, rng: Rng): ConcentricState {
  if (cards.length === 0) return state
  const added = new Set(cards)
  return {
    ...state,
    stack: insertRandomly(state.stack, cards, keepFront, rng),
    pool: state.pool.filter((i) => !added.has(i)),
    retired: withoutAll(state.retired, cards),
    easyStreaks: without(state.easyStreaks, cards),
  }
}

/** The size shown in the deck's number box: X, capped by what is eligible. */
export function displaySize(state: ConcentricState): number {
  return Math.min(state.target, state.eligible.length)
}

/** Pool cards still to be folded in: the pool less the retired cards. */
export function cardsLeft(state: ConcentricState): number {
  return state.pool.length - state.retired.size
}

/**
 * Whether the stack counts as mastered: nothing unretired is left to fold in,
 * and every card in play is known (an empty stack trivially is).
 */
export function isMasteredState(state: ConcentricState): boolean {
  return cardsLeft(state) === 0 && state.stack.every((i) => isKnown(state, i))
}

/** Add pool cards, in pick order, until the stack holds `size`. */
function fillTo(state: ConcentricState, size: number, keepFront: boolean, rng: Rng): ConcentricState {
  const need = size - state.stack.length
  if (need <= 0 || state.pool.length === 0) return state
  return bringIn(state, pickByPriority(state.pool, need, state, rng), keepFront, rng)
}

// ========================================
// Operations
// ========================================

/**
 * Start a session: X = min(requested, eligible) cards, seeded with
 * `ceil(25% × X)` challenging cards when the note has any history, shuffled.
 */
export function startConcentric(options: ConcentricOptions): ConcentricState {
  const eligible = Array.from(new Set(options.eligible))
  const target = Math.max(1, Math.floor(options.size ?? CONCENTRIC_DEFAULT_SIZE))
  const size = Math.min(target, eligible.length)
  const seeded = options.latest.some((rating) => rating !== null)
  const chosen = seedPick(eligible, size, challengingQuota(size), options.latest, seeded, options.rng)
  const chosenSet = new Set(chosen)
  return {
    stack: shuffle(chosen, options.rng),
    pool: eligible.filter((i) => !chosenSet.has(i)),
    eligible,
    target,
    threshold: options.threshold,
    latest: options.latest.slice(),
    seeded,
    sessionRatings: new Map(),
    easyStreaks: new Map(),
    retired: new Set(),
    mastered: false,
    lastChange: NO_CHANGE,
  }
}

/**
 * Where a card rated `rating` goes back in, given L cards behind it: a jittered
 * fraction of L from {@link PLACEMENT}, raised to the rating's
 * {@link MIN_GAP}, clamped to [0, L] — so a floor larger than L means the very
 * back.
 */
export function placementIndex(rating: Rating, remaining: number, rng: Rng): number {
  if (remaining <= 0) return 0
  const [lo, hi] = PLACEMENT[rating]
  const fraction = lo + rng() * (hi - lo)
  const index = Math.max(Math.round(remaining * fraction), MIN_GAP[rating])
  return Math.max(0, Math.min(remaining, index))
}

/**
 * Fold in unretired pool cards one at a time while the known share of the
 * stack meets the threshold. Each new card is unrated, so it lowers the share:
 * 6 of 6 known grows to 9 (6/7, 6/8 still pass at 70%; 6/9 does not).
 */
function grow(state: ConcentricState, rng: Rng): { state: ConcentricState; added: number } {
  let next = state
  let added = 0
  let known = next.stack.filter((i) => isKnown(next, i)).length
  while (next.stack.length > 0 && known / next.stack.length >= next.threshold) {
    const unretired = next.pool.filter((i) => !next.retired.has(i))
    const card = pickOne(unretired, next, rng)
    if (card === undefined) break
    next = bringIn(next, [card], true, rng)
    if (isKnown(next, card)) known++
    added++
  }
  return { state: next, added }
}

/**
 * Rate the front card. Two Easy ratings of it in a row retire it to the pool
 * (and swap the best pool card in); otherwise it goes back by
 * {@link placementIndex}. Then the stack grows while it is known well enough,
 * and `lastChange` says what was added and retired.
 */
export function rateConcentric(state: ConcentricState, rating: Rating, rng: Rng): ConcentricState {
  const [card, ...rest] = state.stack
  if (card === undefined) return { ...state, mastered: false, lastChange: NO_CHANGE }

  const latest = state.latest.slice()
  latest[card] = rating
  const sessionRatings = new Map(state.sessionRatings).set(card, rating)
  const streak = rating === 'easy' ? (state.easyStreaks.get(card) ?? 0) + 1 : 0
  const easyStreaks = new Map(state.easyStreaks)
  if (streak > 0 && streak < RETIRE_AFTER_EASY) easyStreaks.set(card, streak)
  else easyStreaks.delete(card)

  const rated: ConcentricState = { ...state, latest, sessionRatings, easyStreaks, mastered: false }
  let next: ConcentricState
  let retired = 0
  let added = 0
  if (streak >= RETIRE_AFTER_EASY) {
    // Retire: off the stack and into the pool, with one card swapped in for
    // it. The swap is picked before the card joins the pool, so it is never
    // the card just put away.
    const incoming = pickOne(state.pool, rated, rng)
    const parked: ConcentricState = {
      ...rated,
      stack: rest,
      pool: [...state.pool, card],
      retired: new Set(state.retired).add(card),
    }
    next = incoming === undefined ? parked : bringIn(parked, [incoming], true, rng)
    retired = 1
    added = incoming === undefined ? 0 : 1
  } else {
    const stack = rest.slice()
    stack.splice(placementIndex(rating, rest.length, rng), 0, card)
    next = { ...rated, stack }
  }

  const grown = grow(next, rng)
  next = grown.state
  added += grown.added
  return {
    ...next,
    target: Math.max(state.target, next.stack.length),
    // Only the rating that makes it true reports it: "Keep practising" would
    // otherwise land back on the mastered screen with the very next rating.
    mastered: isMasteredState(next) && !isMasteredState(state),
    lastChange: added === 0 && retired === 0 ? NO_CHANGE : { added, retired },
  }
}

/** Shrink order: the best-known cards leave first. */
const STRENGTH: Readonly<Record<Rating | 'unrated', number>> = {
  easy: 4,
  good: 3,
  unrated: 2,
  hard: 1,
  again: 0,
}

/**
 * The user set X to `n` (clamped to 1..eligible). Growing adds pool cards in
 * pick order (struggled, never rated, Good, Easy, retired) at random positions
 * behind the front; shrinking returns the strongest cards by this session's
 * last rating (Easy, Good, unrated, Hard, Again; ties random) to the pool,
 * never the card on screen.
 */
export function resizeConcentric(state: ConcentricState, n: number, rng: Rng): ConcentricState {
  const max = state.eligible.length
  if (max === 0 || !Number.isFinite(n)) return state
  const size = Math.max(1, Math.min(max, Math.floor(n)))
  const next: ConcentricState = { ...state, target: size, mastered: false, lastChange: NO_CHANGE }

  if (size > state.stack.length) return fillTo(next, size, true, rng)
  if (size === state.stack.length) return next

  const [front, ...rest] = state.stack
  const strength = (i: number) => STRENGTH[state.sessionRatings.get(i) ?? 'unrated']
  // Shuffle first so the stable sort breaks ties randomly.
  const leaving = new Set(
    shuffle(rest, rng)
      .sort((a, b) => strength(b) - strength(a))
      .slice(0, state.stack.length - size)
  )
  return {
    ...next,
    stack: [front, ...rest.filter((i) => !leaving.has(i))],
    pool: [...state.pool, ...state.eligible.filter((i) => leaving.has(i))],
  }
}

/**
 * The section filter changed what is eligible: drop cards no longer eligible
 * (and their retired flags and streaks), rebuild the pool, and refill toward X
 * (kept as the target, so widening the filter again brings the stack back to
 * the size the user chose).
 */
export function setConcentricEligible(state: ConcentricState, eligible: readonly number[], rng: Rng): ConcentricState {
  const unique = Array.from(new Set(eligible))
  const allowed = new Set(unique)
  const stack = state.stack.filter((i) => allowed.has(i))
  const inPlay = new Set(stack)
  const keepFront = stack.length > 0 && stack[0] === state.stack[0]
  const gone = [...state.stack, ...state.pool].filter((i) => !allowed.has(i))
  const narrowed: ConcentricState = {
    ...state,
    eligible: unique,
    stack,
    pool: unique.filter((i) => !inPlay.has(i)),
    retired: withoutAll(state.retired, gone),
    easyStreaks: without(state.easyStreaks, gone),
    mastered: false,
    lastChange: NO_CHANGE,
  }
  return fillTo(narrowed, Math.min(state.target, unique.length), keepFront, rng)
}
