import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  AGENDA_NEXT_CLASS,
  AGENDA_NOW_CLASS,
  AGENDA_TIME_HIGHLIGHT,
  agendaIdFromHash,
  enhancePage,
  nowAndNext,
} from './agenda-page.ts'

const hm = (h: number, m = 0): number => h * 60 + m

function mount(html: string): HTMLElement {
  const el = document.createElement('main')
  el.id = 'wrapper'
  el.innerHTML = html
  document.body.append(el)
  return el
}

const DAY = `<ul>
<li>09:00 - 09:15 Standup</li>
<li>Not an entry</li>
<li>10:30 - 11:00 1:1</li>
<li>14:00 Demos</li>
</ul>`

afterEach(() => {
  document.body.innerHTML = ''
  window.location.hash = ''
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('nowAndNext', () => {
  const entries = [{ start: hm(9), end: hm(9, 15) }, { start: hm(10, 30), end: hm(11) }, { start: hm(14) }]
  it('finds the entry in progress and the next one', () => {
    expect(nowAndNext(entries, hm(9, 5))).toEqual({ now: entries[0], next: entries[1] })
    expect(nowAndNext(entries, hm(12))).toEqual({ now: null, next: entries[2] })
    // An entry without an end lasts half an hour.
    expect(nowAndNext(entries, hm(14, 20))).toEqual({ now: entries[2], next: null })
    expect(nowAndNext(entries, hm(14, 30))).toEqual({ now: null, next: null })
  })
})

describe('agendaIdFromHash', () => {
  it('accepts only agenda fragments', () => {
    expect(agendaIdFromHash('#mbr-agenda-3')).toBe('mbr-agenda-3')
    expect(agendaIdFromHash('#mbr-task-3')).toBeNull()
    expect(agendaIdFromHash('#mbr-agenda-3x')).toBeNull()
  })
})

describe('enhancePage', () => {
  it('numbers the agenda items and reveals a fragment once the ids exist', () => {
    window.location.hash = '#mbr-agenda-2'
    const root = mount(DAY)
    const reveal = vi.fn()
    const { entries, dispose } = enhancePage({ root, agendaChips: false, pageDay: null, reveal })
    expect(entries.map((e) => e.li.id)).toEqual(['mbr-agenda-1', 'mbr-agenda-2', 'mbr-agenda-3'])
    expect(reveal).toHaveBeenCalledWith(entries[1].li)
    dispose()
  })

  it('marks now and next on today’s day note only', () => {
    const root = mount(DAY)
    const now = () => new Date(2026, 9, 31, 10, 40)
    const today = enhancePage({ root, agendaChips: true, pageDay: '2026-10-31', reveal: () => {}, now })
    const classes = () => today.entries.map((e) => [e.li.classList.contains(AGENDA_NOW_CLASS), e.li.classList.contains(AGENDA_NEXT_CLASS)])
    expect(classes()).toEqual([
      [false, false],
      [true, false],
      [false, true],
    ])
    today.dispose()
    expect(classes().flat().some(Boolean)).toBe(false)

    const other = enhancePage({ root, agendaChips: true, pageDay: '2026-10-30', reveal: () => {}, now })
    expect(other.entries.some((e) => e.li.classList.contains(AGENDA_NOW_CLASS))).toBe(false)
    other.dispose()
  })

  it('paints the time chips through CSS.highlights when available', () => {
    const registry = new Map<string, unknown>()
    class FakeHighlight {
      ranges: unknown[]
      constructor(...ranges: unknown[]) {
        this.ranges = ranges
      }
    }
    class FakeStaticRange {
      constructor(public init: { startContainer: Text; startOffset: number; endOffset: number }) {}
    }
    vi.stubGlobal('CSS', { highlights: registry })
    vi.stubGlobal('Highlight', FakeHighlight)
    vi.stubGlobal('StaticRange', FakeStaticRange)

    const root = mount(DAY)
    const { dispose } = enhancePage({ root, agendaChips: true, pageDay: null, reveal: () => {} })
    const highlight = registry.get(AGENDA_TIME_HIGHLIGHT) as FakeHighlight
    const texts = highlight.ranges.map((r) => {
      const { startContainer, startOffset, endOffset } = (r as FakeStaticRange).init
      return startContainer.data.slice(startOffset, endOffset)
    })
    expect(texts).toEqual(['09:00 - 09:15', '10:30 - 11:00', '14:00'])
    dispose()
    expect(registry.has(AGENDA_TIME_HIGHLIGHT)).toBe(false)
  })

  it('paints nothing when agenda_chips is off', () => {
    const registry = new Map<string, unknown>()
    vi.stubGlobal('CSS', { highlights: registry })
    vi.stubGlobal('Highlight', class {})
    const root = mount(DAY)
    enhancePage({ root, agendaChips: false, pageDay: '2026-10-31', reveal: () => {} }).dispose()
    expect(registry.size).toBe(0)
  })
})
