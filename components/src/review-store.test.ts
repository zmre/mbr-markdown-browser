import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  addNote,
  allNotes,
  applyReanchor,
  clearNotes,
  isWritable,
  noteCount,
  notesFor,
  removeNote,
  resetStoreCache,
  reviewStore,
  saveNote,
  subscribe,
} from './review-store.ts'
import { ENVELOPE_VERSION, STORAGE_KEY, serializeEnvelope } from './review/note-model.ts'
import { makeNote, resetNoteIds } from './review/test-fixtures.ts'

beforeEach(() => {
  localStorage.clear()
  resetStoreCache()
  resetNoteIds()
})

afterEach(() => {
  vi.restoreAllMocks()
  localStorage.clear()
  resetStoreCache()
})

describe('reading', () => {
  it('is empty when nothing is stored', () => {
    expect(allNotes()).toEqual([])
    expect(noteCount()).toBe(0)
    expect(isWritable()).toBe(true)
  })

  it('reads a store written by a previous session', () => {
    const note = makeNote({ line: 4 })
    localStorage.setItem(STORAGE_KEY, serializeEnvelope([note]))
    expect(allNotes()).toEqual([note])
  })

  it('survives storage that throws on read', () => {
    // Private mode and enterprise policy both throw rather than return null.
    // `test-setup.ts` installs localStorage as a plain vi.fn mock, so the seam
    // is the method itself rather than `Storage.prototype`.
    vi.mocked(localStorage.getItem).mockImplementationOnce(() => {
      throw new Error('denied')
    })
    expect(allNotes()).toEqual([])
  })

  it('survives a corrupt store', () => {
    localStorage.setItem(STORAGE_KEY, '{ not json')
    expect(allNotes()).toEqual([])
    expect(isWritable()).toBe(true)
  })

  it('filters to one file', () => {
    localStorage.setItem(
      STORAGE_KEY,
      serializeEnvelope([makeNote({ file: 'a.md' }), makeNote({ file: 'b.md' })])
    )
    expect(notesFor('a.md')).toHaveLength(1)
    expect(notesFor('a.md')[0]?.file).toBe('a.md')
  })
})

describe('writing', () => {
  it('adds a note and persists it', () => {
    const note = addNote({ file: 'doc.md', type: 'issue', body: 'A problem.', line: 12 })
    expect(note).not.toBeNull()
    resetStoreCache()
    expect(allNotes()).toHaveLength(1)
    expect(allNotes()[0]?.body).toBe('A problem.')
  })

  it('replaces a note by id rather than duplicating it', () => {
    const note = addNote({ file: 'doc.md', type: 'note', body: 'first', line: 1 })!
    saveNote({ ...note, body: 'second' })
    expect(allNotes()).toHaveLength(1)
    expect(allNotes()[0]?.body).toBe('second')
  })

  it('keeps the store in display order', () => {
    addNote({ file: 'b.md', type: 'note', body: '', line: 1 })
    addNote({ file: 'a.md', type: 'note', body: '', line: 9 })
    addNote({ file: 'a.md', type: 'note', body: '', line: 2 })
    expect(allNotes().map((n) => `${n.file}:${n.line}`)).toEqual(['a.md:2', 'a.md:9', 'b.md:1'])
  })

  it('removes a note', () => {
    const note = addNote({ file: 'doc.md', type: 'note', body: 'x', line: 1 })!
    expect(removeNote(note.id)).toBe(true)
    expect(allNotes()).toEqual([])
  })

  it('clears every note', () => {
    addNote({ file: 'a.md', type: 'note', body: 'x' })
    addNote({ file: 'b.md', type: 'note', body: 'y' })
    expect(clearNotes()).toBe(true)
    expect(noteCount()).toBe(0)
  })

  it('reports a refused write instead of losing the text silently', () => {
    vi.mocked(localStorage.setItem).mockImplementationOnce(() => {
      throw new DOMException('quota', 'QuotaExceededError')
    })
    expect(saveNote(makeNote())).toBe(false)
  })

  it('refuses to overwrite a store written by a newer mbr', () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ v: ENVELOPE_VERSION + 1, notes: [{ id: 'future' }] })
    )
    expect(isWritable()).toBe(false)
    expect(saveNote(makeNote())).toBe(false)
    // And the future store is still on disk, unmodified.
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!).v).toBe(ENVELOPE_VERSION + 1)
  })
})

