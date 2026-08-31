/**
 * Shared note fixtures for the review tests.
 *
 * Named `test-fixtures.ts` deliberately: `edit-token.test.ts`'s
 * `shippedModules()` filter drops that exact name, so nothing here counts as
 * shipped source for the web-storage scan.
 */

import type { NoteType, ReviewNote } from './types.ts'

/** A fixed clock, so every fixture is byte-stable. */
export const T0 = 1_700_000_000_000

let counter = 0

/** Reset the id counter so a test's ids are predictable. */
export function resetNoteIds(): void {
  counter = 0
}

/**
 * A complete note with sane defaults. Override anything.
 *
 * `id` auto-increments with a zero-padded suffix so the default ordering is
 * also the creation ordering — `n-10` must not sort before `n-2`.
 */
export function makeNote(overrides: Partial<ReviewNote> = {}): ReviewNote {
  counter += 1
  const line = overrides.line === undefined ? 10 : overrides.line
  return {
    id: `n-${String(counter).padStart(3, '0')}`,
    file: 'doc.md',
    line,
    endLine: null,
    quote: 'the quoted text',
    type: 'note' as NoteType,
    body: 'A comment.',
    suggestion: null,
    anchorState: line === null ? null : 'exact',
    createdAt: T0,
    updatedAt: T0,
    ...overrides,
  }
}
