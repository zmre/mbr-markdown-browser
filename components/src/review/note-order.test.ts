import { beforeEach, describe, expect, it } from 'vitest'
import { compareNotes, groupByFile, notesForFile, sortNotes } from './note-order.ts'
import { makeNote, resetNoteIds } from './test-fixtures.ts'

beforeEach(() => resetNoteIds())

describe('compareNotes', () => {
  it('orders by file first', () => {
    const a = makeNote({ file: 'a.md', line: 99 })
    const b = makeNote({ file: 'b.md', line: 1 })
    expect(compareNotes(a, b)).toBeLessThan(0)
  })

  it('compares file names by code unit, not locale', () => {
    // localeCompare would order these by collation rules that vary with the
    // engine's ICU data — and this order is baked into the export's item
    // numbers, so it has to be the same on every machine.
    const upper = makeNote({ file: 'B.md' })
    const lower = makeNote({ file: 'a.md' })
    expect(compareNotes(upper, lower)).toBeLessThan(0)
    expect('B.md'.localeCompare('a.md')).toBeGreaterThan(0)
  })

  it('puts a file-level note before the line notes of the same file', () => {
    const fileLevel = makeNote({ line: null })
    const lineOne = makeNote({ line: 1 })
    expect(compareNotes(fileLevel, lineOne)).toBeLessThan(0)
    expect(compareNotes(lineOne, fileLevel)).toBeGreaterThan(0)
  })

  it('orders by line within a file', () => {
    expect(compareNotes(makeNote({ line: 5 }), makeNote({ line: 40 }))).toBeLessThan(0)
  })

  it('breaks a line tie by endLine, then createdAt, then id', () => {
    const short = makeNote({ id: 'z', line: 5, endLine: null, createdAt: 100 })
    const long = makeNote({ id: 'a', line: 5, endLine: 9, createdAt: 100 })
    expect(compareNotes(short, long)).toBeLessThan(0)

    const early = makeNote({ id: 'z', line: 5, createdAt: 1 })
    const late = makeNote({ id: 'a', line: 5, createdAt: 2 })
    expect(compareNotes(early, late)).toBeLessThan(0)

    const idA = makeNote({ id: 'a', line: 5, createdAt: 1 })
    const idB = makeNote({ id: 'b', line: 5, createdAt: 1 })
    expect(compareNotes(idA, idB)).toBeLessThan(0)
  })

  it('is total: only an identical note compares equal', () => {
    const note = makeNote()
    expect(compareNotes(note, note)).toBe(0)
  })
})

describe('sortNotes', () => {
  it('does not mutate its input', () => {
    const input = [makeNote({ line: 9 }), makeNote({ line: 1 })]
    const before = input.map((n) => n.line)
    sortNotes(input)
    expect(input.map((n) => n.line)).toEqual(before)
  })

  it('produces the same order regardless of input order', () => {
    const a = makeNote({ file: 'a.md', line: 3 })
    const b = makeNote({ file: 'a.md', line: 1 })
    const c = makeNote({ file: 'b.md', line: 2 })
    const one = sortNotes([a, b, c]).map((n) => n.id)
    const two = sortNotes([c, a, b]).map((n) => n.id)
    expect(one).toEqual(two)
  })
})

describe('groupByFile', () => {
  it('groups in sorted order with each file appearing once', () => {
    const groups = groupByFile([
      makeNote({ file: 'b.md', line: 2 }),
      makeNote({ file: 'a.md', line: 5 }),
      makeNote({ file: 'b.md', line: 1 }),
      makeNote({ file: 'a.md', line: 1 }),
    ])
    expect(groups.map((g) => g.file)).toEqual(['a.md', 'b.md'])
    expect(groups[0]?.notes.map((n) => n.line)).toEqual([1, 5])
    expect(groups[1]?.notes.map((n) => n.line)).toEqual([1, 2])
  })

  it('returns nothing for an empty set', () => {
    expect(groupByFile([])).toEqual([])
  })

  it('agrees with sortNotes, so the panel and the export cannot diverge', () => {
    const notes = [
      makeNote({ file: 'b.md', line: 2 }),
      makeNote({ file: 'a.md', line: null }),
      makeNote({ file: 'a.md', line: 4 }),
    ]
    expect(groupByFile(notes).flatMap((g) => g.notes.map((n) => n.id))).toEqual(
      sortNotes(notes).map((n) => n.id)
    )
  })
})

describe('notesForFile', () => {
  it('selects one file and sorts it', () => {
    const notes = [
      makeNote({ file: 'a.md', line: 9 }),
      makeNote({ file: 'b.md', line: 1 }),
      makeNote({ file: 'a.md', line: 2 }),
    ]
    expect(notesForFile(notes, 'a.md').map((n) => n.line)).toEqual([2, 9])
  })

  it('is empty for a file with no notes', () => {
    expect(notesForFile([makeNote({ file: 'a.md' })], 'other.md')).toEqual([])
  })
})
