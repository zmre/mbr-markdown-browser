import { describe, it, expect, afterEach } from 'vitest'
import { isInputTarget, isModalOpen } from './mbr-keys.js'

/**
 * Tests for the shared keyboard-guard helpers. `isInputTarget` must see the
 * TRUE event target through shadow-root retargeting (via composedPath), since
 * document-level listeners otherwise only see the shadow host. `isModalOpen`
 * reports whether any known modal/panel is open.
 */

/**
 * Dispatch a real composed keydown from `origin` and capture what
 * `isInputTarget` reports at the document level (where the components listen).
 * `composedPath()` is only populated during dispatch, so the check must run
 * inside a listener. Returns null if the event never reached the document,
 * so a broken propagation path fails the test instead of passing vacuously.
 */
function isInputTargetAtDocument(origin: Element): boolean | null {
  let result: boolean | null = null
  const listener = (e: Event) => {
    result = isInputTarget(e as KeyboardEvent)
  }
  document.addEventListener('keydown', listener)
  origin.dispatchEvent(new KeyboardEvent('keydown', { key: 'e', bubbles: true, composed: true }))
  document.removeEventListener('keydown', listener)
  return result
}

afterEach(() => {
  document.body.innerHTML = ''
})

describe('isInputTarget', () => {
  it('returns true for a light-DOM input', () => {
    const input = document.body.appendChild(document.createElement('input'))
    expect(isInputTargetAtDocument(input)).toBe(true)
  })

  it('returns true for a textarea', () => {
    const textarea = document.body.appendChild(document.createElement('textarea'))
    expect(isInputTargetAtDocument(textarea)).toBe(true)
  })

  it('returns true for a select', () => {
    const select = document.body.appendChild(document.createElement('select'))
    expect(isInputTargetAtDocument(select)).toBe(true)
  })

  it('returns true for a contenteditable element', () => {
    const div = document.body.appendChild(document.createElement('div'))
    div.setAttribute('contenteditable', 'true')
    expect(isInputTargetAtDocument(div)).toBe(true)
  })

  it('returns false for a plain div', () => {
    const div = document.body.appendChild(document.createElement('div'))
    expect(isInputTargetAtDocument(div)).toBe(false)
  })

  it('returns true for an input inside a shadow root (the retargeting case)', () => {
    const host = document.body.appendChild(document.createElement('div'))
    const shadow = host.attachShadow({ mode: 'open' })
    const input = shadow.appendChild(document.createElement('input'))
    // At the document level the event is retargeted to `host`; only
    // composedPath still reveals the inner input.
    expect(isInputTargetAtDocument(input)).toBe(true)
  })
})

describe('isModalOpen', () => {
  it('returns false with no modal elements present', () => {
    expect(isModalOpen()).toBe(false)
  })

  it('detects an open mbr-search modal via its _isOpen state', () => {
    const search = document.body.appendChild(document.createElement('mbr-search'))
    ;(search as any)._isOpen = false
    expect(isModalOpen()).toBe(false)
    ;(search as any)._isOpen = true
    expect(isModalOpen()).toBe(true)
  })

  it('detects an open mbr-browse panel', () => {
    const browse = document.body.appendChild(document.createElement('mbr-browse'))
    ;(browse as any)._isOpen = true
    expect(isModalOpen()).toBe(true)
  })

  it('detects an open mbr-browse-single drawer', () => {
    const single = document.body.appendChild(document.createElement('mbr-browse-single'))
    ;(single as any)._isDrawerOpen = true
    expect(isModalOpen()).toBe(true)
  })

  it('detects an open mbr-fuzzy-nav modal', () => {
    const fuzzy = document.body.appendChild(document.createElement('mbr-fuzzy-nav'))
    ;(fuzzy as any)._isOpen = true
    expect(isModalOpen()).toBe(true)
  })

  it('detects an open info panel via its toggle checkbox', () => {
    const checkbox = document.body.appendChild(document.createElement('input'))
    checkbox.type = 'checkbox'
    checkbox.id = 'info-panel-toggle'
    expect(isModalOpen()).toBe(false)
    checkbox.checked = true
    expect(isModalOpen()).toBe(true)
  })
})
