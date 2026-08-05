import { describe, it, expect, afterEach, beforeEach, vi } from 'vitest'
import type { LitElement } from 'lit'
import { isInputTarget, isMacPlatform, isModalOpen } from './mbr-keys.js'
import type { MbrKeysElement } from './mbr-keys.js'
import { isAnyOverlayOpen, findOverlay, OVERLAY_TAGS } from './overlay.js'
import type { MbrOverlay, OverlayTag } from './overlay.js'
// Side-effect imports: the overlays must be REGISTERED for these tests to
// exercise the real contract. Without them `document.createElement('mbr-search')`
// yields a bare HTMLElement and any assertion about open/closed state is
// vacuous (which is how the previous private-field tests passed).
import './mbr-search.js'
import './mbr-browse.js'
import './mbr-browse-single.js'
import './mbr-fuzzy-nav.js'
import './mbr-find-bar.js'
import './mbr-tasks.js'
import { setTasksChunkImporter } from './mbr-tasks.js'

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

/**
 * Every overlay element, created through `document.createElement` so the tag
 * name map supplies its concrete element type.
 *
 * The `MbrOverlay & LitElement` annotation is the COMPILE-TIME half of the
 * contract test: if a component drops, renames or narrows `isOpen`/`open()`/
 * `close()`, this array stops type-checking. (The components also declare
 * `implements MbrOverlay`, so the same rename fails at the definition site.)
 */
const OVERLAY_CASES: ReadonlyArray<{ tag: OverlayTag; create: () => MbrOverlay & LitElement }> = [
  { tag: 'mbr-search', create: () => document.createElement('mbr-search') },
  { tag: 'mbr-browse', create: () => document.createElement('mbr-browse') },
  { tag: 'mbr-browse-single', create: () => document.createElement('mbr-browse-single') },
  { tag: 'mbr-fuzzy-nav', create: () => document.createElement('mbr-fuzzy-nav') },
  { tag: 'mbr-find-bar', create: () => document.createElement('mbr-find-bar') },
  { tag: 'mbr-tasks', create: () => document.createElement('mbr-tasks') },
]

