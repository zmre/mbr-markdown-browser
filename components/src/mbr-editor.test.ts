import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import './mbr-editor.js'

/**
 * Tests for the "e" keyboard shortcut guard in <mbr-editor>.
 *
 * Regression coverage for the shadow-DOM bug: typing "e" into an input inside
 * another component's shadow root (e.g. the <mbr-search> input) must NOT open
 * the editor. Document-level listeners see such events retargeted to the
 * shadow HOST, so the guard has to use composedPath (via mbr-keys'
 * isInputTarget), not document.activeElement.
 *
 * `window.frontmatter` is left without a `markdown_source`, so when the guard
 * passes, `_open()` returns early before any chunk import or network access.
 * That makes `event.defaultPrevented` the observable for "the shortcut fired"
 * without loading the real editor chunk.
 */
describe('MbrEditorElement keyboard shortcut', () => {
  let editor: HTMLElement

  beforeEach(() => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: true }
    // No markdown_source: _open() bails out immediately after the guard.
    window.frontmatter = { title: 'Test Note' }
    editor = document.createElement('mbr-editor')
    document.body.appendChild(editor)
  })

  afterEach(() => {
    editor.remove()
    document.body.innerHTML = ''
    window.__MBR_CONFIG__ = undefined
    window.frontmatter = undefined
  })

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

  it('triggers on "e" for a plain body target (guard passes, preventDefault called)', () => {
    const event = dispatchE(document.body)
    expect(event.defaultPrevented).toBe(true)
    // _open() early-returned (no markdown_source), so no loading state.
    expect((editor as any)._loading).toBe(false)
    expect((editor as any)._isOpen).toBe(false)
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
    ;(search as any)._isOpen = true

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
