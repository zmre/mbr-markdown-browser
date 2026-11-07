import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { MarkdownFile } from '../sorting.ts'
import { DEFAULT_CALENDAR_CONFIG } from './config.ts'
import { OVERLAY_CLOSE_EVENT, type CalendarScope } from './types.ts'
import './mbr-calendar-overlay.ts'
import type { MbrCalendarOverlayElement } from './mbr-calendar-overlay.ts'
import type { MbrCalendarMonthGridElement } from './mbr-calendar-grid.ts'

const dated = (raw: string, date: string, extra: Partial<MarkdownFile> = {}): MarkdownFile => ({
  url_path: `/${raw.replace(/\.md$/, '')}/`,
  raw_path: raw,
  created: 0,
  modified: 0,
  frontmatter: null,
  calendar: { date, source: 'filename' },
  ...extra,
})

const daily = (date: string): MarkdownFile =>
  dated(`daily/${date}.md`, date, { calendar: { date, source: 'filename', period: date } })

const FILES: MarkdownFile[] = [
  daily('2026-10-30'),
  daily('2026-10-31'),
  dated('meetings/2026-10-31 Acme sync.md', '2026-10-31', { frontmatter: { type: 'meeting', title: 'Acme sync' } }),
  dated('meetings/2026-10-31 Retro.md', '2026-10-31', { frontmatter: { type: 'meeting', title: 'Retro' } }),
  dated('blog/2026-10-12-launch.md', '2026-10-12', { frontmatter: { title: 'Launch' } }),
  dated('blog/2025-03-02-old.md', '2025-03-02', { frontmatter: { title: 'Old post' } }),
  dated('weekly/2026-W44.md', '2026-10-26', { calendar: { date: '2026-10-26', source: 'filename', period: '2026-W44' } }),
]

const NOW = new Date(2026, 9, 31, 10, 0)

let navigate: ReturnType<typeof vi.fn<(url: string) => void>>
let closed: number

async function mount(scope: CalendarScope = { label: 'All notes' }, width = 1200): Promise<MbrCalendarOverlayElement> {
  vi.spyOn(window, 'innerWidth', 'get').mockReturnValue(width)
  const el = document.createElement('mbr-calendar-overlay')
  el.siteIndex = () => Promise.resolve({ markdown_files: FILES })
  el.navigate = navigate
  el.now = () => NOW
  el.locale = 'en-US'
  el.config = { ...DEFAULT_CALENDAR_CONFIG, weekStart: 'monday' }
  el.addEventListener(OVERLAY_CLOSE_EVENT, () => {
    closed++
    el.open = false
  })
  document.body.append(el)
  el.scope = scope
  el.open = true
  await settle(el)
  return el
}

async function settle(el: MbrCalendarOverlayElement): Promise<void> {
  for (let i = 0; i < 4; i++) {
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const grid = el.shadowRoot?.querySelector('mbr-calendar-month-grid, mbr-calendar-week-grid') as HTMLElement & {
      updateComplete?: Promise<unknown>
    }
    await grid?.updateComplete
  }
}

const root = (el: MbrCalendarOverlayElement) => el.shadowRoot!
const grid = (el: MbrCalendarOverlayElement) =>
  root(el).querySelector('mbr-calendar-month-grid') as MbrCalendarMonthGridElement | null
const cell = (el: MbrCalendarOverlayElement, date: string) =>
  grid(el)!.shadowRoot!.querySelector<HTMLElement>(`[data-date="${date}"]`)!
const count = (el: MbrCalendarOverlayElement) => root(el).querySelector('.count')!.textContent!.trim()
const press = (key: string, init: KeyboardEventInit = {}) =>
  document.body.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, composed: true, cancelable: true, ...init }))

beforeEach(() => {
  navigate = vi.fn<(url: string) => void>()
  closed = 0
})

afterEach(() => {
  document.body.innerHTML = ''
  vi.restoreAllMocks()
  vi.useRealTimers()
})