describe('lost-update safety', () => {
  it('merges against what is on disk, not against a stale cache', () => {
    // Two windows: this one reads, another writes, then this one saves. The
    // other window's note must survive.
    const mine = addNote({ file: 'doc.md', type: 'note', body: 'mine', line: 1 })!
    expect(allNotes()).toHaveLength(1)

    const theirs = makeNote({ id: 'from-other-window', file: 'doc.md', line: 5, body: 'theirs' })
    localStorage.setItem(STORAGE_KEY, serializeEnvelope([mine, theirs]))
    // This window has not noticed; its cache still holds one note.

    saveNote({ ...mine, body: 'mine, edited' })

    const bodies = allNotes().map((n) => n.body).sort()
    expect(bodies).toEqual(['mine, edited', 'theirs'])
  })

  it('deletes by id without dropping a concurrently added note', () => {
    const a = addNote({ file: 'doc.md', type: 'note', body: 'a', line: 1 })!
    const b = makeNote({ id: 'other', file: 'doc.md', line: 2, body: 'b' })
    localStorage.setItem(STORAGE_KEY, serializeEnvelope([a, b]))

    removeNote(a.id)
    expect(allNotes().map((n) => n.id)).toEqual(['other'])
  })
})

describe('applyReanchor', () => {
  it('writes every update in one commit', () => {
    const a = addNote({ file: 'doc.md', type: 'note', body: 'a', line: 1 })!
    const b = addNote({ file: 'doc.md', type: 'note', body: 'b', line: 2 })!

    const setItem = vi.mocked(localStorage.setItem)
    setItem.mockClear()
    applyReanchor(
      new Map([
        [a.id, { line: 10, anchorState: 'moved' as const }],
        [b.id, { anchorState: 'lost' as const }],
      ])
    )
    expect(setItem).toHaveBeenCalledTimes(1)

    const byId = new Map(allNotes().map((n) => [n.id, n]))
    expect(byId.get(a.id)?.line).toBe(10)
    expect(byId.get(a.id)?.anchorState).toBe('moved')
    expect(byId.get(b.id)?.anchorState).toBe('lost')
  })

  it('does not touch storage when nothing moved', () => {
    addNote({ file: 'doc.md', type: 'note', body: 'a', line: 1 })
    const setItem = vi.mocked(localStorage.setItem)
    setItem.mockClear()
    expect(applyReanchor(new Map())).toBe(true)
    expect(setItem).not.toHaveBeenCalled()
  })

  it('ignores an update for a note that no longer exists', () => {
    addNote({ file: 'doc.md', type: 'note', body: 'a', line: 1 })
    expect(applyReanchor(new Map([['gone', { line: 99 }]]))).toBe(true)
    expect(allNotes()[0]?.line).toBe(1)
  })
})

describe('subscribe', () => {
  it('notifies on change and stops after unsubscribe', () => {
    const listener = vi.fn()
    const off = subscribe(listener)
    addNote({ file: 'doc.md', type: 'note', body: 'x' })
    expect(listener).toHaveBeenCalledTimes(1)
    off()
    addNote({ file: 'doc.md', type: 'note', body: 'y' })
    expect(listener).toHaveBeenCalledTimes(1)
  })

  it('keeps notifying the other listeners when one throws', () => {
    const good = vi.fn()
    const off1 = subscribe(() => {
      throw new Error('boom')
    })
    const off2 = subscribe(good)
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    addNote({ file: 'doc.md', type: 'note', body: 'x' })
    expect(good).toHaveBeenCalled()
    off1()
    off2()
  })

  it('refreshes when another window writes', () => {
    const listener = vi.fn()
    const off = subscribe(listener)
    // Prime the cache so the refresh is observable.
    expect(noteCount()).toBe(0)

    localStorage.setItem(STORAGE_KEY, serializeEnvelope([makeNote()]))
    window.dispatchEvent(new StorageEvent('storage', { key: STORAGE_KEY }))

    expect(listener).toHaveBeenCalled()
    expect(noteCount()).toBe(1)
    off()
  })

  it('ignores a storage event for an unrelated key', () => {
    const listener = vi.fn()
    const off = subscribe(listener)
    window.dispatchEvent(new StorageEvent('storage', { key: 'mbr_recent_files' }))
    expect(listener).not.toHaveBeenCalled()
    off()
  })
})