describe('isModalOpen', () => {
  beforeEach(() => {
    // `tasksEnabled` matters here: <mbr-tasks> refuses to open without the
    // endpoint, since an invisible "open" overlay would suppress every
    // bare-letter shortcut on the page.
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, tasksEnabled: true }
    window.headings = []
    // <mbr-tasks>.open() lazy-imports its panel chunk from a runtime URL, which
    // happy-dom cannot execute; stub the seam.
    setTasksChunkImporter(() => Promise.resolve({}))
    // <mbr-fuzzy-nav>.open() kicks off a links.json fetch; keep it well-formed
    // so the modal can render without the shared cache seeing garbage.
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => ({ inbound: [], outbound: [] }) }),
    )
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    window.__MBR_CONFIG__ = undefined
  })

  it('returns false with no modal elements present', () => {
    expect(isModalOpen()).toBe(false)
  })

  it('covers every registered overlay tag', () => {
    expect(OVERLAY_CASES.map((c) => c.tag)).toEqual([...OVERLAY_TAGS])
  })

  it('returns false when every overlay is present but closed', async () => {
    for (const { create } of OVERLAY_CASES) {
      const el = document.body.appendChild(create())
      await el.updateComplete
    }
    expect(isModalOpen()).toBe(false)
    expect(isAnyOverlayOpen()).toBe(false)
  })

  // Drives each overlay through its PUBLIC contract instead of poking the
  // private backing field (`_isOpen` / `_isDrawerOpen`) the way this suite used
  // to: those assertions passed against un-upgraded elements and would not have
  // noticed a rename on the component.
  for (const { tag, create } of OVERLAY_CASES) {
    it(`detects an open <${tag}> through its public open()/close()`, async () => {
      const overlay = document.body.appendChild(create())
      await overlay.updateComplete

      expect(overlay.isOpen).toBe(false)
      expect(isModalOpen()).toBe(false)

      overlay.open()
      expect(overlay.isOpen).toBe(true)
      expect(isModalOpen()).toBe(true)

      overlay.close()
      expect(overlay.isOpen).toBe(false)
      expect(isModalOpen()).toBe(false)
    })
  }

  it('reads the public isOpen contract rather than a specific private field', () => {
    // Stand-in for a component whose backing state was renamed: the private
    // fields the old implementation read (`_isOpen`, `_isDrawerOpen`) say
    // "closed", but the contract says open. The old private-field lookup
    // reported false here and let bare-letter shortcuts hijack the keyboard.
    const overlay = document.body.appendChild(document.createElement('mbr-browse-single'))
    Object.defineProperty(overlay, 'isOpen', { get: () => true, configurable: true })

    expect((overlay as unknown as { _isDrawerOpen: boolean })._isDrawerOpen).toBe(false)
    expect(isModalOpen()).toBe(true)
  })

  it('treats a present-but-not-upgraded overlay element as closed', () => {
    // An element can sit in the DOM before its definition runs (lazy chunk, a
    // bundle that failed to load), exposing neither `isOpen` nor `open()`.
    // Shadow both to model that: it must count as closed and must not be
    // driven, rather than throwing or falling back to the private field.
    const stale = document.body.appendChild(document.createElement('mbr-search'))
    Object.defineProperty(stale, 'isOpen', { value: undefined, configurable: true })
    Object.defineProperty(stale, 'open', { value: undefined, configurable: true })
    ;(stale as unknown as { _isOpen: boolean })._isOpen = true

    expect(isAnyOverlayOpen()).toBe(false)
    expect(isModalOpen()).toBe(false)
    expect(findOverlay('mbr-search')).toBeNull()
  })

  it('finds an upgraded overlay through findOverlay', async () => {
    const search = document.body.appendChild(document.createElement('mbr-search'))
    await search.updateComplete

    const found = findOverlay('mbr-search')
    expect(found).toBe(search)
    found?.open()
    expect(search.isOpen).toBe(true)
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

/**
 * The global handler opens overlays through the same public contract, so a
 * component that renames its internals breaks the build instead of quietly
 * making a shortcut do nothing.
 */
describe('MbrKeysElement overlay shortcuts', () => {
  let keys: MbrKeysElement

  beforeEach(() => {
    // `tasksEnabled` matters here: <mbr-tasks> refuses to open without the
    // endpoint, since an invisible "open" overlay would suppress every
    // bare-letter shortcut on the page.
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, tasksEnabled: true }
    window.headings = []
    // <mbr-tasks>.open() lazy-imports its panel chunk from a runtime URL, which
    // happy-dom cannot execute; stub the seam.
    setTasksChunkImporter(() => Promise.resolve({}))
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => ({ inbound: [], outbound: [] }) }),
    )
    keys = document.body.appendChild(document.createElement('mbr-keys'))
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    window.__MBR_CONFIG__ = undefined
  })

  /** Dispatch a bare-key keydown from document.body and let Lit settle. */
  async function press(key: string, init: KeyboardEventInit = {}): Promise<KeyboardEvent> {
    const event = new KeyboardEvent('keydown', { key, bubbles: true, composed: true, cancelable: true, ...init })
    document.body.dispatchEvent(event)
    await keys.updateComplete
    return event
  }

  it('opens <mbr-search> on /', async () => {
    const search = document.body.appendChild(document.createElement('mbr-search'))
    const event = await press('/')
    expect(search.isOpen).toBe(true)
    expect(event.defaultPrevented).toBe(true)
  })

  it('opens the media browser on = via the public method', async () => {
    const search = document.body.appendChild(document.createElement('mbr-search'))
    // Spying is only possible because the entry point is public; it also keeps
    // the lazily imported <mbr-media-browser> chunk out of the test.
    const spy = vi.spyOn(search, 'openMediaBrowser').mockResolvedValue(undefined)
    await press('=')
    expect(spy).toHaveBeenCalledTimes(1)
  })

  it('opens <mbr-browse> on F2', async () => {
    // <mbr-browse> lives in the header (_nav.html) and <mbr-keys> in the footer,
    // so mbr-browse registers its own document listener first and its F2
    // `toggle()` runs before mbr-keys' `open()`. Re-connect mbr-keys to
    // reproduce that ordering; with it reversed the two handlers cancel out.
    keys.remove()
    const browse = document.body.appendChild(document.createElement('mbr-browse'))
    document.body.appendChild(keys)

    await press('F2')
    expect(browse.isOpen).toBe(true)
  })

  it('opens <mbr-fuzzy-nav> on f, F and T', async () => {
    for (const [key, init] of [['f', {}], ['F', { shiftKey: true }], ['T', { shiftKey: true }]] as const) {
      const nav = document.body.appendChild(document.createElement('mbr-fuzzy-nav'))
      await press(key, init)
      expect(nav.isOpen).toBe(true)
      nav.remove()
    }
  })

  it('does not open a second overlay while one is already open', async () => {
    const search = document.body.appendChild(document.createElement('mbr-search'))
    const nav = document.body.appendChild(document.createElement('mbr-fuzzy-nav'))
    search.open()

    const event = await press('f')
    expect(nav.isOpen).toBe(false)
    expect(event.defaultPrevented).toBe(false)
  })

  it('does not throw when the overlay element is absent', async () => {
    await expect(press('/')).resolves.toBeDefined()
    await expect(press('F2')).resolves.toBeDefined()
    await expect(press('f')).resolves.toBeDefined()
  })
})

