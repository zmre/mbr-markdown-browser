import { beforeEach, describe, expect, it } from 'vitest'
import {
  coerceNote,
  createNote,
  displayQuote,
  ENVELOPE_VERSION,
  MAX_BODY,
  MAX_QUOTE,
  newId,
  parseEnvelope,
  serializeEnvelope,
  STORAGE_KEY,
} from './note-model.ts'
import { makeNote, resetNoteIds, T0 } from './test-fixtures.ts'

beforeEach(() => resetNoteIds())

/** The block separator `find-in-page.ts` emits between block elements. */
const SEP = '\u0000'

describe('STORAGE_KEY', () => {
  it('follows the mbr_* convention the other stores use', () => {
    expect(STORAGE_KEY).toBe('mbr_review_notes')
  })
})

describe('parseEnvelope', () => {
  it('returns an empty writable store for absent or empty input', () => {
    expect(parseEnvelope(null, T0)).toEqual({ notes: [], writable: true })
    expect(parseEnvelope('', T0)).toEqual({ notes: [], writable: true })
  })

  it('never throws on malformed JSON', () => {
    expect(parseEnvelope('{not json', T0)).toEqual({ notes: [], writable: true })
    expect(parseEnvelope('null', T0)).toEqual({ notes: [], writable: true })
    expect(parseEnvelope('[1,2,3]', T0)).toEqual({ notes: [], writable: true })
  })

  it('round-trips a store it wrote', () => {
    const notes = [makeNote({ line: 3 }), makeNote({ line: 7, type: 'issue' })]
    const parsed = parseEnvelope(serializeEnvelope(notes), T0)
    expect(parsed.writable).toBe(true)
    expect(parsed.notes).toEqual(notes)
  })

  it('drops only the malformed entries, keeping the rest', () => {
    // The whole point of per-note coercion: one bad entry must not throw away
    // a review the user spent an hour on.
    const good = makeNote({ line: 4 })
    const raw = JSON.stringify({
      v: ENVELOPE_VERSION,
      notes: [good, { id: '', file: 'x.md', body: 'no id' }, { file: 'x.md' }, null, 42, good],
    })
    const parsed = parseEnvelope(raw, T0)
    expect(parsed.notes).toHaveLength(1)
    expect(parsed.notes[0]?.id).toBe(good.id)
  })

  it('keeps the first of a duplicated id', () => {
    const a = makeNote({ id: 'dup', body: 'first' })
    const b = makeNote({ id: 'dup', body: 'second' })
    const parsed = parseEnvelope(JSON.stringify({ v: 1, notes: [a, b] }), T0)
    expect(parsed.notes).toHaveLength(1)
    expect(parsed.notes[0]?.body).toBe('first')
  })

  it('refuses to write over a newer envelope', () => {
    // Writing would silently drop fields this build knows nothing about.
    const raw = JSON.stringify({ v: ENVELOPE_VERSION + 1, notes: [makeNote()] })
    expect(parseEnvelope(raw, T0)).toEqual({ notes: [], writable: false })
  })

  it('starts clean on an unrecognised older version', () => {
    const raw = JSON.stringify({ v: 0, notes: [makeNote()] })
    expect(parseEnvelope(raw, T0)).toEqual({ notes: [], writable: true })
  })
})

