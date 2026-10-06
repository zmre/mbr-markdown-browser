/**
 * Spaced-repetition scheduling for the deck: FSRS via `ts-fsrs`, plus the
 * session queue built on it. Pure apart from the clock, which every function
 * takes as a parameter, so it is testable without mocking time.
 *
 * Lazy chunk only — `ts-fsrs` must never reach the main bundle.
 *
 * # Replaying history instead of storing state
 *
 * Nothing but the review log is stored (in the note, as human-readable
 * `YYYY-MM-DD HH:MM - Rating` lines). A card's FSRS state — due date,
 * stability, difficulty — is recomputed on every open by feeding its sorted
 * history through `next()` from an empty card created at the first review.
 * That is exactly what `reschedule()` does internally, written as the loop
 * because the loop is the easier of the two to read and to test.
 *
 * # Determinism
 *
 * Fuzz is **off** (`enable_fuzz: false`, also ts-fsrs's default). With fuzz on,
 * intervals are jittered by a seed derived from the card's review count and
 * timing — deterministic in principle, but the jitter would make the same
 * history replay to a due date nobody can predict by reading it, and spread
 * across a whole deck it is exactly the "why is this due today?" puzzle a
 * plain-text log should not create. Everything else is ts-fsrs's default:
 * `request_retention` 0.9, short-term scheduling on, learning steps 1m/10m,
 * relearning step 10m.
 */
import {
  Rating as FsrsRating,
  State,
  createEmptyCard,
  fsrs,
  type Card,
  type FSRS,
  type Grade,
} from 'ts-fsrs'
import type { HistoryEntry, Rating } from './history.js'

export type { Card } from 'ts-fsrs'

/**
 * A card whose next review falls within this window of now is shown again in
 * the same session, after the cards still waiting — Anki's "learn ahead"
 * limit, and the same 20 minutes. It is what makes a lapsed card's 10-minute
 * relearning step (or a new card's 1m/10m learning steps) happen *now* rather
 * than next time the deck is opened.
 */
export const LEARN_AHEAD_MS = 20 * 60 * 1000

const GRADES: Readonly<Record<Rating, Grade>> = {
  again: FsrsRating.Again,
  hard: FsrsRating.Hard,
  good: FsrsRating.Good,
  easy: FsrsRating.Easy,
}

/** The scheduler every deck uses. */
export function makeScheduler(): FSRS {
  return fsrs({ enable_fuzz: false, enable_short_term: true })
}

/** A card's FSRS state after its whole history, or `null` for a new card. */
export function replay(scheduler: FSRS, history: readonly HistoryEntry[]): Card | null {
  if (history.length === 0) return null
  let card = createEmptyCard(history[0].at)
  for (const entry of history) {
    card = scheduler.next(card, entry.at, GRADES[entry.rating]).card
  }
  return card
}

/** When each rating would make `card` due again, if given `now`. */
export function previewDue(
  scheduler: FSRS,
  card: Card | null,
  now: Date
): Readonly<Record<Rating, Date>> {
  const preview = scheduler.repeat(card ?? createEmptyCard(now), now)
  return {
    again: preview[FsrsRating.Again].card.due,
    hard: preview[FsrsRating.Hard].card.due,
    good: preview[FsrsRating.Good].card.due,
    easy: preview[FsrsRating.Easy].card.due,
  }
}

/** The state of `card` after `rating` at `now`. */
export function rate(scheduler: FSRS, card: Card | null, rating: Rating, now: Date): Card {
  return scheduler.next(card ?? createEmptyCard(now), now, GRADES[rating]).card
}

/** Compact interval for a rating button: `1m`, `10m`, `3h`, `4d`, `2.5mo`, `1.2y`. */
export function formatInterval(from: Date, to: Date): string {
  const minutes = Math.max(0, (to.getTime() - from.getTime()) / 60_000)
  if (minutes < 1) return '<1m'
  if (minutes < 60) return `${Math.round(minutes)}m`
  const hours = minutes / 60
  if (hours < 24) return `${Math.round(hours)}h`
  const days = hours / 24
  if (days < 30) return `${Math.round(days)}d`
  const oneDecimal = (n: number) => n.toFixed(1).replace(/\.0$/, '')
  if (days < 365) return `${oneDecimal(days / 30)}mo`
  return `${oneDecimal(days / 365)}y`
}

/** Human name for an FSRS state, for the history panel. */
export function stateName(card: Card): string {
  switch (card.state) {
    case State.New:
      return 'New'
    case State.Learning:
      return 'Learning'
    case State.Review:
      return 'Review'
    case State.Relearning:
      return 'Relearning'
    default:
      return 'Unknown'
  }
}

/** The opening order of a spaced-repetition session. */
export interface SessionPlan {
  /** Card indices to show, due reviews first, then new cards. */
  readonly queue: readonly number[]
  /** The subset of `queue` that has never been reviewed. */
  readonly fresh: ReadonlySet<number>
  /** The soonest future due date among cards not in the queue, if any. */
  readonly nextDue: Date | null
}

/**
 * Order a session.
 *
 * Due cards (due ≤ now) come first, least retrievable first — the card most
 * likely to have been forgotten is the one most worth seeing — with ties
 * broken by due date and then document order. New cards follow in document
 * order (Anki's "show new cards after reviews"). No daily limits: the deck is
 * one note, not a collection of thousands.
 */
export function planSession(
  scheduler: FSRS,
  states: readonly (Card | null)[],
  now: Date
): SessionPlan {
  const due: { index: number; retrievability: number; due: number }[] = []
  const fresh: number[] = []
  let nextDue: Date | null = null
  states.forEach((card, index) => {
    if (!card) {
      fresh.push(index)
    } else if (card.due.getTime() <= now.getTime()) {
      due.push({
        index,
        retrievability: scheduler.get_retrievability(card, now, false),
        due: card.due.getTime(),
      })
    } else if (!nextDue || card.due.getTime() < nextDue.getTime()) {
      nextDue = card.due
    }
  })
  due.sort((a, b) => a.retrievability - b.retrievability || a.due - b.due || a.index - b.index)
  return {
    queue: [...due.map((d) => d.index), ...fresh],
    fresh: new Set(fresh),
    nextDue,
  }
}

/** True when a just-rated card should come round again this session. */
export function dueThisSession(card: Card, now: Date): boolean {
  return card.due.getTime() - now.getTime() <= LEARN_AHEAD_MS
}

/** `in 3 days`, `in 5 hours`, `tomorrow`, in the reader's locale. */
export function relativeTime(to: Date, now: Date): string {
  const seconds = (to.getTime() - now.getTime()) / 1000
  const format = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' })
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ['year', 365 * 86400],
    ['month', 30 * 86400],
    ['day', 86400],
    ['hour', 3600],
    ['minute', 60],
  ]
  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) return format.format(Math.round(seconds / size), unit)
  }
  return format.format(Math.round(seconds), 'second')
}