/**
 * `Ctrl+f` used to be vim page-down on every platform, which meant a reader on
 * Windows or Linux had no way to reach the browser's own find-in-page — in
 * server and static modes as much as in GUI mode. It is macOS-only now, and
 * GUI mode gets `<mbr-find-bar>` from the native Edit menu instead.
 */
describe('MbrKeysElement Ctrl+f page-down', () => {
  let scrollBy: ReturnType<typeof vi.spyOn>

  /** Pin navigator.platform, which is what isMacPlatform reads. */
  function setPlatform(platform: string): void {
    Object.defineProperty(navigator, 'platform', { value: platform, configurable: true })
  }

  beforeEach(() => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    document.body.appendChild(document.createElement('mbr-keys'))
    scrollBy = vi.spyOn(window, 'scrollBy').mockImplementation(() => {})
  })

  afterEach(() => {
    scrollBy.mockRestore()
    // Drop the own-property shadow so the prototype getter is live again.
    delete (navigator as unknown as Record<string, unknown>).platform
    window.__MBR_CONFIG__ = undefined
  })

  /** Dispatch Ctrl+<key> from document.body and return the event. */
  function pressCtrl(key: string): KeyboardEvent {
    const event = new KeyboardEvent('keydown', { key, ctrlKey: true, bubbles: true, composed: true, cancelable: true })
    document.body.dispatchEvent(event)
    return event
  }

  it('reads macOS from navigator.platform', () => {
    setPlatform('MacIntel')
    expect(isMacPlatform()).toBe(true)
    setPlatform('Win32')
    expect(isMacPlatform()).toBe(false)
    setPlatform('Linux x86_64')
    expect(isMacPlatform()).toBe(false)
  })

  it('scrolls a full page on macOS', () => {
    setPlatform('MacIntel')
    const event = pressCtrl('f')
    expect(scrollBy).toHaveBeenCalledTimes(1)
    expect(event.defaultPrevented).toBe(true)
  })

  it('leaves Ctrl+f to the browser on Windows and Linux', () => {
    for (const platform of ['Win32', 'Linux x86_64']) {
      setPlatform(platform)
      const event = pressCtrl('f')
      expect(scrollBy).not.toHaveBeenCalled()
      // Not preventing default is what lets the real find bar open.
      expect(event.defaultPrevented).toBe(false)
    }
  })

  it('still scrolls with Ctrl+b, Ctrl+d and Ctrl+u off macOS', () => {
    setPlatform('Win32')
    for (const key of ['b', 'd', 'u']) {
      scrollBy.mockClear()
      const event = pressCtrl(key)
      expect(scrollBy).toHaveBeenCalledTimes(1)
      expect(event.defaultPrevented).toBe(true)
    }
  })
})

/**
 * The find bar only exists in GUI mode (`templates/_footer.html` gates it on
 * `gui_mode`), so listing its keys anywhere else would document a feature the
 * reader does not have.
 */
