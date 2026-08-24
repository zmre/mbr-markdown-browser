import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import './mbr-find-bar.js'
import type { MbrFindBarElement } from './mbr-find-bar.js'

/**
 * `<mbr-find-bar>` is driven entirely from outside: the native Edit menu built
 * in `src/browser.rs` calls `open()`, `findNext()` and `findPrevious()` through
 * `evaluate_script`, from Rust STRING LITERALS. A TypeScript rename therefore
 * cannot fail at compile time — the first test below is the only thing standing
 * between one and a silently dead menu item.
 */

/** Stand-in for the Custom Highlight API, which happy-dom does not implement. */
class FakeHighlight extends Set<AbstractRange> {}

const PAGE = `
  <span class="sr-only" data-pagefind-weight="10">Guide</span>
  <h1>Guide</h1>
  <p>alpha beta alpha</p>
  <p>gamma alpha</p>
`

let bar: MbrFindBarElement
let registry: Map<string, FakeHighlight>

/** Type into the find input and let the trailing debounce fire. */
async function type(text: string): Promise<void> {
  const input = bar.shadowRoot!.querySelector('#find-input') as HTMLInputElement
  input.value = text
  input.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
  await vi.advanceTimersByTimeAsync(200)
  await bar.updateComplete
}

/** Dispatch a keydown from the find input, the way a reader would. */
async function press(key: string, init: KeyboardEventInit = {}): Promise<void> {
  const input = bar.shadowRoot!.querySelector('#find-input') as HTMLInputElement
  input.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, composed: true, cancelable: true, ...init }))
  await bar.updateComplete
}

/** The "N of M" status the bar is currently showing. */
function status(): string {
  return bar.shadowRoot?.querySelector('.status')?.textContent?.trim() ?? ''
}

beforeEach(async () => {
  vi.useFakeTimers()
  registry = new Map()
  vi.stubGlobal('CSS', { highlights: registry })
  vi.stubGlobal('Highlight', FakeHighlight)

  const wrapper = document.createElement('main')
  wrapper.id = 'wrapper'
  wrapper.innerHTML = PAGE
  document.body.appendChild(wrapper)

  bar = document.createElement('mbr-find-bar')
  document.body.appendChild(bar)
  await bar.updateComplete
})

