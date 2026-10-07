import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { setFlashcardsChunkImporter, type MbrFlashcardsElement } from './mbr-flashcards.js'
import { installDeckPage } from './flashcards/test-fixtures.js'

let trigger: MbrFlashcardsElement

async function mount(): Promise<MbrFlashcardsElement> {
  trigger = document.createElement('mbr-flashcards')
  document.body.append(trigger)
  // waitForDom() resolves on a microtask; the render follows.
  await new Promise((resolve) => setTimeout(resolve, 0))
  await trigger.updateComplete
  return trigger
}

const button = () => trigger.shadowRoot!.querySelector('button')

function pressP(init: KeyboardEventInit = {}, target: EventTarget = document) {
  target.dispatchEvent(new KeyboardEvent('keydown', { key: 'p', bubbles: true, composed: true, cancelable: true, ...init }))
}

/** What the deck chunk's writer factory was handed, by the last open. */
let services: unknown = null
/** What the stub writer answers. */
let writeReply: unknown = { ok: false, kind: 'other', message: 'stub' }

/**
 * The one importer seam, answering per file: the real reading entry (cheap and
 * pure), and a stand-in for the deck chunk that records the injected services.
 * `log` collects every file asked for.
 */
function stubChunks(log: string[] = []) {
  return (file: string): Promise<unknown> => {
    log.push(file)
    if (file === 'mbr-flashcards-reading.min.js') return import('./flashcards/reading.js')
    return Promise.resolve({
      makeReviewRecorder: (s: unknown) => {
        services = s
        return () => Promise.resolve(writeReply)
      },
    })
  }
}

beforeEach(() => {
  services = null
  writeReply = { ok: false, kind: 'other', message: 'stub' }
  setFlashcardsChunkImporter(stubChunks())
  window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
})

afterEach(() => {
  trigger?.remove()
  document.querySelectorAll('mbr-flashcard-deck').forEach((el) => el.remove())
  document.body.className = ''
  document.body.innerHTML = ''
  window.__MBR_CONFIG__ = undefined
})

