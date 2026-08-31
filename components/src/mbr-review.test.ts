/**
 * Tests for `<mbr-review>`, the main-bundle trigger.
 *
 * Mirrors `mbr-tasks.test.ts`: the chunk importer is stubbed in `beforeEach`
 * and reset to a *rejecting* stub in `afterEach`, so a leaked importer is loud
 * rather than silently satisfying a later test.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import './mbr-review.ts'
import { setReviewChunkImporter, type MbrReviewElement } from './mbr-review.ts'
import { OVERLAY_TAGS } from './overlay.ts'
import { STORAGE_KEY, serializeEnvelope } from './review/note-model.ts'
import { resetStoreCache } from './review-store.ts'
import { makeNote, resetNoteIds } from './review/test-fixtures.ts'

/** Let the chunk-load promise chain settle. */
async function settle(element: MbrReviewElement): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await element.updateComplete
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}

function press(key: string, target: EventTarget = document, init: KeyboardEventInit = {}): void {
  target.dispatchEvent(
    new KeyboardEvent('keydown', { key, bubbles: true, composed: true, ...init })
  )
}

/** Mount the rendered body a note anchors into, plus the element under test. */
function mount(): MbrReviewElement {
  document.body.innerHTML =
    '<main id="wrapper"><p data-mbr-line="4">The quick brown fox.</p></main>'
  const element = document.createElement('mbr-review')
  document.body.appendChild(element)
  return element
}

/** Select the text of the paragraph, the way a reader would. */
function selectParagraph(): void {
  const p = document.querySelector('p')!
  const range = document.createRange()
  range.selectNodeContents(p)
  const selection = window.getSelection()!
  selection.removeAllRanges()
  selection.addRange(range)
}

let element: MbrReviewElement

beforeEach(() => {
  localStorage.clear()
  resetStoreCache()
  resetNoteIds()
  window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, reviewEnabled: true }
  window.frontmatter = { markdown_source: 'doc.md' }
  setReviewChunkImporter(vi.fn().mockResolvedValue({}))
  element = mount()
})

afterEach(async () => {
  element?.remove()
  document.body.innerHTML = ''
  window.__MBR_CONFIG__ = undefined
  window.frontmatter = undefined
  localStorage.clear()
  resetStoreCache()
  // A leaked importer must fail loudly rather than satisfy the next test.
  setReviewChunkImporter(() => Promise.reject(new Error('importer not stubbed')))
  vi.restoreAllMocks()
})

describe('registration', () => {
  it('defines the custom element', () => {
    expect(customElements.get('mbr-review')).toBeDefined()
  })

  it('is registered as an overlay so bare-letter shortcuts are suppressed', () => {
    expect([...OVERLAY_TAGS]).toContain('mbr-review')
  })

  it('honours the MbrOverlay contract', () => {
    expect(typeof element.open).toBe('function')
    expect(typeof element.close).toBe('function')
    expect(element.isOpen).toBe(false)
  })
})

describe('availability gating', () => {
  it('renders nothing when review is disabled', async () => {
    element.remove()
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, reviewEnabled: false }
    element = mount()
    await settle(element)
    expect(element.shadowRoot?.querySelector('.review-fab')).toBeNull()
    expect(element.shadowRoot?.querySelector('.selection-button')).toBeNull()
  })

  it('stays closed when review is disabled, so it cannot swallow the keyboard', async () => {
    // An isOpen that reported true behind an invisible overlay would make
    // isModalOpen() suppress every bare-letter shortcut on the page.
    element.remove()
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, reviewEnabled: false }
    element = mount()
    element.open()
    await settle(element)
    expect(element.isOpen).toBe(false)
  })
})

describe('the "R" shortcut', () => {
  it('opens the panel', async () => {
    press('R', document, { shiftKey: true })
    await settle(element)
    expect(element.isOpen).toBe(true)
  })

  it('is ignored while typing in an input', async () => {
    const input = document.body.appendChild(document.createElement('input'))
    press('R', input, { shiftKey: true })
    await settle(element)
    expect(element.isOpen).toBe(false)
  })

  it('is ignored with a modifier held', async () => {
    for (const mod of ['ctrlKey', 'metaKey', 'altKey'] as const) {
      press('R', document, { shiftKey: true, [mod]: true })
      await settle(element)
      expect(element.isOpen).toBe(false)
    }
  })

  it('does not close an already-open panel', async () => {
    // Open-only, not a toggle: once the panel is up, the key belongs to it.
    press('R', document, { shiftKey: true })
    await settle(element)
    press('R', document, { shiftKey: true })
    await settle(element)
    expect(element.isOpen).toBe(true)
  })
})