afterEach(() => {
  bar.remove()
  document.body.innerHTML = ''
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('MbrFindBarElement public contract', () => {
  it('exposes open, close, findNext and findPrevious as callable methods', () => {
    // These four names are hard-coded in Rust string literals; renaming any of
    // them breaks the Edit menu with no compile error anywhere.
    for (const name of ['open', 'close', 'findNext', 'findPrevious'] as const) {
      expect(typeof bar[name]).toBe('function')
    }
    expect(bar.isOpen).toBe(false)
  })

  it('is idempotent on open(): three calls leave it open', async () => {
    // A menu accelerator can fire more than once for a single keystroke, and
    // the Rust open script polls until the element upgrades. A toggle here
    // would leave the bar shut.
    bar.open()
    bar.open()
    bar.open()
    await bar.updateComplete

    expect(bar.isOpen).toBe(true)
    expect(bar.shadowRoot?.querySelector('.find-bar')).not.toBeNull()
  })

  it('refocuses and selects the existing text when reopened', async () => {
    bar.open()
    await bar.updateComplete
    await type('alpha')
    bar.findNext()
    await bar.updateComplete

    const input = bar.shadowRoot!.querySelector('#find-input') as HTMLInputElement
    const select = vi.spyOn(input, 'select')
    const focus = vi.spyOn(input, 'focus')

    bar.open()
    await bar.updateComplete
    expect(bar.isOpen).toBe(true)
    expect(focus).toHaveBeenCalled()
    expect(select).toHaveBeenCalled()
    expect(input.value).toBe('alpha')
    // A repeat menu fire must not scroll the reader back to the first match.
    expect(status()).toBe('2 of 3')
  })

  it('renders nothing until opened', () => {
    expect(bar.shadowRoot?.querySelector('.find-bar')).toBeNull()
  })
})

describe('MbrFindBarElement searching', () => {
  beforeEach(async () => {
    bar.open()
    await bar.updateComplete
  })

  it('counts matches and shows "N of M"', async () => {
    await type('alpha')
    // Three visible occurrences; the .sr-only title duplicate is not one of them.
    expect(status()).toBe('1 of 3')
  })

  it('reports no results for a query that is not on the page', async () => {
    await type('nonexistent')
    expect(status()).toBe('No results')
  })

  it('shows nothing at all for an empty query', async () => {
    await type('alpha')
    await type('')
    expect(status()).toBe('')
    expect(registry.size).toBe(0)
  })

  it('registers both highlight registries for a settled query', async () => {
    await type('alpha')
    expect(registry.get('mbr-find')?.size).toBe(2)
    expect(registry.get('mbr-find-active')?.size).toBe(1)
  })

  it('keeps focus in the input when a settled scan changes the document selection', async () => {
    // WebKit blurs whatever was focused when window.getSelection() changes
    // outside it. Simulate that so a settled scan mid-keystroke cannot regress
    // to stealing focus from the input.
    const realAddRange = Selection.prototype.addRange
    vi.spyOn(Selection.prototype, 'addRange').mockImplementation(function (this: Selection, range: Range) {
      realAddRange.call(this, range)
      const active = bar.shadowRoot?.activeElement as HTMLElement | null
      active?.blur()
    })

    const input = bar.shadowRoot!.querySelector('#find-input') as HTMLInputElement
    input.focus()
    expect(bar.shadowRoot?.activeElement).toBe(input)

    await type('alpha')

    expect(bar.shadowRoot?.activeElement).toBe(input)
  })

  it('honours the case-sensitivity toggle', async () => {
    await type('GUIDE')
    expect(status()).toBe('1 of 1')

    const toggle = bar.shadowRoot!.querySelector('.toggle') as HTMLButtonElement
    toggle.click()
    await bar.updateComplete
    expect(status()).toBe('No results')
  })

  it('flushes a pending debounced scan before stepping', async () => {
    const input = bar.shadowRoot!.querySelector('#find-input') as HTMLInputElement
    input.value = 'gamma'
    input.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
    // No timer advance: the scan is still pending. Stepping must not act on the
    // previous (empty) match set.
    bar.findNext()
    await bar.updateComplete
    expect(status()).toBe('1 of 1')
  })
})

describe('MbrFindBarElement stepping', () => {
  beforeEach(async () => {
    bar.open()
    await bar.updateComplete
    await type('alpha')
  })

  it('advances and wraps past the last match', async () => {
    expect(status()).toBe('1 of 3')
    bar.findNext()
    await bar.updateComplete
    expect(status()).toBe('2 of 3')
    bar.findNext()
    await bar.updateComplete
    expect(status()).toBe('3 of 3')
    bar.findNext()
    await bar.updateComplete
    expect(status()).toBe('1 of 3')
  })

  it('steps backwards and wraps past the first match', async () => {
    bar.findPrevious()
    await bar.updateComplete
    expect(status()).toBe('3 of 3')
    bar.findPrevious()
    await bar.updateComplete
    expect(status()).toBe('2 of 3')
  })

  it('moves the active highlight without changing the total', async () => {
    const first = [...registry.get('mbr-find-active')!][0]
    bar.findNext()
    await bar.updateComplete
    const second = [...registry.get('mbr-find-active')!][0]
    expect(second).not.toBe(first)
    expect(registry.get('mbr-find')?.size).toBe(2)
  })

  it('does nothing when there are no matches', async () => {
    await type('nonexistent')
    expect(() => {
      bar.findNext()
      bar.findPrevious()
    }).not.toThrow()
    expect(status()).toBe('No results')
  })

  it('reopens the bar when stepping while closed', async () => {
    bar.close()
    expect(bar.isOpen).toBe(false)

    bar.findNext()
    await bar.updateComplete
    expect(bar.isOpen).toBe(true)
    // The query survived the close, so the search resumes rather than restarts.
    expect(status()).toBe('2 of 3')
  })
})

describe('MbrFindBarElement keyboard', () => {
  beforeEach(async () => {
    bar.open()
    await bar.updateComplete
    await type('alpha')
  })

  it('steps forward on Enter and back on Shift+Enter', async () => {
    await press('Enter')
    expect(status()).toBe('2 of 3')
    await press('Enter', { shiftKey: true })
    expect(status()).toBe('1 of 3')
  })

  it('closes on Escape', async () => {
    await press('Escape')
    expect(bar.isOpen).toBe(false)
    expect(bar.shadowRoot?.querySelector('.find-bar')).toBeNull()
  })
})

describe('MbrFindBarElement close()', () => {
  beforeEach(async () => {
    bar.open()
    await bar.updateComplete
    await type('alpha')
  })

  it('deletes both highlight registries', () => {
    expect(registry.has('mbr-find')).toBe(true)
    bar.close()
    expect(registry.has('mbr-find')).toBe(false)
    expect(registry.has('mbr-find-active')).toBe(false)
  })

  it('drops the index and stops observing the page', async () => {
    bar.close()
    const wrapper = document.getElementById('wrapper')!
    wrapper.insertAdjacentHTML('beforeend', '<p>alpha</p>')
    await vi.advanceTimersByTimeAsync(500)
    // A closed bar must do no work at all in response to page mutations.
    expect(registry.size).toBe(0)
    expect(bar.isOpen).toBe(false)
  })

  it('keeps the query so a later open() resumes the search', async () => {
    bar.close()
    bar.open()
    await bar.updateComplete
    expect(status()).toBe('1 of 3')
  })

  it('is safe to call when never opened', () => {
    const fresh = document.body.appendChild(document.createElement('mbr-find-bar'))
    expect(() => fresh.close()).not.toThrow()
    expect(fresh.isOpen).toBe(false)
  })
})

describe('MbrFindBarElement without the Custom Highlight API', () => {
  beforeEach(async () => {
    // The realistic gap is an older WebKitGTK. Everything except painting has
    // to keep working, which is what makes this better than window.find().
    vi.stubGlobal('CSS', {})
    vi.stubGlobal('Highlight', undefined)
    bar.open()
    await bar.updateComplete
  })

  it('still counts and steps through matches', async () => {
    await type('alpha')
    expect(status()).toBe('1 of 3')
    bar.findNext()
    await bar.updateComplete
    expect(status()).toBe('2 of 3')
  })

  it('falls back to a real Selection on the active match', async () => {
    await type('gamma')
    expect(window.getSelection()?.toString()).toBe('gamma')
  })
})

describe('MbrFindBarElement reindexing', () => {
  it('picks up content added while the bar is open', async () => {
    bar.open()
    await bar.updateComplete
    await type('alpha')
    expect(status()).toBe('1 of 3')

    // Stands in for hljs / KaTeX / Mermaid finishing after the bar opened.
    document.getElementById('wrapper')!.insertAdjacentHTML('beforeend', '<p>alpha</p>')
    await vi.advanceTimersByTimeAsync(500)
    await bar.updateComplete

    expect(status()).toBe('1 of 4')
  })
})
