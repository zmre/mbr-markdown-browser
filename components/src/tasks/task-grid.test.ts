import { describe, expect, it } from 'vitest'
import { effectiveView, gridDayMarks, gridEntries, gridRange, gridTitle, stepGrid } from './task-grid.ts'
import { makeGridHit } from './test-fixtures.ts'

describe('task-grid', () => {
  it('covers the week from the week start, or the month’s six rows', () => {
    expect(gridRange('week', '2026-11-07', 1)).toEqual({ from: '2026-11-02', to: '2026-11-08' })
    expect(gridRange('week', '2026-11-07', 0)).toEqual({ from: '2026-11-01', to: '2026-11-07' })
    expect(gridRange('month', '2026-11-07', 0)).toEqual({ from: '2026-11-01', to: '2026-12-12' })
    expect(gridRange('month', '2026-02-10', 1)).toEqual({ from: '2026-01-26', to: '2026-03-08' })
  })

  it('pages by a week or a calendar month', () => {
    expect(stepGrid('week', '2026-11-07', 1)).toBe('2026-11-14')
    expect(stepGrid('month', '2026-01-31', 1)).toBe('2026-02-28')
  })

  it('falls back from Month to Week only on a narrow screen', () => {
    expect(effectiveView('month', true)).toBe('week')
    expect(effectiveView('month', false)).toBe('month')
    expect(effectiveView('list', true)).toBe('list')
  })

  it('titles the view in the reader’s locale', () => {
    expect(gridTitle('month', '2026-11-07', 1, 'en-US')).toBe('November 2026')
    expect(gridTitle('week', '2026-11-07', 1, 'en-US')).toBe('Nov 2 – Nov 8, 2026')
  })

  it('labels a time only when the text does not already start with it', () => {
    const day = {
      tasks: [
        makeGridHit({ text: 'call Bob', line: 1, time: '14:00', note: 'Plan' }),
        makeGridHit({ text: '9:30 standup', line: 2, time: '09:30' }),
      ],
      open: 2,
      done: 0,
      overdue: false,
    }
    const entries = gridEntries(day, (h) => h.status, (h) => `/x/#mbr-task-${h.line}`)
    expect(entries.map((e) => [e.key, e.title, e.time, e.href, e.tooltip])).toEqual([
      ['notes.md:1', 'call Bob', '14:00', '/x/#mbr-task-1', 'call Bob — Plan'],
      ['notes.md:2', '9:30 standup', undefined, '/x/#mbr-task-2', '9:30 standup — Notes'],
    ])
    expect(gridEntries(undefined, (h) => h.status, () => '')).toEqual([])
  })

  it('marks a day with its open count and overdue colour, never as notes', () => {
    expect(gridDayMarks({ tasks: [], open: 2, done: 1, overdue: true })).toEqual({ notes: 0, open: 2, overdue: true })
    expect(gridDayMarks(undefined)).toEqual({ notes: 0 })
  })
})
