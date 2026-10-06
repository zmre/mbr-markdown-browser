/**
 * The contract between the `<mbr-flashcards>` trigger (main bundle) and the
 * lazy deck chunk. Types only, so importing it costs neither bundle a byte.
 */
import type { Rating } from './history.js'

export type { Rating } from './history.js'

/** One review to record: the card's term line and the self-rating. */
export interface ReviewTarget {
  /** 1-based source line of the card's `<dt>` (its `data-mbr-line`). */
  readonly line: number
  readonly rating: Rating
}

/**
 * The result of `POST /.mbr/flashcard-review`, resolved rather than rejected
 * so the caller cannot forget its failure path.
 *
 * `kind: 'conflict'` means the file changed under the page: every further
 * write would be refused too, so the deck stops writing for the session.
 */
export type ReviewOutcome =
  | {
      readonly ok: true
      /** The entry as written, e.g. `2026-10-06 13:45 - Good`. */
      readonly entry: string
      /** 1-based line of the new bullet. */
      readonly line: number
      /** 1-based line of the first inserted line. */
      readonly insertedAt: number
      /** How many lines were inserted (1, or 2 with a new history label). */
      readonly insertedCount: number
    }
  | { readonly ok: false; readonly kind: 'conflict' | 'auth' | 'other'; readonly message: string }

/** The writer the trigger injects into the deck as `.recordReview`. */
export type ReviewRecorder = (target: ReviewTarget) => Promise<ReviewOutcome>
