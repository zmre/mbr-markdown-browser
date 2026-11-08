import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mountQuickLookCalendar, type MountedCalendar } from './mount.ts'
import { QUICKLOOK_CALENDAR_GLOBAL, COLUMN_MIN_WIDTH } from './model.ts'
import { BODY_COLUMN_CLASS, STYLE_ID } from './styles.ts'
import { PX_PER_MINUTE } from '../calendar/agenda-layout.ts'

/** What QuickLook renders for a day note's `## Calendar` section, plus tasks. */
const NOTE = `<h2 id="calendar">Calendar</h2>
<ul>
<li>09:00 - 09:15 Standup
<ul>
<li>Bob Wall, Patrick Walsh</li>
<li>Slack call</li>
</ul>
</li>
<li>09:00 - 10:00 Overlapping review</li>
<li>14:00 Demos</li>
</ul>
<ul>
<li><input type="checkbox" class="mbr-task-check" data-mbr-task-status="done" checked disabled> <span class="mbr-task-text">16:00 Ship it</span></li>
</ul>
<blockquote><ul><li>11:00 Quoted is not an entry</li></ul></blockquote>`

const globals = window as unknown as Record<string, unknown>
let mounted: MountedCalendar | null = null

function setWidth(width: number): void {
  Object.defineProperty(document.documentElement, 'clientWidth', { value: width, configurable: true })
}

function setUp(html: string, payload: unknown, width = 1200): void {
  document.body.innerHTML = `<header class="container"></header><main id="wrapper" class="container">${html}</main>`
  globals[QUICKLOOK_CALENDAR_GLOBAL] = payload
  setWidth(width)
}

/** 2026-11-03 at 09:30 local time. */
const at = (day: number, h: number, m = 0) => () => new Date(2026, 10, day, h, m)

beforeEach(() => {
  document.getElementById(STYLE_ID)?.remove()
})

afterEach(() => {
  mounted?.dispose()
  mounted = null
  delete globals[QUICKLOOK_CALENDAR_GLOBAL]
  document.body.className = ''
})

