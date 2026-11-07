import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  addMonths,
  addMonthsToDate,
  isoWeekOf,
  isoWeekStart,
  localeWeekStart,
  monthGrid,
  moveFocus,
  periodStart,
  resolveWeekStart,
  weekdayLabels,
  weekdayOf,
  type Weekday,
} from './month-grid.ts'
import { addDays } from './note-date.ts'

afterEach(() => {
  vi.restoreAllMocks()
})

describe('isoWeekOf', () => {
  it('labels 2026-12-28 … 2027-01-03 as 2026-W53 (2026 has 53 weeks)', () => {
    for (let i = 0; i < 7; i++) {
      expect(isoWeekOf(addDays('2026-12-28', i)).key).toBe('2026-W53')
    }
    expect(isoWeekOf('2027-01-04').key).toBe('2027-W01')
  })

  it('handles the classic edge years', () => {
    expect(isoWeekOf('2020-12-31').key).toBe('2020-W53')
    expect(isoWeekOf('2021-01-03').key).toBe('2020-W53')
    expect(isoWeekOf('2025-12-29').key).toBe('2026-W01')
    expect(isoWeekOf('2026-10-31').key).toBe('2026-W44')
  })

  it('round-trips with isoWeekStart', () => {
    expect(isoWeekStart('2026-W01')).toBe('2025-12-29')
    expect(isoWeekStart('2026-W53')).toBe('2026-12-28')
    expect(isoWeekStart('2025-W53')).toBeNull()
    for (let d = '2024-01-01'; d < '2028-01-01'; d = addDays(d, 5)) {
      const week = isoWeekOf(d)
      const start = isoWeekStart(week.key)!
      expect(weekdayOf(start)).toBe(1)
      expect(start <= d && d <= addDays(start, 6)).toBe(true)
    }
  })
})

describe('monthGrid', () => {
  const STARTS: Weekday[] = [0, 1, 6]

  for (const weekStart of STARTS) {
    it(`lays out six rows starting on weekday ${weekStart}, holding the whole month`, () => {
      for (let m = '2026-01'; m <= '2027-12'; m = addMonths(m, 1)) {
        const grid = monthGrid(m, weekStart)
        expect(grid.weeks).toHaveLength(6)
        const days = grid.weeks.flatMap((w) => w.days)
        expect(days).toHaveLength(42)
        expect(weekdayOf(days[0])).toBe(weekStart)
        // Consecutive days, the 1st in the first row.
        days.forEach((d, i) => i > 0 && expect(d).toBe(addDays(days[i - 1], 1)))
        expect(grid.weeks[0].days).toContain(`${m}-01`)
        // Every row is labelled with the ISO week of its Thursday.
        for (const week of grid.weeks) {
          const thursday = week.days.find((d) => weekdayOf(d) === 4)!
          expect(week.isoWeek).toBe(isoWeekOf(thursday).key)
        }
      }
    })
  }

  it('labels the rows across the 2026/2027 boundary for every week start', () => {
    // Monday start: one row is exactly the ISO week.
    const monday = monthGrid('2026-12', 1).weeks.find((w) => w.days[0] === '2026-12-28')!
    expect(monday.days).toEqual([
      '2026-12-28', '2026-12-29', '2026-12-30', '2026-12-31', '2027-01-01', '2027-01-02', '2027-01-03',
    ])
    expect(monday.isoWeek).toBe('2026-W53')

    // Sunday start: the ISO week spans two rows; each takes its Thursday's.
    const sunday = monthGrid('2026-12', 0).weeks
    expect(sunday.find((w) => w.days[0] === '2026-12-27')!.isoWeek).toBe('2026-W53')
    expect(monthGrid('2027-01', 0).weeks.find((w) => w.days[0] === '2027-01-03')!.isoWeek).toBe('2027-W01')

    // Saturday start: Sat 12-26 … Fri 01-01 holds Thursday 12-31.
    const saturday = monthGrid('2026-12', 6).weeks.find((w) => w.days[0] === '2026-12-26')!
    expect(saturday.isoWeek).toBe('2026-W53')
    expect(monthGrid('2027-01', 6).weeks.find((w) => w.days[0] === '2027-01-02')!.isoWeek).toBe('2027-W01')
  })

  it('starts October 2026 (the 1st is a Thursday) where each week start puts it', () => {
    expect(monthGrid('2026-10', 1).weeks[0].days[0]).toBe('2026-09-28')
    expect(monthGrid('2026-10', 0).weeks[0].days[0]).toBe('2026-09-27')
    expect(monthGrid('2026-10', 6).weeks[0].days[0]).toBe('2026-09-26')
  })

  it('rejects a bad month key', () => {
    expect(() => monthGrid('2026-13', 1)).toThrow(RangeError)
  })
})

