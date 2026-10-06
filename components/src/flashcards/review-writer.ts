/**
 * Recording a flashcard review: the one implementation of
 * `POST /.mbr/flashcard-review`.
 *
 * Lives in the main bundle because it shares `task-toggle.ts`'s state — the
 * source-line cache that supplies `expected`, and the self-write window that
 * keeps `<mbr-live-reload>` from reloading the page under the overlay (which
 * would also drop an in-memory edit token). The lazy deck receives
 * {@link recordReview} as an injected property; it must not import this.
 *
 * The page itself is brought up to date by the deck (`flashcards/dom.ts`),
 * which knows which card was reviewed; this module keeps only the *source*
 * cache in step, by splicing in the lines the server inserted.
 */
import { editAuthHeaders, noteEditTokenRequired } from './edit-token.js'
import {
  TOKEN_MESSAGE,
  currentDocumentPath,
  forgetSourceLines,
  noteSelfWrite,
  readSourceLines,
} from './task-toggle.js'
import type { ReviewOutcome, ReviewTarget } from './flashcards/types.js'

/** Endpoint for one review (`server.rs::flashcard_review_handler`). */
const REVIEW_ENDPOINT = '/.mbr/flashcard-review'

const CONFLICT_MESSAGE = 'This note changed on disk — reload to continue reviewing.'

/** Human-readable reason for a failed read or write, by status. */
function failure(status: number): Extract<ReviewOutcome, { ok: false }> {
  switch (status) {
    case 409:
      return { ok: false, kind: 'conflict', message: CONFLICT_MESSAGE }
    case 401:
      return { ok: false, kind: 'auth', message: TOKEN_MESSAGE }
    case 403:
      return { ok: false, kind: 'auth', message: 'Editing is not enabled on this server.' }
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
 * Record one review of a card on the page being viewed.
 *
 * Resolves rather than rejects, like `toggleTask`.
 */
export async function recordReview(target: ReviewTarget): Promise<ReviewOutcome> {
  const path = currentDocumentPath()
  if (!path) return failure(404)

  let read
  try {
    read = await readSourceLines(path)
  } catch {
    read = { ok: false as const, status: 0 }
  }
  if (!read.ok) {
    forgetSourceLines(path)
    if (read.status === 401) noteEditTokenRequired()
    return failure(read.status)
  }
  const expected = read.lines[target.line - 1]
  if (expected === undefined) {
    // No such line in the file we hold: it is shorter than the page thinks.
    forgetSourceLines(path)
    return failure(409)
  }

  // Before the request, not after: see `noteSelfWrite`.
  noteSelfWrite(path)

  let response: Response
  try {
    response = await fetch(REVIEW_ENDPOINT, {
      method: 'POST',
      headers: editAuthHeaders({ 'Content-Type': 'application/json' }),
      credentials: 'same-origin',
      body: JSON.stringify({ path, line: target.line, expected, rating: target.rating }),
    })
  } catch (err) {
    console.warn('Flashcard review request failed:', err)
    return failure(0)
  }

  if (!response.ok) {
    if (response.status === 409) forgetSourceLines(path)
    if (response.status === 401) noteEditTokenRequired()
    return failure(response.status)
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
    forgetSourceLines(path)
    return failure(409)
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
