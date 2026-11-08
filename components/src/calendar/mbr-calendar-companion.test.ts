import { afterEach, describe, expect, it, vi } from 'vitest'
import type { MarkdownFile } from '../sorting.ts'
import { agendaEntries } from './agenda-parse.ts'
import { DEFAULT_CALENDAR_CONFIG } from './config.ts'
import { clearDayAgendaCache } from './day-agenda.ts'
import { COMPANION_CLOSE_EVENT, type DayTaskCount, type TaskMarkerSource } from './types.ts'
import './mbr-calendar-companion.ts'
import type { MbrCalendarCompanionElement } from './mbr-calendar-companion.ts'
import type { MbrCalendarMonthElement } from './mbr-calendar-month.ts'

const day = (date: string, extra: Partial<MarkdownFile> = {}): MarkdownFile => ({
  url_path: `/daily/${date}/`,
  raw_path: `daily/${date}.md`,
  created: 0,
  modified: 0,
  frontmatter: null,
  calendar: { date, source: 'filename', period: date },
  ...extra,
})

const FILES: MarkdownFile[] = [
  day('2026-10-30'),
  day('2026-10-31'),
  {
    url_path: '/meetings/2026-10-31 Acme sync/',
    raw_path: 'meetings/2026-10-31 Acme sync.md',
    created: 0,
    modified: 0,
    frontmatter: { type: 'meeting', title: 'Acme sync' },
    calendar: { date: '2026-10-31', source: 'filename' },
  },
  {
    url_path: '/weekly/2026-W44/',
    raw_path: 'weekly/2026-W44.md',
    created: 0,
    modified: 0,
    frontmatter: null,
    calendar: { date: '2026-10-26', source: 'filename', period: '2026-W44' },
  },
]

const TODAY_NOTE = `<ul>
<li>09:00 - 09:15 Standup<ul><li>Bob Wall</li></ul></li>
<li>10:30 - 11:00 Bob / Patrick 1:1</li>
</ul>`

const YESTERDAY_PAGE = `<html><body><main id="wrapper"><ul><li>16:00 - 17:00 Retro</li></ul></main></body></html>`

interface Setup {
  taskMarkers?: TaskMarkerSource | null
  pagePeriod?: string | null
  currentPath?: string
  body?: string
  fetchPageHtml?: (url: string) => Promise<string>
}

async function mount(setup: Setup = {}) {
  const root = document.createElement('main')
  root.id = 'wrapper'
  root.innerHTML = setup.body ?? TODAY_NOTE
  document.body.append(root)
  const navigate = vi.fn()
  const reveal = vi.fn()
  const fetchPageHtml = vi.fn(setup.fetchPageHtml ?? (() => Promise.resolve(YESTERDAY_PAGE)))
  const el = document.createElement('mbr-calendar-companion') as MbrCalendarCompanionElement
  Object.assign(el, {
    placement: 'docked',
    pageEntries: agendaEntries(root),
    siteIndex: () => Promise.resolve({ markdown_files: FILES }),
    fetchPageHtml,
    resolveUrl: (p: string) => `/base${p}`,
    navigate,
    reveal,
    config: { ...DEFAULT_CALENDAR_CONFIG, weekStart: 'monday' },
    currentPath: setup.currentPath ?? '/daily/2026-10-31/',
    pagePeriod: setup.pagePeriod === undefined ? '2026-10-31' : setup.pagePeriod,
    locale: 'en-US',
    now: () => new Date(2026, 9, 31, 10, 40),
    taskMarkers: setup.taskMarkers ?? null,
  })
  document.body.append(el)
  await settle(el)
  return { el, root, navigate, reveal, fetchPageHtml }
}

async function settle(el: MbrCalendarCompanionElement): Promise<void> {
  for (let i = 0; i < 4; i++) {
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }
}

const q = <T extends Element = HTMLElement>(el: Element, selector: string): T | null =>
  el.shadowRoot!.querySelector<T>(selector)
const qa = (el: Element, selector: string): HTMLElement[] =>
  Array.from(el.shadowRoot!.querySelectorAll<HTMLElement>(selector))
const month = (el: Element) => q<MbrCalendarMonthElement>(el, 'mbr-calendar-month')!

afterEach(() => {
  document.body.innerHTML = ''
  clearDayAgendaCache()
})

