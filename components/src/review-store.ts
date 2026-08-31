/**
 * Durable storage for review notes.
 *
 * # Why this module names `localStorage` directly
 *
 * `edit-token.test.ts` scans every shipped module's source for the identifier
 * and pins the list of modules allowed to use it. The scan is a plain substring
 * match **per module**, so routing storage through a shared wrapper would make
 * every consumer invisible to it and leave one file on the list standing in for
 * all of them. This module therefore calls `localStorage` itself, and is the
 * only part of the review feature that does — which is also why it lives here
 * rather than in `review/`: the lazy chunk must not import it (see
 * `review/index.ts`), and a stateful module inside the chunk's own directory
 * would invite exactly that.
 *
 * # What is stored, and why that is acceptable
 *
 * A review note holds the reader's own words about a document already open in
 * front of them, plus a quote of text already on the page and the file and line
 * it came from. Nothing here is a credential, and nothing crosses an origin.
 * Durability is the entire point of the feature: a live reload after an edit,
 * or a navigation to check another page, must not throw the review away. That
 * is the argument the allowlist asks each entry to make.
 *
 * Contrast `edit-token.ts`, which keeps the edit bearer token in a single
 * in-memory variable precisely *because* it is a credential.
 *
 * # Lost-update safety
 *
 * Every mutation is a read-modify-write keyed on the note's `id`, not a write
 * of a cached array. Two mbr windows open on the same repo would otherwise
 * clobber each other silently — one saves, the other saves its own stale copy
 * of the whole list, and the first note is gone. A `storage` listener keeps
 * both windows' markers and note counts live.
 */

import {
  createNote,
  parseEnvelope,
  serializeEnvelope,
  STORAGE_KEY,
  type ParsedEnvelope,
} from './review/note-model.ts'
import { sortNotes } from './review/note-order.ts'
import type { NoteDraft, ReviewNote, ReviewStoreApi } from './review/types.ts'
import { getRepoId } from './shared.ts'

/** Cached view of the store, or `null` before the first read. */
let cache: ParsedEnvelope | null = null

/** Listeners notified after any change, including one made by another window. */
const listeners = new Set<() => void>()

/** True once the `storage` listener is attached. */
let listening = false

/**
 * The `localStorage` key this page's notes live under.
 *
 * `STORAGE_KEY` suffixed with the served repository's id, so two repos
 * served from the same origin (GUI mode's fixed default port, or a reused
 * dev port) get independent stores instead of bleeding into each other. When
 * `getRepoId()` is empty — static builds, the CLI, QuickLook, none of which
 * have review notes — this falls back to the bare `STORAGE_KEY`, which is
 * also what a store written before this scoping existed used.
 */
function storageKey(): string {
  const repoId = getRepoId()
  return repoId === '' ? STORAGE_KEY : `${STORAGE_KEY}:${repoId}`
}

/**
 * Read the raw string, tolerating storage that is absent, disabled or throwing.
 *
 * Private-browsing modes and enterprise policies both make `localStorage`
 * getters throw rather than return null, so the guard is not decorative.
 */
function readRaw(): string | null {
  try {
    return localStorage.getItem(storageKey())
  } catch {
    return null
  }
}

/** Write the raw string. Returns false when storage refused it (quota, policy). */
function writeRaw(value: string): boolean {
  try {
    localStorage.setItem(storageKey(), value)
    return true
  } catch {
    // Quota exceeded, disabled storage, private mode. The caller surfaces this
    // rather than dropping the user's text on the floor.
    return false
  }
}

/**
 * Parse the store fresh.
 *
 * `Date.now()` is read here and passed down, so everything under
 * `review/note-model.ts` stays pure and clock-free.
 */
function readFresh(): ParsedEnvelope {
  return parseEnvelope(readRaw(), Date.now())
}

