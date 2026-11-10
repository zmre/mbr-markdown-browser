import { afterEach, describe, expect, it, vi } from 'vitest'
import { agendaEntries } from './agenda-parse.ts'
import {
  AGENDA_LINE_CLASS,
  AGENDA_NEXT_CLASS,
  AGENDA_NOW_CLASS,
  AGENDA_TIME_HIGHLIGHT,
  agendaIdFromHash,
  enhancePage,
  nowAndNext,
  wrapAnonymousLines,
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

describe('wrapAnonymousLines', () => {
  // The renderer's shapes: a tight item with sub-bullets (the one WebKit
  // paints no highlight in), a plain item, a task item and a loose item.
  const SHAPES = `<ul>
<li>09:00 - 09:15 Standup
<ul>
<li>Bob</li>
</ul>
</li>
<li>10:00 Plain</li>
<li><input type="checkbox" class="mbr-task-check" data-mbr-task-status="open" disabled> <span class="mbr-task-text">11:00 Task</span>
<ul>
<li>sub</li>
</ul>
</li>
<li>
<p>12:00 Loose</p>
<ul>
<li>sub</li>
</ul>
</li>
</ul>`

  const summary = (entries: ReturnType<typeof agendaEntries>) =>
    entries.map((e) => ({
      title: e.title,
      start: e.start,
      end: e.end,
      details: e.details,
      time: e.timeRange && e.timeRange.startContainer.data.slice(e.timeRange.startOffset, e.timeRange.endOffset),
    }))

  it('wraps only the bare leading line of a tight item beside a block, keeping its text nodes', () => {
    const root = mount(SHAPES)
    const entries = agendaEntries(root)
    expect(entries).toHaveLength(4)
    const timeNode = entries[0].timeRange!.startContainer
    const before = root.innerHTML

    wrapAnonymousLines(entries)

    const spans = root.querySelectorAll(`span.${AGENDA_LINE_CLASS}`)
    expect(spans).toHaveLength(1)
    const span = spans[0]
    expect(span.parentElement).toBe(entries[0].li)
    expect(entries[0].li.firstChild).toBe(span)
    expect(span.nextElementSibling?.tagName).toBe('UL')
    // Moved, not copied or split.
    expect(span.firstChild).toBe(timeNode)
    expect(timeNode.data).toBe('09:00 - 09:15 Standup\n')
    // The other shapes are untouched.
    expect(root.innerHTML).toBe(before.replace('09:00 - 09:15 Standup\n', `<span class="${AGENDA_LINE_CLASS}">09:00 - 09:15 Standup\n</span>`))
  })

  it('is idempotent, and the entries read the same after wrapping', () => {
    const root = mount(SHAPES)
    const entries = agendaEntries(root)
    const parsedBefore = summary(entries)
    wrapAnonymousLines(entries)
    const once = root.innerHTML
    wrapAnonymousLines(entries)
    wrapAnonymousLines(agendaEntries(root))
    expect(root.innerHTML).toBe(once)

    const reparsed = agendaEntries(root)
    expect(summary(reparsed)).toEqual(parsedBefore)
    expect(reparsed[0].timeRange!.startContainer).toBe(entries[0].timeRange!.startContainer)
  })

  it('leaves a tight item without a block child alone', () => {
    const root = mount(DAY)
    const before = root.innerHTML
    wrapAnonymousLines(agendaEntries(root))
    expect(root.innerHTML).toBe(before)
  })

  it('runs when chips are painted, not when they are off', () => {
    const registry = new Map<string, unknown>()
    vi.stubGlobal('CSS', { highlights: registry })
    vi.stubGlobal('Highlight', class {})
    vi.stubGlobal('StaticRange', class {})
    const off = mount(SHAPES)
    enhancePage({ root: off, agendaChips: false, pageDay: null, reveal: () => {} }).dispose()
    expect(off.querySelector(`.${AGENDA_LINE_CLASS}`)).toBeNull()

    const on = mount(SHAPES)
    const page = enhancePage({ root: on, agendaChips: true, pageDay: null, reveal: () => {} })
    expect(on.querySelectorAll(`.${AGENDA_LINE_CLASS}`)).toHaveLength(1)
    page.dispose()
    // The wrapper stays, like the ids.
    expect(on.querySelectorAll(`.${AGENDA_LINE_CLASS}`)).toHaveLength(1)
  })
})
