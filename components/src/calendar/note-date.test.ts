import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { addDays, dateKey, localDateOf, noteDate, parseDateKey, todayKey } from './note-date.js'

/**
 * `localDateOf` must give the reader's *local* calendar date. Node re-reads
 * `process.env.TZ` on assignment, so each test pins a zone with a DST change
 * (America/New_York: 2026-03-08 02:00 → 03:00, 2026-11-01 02:00 → 01:00).
 */
describe('note-date', () => {
  const originalTz = process.env.TZ
  beforeEach(() => {
    process.env.TZ = 'America/New_York'
  })
  afterEach(() => {
    process.env.TZ = originalTz
  })

  it('formats and validates date keys', () => {
    expect(dateKey(2026, 1, 5)).toBe('2026-01-05')
    expect(parseDateKey('2024-02-29')).toEqual({ year: 2024, month: 2, day: 29 })
    expect(parseDateKey('2026-02-30')).toBeNull()
    expect(parseDateKey('2026-2-3')).toBeNull()
    expect(parseDateKey('20261031')).toBeNull()
  })

  it('reads created in the local zone, not UTC', () => {
    // 2026-11-01T02:30:00Z is still Oct 31 in New York (EDT, UTC-4).
    expect(localDateOf(1_793_500_200)).toBe('2026-10-31')
    process.env.TZ = 'UTC'
    expect(localDateOf(1_793_500_200)).toBe('2026-11-01')
    expect(localDateOf(Number.NaN)).toBeNull()
    expect(localDateOf(undefined)).toBeNull()
  })

  it('uses calendar.date first and created only as the fallback', () => {
    expect(noteDate({ created: 1_793_500_200, calendar: { date: '2020-01-01', source: 'frontmatter' } })).toBe(
      '2020-01-01'
    )
    expect(noteDate({ created: 1_793_500_200 })).toBe('2026-10-31')
    expect(noteDate({ created: 1_793_500_200, calendar: null })).toBe('2026-10-31')
    expect(noteDate({})).toBeNull()
  })

  it('local midnights either side of the fall-back change are distinct days', () => {
    // 2026-11-01 is a 25-hour day in New York.
    const before = new Date(2026, 9, 31, 23, 59).getTime() / 1000
    const after = new Date(2026, 10, 1, 0, 1).getTime() / 1000
    const nextDay = new Date(2026, 10, 2, 0, 1).getTime() / 1000
    expect(localDateOf(before)).toBe('2026-10-31')
    expect(localDateOf(after)).toBe('2026-11-01')
    expect(localDateOf(nextDay)).toBe('2026-11-02')
    // The trap this module avoids: adding 24 h from local midnight of a
    // 25-hour day lands on the same date.
    const naive = new Date(new Date(2026, 10, 1).getTime() + 86_400_000)
    expect(todayKey(naive)).toBe('2026-11-01')
    expect(addDays('2026-11-01', 1)).toBe('2026-11-02')
  })

  it('adds days by calendar arithmetic across DST, months and years', () => {
    expect(addDays('2026-03-07', 1)).toBe('2026-03-08')
    expect(addDays('2026-03-08', 1)).toBe('2026-03-09')
    expect(addDays('2026-12-31', 1)).toBe('2027-01-01')
    expect(addDays('2024-03-01', -1)).toBe('2024-02-29')
    expect(addDays('2026-01-01', 365)).toBe('2027-01-01')
    expect(() => addDays('nope', 1)).toThrow(RangeError)
  })
})
