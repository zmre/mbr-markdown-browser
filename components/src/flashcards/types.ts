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
 * `kind: 'refused'` (a 403) is just as permanent: editing is off, or the server
 * will not take edits from the address this page was loaded at (a `Host` it
 * does not recognise, e.g. behind a reverse proxy or `tailscale serve`). The
 * deck stops writing, and the trigger builds no writer for later opens.
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
  | { readonly ok: false; readonly kind: 'conflict' | 'refused' | 'auth' | 'other'; readonly message: string }

/** The writer the deck uses (built by `review-writer.ts::makeReviewRecorder`). */
export type ReviewRecorder = (target: ReviewTarget) => Promise<ReviewOutcome>

/** A file's source lines as `task-toggle.ts` reads them (mirrors its `SourceRead`). */
export type SourceLines = { ok: true; lines: string[] } | { ok: false; status: number }

/**
 * The main-bundle state the writer needs, injected by the `<mbr-flashcards>`
 * trigger. Each member is a `task-toggle.ts` / `edit-token.ts` export: the
 * chunk must use *those* instances — one source-line cache, one self-write
 * window, one in-memory token — so it receives them rather than importing a
 * second copy.
 */
export interface ReviewServices {
  /** Repo-relative source path of the page (`currentDocumentPath()`). */
  readonly path: string
  /** `readSourceLines` */
  readonly read: (path: string) => Promise<SourceLines>
  /** `forgetSourceLines` */
  readonly forget: (path: string) => void
  /** `noteSelfWrite` */
  readonly selfWrite: (path: string) => void
  /** `editAuthHeaders` */
  readonly headers: (extra?: Record<string, string>) => Record<string, string>
  /** `noteEditTokenRequired` */
  readonly tokenRequired: () => void
  /** What to say when a token is missing (`task-toggle.ts::TOKEN_MESSAGE`). */
  readonly tokenMessage: string
}
