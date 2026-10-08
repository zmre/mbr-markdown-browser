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
class FakeHighlight extends Set<AbstractRange> {
  priority = 0
}

/** Stand-in for `StaticRange`, which happy-dom does not implement either. */
class FakeStaticRange {
  readonly startContainer: Node
  readonly startOffset: number
  readonly endContainer: Node
  readonly endOffset: number
  constructor(init: StaticRangeInit) {
    this.startContainer = init.startContainer
    this.startOffset = init.startOffset
    this.endContainer = init.endContainer
    this.endOffset = init.endOffset
  }
}

const PAGE = `
  <span class="sr-only" data-pagefind-weight="10">Guide</span>
  <h1>Guide</h1>
  <p>alpha beta alpha</p>
  <p>gamma alpha</p>
`

let bar: MbrFindBarElement
let registry: Map<string, FakeHighlight>

/** Set the find input's value and fire `input`, without waiting for anything. */
function enter(text: string): void {
  const input = bar.shadowRoot!.querySelector('#find-input') as HTMLInputElement
  input.value = text
  input.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
}

/** Type into the find input and let the (longest, one-letter) debounce fire. */
async function type(text: string): Promise<void> {
  enter(text)
  await vi.advanceTimersByTimeAsync(400)
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
  vi.stubGlobal('StaticRange', FakeStaticRange)

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
  window.getSelection()?.removeAllRanges()
  // Several tests spy on Selection.prototype.addRange; an unrestored spy would
  // be captured as the "real" one by the next and recurse.
  vi.restoreAllMocks()
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
    // "All" includes the active match: the active highlight sits above it via
    // priority, which is what lets stepping leave "all" untouched.
    expect(registry.get('mbr-find')?.size).toBe(3)
    expect(registry.get('mbr-find-active')?.size).toBe(1)
    expect(registry.get('mbr-find-active')?.priority).toBe(1)
  })

  it('never touches the document selection, so typing keeps focus in the input', async () => {
    // WebKit applies the blur caused by a selection moving outside the focused
    // element AFTER addRange() returns, during its next selection update, so a
    // refocus straight after the call is a no-op and the blur still lands.
    // Model that deferral: against code that selects the active match, the
    // input is blurred once the microtask runs, whatever it did in between.
    const realAddRange = Selection.prototype.addRange
    const addRange = vi
      .spyOn(Selection.prototype, 'addRange')
      .mockImplementation(function (this: Selection, range: Range) {
        realAddRange.call(this, range)
        queueMicrotask(() => (bar.shadowRoot?.activeElement as HTMLElement | null)?.blur())
      })

    const input = bar.shadowRoot!.querySelector('#find-input') as HTMLInputElement
    input.focus()
    expect(bar.shadowRoot?.activeElement).toBe(input)
    const selectionBefore = document.getSelection()?.rangeCount ?? 0

    await type('a')
    await type('al')
    await type('alpha')
    await vi.advanceTimersByTimeAsync(0)

    expect(status()).toBe('1 of 3')
    expect(addRange).not.toHaveBeenCalled()
    expect(document.getSelection()?.rangeCount ?? 0).toBe(selectionBefore)
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
    expect(registry.get('mbr-find')?.size).toBe(3)
  })

  it('steps without re-registering the "all" highlight', async () => {
    // Rebuilding it re-registers every range, which WebKit charges ~45us each
    // for on the next frame — ~95 ms per Enter at the old cap.
    const all = registry.get('mbr-find')
    bar.findNext()
    bar.findNext()
    await bar.updateComplete
    expect(registry.get('mbr-find')).toBe(all)
    expect(status()).toBe('3 of 3')
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

/**
 * Regression: WebKit paints `::highlight()` through its selection code, which
 * moves an endpoint in `user-select: none` text forward to the next selectable
 * position. theme.css makes every FAQ question (`main dl > dt`) unselectable,
 * so searching a flashcard question painted the next heading a few collapsed
 * answers down and not the match. The registered ranges were correct
 * throughout, so the only check happy-dom can make is the cause: while the bar
 * is open, page text must compute as selectable.
 */
describe('MbrFindBarElement unselectable text', () => {
  const selectableStyle = () => document.querySelectorAll('head style#mbr-find-selectable')

  beforeEach(() => {
    const theme = document.createElement('style')
    theme.id = 'theme-under-test'
    theme.textContent = 'main dl > dt { -webkit-user-select: none; user-select: none; }'
    document.head.appendChild(theme)
    document.getElementById('wrapper')!.insertAdjacentHTML(
      'beforeend',
      '<dl><dt><strong>T1-B11: question</strong></dt><dd>answer</dd></dl>',
    )
  })

  afterEach(() => {
    document.getElementById('theme-under-test')?.remove()
  })

  const userSelect = (el: Element) => getComputedStyle(el).getPropertyValue('user-select')

  it('makes a user-select: none question selectable only while open', async () => {
    const dt = document.querySelector('dt')!
    const strong = dt.querySelector('strong')!
    expect(userSelect(dt)).toBe('none')

    bar.open()
    await bar.updateComplete
    expect(userSelect(dt)).toBe('text')
    expect(userSelect(strong)).toBe('text')

    bar.close()
    expect(userSelect(dt)).toBe('none')
  })

  it('installs the override before the first paint and finds the question', async () => {
    bar.open()
    await bar.updateComplete
    await type('t1-b11')
    expect(status()).toBe('1 of 1')
    expect(selectableStyle()).toHaveLength(1)
    // Outside main#wrapper, so installing it cannot trigger a reindex.
    expect(document.getElementById('wrapper')!.querySelector('style')).toBeNull()
  })

  it('installs exactly one override however often open() fires', async () => {
    bar.open()
    bar.open()
    bar.open()
    await bar.updateComplete
    expect(selectableStyle()).toHaveLength(1)
  })

  it('removes the override on close and when the element is disconnected', async () => {
    bar.open()
    await bar.updateComplete
    bar.close()
    expect(selectableStyle()).toHaveLength(0)

    bar.open()
    await bar.updateComplete
    bar.remove()
    expect(selectableStyle()).toHaveLength(0)
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

  it('moves the fallback Selection with the active match', async () => {
    await type('alpha')
    const first = window.getSelection()!.getRangeAt(0)
    bar.findNext()
    await bar.updateComplete
    const second = window.getSelection()!.getRangeAt(0)
    expect(window.getSelection()?.toString()).toBe('alpha')
    expect(second.startContainer === first.startContainer && second.startOffset === first.startOffset).toBe(false)
  })

  it('keeps focus in the input when a settled scan changes the document selection', async () => {
    // Without CSS.highlights the Selection is the only paint, so it has to be
    // set. Simulate an engine that blurs synchronously inside addRange (the
    // case the refocus can repair) and check the active match is still the
    // selection afterwards.
    const realAddRange = Selection.prototype.addRange
    vi.spyOn(Selection.prototype, 'addRange').mockImplementation(function (this: Selection, range: Range) {
      realAddRange.call(this, range)
      const active = bar.shadowRoot?.activeElement as HTMLElement | null
      active?.blur()
    })

    const input = bar.shadowRoot!.querySelector('#find-input') as HTMLInputElement
    input.focus()
    expect(bar.shadowRoot?.activeElement).toBe(input)

    await type('gamma')

    expect(bar.shadowRoot?.activeElement).toBe(input)
    expect(window.getSelection()?.toString()).toBe('gamma')
  })

  it('releases the fallback Selection on close', async () => {
    await type('gamma')
    bar.close()
    expect(window.getSelection()?.rangeCount).toBe(0)
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

/**
 * Enough matches that a scan takes several slices and painting several frames
 * (`PAINT_SLICE` is 100).
 */
function bigPage(count: number): string {
  return Array.from({ length: count }, (_, i) => `<p>alpha ${i} gamma</p>`).join('')
}

describe('MbrFindBarElement input responsiveness', () => {
  beforeEach(async () => {
    bar.open()
    await bar.updateComplete
  })

  it('debounces short queries longer than long ones', async () => {
    // One letter: 350 ms.
    enter('a')
    await vi.advanceTimersByTimeAsync(340)
    await bar.updateComplete
    expect(status()).toBe('')
    await vi.advanceTimersByTimeAsync(20)
    await bar.updateComplete
    expect(status()).toBe('1 of 9')

    // Two letters: 200 ms.
    enter('al')
    await vi.advanceTimersByTimeAsync(190)
    await bar.updateComplete
    expect(status()).toBe('1 of 9')
    await vi.advanceTimersByTimeAsync(20)
    await bar.updateComplete
    expect(status()).toBe('1 of 3')

    // Three or more: 120 ms.
    enter('gam')
    await vi.advanceTimersByTimeAsync(110)
    await bar.updateComplete
    expect(status()).toBe('1 of 3')
    await vi.advanceTimersByTimeAsync(20)
    await bar.updateComplete
    expect(status()).toBe('1 of 1')
  })

  it('restarts the debounce on every keystroke', async () => {
    enter('g')
    await vi.advanceTimersByTimeAsync(300)
    enter('ga')
    await vi.advanceTimersByTimeAsync(150)
    await bar.updateComplete
    // Neither the one-letter nor the two-letter timer has fired yet.
    expect(status()).toBe('')
    await vi.advanceTimersByTimeAsync(60)
    await bar.updateComplete
    expect(status()).toBe('1 of 1')
  })

  it('flushes a pending one-letter search immediately on Enter', async () => {
    enter('g')
    await press('Enter')
    // Flushed (match 1), then stepped.
    expect(status()).toBe('2 of 2')
  })
})

describe('MbrFindBarElement sliced work', () => {
  /** Make every clock read 10 ms later than the last, so each scan slice ends after one check. */
  function slowClock(): void {
    let now = 0
    vi.spyOn(performance, 'now').mockImplementation(() => (now += 10))
  }

  beforeEach(async () => {
    document.getElementById('wrapper')!.innerHTML = bigPage(300)
    bar.open()
    await bar.updateComplete
  })

  it('yields between scan slices and still reports the exact total', async () => {
    slowClock()
    enter('alpha')
    await vi.advanceTimersByTimeAsync(120)
    await bar.updateComplete
    // First slice ran and yielded; nothing applied yet.
    expect(status()).toBe('')
    await vi.advanceTimersByTimeAsync(50)
    await bar.updateComplete
    expect(status()).toBe('1 of 300')
  })

  it('abandons a stale scan when the query changes mid-scan', async () => {
    slowClock()
    const set = vi.spyOn(registry, 'set')
    enter('alpha')
    await vi.advanceTimersByTimeAsync(120)
    // Scan for "alpha" is in flight. Typing must drop it, not finish it.
    enter('alpha 299 gamma')
    await vi.advanceTimersByTimeAsync(500)
    await bar.updateComplete

    expect(status()).toBe('1 of 1')
    // Only the "alpha 299 gamma" result was ever painted: one "all", one active.
    expect(set.mock.calls.map(([name]) => name)).toEqual(['mbr-find', 'mbr-find-active'])
  })

  it('finishes an in-flight scan synchronously when stepping', async () => {
    slowClock()
    enter('alpha')
    await vi.advanceTimersByTimeAsync(120)
    expect(status()).toBe('')

    bar.findNext()
    await bar.updateComplete
    expect(status()).toBe('2 of 300')
  })

  it('paints nearest-first, one slice per frame, until every match is painted', async () => {
    enter('alpha')
    await vi.advanceTimersByTimeAsync(120)
    await bar.updateComplete
    const all = registry.get('mbr-find')!
    expect(status()).toBe('1 of 300')
    // The first slice is synchronous; the rest follow frame by frame.
    expect(all.size).toBe(100)
    await vi.advanceTimersByTimeAsync(500)
    expect(registry.get('mbr-find')).toBe(all)
    expect(all.size).toBe(300)
  })

  it('stops painting the previous result as soon as the query changes', async () => {
    enter('alpha')
    await vi.advanceTimersByTimeAsync(120)
    const all = registry.get('mbr-find')!
    expect(all.size).toBe(100)
    // Next keystroke: its debounce has not fired, but the old paint frames
    // must already be off the main thread.
    enter('alph')
    await vi.advanceTimersByTimeAsync(100)
    expect(all.size).toBe(100)
  })

  it('cancels in-flight scanning and painting on close', async () => {
    slowClock()
    enter('alpha')
    await vi.advanceTimersByTimeAsync(120)
    bar.close()
    await vi.advanceTimersByTimeAsync(500)
    expect(registry.size).toBe(0)

    vi.mocked(performance.now).mockRestore()
    bar.open()
    await vi.advanceTimersByTimeAsync(0)
    expect(registry.get('mbr-find')!.size).toBeLessThan(300)
    bar.close()
    await vi.advanceTimersByTimeAsync(500)
    // No paint frame survived the close to re-register anything.
    expect(registry.size).toBe(0)
  })

  it('paints with StaticRange, not live ranges', async () => {
    await type('alpha 7 gamma')
    const [painted] = [...registry.get('mbr-find')!]
    const [active] = [...registry.get('mbr-find-active')!]
    expect(painted).toBeInstanceOf(FakeStaticRange)
    expect(active).toBeInstanceOf(FakeStaticRange)
  })
})

/**
 * theme.css collapses every `main dl > dd` until its question has focus. The
 * find bar keeps focus in its input, so it reveals the answer holding the
 * ACTIVE match with `mbr-find-reveal` instead. The stylesheet below is the
 * relevant slice of theme.css; happy-dom cascades it into getComputedStyle,
 * which is what the bar reads to decide an answer is collapsed.
 */
describe('MbrFindBarElement collapsed answers', () => {
  const REVEAL = 'mbr-find-reveal'

  beforeEach(async () => {
    const theme = document.createElement('style')
    theme.id = 'theme-under-test'
    theme.textContent = [
      'main dl > dd { visibility: hidden; }',
      'main dl > dd.mbr-find-reveal { visibility: visible; }',
      // Stands in for an answer the reader opened by some other means.
      'main dl > dd.user-open { visibility: visible; }',
    ].join('\n')
    document.head.appendChild(theme)
    document.getElementById('wrapper')!.insertAdjacentHTML(
      'beforeend',
      `<p id="prose">licensing in prose</p>
       <dl>
         <dt id="q1">Question one</dt>
         <dd id="a1">first licensing answer</dd>
         <dd id="a1b">its second answer</dd>
         <dt id="q2">Question two</dt>
         <dd id="a2">another licensing answer</dd>
         <dt id="q3">Question three</dt>
         <dd id="a3" class="user-open">already open licensing answer</dd>
       </dl>`,
    )
    bar.open()
    await bar.updateComplete
  })

  afterEach(() => {
    document.getElementById('theme-under-test')?.remove()
  })

  const revealed = () => [...document.querySelectorAll(`.${REVEAL}`)].map((el) => el.id)
  const input = () => bar.shadowRoot!.querySelector('#find-input') as HTMLInputElement

  it('leaves a match outside any answer alone', async () => {
    await type('licensing')
    expect(status()).toBe('1 of 4')
    expect(revealed()).toEqual([])
  })

  it('opens the whole entry holding the active match, and only that one', async () => {
    await type('licensing')
    bar.findNext()
    await bar.updateComplete
    expect(status()).toBe('2 of 4')
    // Both answers of question one, as a click on it would open; question
    // two's answer also has a (painted) match but is not active, so stays shut.
    expect(revealed()).toEqual(['a1', 'a1b'])
    expect(getComputedStyle(document.getElementById('a1')!).visibility).toBe('visible')
    expect(getComputedStyle(document.getElementById('a2')!).visibility).toBe('hidden')
  })

  it('closes the answer it opened when the active match moves on', async () => {
    await type('licensing')
    bar.findNext()
    bar.findNext()
    expect(revealed()).toEqual(['a2'])
    bar.findPrevious()
    bar.findPrevious()
    expect(revealed()).toEqual([])
  })

  it('closes the answer it opened when the query changes or is cleared', async () => {
    await type('licensing')
    bar.findNext()
    expect(revealed()).toEqual(['a1', 'a1b'])
    await type('prose')
    expect(revealed()).toEqual([])

    await type('another')
    expect(revealed()).toEqual(['a2'])
    enter('')
    expect(revealed()).toEqual([])
  })

  it('never touches an answer that was already open', async () => {
    await type('licensing')
    bar.findPrevious()
    await bar.updateComplete
    expect(status()).toBe('4 of 4')
    expect(revealed()).toEqual([])
    bar.findNext()
    const a3 = document.getElementById('a3')!
    expect(a3.className).toBe('user-open')
    expect(getComputedStyle(a3).visibility).toBe('visible')
  })

  it('reveals before it scrolls, so the scroll measures the open layout', async () => {
    const a1 = document.getElementById('a1')!
    const openAtMeasure: boolean[] = []
    vi.spyOn(Range.prototype, 'getBoundingClientRect').mockImplementation(function (this: Range) {
      openAtMeasure.push(a1.classList.contains(REVEAL))
      // Far below the viewport, so scrollRangeIntoView actually scrolls.
      return { top: 5000, bottom: 5020, left: 0, right: 50, width: 50, height: 20, x: 0, y: 5000 } as DOMRect
    })
    const scrollTo = vi.spyOn(window, 'scrollTo').mockImplementation(() => {})
    await type('first licensing')
    expect(scrollTo).toHaveBeenCalled()
    expect(openAtMeasure).toEqual([true])
  })

  it('keeps focus in the find input while revealing', async () => {
    await type('licensing')
    expect(bar.shadowRoot?.activeElement).toBe(input())
    bar.findNext()
    bar.findNext()
    await bar.updateComplete
    expect(revealed()).toEqual(['a2'])
    expect(bar.shadowRoot?.activeElement).toBe(input())
    expect(window.getSelection()?.rangeCount ?? 0).toBe(0)
  })

  it('does not reindex because of its own class changes', async () => {
    await type('licensing')
    const rebuild = vi.spyOn(bar as unknown as { _rebuildIndex(): void }, '_rebuildIndex')
    bar.findNext()
    bar.findNext()
    await vi.advanceTimersByTimeAsync(1000)
    await bar.updateComplete
    expect(rebuild).not.toHaveBeenCalled()
    expect(status()).toBe('3 of 4')
  })

  it('leaves the last answer open on close, until the reader clicks elsewhere', async () => {
    await type('licensing')
    bar.findNext()
    bar.close()
    expect(revealed()).toEqual(['a1', 'a1b'])

    // Selecting text inside the answer must not shut it.
    document.getElementById('a1')!.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    expect(revealed()).toEqual(['a1', 'a1b'])

    document.getElementById('prose')!.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    expect(revealed()).toEqual([])
  })

  it('closes it on a focus move elsewhere, and stops listening afterwards', async () => {
    await type('licensing')
    bar.findNext()
    bar.close()
    const remove = vi.spyOn(document, 'removeEventListener')
    document.getElementById('q2')!.dispatchEvent(new FocusEvent('focusin', { bubbles: true }))
    expect(revealed()).toEqual([])
    expect(remove.mock.calls.map(([type]) => type).sort()).toEqual(['focusin', 'pointerdown'])
  })

  it('hands the open answer back to the bar on reopen, and clears it on disconnect', async () => {
    await type('licensing')
    bar.findNext()
    bar.close()
    bar.open()
    await vi.advanceTimersByTimeAsync(0)
    await bar.updateComplete
    // The retained query re-ran from match 1, which is in prose.
    expect(status()).toBe('1 of 4')
    expect(revealed()).toEqual([])

    bar.findNext()
    bar.close()
    bar.remove()
    expect(revealed()).toEqual([])
  })
})

/**
 * A reindex (content mutation), case toggle or reopen swaps `_index` and starts
 * a new scan, but `_matchStarts` / `_matchEnds` keep the OLD offsets until that
 * scan lands. A frame-paced paint job still running from before would pair the
 * new index with the old offsets and paint ranges on the wrong text.
 */
describe('MbrFindBarElement reindex during a paint', () => {
  beforeEach(async () => {
    // Past HIGHLIGHT_CAP (1000), so stepping out of the window repaints it.
    document.getElementById('wrapper')!.innerHTML = bigPage(1500)
    bar.open()
    await bar.updateComplete
  })

  /** The text each painted range covers. Every match here sits in one text node. */
  const paintedTexts = (highlight: Set<AbstractRange>) =>
    [...highlight].map((range) => (range.startContainer as Text).data.slice(range.startOffset, range.endOffset))

  it('never paints a slice with the previous scan\'s offsets against the new index', async () => {
    enter('alpha')
    await vi.advanceTimersByTimeAsync(1000)
    await bar.updateComplete
    expect(status()).toBe('1 of 1500')

    // Shifts every later offset by the new paragraph's length. The reindex it
    // schedules fires 250 ms later.
    const p = document.createElement('p')
    p.textContent = 'xyzzy'
    document.getElementById('wrapper')!.prepend(p)
    await vi.advanceTimersByTimeAsync(240)

    // Wrap to the last match: outside the painted window, so a fresh
    // multi-frame paint starts and is still running when the reindex fires.
    bar.findPrevious()
    const all = registry.get('mbr-find')!
    // Keep the reindex scan in flight across several frames, so a surviving
    // paint job would get to run against the new index.
    let now = 0
    vi.spyOn(performance, 'now').mockImplementation(() => (now += 10))
    await vi.advanceTimersByTimeAsync(60)

    const texts = paintedTexts(all)
    expect(texts.length).toBeGreaterThan(0)
    expect(texts.filter((text) => text !== 'alpha')).toEqual([])

    await vi.advanceTimersByTimeAsync(1000)
    await bar.updateComplete
    expect(status()).toBe('1500 of 1500')
    expect(paintedTexts(registry.get('mbr-find')!).filter((text) => text !== 'alpha')).toEqual([])
  })
})