describe('week starts', () => {
  it('maps the fixed settings', () => {
    expect(resolveWeekStart('monday', 'en-US')).toBe(1)
    expect(resolveWeekStart('sunday', 'de-DE')).toBe(0)
    expect(resolveWeekStart('saturday', 'en-US')).toBe(6)
  })

  it('asks Intl.Locale#getWeekInfo for `locale`', () => {
    const proto = Intl.Locale.prototype as unknown as { getWeekInfo?: () => { firstDay: number } }
    const original = proto.getWeekInfo
    proto.getWeekInfo = function () {
      return { firstDay: 7 }
    }
    try {
      expect(resolveWeekStart('locale', 'fr-FR')).toBe(0)
    } finally {
      if (original) proto.getWeekInfo = original
      else delete proto.getWeekInfo
    }
  })

  it('falls back to Sunday for en-US and Monday otherwise without week info', () => {
    const proto = Intl.Locale.prototype as unknown as { getWeekInfo?: unknown; weekInfo?: unknown }
    const getter = Object.getOwnPropertyDescriptor(proto, 'getWeekInfo')
    const info = Object.getOwnPropertyDescriptor(proto, 'weekInfo')
    Object.defineProperty(proto, 'getWeekInfo', { value: undefined, configurable: true })
    Object.defineProperty(proto, 'weekInfo', { get: () => undefined, configurable: true })
    try {
      expect(localeWeekStart('en-US')).toBe(0)
      expect(localeWeekStart('en-GB')).toBe(1)
      expect(localeWeekStart('de')).toBe(1)
      expect(localeWeekStart('not a locale!')).toBe(1)
    } finally {
      if (getter) Object.defineProperty(proto, 'getWeekInfo', getter)
      else delete proto.getWeekInfo
      if (info) Object.defineProperty(proto, 'weekInfo', info)
      else delete proto.weekInfo
    }
  })

  it('labels the columns in order, localized', () => {
    expect(weekdayLabels(1, 'en-US', 'short')).toEqual(['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'])
    expect(weekdayLabels(0, 'en-US', 'short')[0]).toBe('Sun')
    expect(weekdayLabels(6, 'en-US', 'short')[0]).toBe('Sat')
  })
})

describe('month and period arithmetic', () => {
  it('adds months across years', () => {
    expect(addMonths('2026-12', 1)).toBe('2027-01')
    expect(addMonths('2026-01', -1)).toBe('2025-12')
    expect(addMonths('2026-10', 14)).toBe('2027-12')
  })

  it('clamps the day when moving a date by months', () => {
    expect(addMonthsToDate('2026-01-31', 1)).toBe('2026-02-28')
    expect(addMonthsToDate('2028-01-31', 1)).toBe('2028-02-29')
    expect(addMonthsToDate('2026-03-31', -1)).toBe('2026-02-28')
    expect(addMonthsToDate('2026-12-15', 1)).toBe('2027-01-15')
  })

  it('finds the first day of every period kind', () => {
    expect(periodStart('2026-10-31')).toBe('2026-10-31')
    expect(periodStart('2026-W44')).toBe('2026-10-26')
    expect(periodStart('2026-10')).toBe('2026-10-01')
    expect(periodStart('2026-Q4')).toBe('2026-10-01')
    expect(periodStart('2026')).toBe('2026-01-01')
    expect(periodStart('nope')).toBeNull()
  })
})

describe('moveFocus (the grid keyboard map)', () => {
  it('moves by a day, a week and a month', () => {
    expect(moveFocus('ArrowLeft', '2026-11-01')).toBe('2026-10-31')
    expect(moveFocus('h', '2026-11-01')).toBe('2026-10-31')
    expect(moveFocus('ArrowRight', '2026-12-31')).toBe('2027-01-01')
    expect(moveFocus('l', '2026-12-31')).toBe('2027-01-01')
    expect(moveFocus('ArrowUp', '2026-11-03')).toBe('2026-10-27')
    expect(moveFocus('k', '2026-11-03')).toBe('2026-10-27')
    expect(moveFocus('ArrowDown', '2026-10-27')).toBe('2026-11-03')
    expect(moveFocus('j', '2026-10-27')).toBe('2026-11-03')
    expect(moveFocus('PageUp', '2026-03-31')).toBe('2026-02-28')
    expect(moveFocus('PageDown', '2026-01-31')).toBe('2026-02-28')
  })

  it('leaves other keys alone', () => {
    expect(moveFocus('Enter', '2026-11-01')).toBeNull()
    expect(moveFocus('x', '2026-11-01')).toBeNull()
  })
})
