import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import { setCalendarChunkImporter } from './calendar-chunk.ts'
import { COMPANION_HIDDEN_CLASS, COMPANION_HIDDEN_KEY } from './calendar/companion-pref.ts'
import { MbrCalendarElement, companionDockable } from './mbr-calendar.ts'

declare const __MBR_THEME_CSS__: string

const DAY_NAV = '<nav class="mbr-calendar-nav" data-mbr-period="2026-10-31" data-mbr-period-kind="day"></nav>'

let enhancePage: Mock<(options: unknown) => { entries: never[]; dispose: () => void }>
let importer: Mock<() => Promise<unknown>>

/**
 * What theme.css's width rules decide: `--mbr-companion-docked: 1` on a body
 * wide enough for the column (happy-dom applies no media queries, so the
 * tests set the property the stylesheet would).
 */
function stubDocked(docked: boolean): void {
  if (docked) document.body.style.setProperty('--mbr-companion-docked', '1')
  else document.body.style.removeProperty('--mbr-companion-docked')
}

async function mount(main: string, { bodyClass = '' } = {}): Promise<MbrCalendarElement> {
  if (bodyClass) document.body.className = bodyClass
  document.body.insertAdjacentHTML('beforeend', `<main id="wrapper">${main}</main>`)
  const el = document.createElement('mbr-calendar')
  document.body.append(el)
  await settle(el)
  return el
}

async function settle(el: MbrCalendarElement): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }
}

const press = (key: string, init: KeyboardEventInit = {}) =>
  document.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...init }))

const fab = (el: MbrCalendarElement) => el.shadowRoot!.querySelector<HTMLButtonElement>('.calendar-fab')
const companion = (el: MbrCalendarElement) => el.shadowRoot!.querySelector('mbr-calendar-companion')

beforeEach(() => {
  window.__MBR_CONFIG__ = { serverMode: true, calendar: {} } as typeof window.__MBR_CONFIG__
  enhancePage = vi.fn((_options: unknown) => ({ entries: [], dispose: () => {} }))
  // The real chunk elements would fetch site.json; a stand-in is enough here.
  importer = vi.fn((): Promise<unknown> => Promise.resolve({ enhancePage }))
  setCalendarChunkImporter(importer)
  stubDocked(false)
})

afterEach(() => {
  document.body.innerHTML = ''
  document.body.className = ''
  document.body.removeAttribute('style')
  document.documentElement.classList.remove(COMPANION_HIDDEN_CLASS)
  localStorage.removeItem(COMPANION_HIDDEN_KEY)
  window.__MBR_CONFIG__ = undefined
  vi.restoreAllMocks()
})

describe('the prefilter', () => {
  it('loads nothing on an ordinary note', async () => {
    const el = await mount('<ul><li>2026 goals</li></ul>')
    expect(importer).not.toHaveBeenCalled()
    expect(fab(el)).toBeNull()
  })

  it('loads the chunk at idle on a day note and enhances the page with its day', async () => {
    const el = await mount(`${DAY_NAV}<p>x</p>`)
    expect(importer).toHaveBeenCalledTimes(1)
    expect(enhancePage).toHaveBeenCalledWith(
      expect.objectContaining({ pageDay: '2026-10-31', agendaChips: true, root: document.querySelector('main') })
    )
    expect(fab(el)).not.toBeNull()
  })

  it('loads it for time-like bullets in any note, without claiming a day', async () => {
    await mount('<ul><li>07:30 BP 118/76</li></ul>')
    expect(enhancePage).toHaveBeenCalledWith(expect.objectContaining({ pageDay: null }))
  })

  it('does nothing at all when the calendar is disabled', async () => {
    window.__MBR_CONFIG__ = { calendar: { enabled: false } } as typeof window.__MBR_CONFIG__
    const el = await mount(`${DAY_NAV}<ul><li>09:00 Standup</li></ul>`)
    press('C', { shiftKey: true })
    await settle(el)
    expect(importer).not.toHaveBeenCalled()
    expect(el.isOpen).toBe(false)
  })
})

