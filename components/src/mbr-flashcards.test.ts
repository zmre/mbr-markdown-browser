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

beforeEach(() => {
  setFlashcardsChunkImporter(() => Promise.resolve({}))
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

  it('renders the button on a flashcard page and collapses histories', async () => {
    installDeckPage()
    await mount()
    expect(button()?.textContent).toContain('Review flashcards')
    const summaries = document.querySelectorAll('dd.mbr-fc-history summary')
    expect(summaries).toHaveLength(2)
    expect(summaries[0].textContent).toMatch(/^Reviewed 2×/)
  })

  it('renders nothing on a flashcard page without a top-level deck', async () => {
    installDeckPage('<main id="wrapper"><blockquote><dl><dt>a</dt><dd>b</dd></dl></blockquote></main>')
    await mount()
    expect(button()).toBeNull()
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
      srsAvailable: boolean
    }
    expect(deck.root).toBe(document.querySelector('main#wrapper'))
    expect(typeof deck.recordReview).toBe('function')
    expect(deck.srsAvailable).toBe(true)

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
    expect((document.querySelector('mbr-flashcard-deck') as unknown as { srsAvailable: boolean }).srsAvailable).toBe(false)
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
})
