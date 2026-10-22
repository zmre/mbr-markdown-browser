import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import './mbr-editor.js'
import {
  EDITOR_STATE_EVENT,
  newNoteFolder,
  setEditorChunkImporter,
  type MbrEditorElement,
} from './mbr-editor.js'
import {
  isOwnWriteEcho,
  mayBeOwnWriteEcho,
  resetTaskToggleState,
  wasSelfWrite,
} from './task-toggle.js'
// Type-only, so the Milkdown-laden chunk is never actually loaded here.
import type { CreateNoteOptions, OpenEditorOptions } from './editor-crepe.js'
// Registers <mbr-search> so the modal-open test below drives a REAL element
// through its public MbrOverlay contract; an un-upgraded element reports
// nothing to isModalOpen().
import './mbr-search.js'

/**
 * Tests for the "e" keyboard shortcut guard in <mbr-editor>.
 *
 * Regression coverage for the shadow-DOM bug: typing "e" into an input inside
 * another component's shadow root (e.g. the <mbr-search> input) must NOT open
 * the editor. Document-level listeners see such events retargeted to the
 * shadow HOST, so the guard has to use composedPath (via mbr-keys'
 * isInputTarget), not document.activeElement.
 *
 * The chunk importer is stubbed (happy-dom cannot import the real chunk), so
 * `event.defaultPrevented` plus the stub's call count are the observables for
 * "the shortcut fired".
 */

/** A stand-in chunk recording what the trigger asked of it. */
interface StubChunk {
  opened: OpenEditorOptions[]
  created: CreateNoteOptions[]
}

function stubChunk(behaviour: { ready?: boolean } = {}): StubChunk {
  const chunk: StubChunk = { opened: [], created: [] }
  setEditorChunkImporter(async () => ({
    openEditor: async (options: OpenEditorOptions) => {
      chunk.opened.push(options)
      if (behaviour.ready !== false) options.onReady?.()
    },
    createNote: async (options: CreateNoteOptions) => {
      chunk.created.push(options)
    },
  }))
  return chunk
}

/** Lets the importer promise and the chunk call settle. */
const flush = () => new Promise((resolve) => setTimeout(resolve, 0))

function setPage(opts: { editEnabled: boolean; source?: string }) {
  window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: opts.editEnabled }
  window.frontmatter = opts.source
    ? { title: 'Test Note', markdown_source: opts.source }
    : { title: 'Listing' }
}

function mount(): MbrEditorElement {
  const el = document.createElement('mbr-editor')
  document.body.appendChild(el)
  return el
}

function cleanup() {
  document.body.innerHTML = ''
  window.__MBR_CONFIG__ = undefined
  window.frontmatter = undefined
  history.replaceState(null, '', '/')
}