describe('C and the drawer', () => {
  it('toggles the drawer, which owns the keyboard while open', async () => {
    const el = await mount('<p>plain</p>')
    press('C', { shiftKey: true })
    expect(el.isOpen).toBe(true)
    await settle(el)
    expect(companion(el)?.getAttribute('placement') ?? (companion(el) as unknown as { placement: string }).placement).toBe('drawer')
    press('C', { shiftKey: true })
    expect(el.isOpen).toBe(false)
  })

  it('ignores modifiers, typing, and companion = "off"', async () => {
    const el = await mount('<p>plain</p><input id="field">')
    press('C', { shiftKey: true, metaKey: true })
    document.getElementById('field')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'C', shiftKey: true, bubbles: true }))
    expect(el.isOpen).toBe(false)

    document.body.innerHTML = ''
    window.__MBR_CONFIG__ = { calendar: { companion: 'off' } } as typeof window.__MBR_CONFIG__
    const off = await mount(`${DAY_NAV}<p>x</p>`)
    press('C', { shiftKey: true })
    expect(off.isOpen).toBe(false)
    expect(fab(off)).toBeNull()
    // Chips still load.
    expect(enhancePage).toHaveBeenCalled()
  })

  it('opens from the floating button, which then hides', async () => {
    const el = await mount('<ul><li>09:00 Standup</li></ul>')
    fab(el)!.click()
    await settle(el)
    expect(el.isOpen).toBe(true)
    expect(fab(el)).toBeNull()
    expect(companion(el)).not.toBeNull()
  })

  it('closes when the companion asks', async () => {
    const el = await mount('<p>plain</p>')
    el.open()
    await settle(el)
    companion(el)!.dispatchEvent(new CustomEvent('mbr-calendar-close', { bubbles: true, composed: true }))
    expect(el.isOpen).toBe(false)
  })
})

describe('docked', () => {
  it('docks on a wide day note the server marked, with no button', async () => {
    stubDocked(true)
    const el = await mount(`${DAY_NAV}<p>x</p>`, { bodyClass: 'mbr-has-companion' })
    expect(el.isDocked).toBe(true)
    expect(el.isOpen).toBe(false)
    expect((companion(el) as unknown as { placement: string }).placement).toBe('docked')
    expect(fab(el)).toBeNull()
  })

  it('reads the docking from theme.css, which alone holds the breakpoints', () => {
    expect(companionDockable()).toBe(false)
    stubDocked(true)
    expect(companionDockable()).toBe(true)
    // ≥1280px without the docked sidebar, ≥1440px with it: the only two places
    // the property is set, each in its own width rule.
    const css = __MBR_THEME_CSS__.replace(/\/\*[\s\S]*?\*\//g, '')
    const at = (width: number) =>
      new RegExp(`@media screen and \\(min-width: ${width}px\\) \\{\\s*([^{]+)\\{\\s*--mbr-companion-docked: 1;`).exec(css)?.[1].trim()
    expect(at(1280)).toBe('body.mbr-has-companion:not(:has(> mbr-browse-single))')
    expect(at(1440)).toBe('body.mbr-has-companion')
  })

  it('remembers a dismissal, then C brings it back', async () => {
    stubDocked(true)
    const el = await mount(`${DAY_NAV}<p>x</p>`, { bodyClass: 'mbr-has-companion' })
    companion(el)!.dispatchEvent(new CustomEvent('mbr-calendar-close', { bubbles: true, composed: true }))
    await settle(el)
    expect(el.isDocked).toBe(false)
    expect(document.documentElement.classList.contains(COMPANION_HIDDEN_CLASS)).toBe(true)
    expect(localStorage.getItem(COMPANION_HIDDEN_KEY)).toBe('1')
    // Dismissed: the button offers it back; C restores the column, not a drawer.
    expect(fab(el)).not.toBeNull()
    press('C', { shiftKey: true })
    await settle(el)
    expect(el.isDocked).toBe(true)
    expect(el.isOpen).toBe(false)
    expect(localStorage.getItem(COMPANION_HIDDEN_KEY)).toBeNull()
  })

  it('starts dismissed when the pre-paint class says so', async () => {
    stubDocked(true)
    document.documentElement.classList.add(COMPANION_HIDDEN_CLASS)
    const el = await mount(`${DAY_NAV}<p>x</p>`, { bodyClass: 'mbr-has-companion' })
    expect(el.isDocked).toBe(false)
    expect(companion(el)).toBeNull()
  })
})

describe('the floating button', () => {
  it('stacks above the play button, through --mbr-fab-play-offset', () => {
    const css = MbrCalendarElement.elementStyles.map((s) => (s as CSSStyleSheet & { cssText?: string }).cssText ?? '').join('\n')
    expect(css).toMatch(/\.calendar-fab \{[^}]*bottom: calc\(1rem \+ env\(safe-area-inset-bottom, 0px\) \+ var\(--mbr-fab-play-offset, 0px\)\)/)
  })

  it('lifts the review button while it shows', async () => {
    const el = await mount('<ul><li>09:00 Standup</li></ul>')
    expect(fab(el)).not.toBeNull()
    expect(document.documentElement.style.getPropertyValue('--mbr-fab-stack-offset')).toBe('4.25rem')
    el.remove()
    expect(document.documentElement.style.getPropertyValue('--mbr-fab-stack-offset')).toBe('')
  })
})

describe('c and the overlay', () => {
  // A <body> child, never inside the (docked: fixed, z-indexed) <mbr-calendar>.
  const overlay = (_el: MbrCalendarElement) => document.body.querySelector(':scope > mbr-calendar-overlay')

  it('c opens the overlay over all notes, from this page', async () => {
    const el = await mount('<p>plain</p>')
    press('c')
    expect(el.isOpen).toBe(true)
    await settle(el)
    expect(importer).toHaveBeenCalledTimes(1)
    const o = overlay(el) as unknown as { open: boolean; scope: { label: string; pagePath?: string } }
    expect(o.open).toBe(true)
    expect(o.scope.label).toBe('All notes')
    expect(o.scope.pagePath).toBe('/')
  })

  it('opens the overlay outside the docked column, so its stacking context cannot trap it', async () => {
    stubDocked(true)
    const el = await mount(DAY_NAV, { bodyClass: 'mbr-has-companion' })
    expect(el.isDocked).toBe(true)
    press('c')
    await settle(el)
    const o = overlay(el)
    expect(o).not.toBeNull()
    expect(el.contains(o)).toBe(false)
    expect(el.shadowRoot!.querySelector('mbr-calendar-overlay')).toBeNull()
    el.remove()
    expect(o!.isConnected).toBe(false)
  })

  it('ignores c with modifiers, in a field, and while another overlay is open', async () => {
    const el = await mount('<p>plain</p><input id="field">')
    press('c', { metaKey: true })
    press('c', { ctrlKey: true })
    document.getElementById('field')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'c', bubbles: true }))
    expect(el.isOpen).toBe(false)
    press('C', { shiftKey: true })
    expect(el.isOpen).toBe(true)
    press('c')
    // The drawer stays the open overlay; c did not swap it for the calendar.
    await settle(el)
    expect(overlay(el)).toBeNull()
  })

  it('openOverlay keeps the overlay element across close, and is off with the calendar', async () => {
    const el = await mount('<p>plain</p>')
    expect(el.openOverlay({ label: 'Folder: x', files: [] })).toBe(true)
    await settle(el)
    const first = overlay(el)
    el.closeOverlay()
    await settle(el)
    expect(el.isOpen).toBe(false)
    expect(overlay(el)).toBe(first)
    expect((first as unknown as { open: boolean }).open).toBe(false)

    document.body.innerHTML = ''
    window.__MBR_CONFIG__ = { calendar: { enabled: false } } as typeof window.__MBR_CONFIG__
    const off = await mount('<p>x</p>')
    expect(off.overlayAvailable).toBe(false)
    expect(off.openOverlay()).toBe(false)
  })
})