describe('reviewStore (the chunk-facing view)', () => {
  it('exposes exactly the operations the chunk needs', () => {
    // Handed across as a Lit property; the chunk cannot import this module,
    // because a second copy would carry a second cache.
    expect(typeof reviewStore.all).toBe('function')
    expect(typeof reviewStore.save).toBe('function')
    expect(typeof reviewStore.remove).toBe('function')
    expect(typeof reviewStore.writable).toBe('function')
    expect(typeof reviewStore.subscribe).toBe('function')
  })

  it('shares state with the module functions', () => {
    addNote({ file: 'doc.md', type: 'note', body: 'x' })
    expect(reviewStore.all()).toHaveLength(1)
  })
})

describe('repo-scoped storage', () => {
  const originalConfig = window.__MBR_CONFIG__

  beforeEach(() => {
    window.__MBR_CONFIG__ = undefined
  })

  afterEach(() => {
    window.__MBR_CONFIG__ = originalConfig
  })

  it('falls back to the bare STORAGE_KEY when no repoId is configured', () => {
    // Unchanged from before repo scoping existed: static builds, the CLI and
    // QuickLook never set repoId, and a pre-scoping store lived here too.
    addNote({ file: 'doc.md', type: 'note', body: 'x', line: 1 })
    expect(localStorage.getItem(STORAGE_KEY)).not.toBeNull()
  })

  it('writes under a repo-scoped key, leaving the bare key untouched', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, repoId: 'repo-a' }
    addNote({ file: 'doc.md', type: 'note', body: 'x', line: 1 })
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull()
    expect(localStorage.getItem(`${STORAGE_KEY}:repo-a`)).not.toBeNull()
  })

  it('keeps two repos independent, surviving resetStoreCache()', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, repoId: 'repo-a' }
    addNote({ file: 'doc.md', type: 'note', body: 'from a', line: 1 })
    resetStoreCache()

    // Switching repos (same origin, different served repo) must not see
    // repo-a's note.
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, repoId: 'repo-b' }
    resetStoreCache()
    expect(allNotes()).toEqual([])
    addNote({ file: 'doc.md', type: 'note', body: 'from b', line: 2 })
    resetStoreCache()
    expect(allNotes().map((n) => n.body)).toEqual(['from b'])

    // Switching back to repo-a still finds its own note, undisturbed.
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, repoId: 'repo-a' }
    resetStoreCache()
    expect(allNotes().map((n) => n.body)).toEqual(['from a'])
  })

  it('refreshes on a storage event for the current repo-scoped key', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, repoId: 'repo-a' }
    const listener = vi.fn()
    const off = subscribe(listener)
    expect(noteCount()).toBe(0)

    localStorage.setItem(`${STORAGE_KEY}:repo-a`, serializeEnvelope([makeNote()]))
    window.dispatchEvent(new StorageEvent('storage', { key: `${STORAGE_KEY}:repo-a` }))

    expect(listener).toHaveBeenCalled()
    expect(noteCount()).toBe(1)
    off()
  })

  it('ignores a storage event for a different repo-scoped key', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, repoId: 'repo-a' }
    const listener = vi.fn()
    const off = subscribe(listener)

    window.dispatchEvent(new StorageEvent('storage', { key: `${STORAGE_KEY}:repo-b` }))
    expect(listener).not.toHaveBeenCalled()
    off()
  })

  it('ignores a storage event for the legacy unscoped key once a repoId is configured', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, repoId: 'repo-a' }
    const listener = vi.fn()
    const off = subscribe(listener)

    window.dispatchEvent(new StorageEvent('storage', { key: STORAGE_KEY }))
    expect(listener).not.toHaveBeenCalled()
    off()
  })
})