describe('the month view', () => {
  it('opens on today with day notes as shaded links and other notes as entries', async () => {
    const el = await mount()
    expect(root(el).querySelector('.title')!.textContent).toContain('October 2026')
    expect(count(el)).toBe('5 notes in October')
    const day = cell(el, '2026-10-31')
    expect(day.classList.contains('daily')).toBe(true)
    expect(day.classList.contains('today')).toBe(true)
    expect(day.querySelector<HTMLAnchorElement>('a.num')!.getAttribute('href')).toBe('/daily/2026-10-31/')
    expect([...day.querySelectorAll('.entry .t')].map((n) => n.textContent)).toEqual(['Acme sync', 'Retro'])
    // A week note is the week-number link, never an entry.
    expect(cell(el, '2026-10-26').querySelector('.entry')).toBeNull()
    expect(grid(el)!.shadowRoot!.querySelector('a.wk')!.getAttribute('href')).toBe('/weekly/2026-W44/')
  })

  it('colours entries by type', async () => {
    const el = await mount()
    const dot = cell(el, '2026-10-31').querySelector<HTMLElement>('.entry .dot')!
    expect(dot.getAttribute('style')).toContain('--mbr-cal-type-2')
  })

  it('pages with ] and [, and t returns to today', async () => {
    const el = await mount()
    press(']')
    await settle(el)
    expect(root(el).querySelector('.title')!.textContent).toContain('November 2026')
    expect(root(el).querySelector('.empty')!.textContent).toContain('No notes in November')
    press('t')
    await settle(el)
    expect(root(el).querySelector('.title')!.textContent).toContain('October 2026')
  })

  it('keeps every key from the page while open, and only then', async () => {
    const el = await mount()
    const seen: string[] = []
    document.addEventListener('keydown', (e) => seen.push(e.key))
    press('[')
    press('?')
    press('x')
    expect(seen).toEqual([])
    el.open = false
    await settle(el)
    press('[')
    expect(seen).toEqual(['['])
  })

  it('Enter opens the only note of a day, else lists the day', async () => {
    const el = await mount()
    // Today (Oct 31) has a day note and two meetings: the popover.
    press('Enter')
    await settle(el)
    const pop = root(el).querySelector('.popover')!
    expect([...pop.querySelectorAll('.t')].map((n) => n.textContent)).toEqual(['Daily note', 'Acme sync', 'Retro'])
    expect(navigate).not.toHaveBeenCalled()
    press('Escape')
    await settle(el)
    expect(root(el).querySelector('.popover')).toBeNull()
    expect(closed).toBe(0)
    // Oct 30 holds only its day note.
    press('h')
    press('Enter')
    expect(navigate).toHaveBeenCalledWith('/daily/2026-10-30/')
  })

  it('Esc closes the help, then the overlay', async () => {
    const el = await mount()
    press('?')
    await settle(el)
    expect(root(el).querySelector('.help')).not.toBeNull()
    press('Escape')
    await settle(el)
    expect(root(el).querySelector('.help')).toBeNull()
    expect(closed).toBe(0)
    press('Escape')
    expect(closed).toBe(1)
  })
})

describe('views, plot-by and the filter', () => {
  it('switches views with m / w / a and keeps the choice', async () => {
    const el = await mount()
    press('w')
    await settle(el)
    expect(root(el).querySelector('mbr-calendar-week-grid')).not.toBeNull()
    expect(root(el).querySelector('.title')!.textContent).toContain('Oct 26 – Nov 1, 2026')
    expect(root(el).querySelector('.links')!.textContent).toContain('Week 44')
    press('a')
    await settle(el)
    const rows = [...root(el).querySelectorAll('.list .row .t')].map((n) => n.textContent)
    expect(rows).toEqual(['Launch', 'Daily note', 'Daily note', 'Acme sync', 'Retro'])
    el.open = false
    await settle(el)
    el.open = true
    await settle(el)
    expect(root(el).querySelector('.list')).not.toBeNull()
  })

  it('opens in List on a phone', async () => {
    const el = await mount({ label: 'All notes' }, 390)
    expect(root(el).querySelector('.list')).not.toBeNull()
  })

  it('filters after a debounce, and the filter persists across months', async () => {
    vi.useFakeTimers()
    const el = await mountWithFakeTimers()
    const input = root(el).querySelector<HTMLInputElement>('input.filter')!
    input.value = 'acme'
    input.dispatchEvent(new Event('input'))
    await vi.advanceTimersByTimeAsync(150)
    await el.updateComplete
    expect(count(el)).toBe('1 of 5 notes in October')
    press(']')
    press('[')
    await vi.advanceTimersByTimeAsync(0)
    await el.updateComplete
    expect(count(el)).toBe('1 of 5 notes in October')
  })

  it('p cycles plot-by', async () => {
    const el = await mount()
    press('p')
    await settle(el)
    expect(root(el).querySelector<HTMLSelectElement>('.plot select')!.value).toBe('modified')
    // Every note was modified at the epoch: nothing in October 2026.
    expect(count(el)).toBe('0 notes in October')
  })
})

describe('scopes', () => {
  it('opens a narrow scope near its notes, and the chip widens it', async () => {
    const old = FILES.filter((f) => f.raw_path.startsWith('blog/2025'))
    const el = await mount({ label: 'Search: old', files: old, plotBy: 'date' })
    expect(root(el).querySelector('.chip')!.textContent).toContain('Search: old')
    expect(root(el).querySelector('.title')!.textContent).toContain('March 2025')
    root(el).querySelector<HTMLButtonElement>('.chip button')!.click()
    await settle(el)
    expect(root(el).querySelector('.chip')!.textContent!.trim()).toBe('All notes')
  })

  it('offers "This folder" when opened from a page', async () => {
    const el = await mount({ label: 'All notes', pagePath: '/meetings/2026-10-31 Acme sync/' })
    const toggle = root(el).querySelector<HTMLButtonElement>('.chip.toggle')!
    expect(toggle.textContent).toContain('meetings')
    toggle.click()
    await settle(el)
    expect(count(el)).toBe('2 notes in October')
  })

  it('reads site.json alone: no request is made without a tasks endpoint', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch')
    const el = await mount()
    press('w')
    press('a')
    press('p')
    await settle(el)
    expect(fetchSpy).not.toHaveBeenCalled()
  })
})

/** `mount` without the setTimeout-based settle, for fake timers. */
async function mountWithFakeTimers(): Promise<MbrCalendarOverlayElement> {
  vi.spyOn(window, 'innerWidth', 'get').mockReturnValue(1200)
  const el = document.createElement('mbr-calendar-overlay')
  el.siteIndex = () => Promise.resolve({ markdown_files: FILES })
  el.navigate = navigate
  el.now = () => NOW
  el.locale = 'en-US'
  document.body.append(el)
  el.open = true
  for (let i = 0; i < 4; i++) {
    await vi.advanceTimersByTimeAsync(0)
    await el.updateComplete
  }
  return el
}
