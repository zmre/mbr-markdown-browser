/**
 * Concentric mode: a small working stack of cards that grows outward as it is
 * learned. Closely related to Incremental Rehearsal ("folding-in": Tucker
 * 1989; Burns 2004) — a few cards at a time, a failed card comes back soon, a
 * known one goes to the back, and new cards are folded in only once the stack
 * is being answered well. `docs/markdown/flashcards.md` explains where each
 * constant below comes from.
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
 *   screen; rating it removes it and reinserts it further back.
 * - **pool** — eligible cards not yet in play.
 * - **target** — X, the size the user (or growth) asked for. The stack holds
 *   `min(target, eligible)`: a section filter can shrink what is eligible
 *   without forgetting the size the user chose.
 * - **pass** — X consecutive ratings. Only each card's *first* rating in a pass
 *   counts toward growth, so hammering one failed card back to Good cannot
 *   make a stack look learned.
 */
import type { Rating } from './history.js'
import { shuffle, type Rng } from './random.js'

export type { Rng } from './random.js'

/** Cards in play when a session starts (Cowan 2001: about four chunks, plus slack). */
export const CONCENTRIC_DEFAULT_SIZE = 6

/** Cards folded in when a pass meets the threshold. */
export const CONCENTRIC_GROWTH = 2

/**
 * Share of the stack seeded with challenging cards (struggled, then never
 * reviewed) — at the start and when the user raises X.
 */
export const CHALLENGING_SEED_SHARE = 0.25

/**
 * Where a rated card goes back in, as a fraction range of the rest of the
 * stack's length L: Again soon (but never next), Hard about halfway, Good near
 * the back, Easy at the very back.
 */
export const PLACEMENT: Readonly<Record<Rating, readonly [number, number]>> = {
  again: [0.2, 0.35],
  hard: [0.4, 0.6],
  good: [0.75, 1],
  easy: [1, 1],
}