describe('<mbr-calendar-companion> on a day note', () => {
  it('shows the page’s own agenda, with the current-time line on today', async () => {
    const { el } = await mount()
    expect(q(el, '.agenda h3 time')!.getAttribute('datetime')).toBe('2026-10-31')
    expect(qa(el, '.block .title').map((b) => b.textContent)).toEqual(['Standup', 'Bob / Patrick 1:1'])
    expect(qa(el, '.block .detail').map((b) => b.textContent)).toEqual(['Bob Wall'])
    expect(q(el, '.now')).not.toBeNull()
    // No "Open note" link to the page you are on.
    expect(q(el, '.agenda .open-note')).toBeNull()
    // The screen-reader list mirrors the timeline.
    expect(qa(el, 'ol.sr-only li')).toHaveLength(2)
  })

  it('marks a task block by its status, and only an endless entry as open-ended', async () => {
    const { el } = await mount({
      body: '<ul><li><input type="checkbox" class="mbr-task-check" data-mbr-task-status="open" disabled> <span class="mbr-task-text">09:00 - 10:00 Timed task</span></li><li>11:00 Endless</li></ul>',
    })
    const [task, endless] = qa(el, '.block')
    expect(task.className).toBe('block task open')
    expect(endless.className).toBe('block no-end')
  })

  it('reveals a clicked entry of this page instead of navigating', async () => {
    const { el, root, reveal, navigate } = await mount()
    qa(el, '.block')[1].click()
    expect(reveal).toHaveBeenCalledWith(root.querySelectorAll('li')[2])
    expect(navigate).not.toHaveBeenCalled()
  })

  it('lists the notes from this day, without the day note itself', async () => {
    const { el } = await mount()
    const links = qa(el, '.notes a')
    expect(links.map((a) => [a.textContent, a.getAttribute('href')])).toEqual([
      ['Acme sync', '/base/meetings/2026-10-31 Acme sync/'],
    ])
    expect(q(el, '.notes h4')!.textContent).toBe('meeting')
  })

  it('marks the mini month: day notes, dots, today and the shown day', async () => {
    const { el } = await mount()
    const m = month(el)
    await m.updateComplete
    const cell = (date: string) => m.shadowRoot!.querySelector<HTMLElement>(`[data-date="${date}"]`)!
    expect(cell('2026-10-30').classList.contains('daily')).toBe(true)
    expect(cell('2026-10-29').classList.contains('daily')).toBe(false)
    expect(cell('2026-10-31').classList.contains('selected')).toBe(true)
    expect(cell('2026-10-31').classList.contains('today')).toBe(true)
    expect(cell('2026-10-31').querySelectorAll('.dots i')).toHaveLength(1)
    expect(cell('2026-10-31').getAttribute('aria-label')).toBe(
      'Saturday, October 31, 2026, today, daily note, 1 other note'
    )
    // Week numbers default on: the repository has a week note.
    const w44 = m.shadowRoot!.querySelector<HTMLAnchorElement>('a.wk')!
    expect(w44.textContent).toBe('44')
    expect(w44.getAttribute('href')).toBe('/base/weekly/2026-W44/')
  })
})

describe('another day (the fetched path)', () => {
  it('fetches the day note’s HTML and parses its agenda with the same parser', async () => {
    const { el, fetchPageHtml, navigate } = await mount()
    month(el).dispatchEvent(
      new CustomEvent('mbr-calendar-select', { detail: { date: '2026-10-30' }, bubbles: true, composed: true })
    )
    await settle(el)
    expect(fetchPageHtml).toHaveBeenCalledWith('/base/daily/2026-10-30/')
    expect(qa(el, '.block .title').map((b) => b.textContent)).toEqual(['Retro'])
    expect(q(el, '.agenda .open-note')!.getAttribute('href')).toBe('/base/daily/2026-10-30/')
    // Not today: no current-time line.
    expect(q(el, '.now')).toBeNull()

    qa(el, '.block')[0].click()
    expect(navigate).toHaveBeenCalledWith('/base/daily/2026-10-30/#mbr-agenda-1')

    // Back and forth again: served from the per-page cache.
    el.showDay('2026-10-31')
    el.showDay('2026-10-30')
    await settle(el)
    expect(fetchPageHtml).toHaveBeenCalledTimes(1)
  })

  it('opens a fetched entry at its own ordinal, with no DOM kept from the fetch', async () => {
    const page = `<html><body><main id="wrapper"><ul><li>09:00 First</li><li>not a time</li><li>14:00 Second</li></ul></main></body></html>`
    const { el, navigate } = await mount({ fetchPageHtml: () => Promise.resolve(page) })
    el.showDay('2026-10-30')
    await settle(el)
    const blocks = qa(el, '.block')
    expect(blocks.map((b) => b.querySelector('.title')?.textContent)).toEqual(['First', 'Second'])
    blocks[1].click()
    expect(navigate).toHaveBeenCalledWith('/base/daily/2026-10-30/#mbr-agenda-2')
  })

  it('says so when the day has no note, and when the fetch fails', async () => {
    const { el } = await mount({ fetchPageHtml: () => Promise.reject(new Error('offline')) })
    el.showDay('2026-10-29')
    await settle(el)
    expect(q(el, '.agenda .muted')!.textContent).toBe('No note for this day.')

    el.showDay('2026-10-30')
    await settle(el)
    expect(q(el, '.agenda .muted')!.textContent).toBe("Could not load this day's note.")
  })

  it('opens the day note on Enter / double-click (activate)', async () => {
    const { el, navigate } = await mount()
    month(el).dispatchEvent(
      new CustomEvent('mbr-calendar-activate', { detail: { date: '2026-10-30' }, bubbles: true, composed: true })
    )
    expect(navigate).toHaveBeenCalledWith('/base/daily/2026-10-30/')
  })
})

