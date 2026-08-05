/**
 * The edit token must never become durable.
 *
 * mbr renders arbitrary markdown, and markdown may contain raw HTML, so
 * same-origin script execution is a live vector. A token in `sessionStorage`
 * outlives the page and turns "may write files while this page is open" into a
 * credential that can be exfiltrated and replayed later; a module variable dies
 * with the page. This file is the regression guard for that decision, in two
 * layers that fail for different reasons:
 *
 * 1. **A runtime cycle** — open the editor, have its footer publish a token,
 *    then toggle a task — with `localStorage` and `sessionStorage` fully
 *    instrumented. Nothing may be written, and nothing may be read.
 * 2. **A source scan** over every shipped module. The runtime test drives a
 *    *stub* of the editor chunk (the real one pulls in Milkdown), so only the
 *    scan can see a `sessionStorage` call reappearing inside `editor-crepe.ts`.
 *    The same scan pins the list of modules allowed to use `localStorage` at
 *    all, so a new durable store has to be justified here before it ships.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import {
  clearEditToken,
  editAuthHeaders,
  getEditToken,
  isEditTokenRequired,
  noteEditTokenRequired,
  setEditToken,
} from './edit-token.js'
import './mbr-editor.js'
import { setEditorChunkImporter } from './mbr-editor.js'
import { resetTaskToggleState, toggleTask } from './task-toggle.js'
// Type-only, so the Milkdown-laden chunk is never actually loaded here.
import type { OpenEditorOptions } from './editor-crepe.js'

const TOKEN = 'sup3r-s3cret-token'

// ============================================================================
// Storage instrumentation
// ============================================================================

/** Every storage method call made while a test ran, with its arguments. */
interface StorageLog {
  calls: Array<{ store: 'local' | 'session'; method: string; args: string[] }>
  /** Values actually held, so a write through some other API is still caught. */
  contents: Record<string, string>
}

/**
 * Replace both web storages with instrumented ones.
 *
 * `getItem` is watched as well as `setItem`: a module that reads a key is a
 * module that expects something to have written it, which is the same bug one
 * step earlier. happy-dom supplies a real `sessionStorage`, and `test-setup.ts`
 * defines `localStorage`, so both are overwritten rather than added.
 */
function instrumentStorage(): StorageLog {
  const log: StorageLog = { calls: [], contents: {} }

  const make = (store: 'local' | 'session'): Storage => {
    const backing: Record<string, string> = {}
    const record = (method: string, ...args: string[]) => {
      log.calls.push({ store, method, args })
    }
    return {
      get length() {
        return Object.keys(backing).length
      },
      key: (index: number) => Object.keys(backing)[index] ?? null,
      getItem: (key: string) => {
        record('getItem', key)
        return backing[key] ?? null
      },
      setItem: (key: string, value: string) => {
        record('setItem', key, value)
        backing[key] = value
        log.contents[`${store}:${key}`] = value
      },
      removeItem: (key: string) => {
        record('removeItem', key)
        delete backing[key]
        delete log.contents[`${store}:${key}`]
      },
      clear: () => {
        record('clear')
        for (const key of Object.keys(backing)) delete backing[key]
      },
    } as Storage
  }

  Object.defineProperty(globalThis, 'localStorage', {
    value: make('local'),
    configurable: true,
    writable: true,
  })
  Object.defineProperty(globalThis, 'sessionStorage', {
    value: make('session'),
    configurable: true,
    writable: true,
  })
  return log
}

// ============================================================================
// The in-memory store itself
// ============================================================================

describe('the in-memory edit token', () => {
  beforeEach(() => {
    clearEditToken()
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: true }
  })

  afterEach(() => {
    clearEditToken()
    window.__MBR_CONFIG__ = undefined
  })

  it('remembers a token for the page and puts it on a write request', () => {
    expect(getEditToken()).toBe('')
    expect(editAuthHeaders()).toEqual({ 'X-MBR-Edit': '1' })

    setEditToken(`  ${TOKEN}  `)

    // Trimmed, because the field it comes from is one a human types into.
    expect(getEditToken()).toBe(TOKEN)
    expect(editAuthHeaders({ 'Content-Type': 'application/json' })).toEqual({
      'X-MBR-Edit': '1',
      'Content-Type': 'application/json',
      Authorization: `Bearer ${TOKEN}`,
    })
  })

  it('forgets a token, and the fact that one was demanded, when editing is off', () => {
    setEditToken(TOKEN)
    noteEditTokenRequired()
    expect(isEditTokenRequired()).toBe(true)

    // `edit_enabled` is server-rendered per page, so this is what a navigation
    // to a page with editing off looks like from inside the module.
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: false }

    expect(getEditToken()).toBe('')
    expect(isEditTokenRequired()).toBe(false)
    expect(editAuthHeaders()).toEqual({ 'X-MBR-Edit': '1' })

    // And the value is gone, not merely hidden: turning editing back on (which
    // in reality means another navigation) does not resurrect it.
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: true }
    expect(getEditToken()).toBe('')
  })

  it('refuses to take a token at all on a page with editing off', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: false }
    setEditToken(TOKEN)
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: true }

    expect(getEditToken()).toBe('')
  })

  it('records that the server demanded a token, so the editor can show its field', () => {
    expect(isEditTokenRequired()).toBe(false)
    noteEditTokenRequired()
    expect(isEditTokenRequired()).toBe(true)
  })
})