describe('<mbr-flashcards>', () => {
  it('renders nothing on an ordinary page', async () => {
    document.body.innerHTML = '<main id="wrapper"><dl><dt>Term</dt><dd>Def</dd></dl></main>'
    await mount()
    expect(button()).toBeNull()
    pressP()
    expect(trigger.isOpen).toBe(false)
  })

  it('renders the button on a flashcard page and collapses histories at idle', async () => {
    installDeckPage()
    await mount()
    expect(button()?.textContent).toContain('Review flashcards')
    await vi.waitFor(() => expect(document.querySelectorAll('dd.mbr-fc-history summary')).toHaveLength(2))
    const summaries = document.querySelectorAll('dd.mbr-fc-history summary')
    expect(summaries[0].textContent).toMatch(/^Reviewed 2×/)
  })

  it('renders nothing, and loads no chunk, on a flashcard page without a definition list', async () => {
    const log: string[] = []
    setFlashcardsChunkImporter(stubChunks(log))
    installDeckPage('<main id="wrapper"><p>No cards here.</p></main>')
    await mount()
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(button()).toBeNull()
    expect(log).toEqual([])
  })

  it('loads no chunk at all on an ordinary page', async () => {
    const log: string[] = []
    setFlashcardsChunkImporter(stubChunks(log))
    document.body.innerHTML = '<main id="wrapper"><dl><dt>Term</dt><dd>Def</dd></dl></main>'
    await mount()
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(log).toEqual([])
  })

  it('opens on p, appending the deck with its services', async () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: true }
    installDeckPage()
    window.frontmatter = { markdown_source: 'deck.md' }
    await mount()
    pressP()
    expect(trigger.isOpen).toBe(true)
    await vi.waitFor(() => expect(document.querySelector('mbr-flashcard-deck')).not.toBeNull())
    const deck = document.querySelector('mbr-flashcard-deck') as unknown as {
      root: ParentNode
      recordReview: unknown
    }
    expect(deck.root).toBe(document.querySelector('main#wrapper'))
    expect(typeof deck.recordReview).toBe('function')
    // The writer was built from this bundle's own state.
    expect(services).toMatchObject({ path: 'deck.md' })
    expect(typeof (services as { selfWrite: unknown }).selfWrite).toBe('function')

    // The deck asks to be closed; the trigger removes it.
    ;(deck as unknown as HTMLElement).dispatchEvent(new CustomEvent('mbr-flashcards-close'))
    expect(trigger.isOpen).toBe(false)
    expect(document.querySelector('mbr-flashcard-deck')).toBeNull()
    window.frontmatter = undefined
  })

  it('offers no spaced repetition without editing', async () => {
    installDeckPage()
    await mount()
    trigger.open()
    await vi.waitFor(() => expect(document.querySelector('mbr-flashcard-deck')).not.toBeNull())
    expect((document.querySelector('mbr-flashcard-deck') as unknown as { recordReview: unknown }).recordReview).toBeNull()
    expect(services).toBeNull()
  })

  it('shares the slides guards: not with a modifier, not while typing', async () => {
    installDeckPage()
    document.body.insertAdjacentHTML('beforeend', '<input id="field">')
    await mount()
    pressP({ metaKey: true })
    pressP({ ctrlKey: true })
    pressP({}, document.getElementById('field')!)
    expect(trigger.isOpen).toBe(false)
  })

  it('leaves p to the slides on a page that is both', async () => {
    installDeckPage()
    document.body.classList.add('slides')
    await mount()
    pressP()
    expect(trigger.isOpen).toBe(false)
  })

  it('stays closed when the chunk fails to load', async () => {
    setFlashcardsChunkImporter(() => Promise.reject(new Error('offline')))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    installDeckPage()
    await mount()
    trigger.open()
    await vi.waitFor(() => expect(trigger.isOpen).toBe(false))
    expect(document.querySelector('mbr-flashcard-deck')).toBeNull()
    warn.mockRestore()
  })

  it('draws the progress indicators at idle by default', async () => {
    installDeckPage()
    await mount()
    await vi.waitFor(() => expect(document.querySelector('dt.mbr-fc-last-easy')).not.toBeNull())
  })

  it('skips the indicators, but not the summaries, when the option is off', async () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, flashcardsProgressIndicators: false }
    installDeckPage()
    await mount()
    await vi.waitFor(() => expect(document.querySelectorAll('dd.mbr-fc-history summary')).toHaveLength(2))
    expect(document.querySelector('[class*="mbr-fc-last-"]')).toBeNull()
  })

  it('hands the configured Concentric threshold to the deck', async () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, flashcardsConcentricThreshold: 0.85 }
    installDeckPage()
    await mount()
    trigger.open()
    await vi.waitFor(() => expect(document.querySelector('mbr-flashcard-deck')).not.toBeNull())
    expect((document.querySelector('mbr-flashcard-deck') as unknown as { concentricThreshold: number }).concentricThreshold).toBe(0.85)
  })

  it('redraws the indicators after a session that wrote a review', async () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: true }
    writeReply = { ok: true, entry: '2026-10-20 10:00 - Good', line: 10, insertedAt: 10, insertedCount: 1 }
    installDeckPage()
    window.frontmatter = { markdown_source: 'deck.md' }
    await mount()
    await vi.waitFor(() => expect(document.querySelector('dt.mbr-fc-last-easy')).not.toBeNull())
    trigger.open()
    await vi.waitFor(() => expect(document.querySelector('mbr-flashcard-deck')).not.toBeNull())
    const deck = document.querySelector('mbr-flashcard-deck') as unknown as {
      recordReview: (t: unknown) => Promise<unknown>
    }
    // The deck appends the entry to the page itself; stand in for that here.
    document.querySelector('dd[data-mbr-line="7"] ul')!.insertAdjacentHTML('beforeend', '<li>2026-12-20 10:00 - Again</li>')
    await deck.recordReview({ line: 5, rating: 'again' })
    trigger.close()
    await vi.waitFor(() => expect(document.querySelector('dt.mbr-fc-last-again')).not.toBeNull())
    window.frontmatter = undefined
  })
})