describe('mountQuickLookCalendar', () => {
  it('draws nothing without the payload', () => {
    setUp(NOTE, undefined)
    expect(mountQuickLookCalendar({ now: at(3, 9, 30) })).toBeNull()
    expect(document.querySelector('.mbr-ql-cal')).toBeNull()
    expect(document.getElementById(STYLE_ID)).toBeNull()
  })

  it('draws nothing when the prefilter let through a note the grammar rejects', () => {
    setUp('<ul><li>9:00</li><li>10:30am-ish maybe</li></ul>', { date: '2026-11-03' })
    expect(mountQuickLookCalendar({ now: at(3, 9, 30) })).toBeNull()
    expect(document.querySelector('.mbr-ql-cal')).toBeNull()
  })

  it('draws the header, the month with the day marked and one block per entry', () => {
    setUp(NOTE, { date: '2026-11-03' })
    mounted = mountQuickLookCalendar({ now: at(5, 9, 30) })
    const cal = mounted!.calendar
    expect(document.querySelector('main#wrapper')!.firstElementChild).toBe(cal)
    expect(cal.querySelector('time')!.getAttribute('datetime')).toBe('2026-11-03')
    expect(cal.querySelector('.mbr-ql-cal-weekday')!.textContent).toBe('Tuesday')
    expect(cal.querySelector('.mbr-ql-cal-today-badge')).toBeNull()

    const selected = cal.querySelectorAll('.mbr-ql-cal-day.selected')
    expect(Array.from(selected).map((d) => (d as HTMLElement).dataset.date)).toEqual(['2026-11-03'])
    const today = cal.querySelectorAll('.mbr-ql-cal-day.today')
    expect(Array.from(today).map((d) => (d as HTMLElement).dataset.date)).toEqual(['2026-11-05'])
    expect(cal.querySelectorAll('.mbr-ql-cal-row:not(.labels)')).toHaveLength(6)
    expect(cal.querySelector('.mbr-ql-cal-wk')).toBeNull()

    const blocks = Array.from(cal.querySelectorAll<HTMLElement>('.mbr-ql-cal-block'))
    expect(blocks.map((b) => b.querySelector('.mbr-ql-cal-title')!.textContent)).toEqual([
      'Standup',
      'Overlapping review',
      'Demos',
      'Ship it',
    ])
    // Details as muted lines; an entry without an end is dashed; the task's state.
    expect(Array.from(blocks[0].querySelectorAll('.mbr-ql-cal-detail')).map((d) => d.textContent)).toEqual([
      'Bob Wall, Patrick Walsh',
      'Slack call',
    ])
    expect(blocks[2].classList.contains('no-end')).toBe(true)
    expect(blocks[3].classList.contains('done')).toBe(true)
    expect(blocks[3].querySelector('.mbr-ql-cal-check')).not.toBeNull()
    // The two 09:00 entries overlap: side by side, half width each.
    // Two overlapping blocks share the width: columns 0 and 1 of 2 (styles.ts places them).
    expect([blocks[0].style.getPropertyValue('--col'), blocks[0].style.getPropertyValue('--cols')]).toEqual(['0', '2'])
    expect([blocks[1].style.getPropertyValue('--col'), blocks[1].style.getPropertyValue('--cols')]).toEqual(['1', '2'])
    // Not today: no current-time line, no now/next marks.
    expect(cal.querySelector('.mbr-ql-cal-now')).toBeNull()
    expect(document.querySelector('li.mbr-agenda-now, li.mbr-agenda-next')).toBeNull()
    // Nothing to navigate.
    expect(cal.querySelector('a, button, input')).toBeNull()
    expect(cal.querySelectorAll('.mbr-ql-cal-sr-only li')).toHaveLength(4)
  })

  it('draws the current-time line and marks now/next only on today', () => {
    setUp(NOTE, { date: '2026-11-03' })
    mounted = mountQuickLookCalendar({ now: at(3, 9, 30) })
    const cal = mounted!.calendar
    expect(cal.querySelector('.mbr-ql-cal-today-badge')!.textContent).toBe('Today')
    const timeline = cal.querySelector<HTMLElement>('.mbr-ql-cal-timeline')!
    const from = Number(timeline.dataset.from)
    expect(cal.querySelector<HTMLElement>('.mbr-ql-cal-now')!.style.top).toBe(`${(570 - from) * PX_PER_MINUTE}px`)
    expect(document.querySelector('li.mbr-agenda-now')!.textContent).toContain('Overlapping review')
    expect(document.querySelector('li.mbr-agenda-next')!.textContent).toContain('Demos')
  })

  it('leaves the now/next marks off when agenda_chips is off', () => {
    setUp(NOTE, { date: '2026-11-03', agenda_chips: false })
    mounted = mountQuickLookCalendar({ now: at(3, 9, 30) })
    expect(mounted!.calendar.querySelector('.mbr-ql-cal-now')).not.toBeNull()
    expect(document.querySelector('li.mbr-agenda-now, li.mbr-agenda-next')).toBeNull()
  })

  it('shows ISO week numbers when the repository asks for them', () => {
    setUp(NOTE, { date: '2026-11-03', week_numbers: true, week_start: 'monday' })
    mounted = mountQuickLookCalendar({ now: at(3, 9) })
    const weeks = Array.from(mounted!.calendar.querySelectorAll('.mbr-ql-cal-row:not(.labels) .mbr-ql-cal-wk'))
    expect(weeks.map((w) => w.textContent)).toEqual(['44', '45', '46', '47', '48', '49'])
  })

  it('is a right column on a wide panel and moves above the note when it narrows', () => {
    setUp(NOTE, { date: '2026-11-03' }, COLUMN_MIN_WIDTH + 200)
    mounted = mountQuickLookCalendar({ now: at(3, 9) })
    const cal = mounted!.calendar
    expect(mounted!.placement()).toBe('column')
    expect(cal.dataset.placement).toBe('column')
    expect(document.body.classList.contains(BODY_COLUMN_CLASS)).toBe(true)

    setWidth(500)
    window.dispatchEvent(new Event('resize'))
    expect(mounted!.placement()).toBe('top')
    expect(cal.dataset.placement).toBe('top')
    expect(document.body.classList.contains(BODY_COLUMN_CLASS)).toBe(false)
  })

  it('injects its stylesheet once and removes itself on dispose', () => {
    setUp(NOTE, { date: '2026-11-03' })
    mounted = mountQuickLookCalendar({ now: at(3, 9) })
    mounted!.dispose()
    mounted = mountQuickLookCalendar({ now: at(3, 9) })
    expect(document.querySelectorAll(`#${STYLE_ID}`)).toHaveLength(1)
    expect(document.querySelectorAll('.mbr-ql-cal')).toHaveLength(1)
    mounted!.dispose()
    mounted = null
    expect(document.querySelector('.mbr-ql-cal')).toBeNull()
  })
})