describe('MbrEditorElement keyboard shortcut', () => {
  let editor: HTMLElement
  let chunk: StubChunk

  beforeEach(() => {
    setPage({ editEnabled: true, source: 'notes/test.md' })
    // Not ready: the stub never reports the editor on screen, so `_loading`
    // is only ever set by a shortcut that actually fired.
    chunk = stubChunk({ ready: false })
    editor = mount()
  })

  afterEach(cleanup)

  function dispatchE(origin: Element): KeyboardEvent {
    const event = new KeyboardEvent('keydown', {
      key: 'e',
      bubbles: true,
      composed: true,
      cancelable: true,
    })
    origin.dispatchEvent(event)
    return event
  }

  it('triggers on "e" for a plain body target (guard passes, preventDefault called)', async () => {
    const event = dispatchE(document.body)
    expect(event.defaultPrevented).toBe(true)
    await flush()
    expect(chunk.opened).toHaveLength(1)
    expect(chunk.opened[0].filePath).toBe('notes/test.md')
  })

  it("hands the chunk the page's tag sources for the properties editor", async () => {
    window.__MBR_CONFIG__!.tagSources = [
      { field: 'taxonomy.performers', urlSource: 'performers', label: 'Performer', labelPlural: 'Performers' },
    ]
    dispatchE(document.body)
    await flush()
    expect(chunk.opened[0].tagSources).toEqual([
      { field: 'taxonomy.performers', label: 'Performer', label_plural: 'Performers' },
    ])
  })

  it('leaves "e" alone on a page with no source file (a folder listing)', async () => {
    setPage({ editEnabled: true })
    const event = dispatchE(document.body)
    expect(event.defaultPrevented).toBe(false)
    await flush()
    expect(chunk.opened).toHaveLength(0)
  })

  it('does not trigger when "e" is typed in an input inside a shadow root', () => {
    const host = document.body.appendChild(document.createElement('div'))
    const shadow = host.attachShadow({ mode: 'open' })
    const input = shadow.appendChild(document.createElement('input'))

    // Sanity: the composed event must actually reach the document listener,
    // otherwise this test would pass vacuously.
    let reachedDocument = false
    const probe = () => {
      reachedDocument = true
    }
    document.addEventListener('keydown', probe)
    const event = dispatchE(input)
    document.removeEventListener('keydown', probe)

    expect(reachedDocument).toBe(true)
    expect(event.defaultPrevented).toBe(false)
    expect((editor as any)._loading).toBe(false)
    expect((editor as any)._isOpen).toBe(false)
  })

  it('does not trigger when "e" is typed in a light-DOM input', () => {
    const input = document.body.appendChild(document.createElement('input'))
    const event = dispatchE(input)
    expect(event.defaultPrevented).toBe(false)
  })

  it('does not trigger while a search modal is open (even with focus outside its input)', () => {
    const search = document.body.appendChild(document.createElement('mbr-search'))
    search.open()
    expect(search.isOpen).toBe(true)

    // Target is the body (e.g. arrow-keying through results), not an input:
    // the modal check alone must block the shortcut.
    const event = dispatchE(document.body)
    expect(event.defaultPrevented).toBe(false)
    expect((editor as any)._loading).toBe(false)
  })

  it('does not trigger when editing is disabled', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: false }
    const event = dispatchE(document.body)
    expect(event.defaultPrevented).toBe(false)
  })

  it('does not trigger for modified "e" (Ctrl/Cmd/Alt)', () => {
    for (const modifier of ['ctrlKey', 'metaKey', 'altKey'] as const) {
      const event = new KeyboardEvent('keydown', {
        key: 'e',
        bubbles: true,
        cancelable: true,
        [modifier]: true,
      })
      document.body.dispatchEvent(event)
      expect(event.defaultPrevented).toBe(false)
    }
  })
})