describe('coerceNote', () => {
  it('rejects an entry with no id, no file, or no body', () => {
    expect(coerceNote({ file: 'a.md', body: '' }, T0)).toBeNull()
    expect(coerceNote({ id: 'x', body: '' }, T0)).toBeNull()
    expect(coerceNote({ id: 'x', file: 'a.md' }, T0)).toBeNull()
    expect(coerceNote(null, T0)).toBeNull()
    expect(coerceNote('a string', T0)).toBeNull()
  })

  it('falls back to the default type for an unknown one', () => {
    // The stale-id case genealogy/selector.ts guards against, applied per field.
    const note = coerceNote({ id: 'x', file: 'a.md', body: '', type: 'from-a-newer-mbr' }, T0)
    expect(note?.type).toBe('note')
  })

  it('rejects non-positive and non-integer lines', () => {
    expect(coerceNote({ id: 'x', file: 'a.md', body: '', line: 0 }, T0)?.line).toBeNull()
    expect(coerceNote({ id: 'x', file: 'a.md', body: '', line: -3 }, T0)?.line).toBeNull()
    expect(coerceNote({ id: 'x', file: 'a.md', body: '', line: 1.5 }, T0)?.line).toBeNull()
    expect(coerceNote({ id: 'x', file: 'a.md', body: '', line: '4' }, T0)?.line).toBeNull()
  })

  it('discards an endLine that is not past the start', () => {
    const same = coerceNote({ id: 'x', file: 'a.md', body: '', line: 5, endLine: 5 }, T0)
    const before = coerceNote({ id: 'x', file: 'a.md', body: '', line: 5, endLine: 2 }, T0)
    expect(same?.endLine).toBeNull()
    expect(before?.endLine).toBeNull()
  })

  it('forces anchorState to null on a file-level note', () => {
    const note = coerceNote({ id: 'x', file: 'a.md', body: '', line: null, anchorState: 'moved' }, T0)
    expect(note?.anchorState).toBeNull()
  })

  it('caps oversized text on read, not just on write', () => {
    // Otherwise a hand-edited or future-written store reintroduces an
    // unbounded value that the next write persists.
    const note = coerceNote(
      { id: 'x', file: 'a.md', body: 'b'.repeat(MAX_BODY + 500), quote: 'q'.repeat(MAX_QUOTE + 500) },
      T0
    )
    expect(note?.body).toHaveLength(MAX_BODY)
    expect(note?.quote).toHaveLength(MAX_QUOTE)
  })

  it('defaults missing timestamps to the supplied clock', () => {
    const note = coerceNote({ id: 'x', file: 'a.md', body: '' }, T0)
    expect(note?.createdAt).toBe(T0)
    expect(note?.updatedAt).toBe(T0)
  })
})

describe('createNote', () => {
  it('fills in the generated fields', () => {
    const note = createNote({ file: 'a.md', type: 'issue', body: 'x', line: 4 }, T0, 'fixed-id')
    expect(note).toEqual({
      id: 'fixed-id',
      file: 'a.md',
      line: 4,
      endLine: null,
      quote: null,
      type: 'issue',
      body: 'x',
      suggestion: null,
      anchorState: 'exact',
      createdAt: T0,
      updatedAt: T0,
    })
  })

  it('leaves a file-level note with no anchor state', () => {
    const note = createNote({ file: 'a.md', type: 'note', body: 'x' }, T0, 'id')
    expect(note.line).toBeNull()
    expect(note.anchorState).toBeNull()
  })

  it('keeps an endLine only when it is past the start', () => {
    expect(createNote({ file: 'a.md', type: 'note', body: '', line: 3, endLine: 8 }, T0, 'i').endLine).toBe(8)
    expect(createNote({ file: 'a.md', type: 'note', body: '', line: 3, endLine: 3 }, T0, 'i').endLine).toBeNull()
  })
})

describe('newId', () => {
  it('falls back when crypto.randomUUID is unavailable', () => {
    // `--host 0.0.0.0` reached by IP is not a secure context, so this path is
    // real rather than defensive.
    const original = globalThis.crypto
    Object.defineProperty(globalThis, 'crypto', { value: undefined, configurable: true })
    try {
      const id = newId(T0, () => 0.5)
      expect(typeof id).toBe('string')
      expect(id.length).toBeGreaterThan(0)
    } finally {
      Object.defineProperty(globalThis, 'crypto', { value: original, configurable: true })
    }
  })
})

describe('displayQuote', () => {
  it('turns the block separator into a newline', () => {
    expect(displayQuote(`one${SEP}two`)).toBe('one\ntwo')
  })

  it('is empty for a missing quote', () => {
    expect(displayQuote(null)).toBe('')
  })

  it('leaves ordinary text alone', () => {
    expect(displayQuote('plain text')).toBe('plain text')
  })
})
