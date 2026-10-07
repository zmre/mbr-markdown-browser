/**
 * Recording a flashcard review: the one implementation of
 * `POST /.mbr/flashcard-review` (deck chunk).
 *
 * The writer has to share `task-toggle.ts`'s state — the source-line cache that
 * supplies `expected`, and the self-write window that keeps
 * `<mbr-live-reload>` from reloading the page under the overlay (which would
 * also drop an in-memory edit token). It lives in the lazy chunk anyway, to
 * keep the main bundle small, so that state arrives as {@link ReviewServices}
 * injected by the `<mbr-flashcards>` trigger; nothing here imports a stateful
 * module.
 *
 * The page itself is brought up to date by the deck (`dom.ts`), which knows
 * which card was reviewed; this module keeps only the *source* cache in step,
 * by splicing in the lines the server inserted.
 *
 * The review is stamped here, with the reviewer's local wall-clock time (`at`),
 * because that is how `history.ts` replays it; the server only validates it.
 */
import { formatEntryTime } from './history.js'
import type { ReviewOutcome, ReviewRecorder, ReviewServices, SourceLines } from './types.js'

/** Endpoint for one review (`server.rs::flashcard_review_handler`). */
const REVIEW_ENDPOINT = '/.mbr/flashcard-review'

const CONFLICT_MESSAGE = 'This note changed on disk — reload to continue reviewing.'

/** Human-readable reason for a failed read or write, by status. */
function failure(status: number, tokenMessage: string): Extract<ReviewOutcome, { ok: false }> {
  switch (status) {
    case 409:
      return { ok: false, kind: 'conflict', message: CONFLICT_MESSAGE }
    case 401:
      return { ok: false, kind: 'auth', message: tokenMessage }
    case 403:
      return { ok: false, kind: 'auth', message: 'Editing is not enabled on this server.' }
    case 422:
      // The only 422 a well-formed request from this module can earn is `at`
      // further from the server's clock than any time zone.
      return {
        ok: false,
        kind: 'other',
        message: "The server refused this review's time — check this device's clock.",
      }
    case 0:
      return { ok: false, kind: 'other', message: 'The server could not be reached.' }
    default:
      return { ok: false, kind: 'other', message: `The review could not be saved (${status}).` }
  }
}

/** Shape of a successful response body. */
interface ReviewResponse {
  entry: string
  line: number
  inserted_at: number
  inserted: string[]
}

function isReviewResponse(body: unknown): body is ReviewResponse {
  const b = body as Partial<ReviewResponse> | null
  return (
    typeof b?.entry === 'string' &&
    typeof b.line === 'number' &&
    typeof b.inserted_at === 'number' &&
    Array.isArray(b.inserted) &&
    b.inserted.every((line) => typeof line === 'string')
  )
}

/**
 * A writer for reviews of cards on the page at `services.path`.
 *
 * The recorder resolves rather than rejects, like `toggleTask`.
 */
export function makeReviewRecorder(services: ReviewServices): ReviewRecorder {
  const { path } = services
  const fail = (status: number) => failure(status, services.tokenMessage)

  return async (target) => {
    // Taken when the card is rated, before any round trip.
    const at = formatEntryTime(new Date())
    let read: SourceLines
    try {
      read = await services.read(path)
    } catch {
      read = { ok: false, status: 0 }
    }
    if (!read.ok) {
      services.forget(path)
      if (read.status === 401) services.tokenRequired()
      return fail(read.status)
    }
    const expected = read.lines[target.line - 1]
    if (expected === undefined) {
      // No such line in the file we hold: it is shorter than the page thinks.
      services.forget(path)
      return fail(409)
    }

    // Before the request, not after: see `task-toggle.ts::noteSelfWrite`.
    services.selfWrite(path)

    let response: Response
    try {
      response = await fetch(REVIEW_ENDPOINT, {
        method: 'POST',
        headers: services.headers({ 'Content-Type': 'application/json' }),
        credentials: 'same-origin',
        body: JSON.stringify({ path, line: target.line, expected, rating: target.rating, at }),
      })
    } catch (err) {
      console.warn('Flashcard review request failed:', err)
      return fail(0)
    }

    if (!response.ok) {
      if (response.status === 409) services.forget(path)
      if (response.status === 401) services.tokenRequired()
      return fail(response.status)
    }

    let body: unknown
    try {
      body = await response.json()
    } catch {
      body = null
    }
    if (!isReviewResponse(body)) {
      // The status says it was written, but not where; nothing we hold about
      // this file's lines can be trusted any more.
      services.forget(path)
      return fail(409)
    }
    // Keep the cached source in step, so the next card's `expected` needs no
    // round trip — `read.lines` is the cache's own array.
    read.lines.splice(body.inserted_at - 1, 0, ...body.inserted)
    return {
      ok: true,
      entry: body.entry,
      line: body.line,
      insertedAt: body.inserted_at,
      insertedCount: body.inserted.length,
    }
  }
}
