import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { MarkdownFile } from '../sorting.ts'
import { DEFAULT_CALENDAR_CONFIG } from './config.ts'
import { OVERLAY_CLOSE_EVENT, type CalendarScope, type TaskGridQuery, type TaskGridSource } from './types.ts'
import type { GridTaskHit, TaskGridResponse } from '../tasks/types.ts'
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

async function mount(
  scope: CalendarScope = { label: 'All notes' },
  width = 1200,
  taskSource: TaskGridSource | null = null
): Promise<MbrCalendarOverlayElement> {
  vi.spyOn(window, 'innerWidth', 'get').mockReturnValue(width)
  const el = document.createElement('mbr-calendar-overlay')
  el.taskSource = taskSource
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
const weekGrid = (el: MbrCalendarOverlayElement) => root(el).querySelector('mbr-calendar-week-grid') as HTMLElement | null
/** Every entry title the month grid (or the week grid) draws, by day. */
const drawn = (el: MbrCalendarOverlayElement, date: string) => {
  const g = (grid(el) ?? weekGrid(el))!
  return [...g.shadowRoot!.querySelectorAll(`[data-date="${date}"] .entry .t`)].map((n) => n.textContent)
}
/** Opens the options menu and picks the radio `name=value`. */
async function choose(el: MbrCalendarOverlayElement, name: string, value: string): Promise<void> {
  if (!root(el).querySelector('.options')) {
    press('o')
    await settle(el)
  }
  root(el).querySelector<HTMLInputElement>(`.options input[name="${name}"][value="${value}"]`)!.click()
  await settle(el)
}
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

  it('p cycles plot-by, shown as a chip and in the options menu', async () => {
    const el = await mount()
    expect(root(el).querySelector('.chip')).toBeNull()
    press('p')
    await settle(el)
    expect(root(el).querySelector('.chip')!.textContent).toContain('Plot by modified')
    // Every note was modified at the epoch: nothing in October 2026.
    expect(count(el)).toBe('0 notes in October')
    expect(drawn(el, '2026-10-31')).toEqual([])
    expect(root(el).querySelector('.options-btn .badge')).not.toBeNull()
    press('o')
    await settle(el)
    expect(root(el).querySelector<HTMLInputElement>('.options input[name="plot"]:checked')!.value).toBe('modified')
    // Focus lands on the checked radio.
    expect(el.shadowRoot!.activeElement).toBe(root(el).querySelector('.options input[name="plot"]:checked'))
    // `p` still cycles with focus on the menu's radios.
    root(el)
      .querySelector('.options input:checked')!
      .dispatchEvent(new KeyboardEvent('keydown', { key: 'p', bubbles: true, composed: true, cancelable: true }))
    await settle(el)
    expect(root(el).querySelector<HTMLInputElement>('.options input[name="plot"]:checked')!.value).toBe('created')
    // The chip's ✕ goes back to note date.
    press('Escape')
    await settle(el)
    root(el).querySelector<HTMLButtonElement>('.chip button')!.click()
    await settle(el)
    expect(root(el).querySelector('.chip')).toBeNull()
    expect(drawn(el, '2026-10-31')).toEqual(['Acme sync', 'Retro'])
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
    expect(root(el).querySelector('.chip')).toBeNull()
  })

  it('offers "This folder" in the options menu when opened from a page', async () => {
    const el = await mount({ label: 'All notes', pagePath: '/meetings/2026-10-31 Acme sync/' })
    press('o')
    await settle(el)
    const labels = [...root(el).querySelectorAll('.options fieldset')].map((f) => f.querySelector('legend')!.textContent)
    // No task source: no Tasks group.
    expect(labels).toEqual(['Scope', 'Plot by'])
    expect(root(el).querySelector('.options')!.textContent).toContain('This folder: meetings')
    await choose(el, 'scope', 'folder')
    expect(count(el)).toBe('2 notes in October')
    expect(root(el).querySelector('.chip')!.textContent).toContain('Folder: meetings')
    // Esc closes the menu first, then the overlay.
    press('Escape')
    await settle(el)
    expect(root(el).querySelector('.options')).toBeNull()
    expect(closed).toBe(0)
    // The chip's ✕ turns it off.
    root(el).querySelector<HTMLButtonElement>('.chip button')!.click()
    await settle(el)
    expect(count(el)).toBe('5 notes in October')
  })

  it('offers no scope choice over an explicit scope', async () => {
    const el = await mount({ label: 'Folder: blog', files: FILES.slice(4, 6), pagePath: '/blog/2026-10-12-launch/' })
    press('o')
    await settle(el)
    expect(root(el).querySelector('.options input[name="scope"]')).toBeNull()
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

describe('the grids follow the day data (regression: they kept every note)', () => {
  const FOLDER_PAGE = { label: 'All notes', pagePath: '/blog/2026-10-12-launch/' }

  it('"This folder" re-renders the month grid', async () => {
    const el = await mount(FOLDER_PAGE)
    expect(drawn(el, '2026-10-31')).toEqual(['Acme sync', 'Retro'])
    expect(drawn(el, '2026-10-12')).toEqual(['Launch'])
    await choose(el, 'scope', 'folder')
    expect(drawn(el, '2026-10-31')).toEqual([])
    expect(drawn(el, '2026-10-12')).toEqual(['Launch'])
    // The day note's shading goes with it.
    expect(cell(el, '2026-10-31').classList.contains('daily')).toBe(false)
    await choose(el, 'scope', 'all')
    expect(drawn(el, '2026-10-31')).toEqual(['Acme sync', 'Retro'])
  })

  it('"This folder" re-renders the week grid', async () => {
    const el = await mount({ label: 'All notes', pagePath: '/meetings/2026-10-31 Acme sync/' })
    press('w')
    await settle(el)
    await choose(el, 'scope', 'folder')
    expect(drawn(el, '2026-10-31')).toEqual(['Acme sync', 'Retro'])
    await choose(el, 'plot', 'modified')
    expect(drawn(el, '2026-10-31')).toEqual([])
  })

  it('the filter re-renders the month and week grids', async () => {
    const el = await mount()
    const filter = async (text: string) => {
      const input = root(el).querySelector<HTMLInputElement>('input.filter')!
      input.value = text
      input.dispatchEvent(new Event('input'))
      await new Promise((r) => setTimeout(r, 150))
      await settle(el)
    }
    await filter('retro')
    expect(drawn(el, '2026-10-31')).toEqual(['Retro'])
    expect(drawn(el, '2026-10-12')).toEqual([])
    press('w')
    await settle(el)
    expect(drawn(el, '2026-10-31')).toEqual(['Retro'])
    await filter('')
    expect(drawn(el, '2026-10-31')).toEqual(['Acme sync', 'Retro'])
  })
})

describe('tasks', () => {
  const hit = (over: Partial<GridTaskHit>): GridTaskHit => ({
    kind: 'task',
    line: 3,
    depth: 0,
    status: 'open',
    priority: 'normal',
    text: 'Send the deck',
    marker_start: null,
    marker_end: null,
    tags: [],
    due: '2026-10-31T00:00:00',
    due_has_time: false,
    scheduled: null,
    scheduled_has_time: false,
    done: null,
    done_has_time: false,
    moved_to: null,
    url_path: '/meetings/2026-10-31 Acme sync/',
    path: 'meetings/2026-10-31 Acme sync.md',
    placed_by: 'due',
    note: 'Acme sync',
    ...over,
  })
  const response = (q: TaskGridQuery): TaskGridResponse => {
    const tasks = [
      hit({}),
      hit({ line: 9, text: 'Book the room #ops', tags: ['ops'], placed_by: 'scheduled', time: '14:30' }),
      ...(q.statuses.includes('done') ? [hit({ line: 12, status: 'done', text: 'Draft agenda', placed_by: 'done' })] : []),
    ]
    const inFolder = (h: GridTaskHit) => !q.folder || h.path.startsWith(q.folder.slice(1))
    const kept = tasks.filter(inFolder)
    return {
      from: q.from,
      to: q.to,
      days: kept.length
        ? { '2026-10-31': { tasks: kept, open: kept.filter((h) => h.status === 'open').length, done: kept.length - 2, overdue: false } }
        : {},
      undated_in_day_notes: {},
      overdue_before_range: 0,
      folders: [],
      total_matches: kept.length,
      duration_ms: 1,
      scan_in_progress: false,
    }
  }
  let calls: TaskGridQuery[]
  const source = (): TaskGridSource => (q) => {
    calls.push(q)
    return Promise.resolve(response(q))
  }

  beforeEach(() => {
    calls = []
  })

  it('shows due and done tasks by default, after the notes, over the grid range', async () => {
    const el = await mount({ label: 'All notes' }, 1200, source())
    expect(calls).toEqual([{ from: '2026-09-28', to: '2026-11-08', statuses: ['open', 'done'], folder: null }])
    expect(count(el)).toBe('5 notes · 3 tasks in October')
    // happy-dom lays nothing out: a month cell has its minimum two lines, so
    // the first note and a `+N` for the other four lines.
    expect(drawn(el, '2026-10-31')).toEqual(['Acme sync'])
    expect(cell(el, '2026-10-31').querySelector('.more')!.textContent).toBe('+4')
    // The popover lists everything: the day note, the notes, then the tasks.
    press('Enter')
    await settle(el)
    const pop = root(el).querySelector('.popover')!
    expect([...pop.querySelectorAll('.t')].map((n) => n.textContent)).toEqual([
      'Daily note',
      'Acme sync',
      'Retro',
      'Send the deck',
      'Book the room #ops',
      'Draft agenda',
    ])
    expect([...pop.querySelectorAll('.placed')].map((n) => n.textContent)).toEqual(['Due', 'Scheduled', 'Done'])
    expect(pop.querySelector('.task .time')!.textContent).toBe('2:30 PM')
  })

  it('draws tasks in the week grid with their time', async () => {
    const el = await mount({ label: 'All notes' }, 1200, source())
    press('w')
    await settle(el)
    expect(calls[calls.length - 1]).toEqual({ from: '2026-10-26', to: '2026-11-01', statuses: ['open', 'done'], folder: null })
    expect(drawn(el, '2026-10-31')).toEqual(['Acme sync', 'Retro', 'Send the deck', 'Book the room #ops', 'Draft agenda'])
    const entries = [...weekGrid(el)!.shadowRoot!.querySelectorAll<HTMLAnchorElement>('[data-date="2026-10-31"] .entry')]
    expect(entries[2].getAttribute('href')).toBe('/meetings/2026-10-31 Acme sync/#mbr-task-3')
    expect(entries[2].querySelector('.check.open')).not.toBeNull()
    expect(entries[4].querySelector('.check.done')).not.toBeNull()
    expect(entries[3].getAttribute('title')).toContain('Scheduled')
    const timed = [...weekGrid(el)!.shadowRoot!.querySelectorAll('[data-date="2026-10-31"] .entry')].find((a) =>
      a.textContent!.includes('Book the room')
    )!
    expect(timed.textContent).toContain('2:30')
  })

  it('Due asks for open tasks only; None asks for nothing and hides them', async () => {
    const el = await mount({ label: 'All notes' }, 1200, source())
    await choose(el, 'tasks', 'due')
    expect(calls[calls.length - 1]!.statuses).toEqual(['open'])
    expect(count(el)).toBe('5 notes · 2 tasks in October')
    expect(root(el).querySelector('.options-btn .badge')).not.toBeNull()
    const before = calls.length
    await choose(el, 'tasks', 'none')
    expect(calls.length).toBe(before)
    expect(count(el)).toBe('5 notes in October')
    press('Escape')
    press('w')
    press('a')
    await settle(el)
    expect(calls.length).toBe(before)
  })

  it('"This folder" asks the server for the folder; an explicit scope keeps only its notes\' tasks', async () => {
    const el = await mount({ label: 'All notes', pagePath: '/blog/2026-10-12-launch/' }, 1200, source())
    await choose(el, 'scope', 'folder')
    expect(calls[calls.length - 1]!.folder).toBe('/blog/')
    expect(count(el)).toBe('1 note · 0 tasks in October')
    el.scope = { label: 'Search: retro', files: FILES.filter((f) => f.raw_path.includes('Retro')) }
    await settle(el)
    expect(count(el)).toBe('1 note · 0 tasks in October')
    el.scope = { label: 'Search: acme', files: FILES.filter((f) => f.raw_path.includes('Acme')) }
    await settle(el)
    expect(count(el)).toBe('1 note · 3 tasks in October')
  })

  it('filters tasks by text, note and #tag', async () => {
    const el = await mount({ label: 'All notes' }, 1200, source())
    const filter = async (text: string) => {
      const input = root(el).querySelector<HTMLInputElement>('input.filter')!
      input.value = text
      input.dispatchEvent(new Event('input'))
      await new Promise((r) => setTimeout(r, 150))
      await settle(el)
    }
    await filter('#ops')
    expect(count(el)).toBe('0 of 5 notes · 1 of 3 tasks in October')
    expect(drawn(el, '2026-10-31')).toEqual(['Book the room #ops'])
    await filter('acme')
    expect(count(el)).toBe('1 of 5 notes · 3 of 3 tasks in October')
  })

  it('asks again under a new source, once per range', async () => {
    const el = await mount({ label: 'All notes' }, 1200, source())
    expect(calls.length).toBe(1)
    press('t')
    await settle(el)
    expect(calls.length).toBe(1)
    el.taskSource = source()
    await settle(el)
    expect(calls.length).toBe(2)
    press(']')
    await settle(el)
    expect(calls[calls.length - 1]).toMatchObject({ from: '2026-10-26', to: '2026-12-06' })
  })

  it('lists tasks after the notes in List, and Enter opens one', async () => {
    const el = await mount({ label: 'All notes' }, 390, source())
    expect(calls[calls.length - 1]).toMatchObject({ from: '2026-10-01', to: '2026-10-31' })
    const rows = [...root(el).querySelectorAll('.list .row .t')].map((n) => n.textContent)
    expect(rows).toEqual(['Launch', 'Daily note', 'Daily note', 'Acme sync', 'Retro', 'Send the deck', 'Book the room #ops', 'Draft agenda'])
    for (let i = 0; i < 5; i++) press('j')
    press('Enter')
    expect(navigate).toHaveBeenCalledWith('/meetings/2026-10-31 Acme sync/#mbr-task-3')
  })
})
