import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { makeReviewRecorder } from './review-writer.js'
// The real main-bundle instances, injected exactly as `<mbr-flashcards>` does:
// these tests are what pin that the writer drives *their* cache and window.
import {
  TOKEN_MESSAGE,
  forgetSourceLines,
  noteSelfWrite,
  noteSourceHash,
  readSourceLines,
  resetTaskToggleState,
  serializeWrite,
  toggleTask,
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
  noteHash: noteSourceHash,
  serialize: serializeWrite,
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
    expect(reviewBody()).toMatchObject({ path: 'deck.md', line: 1, expected: 'Q?', rating: 'good' })
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

  it("stamps the review with the reviewer's local wall-clock time", async () => {
    // Local-time constructor: whatever zone the test runs in, the reviewer's
    // clock reads 09:05 (seconds dropped, zero-padded), and that is what the
    // server must write — not its own clock.
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(new Date(2026, 0, 2, 9, 5, 42))
    try {
      route({
        ok: true,
        status: 200,
        json: () =>
          Promise.resolve({ entry: '2026-01-02 09:05 - Hard', line: 4, inserted_at: 3, inserted: ['a', 'b'] }),
      })
      await recordReview({ line: 1, rating: 'hard' })
      expect(reviewBody()).toEqual({
        path: 'deck.md',
        line: 1,
        expected: 'Q?',
        rating: 'hard',
        at: '2026-01-02 09:05',
      })
    } finally {
      vi.useRealTimers()
    }
  })

  it('explains a refused time as a clock problem', async () => {
    route({ ok: false, status: 422 })
    const outcome = await recordReview({ line: 1, rating: 'good' })
    expect(outcome).toMatchObject({ ok: false, kind: 'other' })
    expect(outcome.ok === false && outcome.message).toMatch(/clock/)
  })

  it('reports a 409 as a conflict and forgets the cached file', async () => {
    route({ ok: false, status: 409 })
    const outcome = await recordReview({ line: 1, rating: 'again' })
    expect(outcome).toMatchObject({ ok: false, kind: 'conflict' })
    expect(outcome.ok === false && outcome.message).toContain('changed somewhere else')
    await readSourceLines('deck.md')
    expect(fetchMock.mock.calls.filter((c) => String(c[0]).startsWith('/.mbr/raw/'))).toHaveLength(2)
  })

  it('reports a missing token, and remembers one is needed', async () => {
    route({ ok: false, status: 401 })
    const outcome = await recordReview({ line: 1, rating: 'hard' })
    expect(outcome).toMatchObject({ ok: false, kind: 'auth' })
    expect(isEditTokenRequired()).toBe(true)
  })

  it('reports a 403 on the write as a refusal, not a token problem', async () => {
    route({ ok: false, status: 403 })
    const outcome = await recordReview({ line: 1, rating: 'good' })
    expect(outcome).toMatchObject({ ok: false, kind: 'refused' })
    expect(outcome.ok === false && outcome.message).not.toMatch(/token/)
    expect(isEditTokenRequired()).toBe(false)
  })

  it('reports a 403 on the source read as a refusal, without writing', async () => {
    // `/.mbr/raw` sits behind the same `check_edit_access`, so a refused
    // address fails here first.
    fetchMock.mockImplementation(() => Promise.resolve({ ok: false, status: 403 }))
    const outcome = await recordReview({ line: 1, rating: 'good' })
    expect(outcome).toMatchObject({ ok: false, kind: 'refused' })
    expect(fetchMock.mock.calls.some((c) => c[0] === '/.mbr/flashcard-review')).toBe(false)
  })

  it('reports a 404 on the source read as a conflict: the note moved or changed elsewhere', async () => {
    fetchMock.mockImplementation(() => Promise.resolve({ ok: false, status: 404 }))
    const outcome = await recordReview({ line: 1, rating: 'good' })
    expect(outcome).toMatchObject({ ok: false, kind: 'conflict' })
    expect(fetchMock.mock.calls.some((c) => c[0] === '/.mbr/flashcard-review')).toBe(false)
  })

  it('reports a 503 on the read or the write as still downloading, retryable', async () => {
    fetchMock.mockImplementation(() => Promise.resolve({ ok: false, status: 503 }))
    const read = await recordReview({ line: 1, rating: 'good' })
    expect(read).toMatchObject({ ok: false, kind: 'other' })
    expect(read.ok === false && read.message).toMatch(/still downloading/)

    route({ ok: false, status: 503 })
    const write = await recordReview({ line: 1, rating: 'good' })
    expect(write).toMatchObject({ ok: false, kind: 'other' })
    expect(write.ok === false && write.message).toMatch(/still downloading/)
  })

  it('treats a line past the end of the file as a conflict, without writing', async () => {
    route({ ok: true, status: 200 })
    const outcome = await recordReview({ line: 99, rating: 'easy' })
    expect(outcome).toMatchObject({ ok: false, kind: 'conflict' })
    expect(fetchMock.mock.calls.some((c) => c[0] === '/.mbr/flashcard-review')).toBe(false)
  })
})