describe('on other notes', () => {
  it('titles a non-day note’s own times "Times in this note", with no date claim', async () => {
    const { el } = await mount({ pagePeriod: null, currentPath: '/health/vitals/', body: '<ul><li>07:30 BP 118/76</li></ul>' })
    expect(q(el, '.agenda h3')!.textContent).toBe('Times in this note')
    expect(q(el, '.agenda h3 time')).toBeNull()
    expect(q(el, '.now')).toBeNull()
    expect(q(el, '.notes')).toBeNull()
    // Today switches to today's day note.
    q<HTMLButtonElement>(el, '.today')!.click()
    await settle(el)
    expect(q(el, '.agenda h3 time')!.getAttribute('datetime')).toBe('2026-10-31')
  })

  it('shows today on a note without times', async () => {
    const { el } = await mount({ pagePeriod: null, currentPath: '/plain/', body: '<p>Nothing</p>' })
    expect(q(el, '.agenda h3 time')!.getAttribute('datetime')).toBe('2026-10-31')
  })

  it('opens a week note on its month', async () => {
    const { el } = await mount({ pagePeriod: '2026-W01', currentPath: '/weekly/2026-W01/', body: '<p>x</p>' })
    expect(month(el).month).toBe('2025-12')
  })
})

describe('closing', () => {
  it('asks its host to close from the ✕, and from Escape in a drawer', async () => {
    const { el } = await mount()
    const closed = vi.fn()
    document.addEventListener(COMPANION_CLOSE_EVENT, closed)
    const x = qa(el, 'button[aria-label="Close the calendar"]')[0]
    x.click()
    expect(closed).toHaveBeenCalledTimes(1)

    el.placement = 'drawer'
    await el.updateComplete
    q(el, '.panel')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, composed: true }))
    expect(closed).toHaveBeenCalledTimes(2)
    expect(q(el, '.backdrop')).not.toBeNull()
    document.removeEventListener(COMPANION_CLOSE_EVENT, closed)
  })
})

describe('<mbr-calendar-companion> task markers (§8.3)', () => {
  it('rings the days with open tasks and says how many are due on the shown day', async () => {
    const source = vi.fn<TaskMarkerSource>(() =>
      Promise.resolve(
        new Map<string, DayTaskCount>([
          ['2026-10-30', { open: 2, overdue: true }],
          ['2026-10-31', { open: 3, overdue: false }],
        ])
      )
    )
    const { el } = await mount({ taskMarkers: source })
    await settle(el)
    // The six weeks of October 2026 from a Monday start, at idle.
    expect(source).toHaveBeenCalledWith('2026-09-28', '2026-11-08')
    const grid = month(el)
    expect(grid.dayMarks('2026-10-30')).toMatchObject({ open: 2, overdue: true })
    // Drawn, not just answered: the counts arrive after the first render.
    await grid.updateComplete
    expect(grid.shadowRoot!.querySelector('[data-date="2026-10-30"] .ring.overdue')).not.toBeNull()
    expect(grid.shadowRoot!.querySelector('[data-date="2026-10-31"] .ring:not(.overdue)')).not.toBeNull()
    expect(q(el, '.agenda .due')?.textContent?.trim()).toBe('3 tasks due')
  })

  it('draws a past day plain when the source says it is not overdue (scheduled-only)', async () => {
    const source = vi.fn<TaskMarkerSource>(() =>
      Promise.resolve(new Map<string, DayTaskCount>([['2026-10-20', { open: 2, overdue: false }]]))
    )
    const { el } = await mount({ taskMarkers: source })
    await settle(el)
    const grid = month(el)
    await grid.updateComplete
    expect(grid.dayMarks('2026-10-20')).toMatchObject({ open: 2, overdue: false })
    expect(grid.shadowRoot!.querySelector('[data-date="2026-10-20"] .ring:not(.overdue)')).not.toBeNull()
  })

  it('asks again when the source is replaced (a task write made it stale)', async () => {
    const first = vi.fn<TaskMarkerSource>(() => Promise.resolve(new Map()))
    const { el } = await mount({ taskMarkers: first })
    await settle(el)
    expect(q(el, '.agenda .due')).toBeNull()
    const second = vi.fn<TaskMarkerSource>(() => Promise.resolve(new Map([['2026-10-31', { open: 1, overdue: false }]])))
    el.taskMarkers = second
    await settle(el)
    expect(second).toHaveBeenCalledTimes(1)
    expect(q(el, '.agenda .due')?.textContent?.trim()).toBe('1 task due')
  })

  it('draws no rings and asks nothing without a source (static builds)', async () => {
    const { el } = await mount({ taskMarkers: null })
    expect(month(el).dayMarks('2026-10-30')).not.toHaveProperty('open')
    expect(q(el, '.agenda .due')).toBeNull()
  })
})