// ============================================================================
// Layer 1: the full editor-open -> save -> toggle cycle
// ============================================================================

describe('a full editor-open, save and toggle cycle', () => {
  let log: StorageLog
  let fetchMock: ReturnType<typeof vi.fn>
  /** The options the trigger handed the chunk, captured by the stub. */
  let opened: OpenEditorOptions | null

  /**
   * Stand-in for `editor-crepe.ts`'s `openEditor`.
   *
   * It reproduces the only part of the real chunk this test is about: it takes
   * the token it was given and publishes whatever the user typed back through
   * `onToken`. Typed against the real `OpenEditorOptions`, so renaming either
   * half of that contract fails to compile here. The real chunk itself is not
   * importable under happy-dom (Milkdown), which is exactly why the source scan
   * below exists as well.
   */
  function stubChunk() {
    setEditorChunkImporter(async () => ({
      openEditor: async (options: OpenEditorOptions) => {
        opened = options
        options.onReady?.()
        // The user types the token into the footer field.
        options.onToken?.(TOKEN)
      },
    }))
  }

  beforeEach(() => {
    opened = null
    clearEditToken()
    resetTaskToggleState()
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: true }
    window.frontmatter = { markdown_source: 'notes.md' }
    fetchMock = vi.fn().mockImplementation((url: string) =>
      String(url).startsWith('/.mbr/raw/')
        ? Promise.resolve({ ok: true, status: 200, text: () => Promise.resolve('- [ ] a\n') })
        : Promise.resolve({
            ok: true,
            status: 200,
            json: () => Promise.resolve({ line: 1, text: '- [x] a @done(2026-08-04 22:16)' }),
          })
    )
    globalThis.fetch = fetchMock as unknown as typeof fetch
    stubChunk()
    log = instrumentStorage()
  })

  afterEach(() => {
    clearEditToken()
    resetTaskToggleState()
    document.body.innerHTML = ''
    window.__MBR_CONFIG__ = undefined
    window.frontmatter = undefined
    vi.restoreAllMocks()
  })

  it('has working instrumentation, so the assertions below are not vacuous', () => {
    // Everything in this block proves a negative. If `instrumentStorage` ever
    // failed to install itself, every one of those assertions would pass for
    // the wrong reason — so prove the spies are live first.
    window.localStorage.setItem('probe', 'x')
    window.sessionStorage.getItem('probe')

    expect(log.calls).toEqual([
      { store: 'local', method: 'setItem', args: ['probe', 'x'] },
      { store: 'session', method: 'getItem', args: ['probe'] },
    ])
    expect(log.contents).toEqual({ 'local:probe': 'x' })
  })

  it('carries the editor’s token to a task write without touching web storage', async () => {
    const editor = document.body.appendChild(document.createElement('mbr-editor'))
    // Open the editor: the trigger loads the chunk and hands it the plumbing.
    await (editor as unknown as { _open: () => Promise<void> })._open()

    // The chunk was given the page's (empty) token and a way to hand one back.
    expect(opened).toMatchObject({ token: '', tokenRequired: false })
    // ...and used it, so the page now knows the token in memory.
    expect(getEditToken()).toBe(TOKEN)

    const outcome = await toggleTask({ path: 'notes.md', line: 1, to: 'done' })
    expect(outcome).toEqual({ ok: true, text: '- [x] a @done(2026-08-04 22:16)' })

    // Both requests authenticated with the token the editor collected.
    expect(fetchMock.mock.calls).toHaveLength(2)
    for (const [, init] of fetchMock.mock.calls as Array<[string, RequestInit]>) {
      expect(init.headers).toMatchObject({ Authorization: `Bearer ${TOKEN}` })
    }

    // THE POINT OF THIS FILE: not one byte of that went to web storage, and
    // nothing on the path even looked.
    expect(log.calls).toEqual([])
    expect(log.contents).toEqual({})
  })

  it('leaves no token behind for a page loaded afterwards', async () => {
    const editor = document.body.appendChild(document.createElement('mbr-editor'))
    await (editor as unknown as { _open: () => Promise<void> })._open()
    expect(getEditToken()).toBe(TOKEN)

    // A reload is the end of the token's life. `clearEditToken` stands in for
    // the module being evaluated afresh, which is what really happens.
    clearEditToken()

    expect(getEditToken()).toBe('')
    expect(editAuthHeaders()).toEqual({ 'X-MBR-Edit': '1' })
    // Nothing was left anywhere for the new page to pick it back up from.
    expect(Object.values(log.contents)).not.toContain(TOKEN)
    expect(log.contents).toEqual({})
  })

  it('tells the editor to show its token field once a write has been refused', async () => {
    noteEditTokenRequired()

    const editor = document.body.appendChild(document.createElement('mbr-editor'))
    await (editor as unknown as { _open: () => Promise<void> })._open()

    expect(opened).toMatchObject({ tokenRequired: true })
    expect(log.calls).toEqual([])
  })
})

