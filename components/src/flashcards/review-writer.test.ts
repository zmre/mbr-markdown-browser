import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { makeReviewRecorder } from './review-writer.js'
// The real main-bundle instances, injected exactly as `<mbr-flashcards>` does:
// these tests are what pin that the writer drives *their* cache and window.
import {
  TOKEN_MESSAGE,
  forgetSourceLines,
  noteSelfWrite,
  readSourceLines,
  resetTaskToggleState,
  wasSelfWrite,
} from '../task-toggle.js'
import {
  clearEditToken,
  editAuthHeaders,
  isEditTokenRequired,
  noteEditTokenRequired,
} from '../edit-token.js'

const recordReview = makeReviewRecorder({
  path: 'deck.md',
  read: readSourceLines,
  forget: forgetSourceLines,
  selfWrite: noteSelfWrite,
  headers: editAuthHeaders,
  tokenRequired: noteEditTokenRequired,
  tokenMessage: TOKEN_MESSAGE,
})

const SOURCE = 'Q?\n: A.\n\nNext?\n: B.\n'

let fetchMock: ReturnType<typeof vi.fn>

/** Raw reads return SOURCE; the review endpoint answers with `reply`. */
function route(reply: { ok: boolean; status: number; json?: () => Promise<unknown> }) {
  fetchMock.mockImplementation((url: string) => {
    if (String(url).startsWith('/.mbr/raw/')) {
      return Promise.resolve({ ok: true, status: 200, text: () => Promise.resolve(SOURCE) })
    }
    return Promise.resolve(reply)
  })
}

function reviewBody(): Record<string, unknown> {
  const call = fetchMock.mock.calls.find((c) => c[0] === '/.mbr/flashcard-review')!
  return JSON.parse((call[1] as RequestInit).body as string)
}

beforeEach(() => {
  resetTaskToggleState()
  clearEditToken()
  window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: true }
  fetchMock = vi.fn()
  globalThis.fetch = fetchMock as unknown as typeof fetch
})

afterEach(() => {
  window.__MBR_CONFIG__ = undefined
})

describe('makeReviewRecorder', () => {
  it('sends the term line as `expected`, and splices the insert into the cache', async () => {
    route({
      ok: true,
      status: 200,
      json: () =>
        Promise.resolve({
          entry: '2026-10-06 13:45 - Good',
          line: 4,
          inserted_at: 3,
          inserted: [': ___Review History___', '  * 2026-10-06 13:45 - Good'],
        }),
    })
    const outcome = await recordReview({ line: 1, rating: 'good' })
    expect(outcome).toEqual({
      ok: true,
      entry: '2026-10-06 13:45 - Good',
      line: 4,
      insertedAt: 3,
      insertedCount: 2,
    })
    expect(reviewBody()).toEqual({ path: 'deck.md', line: 1, expected: 'Q?', rating: 'good' })
    const headers = (fetchMock.mock.calls.find((c) => c[0] === '/.mbr/flashcard-review')![1] as RequestInit)
      .headers as Record<string, string>
    expect(headers['X-MBR-Edit']).toBe('1')

    // The page's own write must not reload it.
    expect(wasSelfWrite('deck.md')).toBe(true)

    // The cache now matches the file on disk, so the next card's `expected`
    // (the "Next?" term, two lines further down) is read without a refetch.
    const read = await readSourceLines('deck.md')
    expect(read.ok && read.lines.slice(0, 6)).toEqual([
      'Q?',
      ': A.',
      ': ___Review History___',
      '  * 2026-10-06 13:45 - Good',
      '',
      'Next?',
    ])
    expect(fetchMock.mock.calls.filter((c) => String(c[0]).startsWith('/.mbr/raw/'))).toHaveLength(1)
  })

  it('reports a 409 as a conflict and forgets the cached file', async () => {
    route({ ok: false, status: 409 })
    const outcome = await recordReview({ line: 1, rating: 'again' })
    expect(outcome).toMatchObject({ ok: false, kind: 'conflict' })
    expect(outcome.ok === false && outcome.message).toContain('changed on disk')
    await readSourceLines('deck.md')
    expect(fetchMock.mock.calls.filter((c) => String(c[0]).startsWith('/.mbr/raw/'))).toHaveLength(2)
  })

  it('reports a missing token, and remembers one is needed', async () => {
    route({ ok: false, status: 401 })
    const outcome = await recordReview({ line: 1, rating: 'hard' })
    expect(outcome).toMatchObject({ ok: false, kind: 'auth' })
    expect(isEditTokenRequired()).toBe(true)
  })

  it('treats a line past the end of the file as a conflict, without writing', async () => {
    route({ ok: true, status: 200 })
    const outcome = await recordReview({ line: 99, rating: 'easy' })
    expect(outcome).toMatchObject({ ok: false, kind: 'conflict' })
    expect(fetchMock.mock.calls.some((c) => c[0] === '/.mbr/flashcard-review')).toBe(false)
  })
})
