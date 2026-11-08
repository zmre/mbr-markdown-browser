import { describe, expect, it } from 'vitest'
import {
  adjacent,
  buildPeriodicIndex,
  findPeriod,
  periodKindOf,
  todayDailyNote,
  type PeriodicSource,
} from './periodic.js'
import { DEFAULT_CALENDAR_CONFIG, calendarConfigFrom } from './config.js'

const note = (url: string, period?: string): PeriodicSource => ({
  url_path: url,
  raw_path: url.replace(/^\/|\/$/g, '') + '.md',
  calendar: period ? { period } : undefined,
})

const FILES: PeriodicSource[] = [
  note('/daily/2026-11-02/', '2026-11-02'),
  note('/daily/2026-10-30/', '2026-10-30'),
  note('/daily/2026-10-31/', '2026-10-31'),
  // Names the same day, sorts first by url_path: it wins, as on the server.
  note('/archive/20261031/', '2026-10-31'),
  note('/weekly/2026-W44/', '2026-W44'),
  note('/2026-10/', '2026-10'),
  note('/meetings/2026-10-31 Sync/'),
  note('/plain/'),
]

describe('periodKindOf', () => {
  it('recognizes period keys, W and Q in either case (as Rust does)', () => {
    expect(periodKindOf('2026-10-31')).toBe('day')
    expect(periodKindOf('2026-W44')).toBe('week')
    expect(periodKindOf('2026-10')).toBe('month')
    expect(periodKindOf('2026-Q4')).toBe('quarter')
    expect(periodKindOf('2026')).toBe('year')
    expect(periodKindOf('2026-w44')).toBe('week')
    expect(periodKindOf('2026-q4')).toBe('quarter')
    expect(periodKindOf('20261031')).toBeNull()
    expect(periodKindOf('2026-Q5')).toBeNull()
    expect(periodKindOf('2026-13')).toBeNull()
    expect(periodKindOf('2025-W53')).toBeNull()
  })

  it('finds a note by a lower-case week or quarter key', () => {
    const index = buildPeriodicIndex([note('/w/', '2026-W44'), note('/q/', '2026-Q4')])
    expect(findPeriod(index, '2026-w44')?.url).toBe('/w/')
    expect(findPeriod(index, '2026-q4')?.url).toBe('/q/')
  })
})

describe('buildPeriodicIndex', () => {
  const index = buildPeriodicIndex(FILES)

  it('sorts per kind and keeps the first note by url_path', () => {
    expect(index.day.map((e) => e.period)).toEqual(['2026-10-30', '2026-10-31', '2026-11-02'])
    expect(findPeriod(index, '2026-10-31')?.url).toBe('/archive/20261031/')
    expect(index.week).toEqual([{ period: '2026-W44', url: '/weekly/2026-W44/' }])
    expect(index.month).toHaveLength(1)
    expect(index.year).toHaveLength(0)
  })

  it('steps over gaps and never mixes kinds', () => {
    expect(adjacent(index, '2026-10-31', 1)?.url).toBe('/daily/2026-11-02/')
    expect(adjacent(index, '2026-10-31', -1)?.url).toBe('/daily/2026-10-30/')
    expect(adjacent(index, '2026-10-30', -1)).toBeNull()
    expect(adjacent(index, '2026-11-02', 1)).toBeNull()
    // A period with no note of its own still has neighbours.
    expect(adjacent(index, '2026-11-01', -1)?.period).toBe('2026-10-31')
    expect(adjacent(index, '2026-11-01', 1)?.period).toBe('2026-11-02')
    expect(adjacent(index, '2026-W44', 1)).toBeNull()
    expect(adjacent(index, 'nonsense', 1)).toBeNull()
  })
})

describe('todayDailyNote', () => {
  const index = buildPeriodicIndex(FILES)

  it("prefers today's note", () => {
    expect(todayDailyNote(index, '2026-10-31')?.period).toBe('2026-10-31')
  })

  it('falls back to the latest note before today', () => {
    expect(todayDailyNote(index, '2026-11-01')?.period).toBe('2026-10-31')
    expect(todayDailyNote(index, '2027-06-01')?.period).toBe('2026-11-02')
  })

  it('falls back to the earliest note after today when none is before', () => {
    expect(todayDailyNote(index, '2026-01-01')?.period).toBe('2026-10-30')
  })

  it('is null without day notes', () => {
    expect(todayDailyNote(buildPeriodicIndex([note('/weekly/2026-W44/', '2026-W44')]), '2026-10-31')).toBeNull()
  })
})

describe('calendarConfigFrom', () => {
  it('defaults a missing or malformed value key by key', () => {
    expect(calendarConfigFrom(undefined)).toEqual(DEFAULT_CALENDAR_CONFIG)
    expect(calendarConfigFrom('nope')).toEqual(DEFAULT_CALENDAR_CONFIG)
    const parsed = calendarConfigFrom({
      enabled: false,
      week_start: 'sunday',
      companion: 'sometimes',
      week_numbers: true,
    })
    expect(parsed.enabled).toBe(false)
    expect(parsed.weekStart).toBe('sunday')
    expect(parsed.companion).toBe('auto')
    expect(parsed.weekNumbers).toBe(true)
    expect(parsed.agendaChips).toBe(true)
  })
})