describe('MbrEditorElement app-side contract', () => {
  beforeEach(() => {
    // `#mbr-edit` arrival primes the source cache through `/.mbr/raw`.
    vi.stubGlobal('fetch', vi.fn(async () => new Response('', { status: 404 })))
  })
  afterEach(() => {
    vi.unstubAllGlobals()
    resetTaskToggleState()
    cleanup()
  })

  it('canCreate follows editEnabled, with or without a source file', () => {
    setPage({ editEnabled: true })
    const el = mount()
    expect(el.canCreate).toBe(true)
    expect(el.canEdit).toBe(false)
    setPage({ editEnabled: false, source: 'a.md' })
    expect(el.canCreate).toBe(false)
    expect(el.canEdit).toBe(false)
  })

  it('openNew hands the chunk the note page folder, the token and the auth hook', async () => {
    const chunk = stubChunk()
    setPage({ editEnabled: true, source: 'projects/acme/plan.md' })
    const el = mount()
    el.openNew()
    await flush()
    expect(chunk.created).toHaveLength(1)
    expect(chunk.created[0].folder).toBe('projects/acme')
    expect(chunk.created[0].token).toBe('')
    expect(typeof chunk.created[0].onAuthRequired).toBe('function')
    expect((el as any)._loading).toBe(false)
  })

  it('openNew on a folder listing uses the decoded URL path', async () => {
    const chunk = stubChunk()
    setPage({ editEnabled: true })
    history.replaceState(null, '', '/My%20Notes/2026/')
    const el = mount()
    el.openNew()
    await flush()
    expect(chunk.created[0].folder).toBe('My Notes/2026')
  })

  it("openNew at home uses the root ''", async () => {
    const chunk = stubChunk()
    setPage({ editEnabled: true })
    history.replaceState(null, '', '/')
    const el = mount()
    el.openNew()
    await flush()
    expect(chunk.created[0].folder).toBe('')
  })

  it('openNew is a no-op with editing off', async () => {
    const chunk = stubChunk()
    setPage({ editEnabled: false })
    mount().openNew()
    await flush()
    expect(chunk.created).toHaveLength(0)
  })

  it('newNoteFolder: a top-level note goes in the root', () => {
    expect(newNoteFolder('plan.md', '/plan/')).toBe('')
    expect(newNoteFolder(null, '/%E0%A4%A/')).toBe('%E0%A4%A')
  })

  it('#mbr-edit on arrival opens the editor and is stripped from the URL', async () => {
    const chunk = stubChunk()
    setPage({ editEnabled: true, source: 'new/idea.md' })
    history.replaceState(null, '', '/new/idea/?q=1#mbr-edit')
    mount()
    expect(location.hash).toBe('')
    expect(location.pathname + location.search).toBe('/new/idea/?q=1')
    await flush()
    expect(chunk.opened).toHaveLength(1)
  })

  it("#mbr-edit marks the new note as this page's own write, so the create's echo does not reload", async () => {
    stubChunk()
    resetTaskToggleState()
    const fetchMock = vi.fn(async () => new Response('# idea\n', { status: 200 }))
    vi.stubGlobal('fetch', fetchMock)
    try {
      setPage({ editEnabled: true, source: 'new/idea.md' })
      history.replaceState(null, '', '/new/idea/#mbr-edit')
      mount()
      // The prompt echo is inside the window.
      expect(wasSelfWrite('new/idea.md')).toBe(true)
      await flush()
      expect(fetchMock).toHaveBeenCalledWith('/.mbr/raw/new/idea.md', expect.anything())
      // A late echo is settled by content: the disk still matches.
      expect(mayBeOwnWriteEcho('new/idea.md')).toBe(true)
      await expect(isOwnWriteEcho('new/idea.md')).resolves.toBe(true)
      // A real change does not match, so it still reloads.
      fetchMock.mockImplementation(async () => new Response('# idea\n\nedited\n', { status: 200 }))
      vi.spyOn(Date, 'now').mockReturnValue(Date.now() + 60_000)
      await expect(isOwnWriteEcho('new/idea.md')).resolves.toBe(false)
    } finally {
      vi.restoreAllMocks()
      vi.unstubAllGlobals()
      resetTaskToggleState()
    }
  })

  it('#mbr-edit does nothing (and stays) where the page cannot be edited', async () => {
    const chunk = stubChunk()
    setPage({ editEnabled: false, source: 'new/idea.md' })
    history.replaceState(null, '', '/new/idea/#mbr-edit')
    mount()
    await flush()
    expect(chunk.opened).toHaveLength(0)
    expect(location.hash).toBe('#mbr-edit')
  })

  it('shows the pencil only when the page has a source file', async () => {
    setPage({ editEnabled: true })
    const listing = mount()
    await listing.updateComplete
    expect(listing.shadowRoot?.querySelector('.edit-trigger')).toBeNull()
    listing.remove()

    setPage({ editEnabled: true, source: 'a.md' })
    const note = mount()
    await note.updateComplete
    expect(note.shadowRoot?.querySelector('.edit-trigger')).not.toBeNull()
  })

  it('announces the editor opening and closing on document', async () => {
    const chunk = stubChunk()
    setPage({ editEnabled: true, source: 'a.md' })
    const states: boolean[] = []
    const listener = (e: Event) => states.push((e as CustomEvent<{ open: boolean }>).detail.open)
    document.addEventListener(EDITOR_STATE_EVENT, listener)
    try {
      const el = mount()
      el.open()
      await flush()
      expect(states).toEqual([true])
      chunk.opened[0].onClose()
      expect(states).toEqual([true, false])
      expect(el.canEdit).toBe(true)
    } finally {
      document.removeEventListener(EDITOR_STATE_EVENT, listener)
    }
  })

  it('onSaved reloads only when no live-reload socket can (non-http schemes)', async () => {
    const chunk = stubChunk()
    setPage({ editEnabled: true, source: 'a.md' })
    mount().open()
    await flush()
    const onSaved = chunk.opened[0].onSaved
    expect(onSaved).toBeTypeOf('function')

    const reload = vi.fn()
    const original = window.location
    const fake = (protocol: string) =>
      ({ ...original, protocol, reload, pathname: '/', search: '', hash: '' }) as Location
    try {
      Object.defineProperty(window, 'location', { value: fake('http:'), configurable: true })
      onSaved!()
      expect(reload).not.toHaveBeenCalled()

      Object.defineProperty(window, 'location', { value: fake('mbr:'), configurable: true })
      onSaved!()
      expect(reload).toHaveBeenCalledTimes(1)
    } finally {
      Object.defineProperty(window, 'location', { value: original, configurable: true })
    }
  })
})
