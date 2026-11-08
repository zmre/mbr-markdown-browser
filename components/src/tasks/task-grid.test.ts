import { describe, expect, it } from 'vitest'
import { moveDay, viewTitle } from '../calendar/month-grid.ts'
import { effectiveView, gridDayMarks, gridEntries, gridHiddenCount, gridRange } from './task-grid.ts'
import { makeGridHit } from './test-fixtures.ts'

describe('task-grid', () => {
  it('covers the week from the week start, or the month’s six rows', () => {
    expect(gridRange('week', '2026-11-07', 1)).toEqual({ from: '2026-11-02', to: '2026-11-08' })
    expect(gridRange('week', '2026-11-07', 0)).toEqual({ from: '2026-11-01', to: '2026-11-07' })
    expect(gridRange('month', '2026-11-07', 0)).toEqual({ from: '2026-11-01', to: '2026-12-12' })
    expect(gridRange('month', '2026-02-10', 1)).toEqual({ from: '2026-01-26', to: '2026-03-08' })
  })

  it('pages by a week or a calendar month (the shared key map)', () => {
    expect(moveDay('next', 'week', '2026-11-07')).toBe('2026-11-14')
    expect(moveDay('next', 'month', '2026-01-31')).toBe('2026-02-28')
  })

  it('falls back from Month to Week only on a narrow screen', () => {
    expect(effectiveView('month', true)).toBe('week')
    expect(effectiveView('month', false)).toBe('month')
    expect(effectiveView('list', true)).toBe('list')
  })

  it('titles the view in the reader’s locale', () => {
    expect(viewTitle('month', '2026-11-07', 1, 'en-US')).toBe('November 2026')
    expect(viewTitle('week', '2026-11-07', 1, 'en-US')).toBe('Nov 2 – Nov 8, 2026')
  })

  it('labels the time the server sent (only a placing date with a time has one)', () => {
    const day = {
      tasks: [
        makeGridHit({ text: 'call Bob', line: 1, time: '14:00', note: 'Plan' }),
        makeGridHit({ text: '9:30 standup', line: 2 }),
      ],
      open: 2,
      done: 0,
      overdue: false,
    }
    const entries = gridEntries(day, (h) => h.status, (h) => `/x/#mbr-task-${h.line}`)
    expect(entries.map((e) => [e.title, e.time, e.href, e.tooltip])).toEqual([
      ['call Bob', '14:00', '/x/#mbr-task-1', 'call Bob — Plan'],
      ['9:30 standup', undefined, '/x/#mbr-task-2', '9:30 standup — Notes'],
    ])
    expect(gridEntries(undefined, (h) => h.status, () => '')).toEqual([])
  })

  it('marks a day with its open count and overdue colour, never as notes', () => {
    const day = { tasks: [makeGridHit({ text: 'a', line: 1 }), makeGridHit({ text: 'b', line: 2 })], open: 2, done: 1, overdue: true }
    expect(gridDayMarks(day, ['open'])).toEqual({ notes: 0, open: 2, overdue: true, hidden: 0 })
    expect(gridDayMarks(undefined, ['open'])).toEqual({ notes: 0 })
  })

  it('takes the overdue colour from the server alone, never from the date', () => {
    // A past day whose open tasks are all placed by their scheduled date: the
    // server says not overdue, and nothing here second-guesses it by date.
    const scheduled = makeGridHit({ text: 'plan', line: 1 })
    const past = { tasks: [{ ...scheduled, placed_by: 'scheduled' as const }], open: 1, done: 0, overdue: false }
    expect(gridDayMarks(past, ['open']).overdue).toBe(false)
  })

  it('counts the rows the server’s limit dropped, against the statuses on screen', () => {
    const open = (line: number) => makeGridHit({ text: `o${line}`, line, status: 'open' })
    const done = (line: number) => makeGridHit({ text: `d${line}`, line, status: 'done' })
    // 5 open, 3 done placed; 2 open rows sent.
    const day = { tasks: [open(1), open(2)], open: 5, done: 3, overdue: false }
    expect(gridHiddenCount(day, ['open'])).toBe(3)
    expect(gridHiddenCount(day, [])).toBe(3)
    // The done ones are filtered out, not dropped: never "more".
    expect(gridHiddenCount({ ...day, tasks: [open(1), open(2), open(3), open(4), open(5)] }, ['open'])).toBe(0)
    expect(gridHiddenCount({ ...day, tasks: [open(1), done(2)] }, ['open', 'done'])).toBe(6)
    expect(gridHiddenCount({ ...day, tasks: [done(1)] }, ['done'])).toBe(2)
    expect(gridHiddenCount(undefined, ['open'])).toBe(0)
    expect(gridDayMarks(day, ['open']).hidden).toBe(3)
  })
})