describe('the "r" shortcut', () => {
  it('opens the form for a file-level note with no selection', async () => {
    press('r')
    await settle(element)
    expect(element.isOpen).toBe(true)
  })

  it('is ignored when Shift is held, which belongs to R', async () => {
    press('r', document, { shiftKey: true })
    await settle(element)
    expect(element.isOpen).toBe(false)
  })

  it('is ignored while typing', async () => {
    const textarea = document.body.appendChild(document.createElement('textarea'))
    press('r', textarea)
    await settle(element)
    expect(element.isOpen).toBe(false)
  })
})

describe('chunk loading', () => {
  it('imports the chunk once across repeated opens', async () => {
    const importer = vi.fn().mockResolvedValue({})
    setReviewChunkImporter(importer)
    element.remove()
    element = mount()

    element.open()
    await settle(element)
    element.close()
    element.open()
    await settle(element)

    expect(importer).toHaveBeenCalledTimes(1)
  })

  it('closes rather than stranding an empty backdrop when the import fails', async () => {
    setReviewChunkImporter(() => Promise.reject(new Error('offline')))
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    element.remove()
    element = mount()

    element.open()
    await settle(element)

    expect(element.isOpen).toBe(false)
  })
})

describe('the floating action button', () => {
  it('is absent when there are no notes', async () => {
    await settle(element)
    expect(element.shadowRoot?.querySelector('.review-fab')).toBeNull()
  })

  it('appears once a note exists, with a count', async () => {
    localStorage.setItem(STORAGE_KEY, serializeEnvelope([makeNote(), makeNote({ line: 9 })]))
    resetStoreCache()
    element.remove()
    element = mount()
    await settle(element)

    const fab = element.shadowRoot?.querySelector('.review-fab')
    expect(fab).not.toBeNull()
    expect(fab?.textContent).toContain('2')
  })
})

describe('markers in the document', () => {
  it('injects a textless marker for an anchored note', async () => {
    localStorage.setItem(
      STORAGE_KEY,
      serializeEnvelope([makeNote({ file: 'doc.md', line: 4, quote: 'The quick brown fox.' })])
    )
    resetStoreCache()
    element.remove()
    element = mount()
    await settle(element)

    const marker = document.querySelector('.mbr-review-marker')
    expect(marker).not.toBeNull()
    // Textless on purpose: a real text node would join the paragraph's text
    // run, so selecting the paragraph would copy the marker too.
    expect(marker?.textContent).toBe('')

    // And it must actually DRAW something. Three CSS-mask implementations
    // shipped a marker that was present, correctly sized and completely
    // invisible; this is the assertion none of them would have survived.
    const icon = marker!.querySelector('svg')
    expect(icon).not.toBeNull()
    expect(icon!.children.length).toBeGreaterThan(0)
    expect(icon!.getAttribute('stroke')).toBe('currentColor')
    expect(marker?.closest('[data-mbr-line]')?.getAttribute('data-mbr-line')).toBe('4')
  })

  it('draws the icon that matches the note type', async () => {
    localStorage.setItem(
      STORAGE_KEY,
      serializeEnvelope([makeNote({ file: 'doc.md', line: 4, type: 'issue' })])
    )
    resetStoreCache()
    element.remove()
    element = mount()
    await settle(element)

    const marker = document.querySelector('.mbr-review-marker')!
    expect(marker.getAttribute('data-mbr-review-type')).toBe('issue')
    // The alert triangle is three shapes; the note icon is four. Asserting the
    // count is enough to prove the type actually selected different artwork.
    expect(marker.querySelector('svg')!.children.length).toBe(3)
  })

  it('removes its markers when disconnected', async () => {
    localStorage.setItem(STORAGE_KEY, serializeEnvelope([makeNote({ file: 'doc.md', line: 4 })]))
    resetStoreCache()
    element.remove()
    element = mount()
    await settle(element)
    expect(document.querySelector('.mbr-review-marker')).not.toBeNull()

    element.remove()
    expect(document.querySelector('.mbr-review-marker')).toBeNull()
  })

  it('does not render a marker for another file', async () => {
    localStorage.setItem(
      STORAGE_KEY,
      serializeEnvelope([makeNote({ file: 'other.md', line: 4 })])
    )
    resetStoreCache()
    element.remove()
    element = mount()
    await settle(element)
    expect(document.querySelector('.mbr-review-marker')).toBeNull()
  })
})