function ensureListening(): void {
  if (listening || typeof window === 'undefined') return
  listening = true
  // Fires only for changes made by *other* documents on this origin, which is
  // exactly the case the in-memory cache cannot observe. Compared against
  // storageKey(), not the bare STORAGE_KEY, so a write to a *different*
  // repo's scoped key (or the unscoped legacy key) does not wrongly
  // invalidate this repo's cache.
  window.addEventListener('storage', (event) => {
    if (event.key !== null && event.key !== storageKey()) return
    cache = null
    notify()
  })
}

function current(): ParsedEnvelope {
  ensureListening()
  if (cache === null) cache = readFresh()
  return cache
}

function notify(): void {
  for (const listener of [...listeners]) {
    try {
      listener()
    } catch (err) {
      // One broken subscriber must not stop the others from redrawing.
      console.warn('Review store listener failed:', err)
    }
  }
}

/**
 * Apply `mutate` to the notes currently **on disk**, then persist.
 *
 * Re-reading rather than using the cache is what makes concurrent windows safe.
 */
function commit(mutate: (notes: ReviewNote[]) => ReviewNote[]): boolean {
  ensureListening()
  const fresh = readFresh()
  if (!fresh.writable) {
    // A newer mbr wrote this store; writing would drop fields we cannot see.
    cache = fresh
    return false
  }

  const next = sortNotes(mutate(fresh.notes))
  if (!writeRaw(serializeEnvelope(next))) {
    cache = fresh
    return false
  }

  cache = { notes: next, writable: true }
  notify()
  return true
}

/** Every note, across every file, in display order. */
export function allNotes(): readonly ReviewNote[] {
  return current().notes
}

/** How many notes exist anywhere. Drives the floating action button. */
export function noteCount(): number {
  return current().notes.length
}

/** Notes for one source file, in display order. */
export function notesFor(file: string): ReviewNote[] {
  return current().notes.filter((note) => note.file === file)
}

/** False when a newer mbr's store is present and must not be overwritten. */
export function isWritable(): boolean {
  return current().writable
}

/** Insert or replace a note by id. Returns false when the write was refused. */
export function saveNote(note: ReviewNote): boolean {
  return commit((notes) => {
    const without = notes.filter((existing) => existing.id !== note.id)
    return [...without, note]
  })
}

/** Create and persist a note. Returns it, or null when the write was refused. */
export function addNote(draft: NoteDraft): ReviewNote | null {
  const note = createNote(draft, Date.now())
  return saveNote(note) ? note : null
}

/** Delete a note by id. Returns false when the write was refused. */
export function removeNote(id: string): boolean {
  return commit((notes) => notes.filter((note) => note.id !== id))
}

/** Delete every note. Returns false when the write was refused. */
export function clearNotes(): boolean {
  return commit(() => [])
}

/**
 * Persist re-anchoring results for one file in a single write.
 *
 * A write per note would be N storage round trips and N notifications on every
 * page load; this is one of each, and it is skipped entirely when nothing
 * moved — which is the common case.
 */
export function applyReanchor(updates: ReadonlyMap<string, Partial<ReviewNote>>): boolean {
  if (updates.size === 0) return true
  return commit((notes) =>
    notes.map((note) => {
      const update = updates.get(note.id)
      return update === undefined ? note : { ...note, ...update }
    })
  )
}

/** Subscribe to changes. Returns an unsubscribe function. */
export function subscribe(listener: () => void): () => void {
  ensureListening()
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/**
 * Drop the cached view.
 *
 * For tests, and for the `storage` path above. The next read re-parses.
 */
export function resetStoreCache(): void {
  cache = null
}

/**
 * The store as the lazy chunk sees it.
 *
 * Handed across as a Lit property because the chunk cannot import this module —
 * a second copy would carry a second cache and a second listener set, and a
 * note saved through one would be invisible to the other.
 */
export const reviewStore: ReviewStoreApi = {
  all: allNotes,
  save: saveNote,
  remove: removeNote,
  clear: clearNotes,
  writable: isWritable,
  subscribe,
}
