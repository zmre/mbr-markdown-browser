/**
 * The persisted shape of a review, and the validation that guards it.
 *
 * Pure string/JSON work only — the `localStorage` calls themselves live in
 * `review-store.ts`, which is the single module in the frontend allowed to
 * touch web storage (see `edit-token.test.ts`). Splitting it this way means the
 * whole parse/coerce/serialize path is unit-testable without a storage stub.
 *
 * The convention followed here is `genealogy/selector.ts`'s: **validate on read
 * and fall back**, rather than version-migrate. The difference is that a review
 * is user-authored content, so the fallback is per *note*, never per store — one
 * malformed entry must not throw away the other forty.
 */

import { DEFAULT_NOTE_TYPE, isNoteType, type AnchorState, type NoteDraft, type ReviewNote } from './types.ts'

/** `localStorage` key. `mbr_*` is the house convention (`mbr_genealogy_chart`, `mbr_recent_files`). */
export const STORAGE_KEY = 'mbr_review_notes'

/** Envelope version this build reads and writes. */
export const ENVELOPE_VERSION = 1

/**
 * Caps, applied on write **and** on read.
 *
 * On read as well because a store written by a future build, or edited by hand
 * in devtools, would otherwise reintroduce an unbounded value that the next
 * write persists. `mbr-video-extras.ts` has no cap and grows without limit;
 * this one is bounded by construction.
 */
export const MAX_QUOTE = 400
export const MAX_BODY = 8000
export const MAX_SUGGESTION = 8000

/** The result of reading the store. */
export interface ParsedEnvelope {
  notes: ReviewNote[]
  /**
   * False when the stored envelope came from a NEWER mbr.
   *
   * Writing would silently drop whatever fields this build does not know about,
   * so the store goes read-only and the panel says so. A forward-compatible
   * merge is not possible without knowing the future schema.
   */
  writable: boolean
}

function clamp(value: string, max: number): string {
  return value.length > max ? value.slice(0, max) : value
}

function coerceLine(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0 ? value : null
}

function coerceText(value: unknown, max: number): string | null {
  return typeof value === 'string' ? clamp(value, max) : null
}

function coerceAnchorState(value: unknown): AnchorState | null {
  return value === 'exact' || value === 'moved' || value === 'lost' ? value : null
}

function coerceTimestamp(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

/**
 * Validate one stored entry, or reject it.
 *
 * `id`, `file` and `body` are load-bearing and have no sensible default — a
 * note with no id cannot be edited or deleted, and one with no file cannot be
 * exported — so their absence rejects the entry. Everything else degrades.
 *
 * `now` is a parameter rather than a `Date.now()` call so the function stays
 * pure and its tests need no clock.
 */
export function coerceNote(value: unknown, now: number): ReviewNote | null {
  if (typeof value !== 'object' || value === null) return null
  const raw = value as Record<string, unknown>

  const id = typeof raw.id === 'string' && raw.id.length > 0 ? raw.id : null
  const file = typeof raw.file === 'string' && raw.file.length > 0 ? raw.file : null
  if (id === null || file === null) return null
  if (typeof raw.body !== 'string') return null

  const line = coerceLine(raw.line)
  const rawEnd = coerceLine(raw.endLine)
  // An endLine that is not strictly past the start carries no information, and
  // `formatLocation` and the panel both key off `null` meaning "single line".
  const endLine = line !== null && rawEnd !== null && rawEnd > line ? rawEnd : null

  const createdAt = coerceTimestamp(raw.createdAt, now)

  return {
    id,
    file,
    line,
    endLine,
    quote: coerceText(raw.quote, MAX_QUOTE),
    type: isNoteType(raw.type) ? raw.type : DEFAULT_NOTE_TYPE,
    body: clamp(raw.body, MAX_BODY),
    suggestion: coerceText(raw.suggestion, MAX_SUGGESTION),
    // A file-level note has nothing to re-anchor, so its state is always null.
    anchorState: line === null ? null : coerceAnchorState(raw.anchorState),
    createdAt,
    updatedAt: coerceTimestamp(raw.updatedAt, createdAt),
  }
}

/**
 * Read the envelope.
 *
 * Never throws. Absent, unparseable and unrecognised all yield an empty,
 * writable store — the same "start clean rather than break" posture the other
 * three storage users take.
 */
export function parseEnvelope(raw: string | null, now: number): ParsedEnvelope {
  if (raw === null || raw.length === 0) return { notes: [], writable: true }

  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return { notes: [], writable: true }
  }

  if (typeof parsed !== 'object' || parsed === null) return { notes: [], writable: true }
  const envelope = parsed as Record<string, unknown>

  if (typeof envelope.v === 'number' && envelope.v > ENVELOPE_VERSION) {
    return { notes: [], writable: false }
  }
  if (envelope.v !== ENVELOPE_VERSION || !Array.isArray(envelope.notes)) {
    return { notes: [], writable: true }
  }

  const notes: ReviewNote[] = []
  const seen = new Set<string>()
  for (const entry of envelope.notes) {
    const note = coerceNote(entry, now)
    // A duplicate id would make delete and edit ambiguous; keep the first.
    if (note !== null && !seen.has(note.id)) {
      seen.add(note.id)
      notes.push(note)
    }
  }
  return { notes, writable: true }
}

/** Serialize for storage. The inverse of {@link parseEnvelope}. */
export function serializeEnvelope(notes: readonly ReviewNote[]): string {
  return JSON.stringify({ v: ENVELOPE_VERSION, notes })
}

/**
 * A fresh id.
 *
 * `crypto.randomUUID` needs a secure context, which `--host 0.0.0.0` reached by
 * IP is not, so the fallback is not theoretical. Uniqueness only has to hold
 * within one browser profile's store.
 */
export function newId(now: number, random: () => number = Math.random): string {
  const uuid = globalThis.crypto?.randomUUID
  if (typeof uuid === 'function') {
    try {
      return globalThis.crypto.randomUUID()
    } catch {
      // Fall through to the manual form.
    }
  }
  return `${now.toString(36)}-${Math.floor(random() * 0xffffffff).toString(36)}`
}

/** Build a complete note from the fields a caller supplies. */
export function createNote(draft: NoteDraft, now: number, id: string = newId(now)): ReviewNote {
  const line = draft.line ?? null
  const endLine = line !== null && draft.endLine !== null && draft.endLine !== undefined && draft.endLine > line
    ? draft.endLine
    : null
  return {
    id,
    file: draft.file,
    line,
    endLine,
    quote: draft.quote ? clamp(draft.quote, MAX_QUOTE) : null,
    type: draft.type,
    body: clamp(draft.body, MAX_BODY),
    suggestion: draft.suggestion ? clamp(draft.suggestion, MAX_SUGGESTION) : null,
    anchorState: line === null ? null : (draft.anchorState ?? 'exact'),
    createdAt: now,
    updatedAt: now,
  }
}

/**
 * The block separator `find-in-page.ts` puts between block elements, rendered
 * for human eyes.
 *
 * U+0000 is chosen there precisely because the HTML parser rewrites it to
 * U+FFFD, so it can never occur in real page text — which makes it safe to
 * store inside a quote and translate back here.
 */
export function displayQuote(quote: string | null): string {
  return quote === null ? '' : quote.replace(/\u0000/g, '\n')
}