describe('task markers for the companion', () => {
  const markers = (el: MbrCalendarElement) =>
    (companion(el) as unknown as { taskMarkers: unknown } | null)?.taskMarkers

  it('hands the companion a marker source only where the task endpoint exists', async () => {
    const el = await mount(DAY_NAV, { bodyClass: 'mbr-has-companion' })
    press('C', { shiftKey: true })
    await settle(el)
    expect(companion(el)).not.toBeNull()
    expect(markers(el)).toBeNull()
  })

  it('replaces the source after a task write, so the companion asks again', async () => {
    window.__MBR_CONFIG__ = { serverMode: true, tasksEnabled: true, calendar: {} } as typeof window.__MBR_CONFIG__
    const el = await mount(DAY_NAV)
    press('C', { shiftKey: true })
    await settle(el)
    const before = markers(el)
    expect(typeof before).toBe('function')
    const { toggleTask } = await import('./task-toggle.ts')
    globalThis.fetch = vi.fn((input: RequestInfo | URL) =>
      Promise.resolve(
        String(input).startsWith('/.mbr/raw/')
          ? ({ ok: true, status: 200, headers: new Headers(), text: () => Promise.resolve('- [ ] a\n') } as Response)
          : ({ ok: true, status: 200, json: () => Promise.resolve({ line: 1, text: '- [x] a' }) } as Response)
      )
    ) as unknown as typeof fetch
    expect((await toggleTask({ path: 'n.md', line: 1, to: 'done' })).ok).toBe(true)
    await settle(el)
    expect(markers(el)).not.toBe(before)
    expect(typeof markers(el)).toBe('function')
  })
})