describe('makeReviewRecorder: the version of the note', () => {
  /** Raw reads return SOURCE as version `hash`; every write answers `reply`. */
  function routeVersioned(hash: string, reply: (url: string) => unknown) {
    fetchMock.mockImplementation((url: string) => {
      if (String(url).startsWith('/.mbr/raw/')) {
        return Promise.resolve({
          ok: true,
          status: 200,
          headers: new Headers({ 'x-mbr-content-hash': hash }),
          text: () => Promise.resolve(SOURCE),
        })
      }
      return Promise.resolve(reply(String(url)))
    })
  }

  function bodies(endpoint: string): Array<Record<string, unknown>> {
    return fetchMock.mock.calls
      .filter((c) => c[0] === endpoint)
      .map((c) => JSON.parse((c[1] as RequestInit).body as string))
  }

  beforeEach(() => {
    window.frontmatter = { markdown_source: 'deck.md', markdown_source_hash: 'rendered' }
  })

  afterEach(() => {
    window.frontmatter = undefined
  })

  it('refuses at once, without writing, when the note is no longer the rendered version', async () => {
    routeVersioned('changed-elsewhere', () => ({ ok: true, status: 200 }))
    const outcome = await recordReview({ line: 1, rating: 'good' })
    expect(outcome).toMatchObject({ ok: false, kind: 'conflict' })
    expect(bodies('/.mbr/flashcard-review')).toHaveLength(0)
  })

  it('sends the rendered version, then chains the answered one into a task toggle', async () => {
    routeVersioned('rendered', (url) =>
      url === '/.mbr/flashcard-review'
        ? {
            ok: true,
            status: 200,
            json: () =>
              Promise.resolve({
                entry: '2026-10-06 13:45 - Good',
                line: 4,
                inserted_at: 3,
                inserted: [': ___Review History___', '  * 2026-10-06 13:45 - Good'],
                hash: 'after-review',
              }),
          }
        : {
            ok: true,
            status: 200,
            json: () => Promise.resolve({ line: 6, text: 'Next?', hash: 'after-toggle' }),
          }
    )

    expect(await recordReview({ line: 1, rating: 'good' })).toMatchObject({ ok: true })
    expect(bodies('/.mbr/flashcard-review')[0]).toMatchObject({ base_hash: 'rendered' })

    // Same page, same file: the toggle names the version the review left.
    expect(await toggleTask({ path: 'deck.md', line: 6, to: 'done' })).toMatchObject({ ok: true })
    expect(bodies('/.mbr/task')[0]).toMatchObject({ line: 6, expected: 'Next?', base_hash: 'after-review' })

    // And the review after it names the toggle's, still from the one read.
    await recordReview({ line: 1, rating: 'easy' })
    expect(bodies('/.mbr/flashcard-review')[1]).toMatchObject({ base_hash: 'after-toggle' })
    expect(fetchMock.mock.calls.filter((c) => String(c[0]).startsWith('/.mbr/raw/'))).toHaveLength(1)
  })
})
