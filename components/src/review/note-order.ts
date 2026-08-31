/**
 * The order review notes are listed in, on screen and in the export.
 *
 * One order for both, so what a reader sees is what they copy. Pure; no DOM,
 * no state.
 */

import type { ReviewNote } from './types.ts'

/**
 * A **total** order over notes.
 *
 * Files are compared with `<`/`>` on the raw string, deliberately NOT
 * `localeCompare`: that depends on the engine's ICU data and its default
 * locale, so the same review would come out in a different order on a different
 * machine — and this order is baked into the copied markdown's item numbers.
 *
 * Within a file, a file-level note (`line === null`) sorts first: it is about
 * the document as a whole, so it reads as a preamble to the line notes.
 *
 * `id` is the final tiebreak, which is what makes the order total. Without it
 * two notes created in the same millisecond on the same line would sort
 * arbitrarily, and `Array.prototype.sort` is only stable with respect to the
 * input — which for a store rebuilt from JSON is not a fixed sequence.
 */
export function compareNotes(a: ReviewNote, b: ReviewNote): number {
  if (a.file !== b.file) return a.file < b.file ? -1 : 1

  if (a.line !== b.line) {
    if (a.line === null) return -1
    if (b.line === null) return 1
    return a.line - b.line
  }

  const aEnd = a.endLine ?? a.line ?? 0
  const bEnd = b.endLine ?? b.line ?? 0
  if (aEnd !== bEnd) return aEnd - bEnd

  if (a.createdAt !== b.createdAt) return a.createdAt - b.createdAt
  if (a.id !== b.id) return a.id < b.id ? -1 : 1
  return 0
}

/** A new array in {@link compareNotes} order. Does not mutate the input. */
export function sortNotes(notes: readonly ReviewNote[]): ReviewNote[] {
  return [...notes].sort(compareNotes)
}

/** A file and its notes, both already in order. */
export interface NoteGroup {
  file: string
  notes: ReviewNote[]
}

/**
 * Group sorted notes by file, preserving order.
 *
 * Sorts first rather than trusting the caller, so a group's contents and the
 * group sequence agree even when handed an arbitrary array. Since
 * {@link compareNotes} orders by file before anything else, one pass suffices
 * and a file can never appear twice.
 */
export function groupByFile(notes: readonly ReviewNote[]): NoteGroup[] {
  const groups: NoteGroup[] = []
  for (const note of sortNotes(notes)) {
    const last = groups[groups.length - 1]
    if (last && last.file === note.file) {
      last.notes.push(note)
    } else {
      groups.push({ file: note.file, notes: [note] })
    }
  }
  return groups
}

/** Notes for one file, in order. */
export function notesForFile(notes: readonly ReviewNote[], file: string): ReviewNote[] {
  return sortNotes(notes.filter((note) => note.file === file))
}