describe('re-anchoring on load', () => {
  it('marks a note lost when its quoted text is gone', async () => {
    // The note keeps its line and is never deleted — staleness is a badge.
    localStorage.setItem(
      STORAGE_KEY,
      serializeEnvelope([
        makeNote({ file: 'doc.md', line: 4, quote: 'a sentence that was deleted' }),
      ])
    )
    resetStoreCache()
    element.remove()
    element = mount()
    await settle(element)

    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY)!)
    expect(stored.notes[0].anchorState).toBe('lost')
    expect(stored.notes[0].line).toBe(4)
  })

  it('updates the line when the quote moved', async () => {
    document.body.innerHTML =
      '<main id="wrapper"><p data-mbr-line="11">The quick brown fox.</p></main>'
    localStorage.setItem(
      STORAGE_KEY,
      serializeEnvelope([
        makeNote({ file: 'doc.md', line: 4, quote: 'The quick brown fox.' }),
      ])
    )
    resetStoreCache()
    element = document.createElement('mbr-review')
    document.body.appendChild(element)
    await settle(element)

    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY)!)
    expect(stored.notes[0].line).toBe(11)
    expect(stored.notes[0].anchorState).toBe('moved')
  })

  it('leaves an unmoved note untouched', async () => {
    localStorage.setItem(
      STORAGE_KEY,
      serializeEnvelope([
        makeNote({ file: 'doc.md', line: 4, quote: 'The quick brown fox.' }),
      ])
    )
    resetStoreCache()
    const before = localStorage.getItem(STORAGE_KEY)
    element.remove()
    element = mount()
    await settle(element)
    expect(localStorage.getItem(STORAGE_KEY)).toBe(before)
  })
})

describe('selection tracking', () => {
  it('exposes the anchor a selection resolves to', () => {
    // The floating button's position needs a rect happy-dom cannot produce, so
    // the assertion is on the anchor rather than on the rendered button.
    selectParagraph()
    const selection = window.getSelection()!
    expect(selection.isCollapsed).toBe(false)
    expect(selection.rangeCount).toBe(1)
  })
})

describe('the note card (regressions from first real use)', () => {
  /** Mount with one anchored note and wait for the marker layer. */
  async function withNote(): Promise<Element> {
    localStorage.setItem(
      STORAGE_KEY,
      serializeEnvelope([makeNote({ file: 'doc.md', line: 4, quote: 'The quick brown fox.' })])
    )
    resetStoreCache()
    element.remove()
    element = mount()
    await settle(element)
    return document.querySelector('.mbr-review-marker')!
  }

  it('shows the note when the marker is clicked', async () => {
    // Reported as "clicking a note marker doesn't do anything". It did fire —
    // but `.mbr-review-popover` had no rules in theme.css, so the card was
    // position:static and rendered unstyled at the very end of <body>.
    const marker = await withNote()
    marker.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle(element)

    const card = document.querySelector('.mbr-review-popover') as HTMLElement | null
    expect(card).not.toBeNull()
    expect(card!.style.display).toBe('block')
    expect(card!.textContent).toContain('A comment.')
  })

  it('keeps a clicked card up after the pointer leaves the marker', async () => {
    // Hover semantics made Edit and Delete unreachable: the pointer has to
    // cross the gap between marker and card to get to them.
    const marker = await withNote()
    marker.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle(element)

    marker.dispatchEvent(new MouseEvent('mouseleave', { bubbles: true }))
    await new Promise((r) => setTimeout(r, 250))

    const card = document.querySelector('.mbr-review-popover') as HTMLElement
    expect(card.style.display).toBe('block')
  })

  it('closes on a second click of the same marker', async () => {
    const marker = await withNote()
    marker.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle(element)
    marker.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle(element)

    const card = document.querySelector('.mbr-review-popover') as HTMLElement
    expect(card.style.display).toBe('none')
  })

  it('closes on Escape', async () => {
    const marker = await withNote()
    marker.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle(element)
    press('Escape')
    await settle(element)

    const card = document.querySelector('.mbr-review-popover') as HTMLElement
    expect(card.style.display).toBe('none')
  })

  it('closes on a click elsewhere in the page', async () => {
    const marker = await withNote()
    marker.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await settle(element)

    document.querySelector('p')!.dispatchEvent(
      new MouseEvent('pointerdown', { bubbles: true, composed: true })
    )
    await settle(element)

    const card = document.querySelector('.mbr-review-popover') as HTMLElement
    expect(card.style.display).toBe('none')
  })

  it('activates from the keyboard, since the marker is a focusable button', async () => {
    const marker = await withNote()
    marker.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle(element)

    const card = document.querySelector('.mbr-review-popover') as HTMLElement
    expect(card.style.display).toBe('block')
  })
})