/** One concentric session. */
export interface ConcentricState {
  /** Cards in play, front (on screen) first. */
  readonly stack: readonly number[]
  /** Eligible cards not in play. */
  readonly pool: readonly number[]
  /** Every eligible card index, in document order. */
  readonly eligible: readonly number[]
  /** X: the size asked for; the stack holds `min(target, eligible.length)`. */
  readonly target: number
  /** Share of a pass's first ratings that must be Good/Easy to grow. */
  readonly threshold: number
  /**
   * Per card (by index), its latest rating — from the note's history, then
   * this session's — or `null` when it has never been rated.
   */
  readonly latest: readonly (Rating | null)[]
  /**
   * Whether the note had any review history when the session began. Without
   * one every card is new, "challenging" means nothing, and picks are plain
   * random.
   */
  readonly seeded: boolean
  /** The last rating each card got in this session (for shrinking). */
  readonly sessionRatings: ReadonlyMap<number, Rating>
  /** Ratings needed to finish the current pass (the stack size when it began). */
  readonly passLength: number
  /** Ratings given so far in the current pass. */
  readonly passCount: number
  /** Each card's first rating in the current pass. */
  readonly passFirsts: ReadonlyMap<number, Rating>
  /**
   * True only in the state returned by the rating that completed a passing
   * pass with every eligible card already in play.
   */
  readonly mastered: boolean
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

/** True for a card that counts toward the challenging quota. */
function isChallenging(state: Pick<ConcentricState, 'latest' | 'seeded'>, card: number): boolean {
  if (!state.seeded) return false
  const rating = state.latest[card] ?? null
  return rating === null || isStruggled(rating)
}

/**
 * Pick `count` cards from `pool`: up to `quota` challenging ones first —
 * struggled (latest Again/Hard) before never-reviewed, random within each —
 * then random from the rest. Returns the picks in pick order.
 */
function pick(
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

/** Pass tracking reset, for a stack of `size`. */
function freshPass(size: number): Pick<ConcentricState, 'passLength' | 'passCount' | 'passFirsts'> {
  return { passLength: size, passCount: 0, passFirsts: new Map() }
}

/** The size shown in the deck's number box: X, capped by what is eligible. */
export function displaySize(state: ConcentricState): number {
  return Math.min(state.target, state.eligible.length)
}

/**
 * Add cards from the pool until the stack holds `size`, keeping
 * `challengingQuota(size)` challenging cards in play where the pool allows.
 */
function fillTo(state: ConcentricState, size: number, keepFront: boolean, rng: Rng): ConcentricState {
  const need = size - state.stack.length
  if (need <= 0 || state.pool.length === 0) return state
  const have = state.stack.filter((i) => isChallenging(state, i)).length
  const quota = Math.max(0, challengingQuota(size) - have)
  const added = pick(state.pool, need, quota, state.latest, state.seeded, rng)
  const addedSet = new Set(added)
  return {
    ...state,
    stack: insertRandomly(state.stack, shuffle(added, rng), keepFront, rng),
    pool: state.pool.filter((i) => !addedSet.has(i)),
  }
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
  const chosen = pick(eligible, size, challengingQuota(size), options.latest, seeded, options.rng)
  const chosenSet = new Set(chosen)
  const stack = shuffle(chosen, options.rng)
  return {
    stack,
    pool: eligible.filter((i) => !chosenSet.has(i)),
    eligible,
    target,
    threshold: options.threshold,
    latest: options.latest.slice(),
    seeded,
    sessionRatings: new Map(),
    ...freshPass(stack.length),
    mastered: false,
  }
}

/**
 * Where a card rated `rating` goes back in, given L cards behind it: a jittered
 * fraction of L from {@link PLACEMENT}, clamped to [0, L]. Again is never put
 * at 0 — straight back on screen — when there are two or more to put it among.
 */
export function placementIndex(rating: Rating, remaining: number, rng: Rng): number {
  if (remaining <= 0) return 0
  const [lo, hi] = PLACEMENT[rating]
  const fraction = lo + rng() * (hi - lo)
  let index = Math.round(remaining * fraction)
  if (rating === 'again' && remaining >= 2) index = Math.max(1, index)
  return Math.max(0, Math.min(remaining, index))
}

/**
 * Rate the front card: move it back by {@link placementIndex}, record the
 * rating, and at the end of a pass grow (or report mastery) when enough of the
 * pass's first ratings were Good or Easy.
 */
export function rateConcentric(state: ConcentricState, rating: Rating, rng: Rng): ConcentricState {
  const [card, ...rest] = state.stack
  if (card === undefined) return { ...state, mastered: false }

  const stack = rest.slice()
  stack.splice(placementIndex(rating, rest.length, rng), 0, card)
  const latest = state.latest.slice()
  latest[card] = rating
  const sessionRatings = new Map(state.sessionRatings).set(card, rating)
  const passFirsts = new Map(state.passFirsts)
  if (!passFirsts.has(card)) passFirsts.set(card, rating)
  const passCount = state.passCount + 1

  const next: ConcentricState = { ...state, stack, latest, sessionRatings, passFirsts, passCount, mastered: false }
  if (passCount < state.passLength) return next

  // End of a pass: judge it on each card's first answer.
  const firsts = Array.from(passFirsts.values())
  const known = firsts.filter((r) => r === 'good' || r === 'easy').length
  const meets = firsts.length > 0 && known / firsts.length >= state.threshold
  if (!meets) return { ...next, ...freshPass(stack.length) }
  if (next.pool.length === 0) return { ...next, ...freshPass(stack.length), mastered: true }

  const added = pick(next.pool, Math.min(CONCENTRIC_GROWTH, next.pool.length), 0, latest, false, rng)
  const addedSet = new Set(added)
  const grownStack = insertRandomly(stack, added, false, rng)
  return {
    ...next,
    stack: grownStack,
    pool: next.pool.filter((i) => !addedSet.has(i)),
    target: Math.max(state.target, grownStack.length),
    ...freshPass(grownStack.length),
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
 * The user set X to `n` (clamped to 1..eligible). Growing adds pool cards at
 * random positions behind the front, honouring the challenging quota for the
 * new X; shrinking returns the strongest cards by this session's last rating
 * (Easy, Good, unrated, Hard, Again; ties random) to the pool, never the card
 * on screen. Either way a new pass starts.
 */
export function resizeConcentric(state: ConcentricState, n: number, rng: Rng): ConcentricState {
  const max = state.eligible.length
  if (max === 0 || !Number.isFinite(n)) return state
  const size = Math.max(1, Math.min(max, Math.floor(n)))
  let next: ConcentricState = { ...state, target: size, mastered: false }

  if (size > state.stack.length) {
    next = fillTo(next, size, true, rng)
  } else if (size < state.stack.length) {
    const [front, ...rest] = state.stack
    const strength = (i: number) => STRENGTH[state.sessionRatings.get(i) ?? 'unrated']
    // Shuffle first so the stable sort breaks ties randomly.
    const leaving = new Set(
      shuffle(rest, rng)
        .sort((a, b) => strength(b) - strength(a))
        .slice(0, state.stack.length - size)
    )
    next = {
      ...next,
      stack: [front, ...rest.filter((i) => !leaving.has(i))],
      pool: [...state.pool, ...state.eligible.filter((i) => leaving.has(i))],
    }
  }
  return { ...next, ...freshPass(next.stack.length) }
}

/**
 * The section filter changed what is eligible: drop cards no longer eligible,
 * rebuild the pool, and refill toward X (kept as the target, so widening the
 * filter again brings the stack back to the size the user chose).
 */
export function setConcentricEligible(state: ConcentricState, eligible: readonly number[], rng: Rng): ConcentricState {
  const unique = Array.from(new Set(eligible))
  const allowed = new Set(unique)
  const stack = state.stack.filter((i) => allowed.has(i))
  const inPlay = new Set(stack)
  const keepFront = stack.length > 0 && stack[0] === state.stack[0]
  const narrowed: ConcentricState = {
    ...state,
    eligible: unique,
    stack,
    pool: unique.filter((i) => !inPlay.has(i)),
    mastered: false,
  }
  const filled = fillTo(narrowed, Math.min(state.target, unique.length), keepFront, rng)
  return { ...filled, ...freshPass(filled.stack.length) }
}