// ============================================================================
// Layer 2: the source scan
// ============================================================================

/**
 * The text of every `.ts` file under `src/`, keyed by `./`-relative path.
 *
 * Read through vite's `import.meta.glob` rather than `node:fs`, so the scan
 * needs no node typings and no assumption about the working directory — and so
 * that "every file vite would build" and "every file scanned here" are the same
 * list by construction.
 */
const ALL_SOURCES = import.meta.glob('./**/*.ts', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

/** Every shipped module: the sources above, minus test scaffolding. */
function shippedModules(): string[] {
  return Object.keys(ALL_SOURCES)
    .map((path) => path.replace(/^\.\//, ''))
    .filter(
      (path) =>
        !path.endsWith('.test.ts') &&
        !path.endsWith('.bench.ts') &&
        !path.endsWith('.d.ts') &&
        !path.endsWith('test-setup.ts') &&
        !path.endsWith('test-fixtures.ts')
    )
    .sort()
}

/**
 * Source with comments removed.
 *
 * The scan has to distinguish `sessionStorage.setItem(…)` from a doc comment
 * explaining why nobody calls it — this very file's neighbours are full of the
 * latter. String literals are tracked so that a `//` inside a URL does not eat
 * the rest of a line.
 */
function stripComments(source: string): string {
  let out = ''
  let quote: string | null = null
  for (let i = 0; i < source.length; ) {
    const ch = source[i]
    if (quote !== null) {
      if (ch === '\\') {
        out += ch + (source[i + 1] ?? '')
        i += 2
        continue
      }
      if (ch === quote) quote = null
      out += ch
      i++
      continue
    }
    if (ch === "'" || ch === '"' || ch === '`') {
      quote = ch
      out += ch
      i++
      continue
    }
    if (ch === '/' && source[i + 1] === '/') {
      while (i < source.length && source[i] !== '\n') i++
      continue
    }
    if (ch === '/' && source[i + 1] === '*') {
      i += 2
      while (i < source.length && !(source[i] === '*' && source[i + 1] === '/')) i++
      i += 2
      continue
    }
    out += ch
    i++
  }
  return out
}

/** Shipped modules whose *code* names `identifier`. */
function modulesUsing(identifier: string): string[] {
  return shippedModules().filter((file) =>
    stripComments(ALL_SOURCES[`./${file}`] ?? '').includes(identifier)
  )
}

describe('durable web storage across the whole frontend', () => {
  it('is not used for the edit token, in any bundle', () => {
    // Deliberately a source scan and not a spy: the editor chunk cannot be
    // imported under happy-dom, so a per-tab store reintroduced inside
    // `editor-crepe.ts` would slip past every runtime test in the project.
    // Nothing shipped has any business using that API at all.
    expect(modulesUsing('sessionStorage')).toEqual([])
  })

  it('is used only by the features that were reviewed and found to hold nothing secret', () => {
    // Each of these stores something the reader could get off the page anyway,
    // and none of them stores a credential:
    //   mbr-browse.ts        — recently viewed pages and pinned shortcuts (URLs)
    //   mbr-video-extras.ts  — playback position per video, in seconds
    //   genealogy/selector.ts — which genealogy chart the reader last picked
    // Adding a file here means arguing that its contents are not sensitive.
    expect(modulesUsing('localStorage')).toEqual([
      'genealogy/selector.ts',
      'mbr-browse.ts',
      'mbr-video-extras.ts',
    ])
  })

  it('scans a plausible number of modules, so a broken walk cannot pass vacuously', () => {
    const modules = shippedModules()
    expect(modules.length).toBeGreaterThan(40)
    expect(modules).toContain('edit-token.ts')
    expect(modules).toContain('editor-crepe.ts')
    expect(modules).toContain('task-toggle.ts')
  })
})
