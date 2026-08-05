import { describe, it, expect } from 'vitest'
import {
  chipDatetime,
  formatChipLabel,
  formatDateHeading,
  isOverdue,
  parseNaive,
  priorityDot,
  progressPercent,
  startOfDay,
} from './task-format.js'

/**
 * Node normalizes the AM/PM separator to U+202F (narrow no-break space) in some
 * ICU versions and to a plain space in others. Collapse it so the assertions
 * describe the format, not the runtime.
 */
function normalize(text: string): string {
  return text.replace(/[  ]/g, ' ')
}

describe('parseNaive', () => {
  it('reads a date-only value as LOCAL midnight, not UTC', () => {
    // The whole reason this function exists: `new Date('2026-08-05')` is UTC
    // midnight, which is 2026-08-04 in every negative-offset timezone.
    const date = parseNaive('2026-08-05')
    expect(date).not.toBeNull()
    expect(date!.getFullYear()).toBe(2026)
    expect(date!.getMonth()).toBe(7) // August
    expect(date!.getDate()).toBe(5)
    expect(date!.getHours()).toBe(0)
  })

  it('reads a datetime value as local wall-clock time', () => {
    const date = parseNaive('2026-08-05T15:30:00')
    expect(date!.getHours()).toBe(15)
    expect(date!.getMinutes()).toBe(30)
  })

  it('accepts a space separator and an omitted seconds field', () => {
    expect(parseNaive('2026-08-05 15:30')?.getHours()).toBe(15)
  })

  it('returns null for null, non-strings and malformed values', () => {
    expect(parseNaive(null)).toBeNull()
    expect(parseNaive(undefined)).toBeNull()
    expect(parseNaive('')).toBeNull()
    expect(parseNaive('next tuesday')).toBeNull()
    expect(parseNaive('2026-8-5')).toBeNull()
    expect(parseNaive(42 as unknown as string)).toBeNull()
  })

  it('rejects an impossible calendar date instead of rolling it forward', () => {
    // `new Date(2026, 1, 30)` silently becomes March 2nd.
    expect(parseNaive('2026-02-30')).toBeNull()
    expect(parseNaive('2026-02-28')).not.toBeNull()
  })
})

describe('startOfDay', () => {
  it('drops the time of day', () => {
    const start = startOfDay(new Date(2026, 7, 4, 23, 59, 59))
    expect(start.getHours()).toBe(0)
    expect(start.getDate()).toBe(4)
  })
})

describe('isOverdue', () => {
  const today = new Date(2026, 7, 4, 12, 0, 0)

  it('is true only once the due DAY has passed', () => {
    expect(isOverdue('2026-08-03T09:00:00', today)).toBe(true)
    expect(isOverdue('2026-08-04T00:01:00', today)).toBe(false)
    // The rule that matters: a task due at 09:00 is still "today" at noon.
    expect(isOverdue('2026-08-04T09:00:00', today)).toBe(false)
    expect(isOverdue('2026-08-04T23:59:00', today)).toBe(false)
    expect(isOverdue('2026-08-05T00:00:00', today)).toBe(false)
  })

  it('is false without a due date or with an unparseable one', () => {
    expect(isOverdue(null, today)).toBe(false)
    expect(isOverdue('someday', today)).toBe(false)
  })

  it('handles month and year boundaries', () => {
    expect(isOverdue('2026-07-31T00:00:00', new Date(2026, 7, 1))).toBe(true)
    expect(isOverdue('2025-12-31T00:00:00', new Date(2026, 0, 1))).toBe(true)
  })
})

describe('formatChipLabel', () => {
  it('matches the document renderer: month + day, no year', () => {
    expect(formatChipLabel('2026-08-05T00:00:00', false, 'en-US')).toBe('Aug 5')
  })

  it('appends the time when the annotation carried one', () => {
    expect(normalize(formatChipLabel('2026-08-05T15:00:00', true, 'en-US'))).toBe('Aug 5, 3:00 PM')
  })

  it('is empty for a missing or malformed value', () => {
    expect(formatChipLabel(null, false, 'en-US')).toBe('')
    expect(formatChipLabel('garbage', false, 'en-US')).toBe('')
  })
})

describe('chipDatetime', () => {
  it('emits the machine-readable form the document renderer uses', () => {
    expect(chipDatetime('2026-08-05T00:00:00', false)).toBe('2026-08-05')
    expect(chipDatetime('2026-08-05T15:00:00', true)).toBe('2026-08-05T15:00')
  })

  it('falls back to the date when a time was requested but none exists', () => {
    expect(chipDatetime('2026-08-05', true)).toBe('2026-08-05')
  })

  it('is empty for a missing or malformed value', () => {
    expect(chipDatetime(null, false)).toBe('')
    expect(chipDatetime('nope', false)).toBe('')
  })
})

describe('formatDateHeading', () => {
  const today = new Date(2026, 7, 4)

  it('shows weekday, month and day within the current year', () => {
    expect(formatDateHeading('2026-08-20', today, 'en-US')).toBe('Thu, Aug 20')
  })

  it('adds the year only when it differs from today', () => {
    expect(formatDateHeading('2027-01-04', today, 'en-US')).toContain('2027')
  })

  it('is empty for a missing value', () => {
    expect(formatDateHeading(null, today, 'en-US')).toBe('')
  })
})

describe('progressPercent', () => {
  it('rounds and clamps', () => {
    expect(progressPercent(3, 7)).toBe(43)
    expect(progressPercent(0, 5)).toBe(0)
    expect(progressPercent(5, 5)).toBe(100)
    expect(progressPercent(9, 5)).toBe(100)
    expect(progressPercent(-1, 5)).toBe(0)
  })

  it('is 0 for a zero total rather than NaN', () => {
    // The calendar mode's Overdue bucket is sent as 0/0 on purpose; a NaN
    // width silently paints a FULL bar.
    expect(progressPercent(0, 0)).toBe(0)
    expect(progressPercent(3, 0)).toBe(0)
    expect(progressPercent(NaN, 4)).toBe(0)
  })
})

describe('priorityDot', () => {
  it('maps the wire values onto the theme.css modifier classes', () => {
    expect(priorityDot('high')).toEqual({ className: 'mbr-task-pri-high', label: 'High priority' })
    expect(priorityDot('urgent')).toEqual({
      className: 'mbr-task-pri-urgent',
      label: 'Urgent priority',
    })
  })

  it('draws nothing for the default priority', () => {
    expect(priorityDot('normal')).toBeNull()
    expect(priorityDot('anything else')).toBeNull()
  })
})