describe('MbrKeysElement help overlay GUI gating', () => {
  afterEach(() => {
    window.__MBR_CONFIG__ = undefined
  })

  /** Category titles rendered in the help modal for the given mode. */
  async function categoryTitles(guiMode: boolean): Promise<string[]> {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode }
    const keys = document.body.appendChild(document.createElement('mbr-keys'))
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: '?', bubbles: true, composed: true, cancelable: true }))
    await keys.updateComplete
    return [...(keys.shadowRoot?.querySelectorAll('.shortcut-category h3') ?? [])].map((h) => h.textContent ?? '')
  }

  it('lists Find in Page only in GUI mode', async () => {
    expect(await categoryTitles(true)).toContain('Find in Page')
    document.body.innerHTML = ''
    expect(await categoryTitles(false)).not.toContain('Find in Page')
  })

  it('still lists the always-on categories in both modes', async () => {
    expect(await categoryTitles(false)).toContain('Navigation')
    document.body.innerHTML = ''
    expect(await categoryTitles(true)).toContain('Navigation')
  })
})

describe('MbrKeysElement help overlay', () => {
  let element: MbrKeysElement

  beforeEach(() => {
    element = document.createElement('mbr-keys') as MbrKeysElement
    document.body.appendChild(element)
  })

  /** True when the help modal is currently rendered. */
  function helpIsOpen(): boolean {
    return element.shadowRoot?.querySelector('.help-backdrop') != null
  }

  /**
   * Dispatch a composed, bubbling keydown from `origin` so it reaches the
   * document-level listener, then let Lit re-render. Returns the event so
   * callers can assert on `defaultPrevented`.
   */
  async function press(key: string, origin: EventTarget): Promise<KeyboardEvent> {
    const event = new KeyboardEvent('keydown', { key, bubbles: true, composed: true, cancelable: true })
    origin.dispatchEvent(event)
    await element.updateComplete
    return event
  }

  it('opens help when ? is typed outside an input', async () => {
    const event = await press('?', document.body)
    expect(helpIsOpen()).toBe(true)
    expect(event.defaultPrevented).toBe(true)
  })

  it('toggles help closed when ? is pressed again outside an input', async () => {
    await press('?', document.body)
    expect(helpIsOpen()).toBe(true)
    await press('?', document.body)
    expect(helpIsOpen()).toBe(false)
  })

  it('lets ? through in an input without opening help', async () => {
    const input = document.body.appendChild(document.createElement('input'))
    const event = await press('?', input)
    expect(helpIsOpen()).toBe(false)
    expect(event.defaultPrevented).toBe(false)
  })

  it('lets ? through in a textarea without opening help', async () => {
    const textarea = document.body.appendChild(document.createElement('textarea'))
    const event = await press('?', textarea)
    expect(helpIsOpen()).toBe(false)
    expect(event.defaultPrevented).toBe(false)
  })

  it('lets ? through in a contenteditable without opening help', async () => {
    const editable = document.body.appendChild(document.createElement('div'))
    editable.setAttribute('contenteditable', 'true')
    const event = await press('?', editable)
    expect(helpIsOpen()).toBe(false)
    expect(event.defaultPrevented).toBe(false)
  })

  it('lets ? through in an input inside a shadow root (the retargeting case)', async () => {
    // At the document level the event is retargeted to the shadow HOST, so the
    // guard has to consult composedPath, not e.target.
    const host = document.body.appendChild(document.createElement('div'))
    const shadow = host.attachShadow({ mode: 'open' })
    const input = shadow.appendChild(document.createElement('input'))

    const event = await press('?', input)
    expect(helpIsOpen()).toBe(false)
    expect(event.defaultPrevented).toBe(false)
  })

  it('still closes help with Escape pressed from inside an input', async () => {
    await press('?', document.body)
    expect(helpIsOpen()).toBe(true)

    const input = document.body.appendChild(document.createElement('input'))
    const event = await press('Escape', input)
    expect(helpIsOpen()).toBe(false)
    expect(event.defaultPrevented).toBe(true)
  })
})
