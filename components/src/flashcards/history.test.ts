import { describe, expect, it } from 'vitest'
import {
  formatEntryTime,
  isHistoryLabel,
  parseHistory,
  parseHistoryEntry,
  summarizeHistory,
} from './history.js'

describe('parseHistoryEntry', () => {
  it('reads the written format as local time', () => {
    const entry = parseHistoryEntry('2026-10-06 13:45 - Good')
    expect(entry?.rating).toBe('good')
    expect(entry?.at).toEqual(new Date(2026, 9, 6, 13, 45))
  })

  it('accepts every rating in any case, and Fail as Again', () => {
    expect(parseHistoryEntry('2026-10-06 13:45 - AGAIN')?.rating).toBe('again')
    expect(parseHistoryEntry('2026-10-06 13:45 - hard')?.rating).toBe('hard')
    expect(parseHistoryEntry('2026-10-06 13:45 - Easy')?.rating).toBe('easy')
    expect(parseHistoryEntry('2026-10-06 13:45 - Fail')?.rating).toBe('again')
  })

  it('tolerates a T separator, a single-digit hour, dashes and trailing text', () => {
    expect(parseHistoryEntry('2026-10-06T09:05 - Good')?.at).toEqual(new Date(2026, 9, 6, 9, 5))
    expect(parseHistoryEntry('2026-10-06 9:05 – Good')?.rating).toBe('good')
    expect(parseHistoryEntry('2026-10-06 09:05 — Easy (guessed)')?.rating).toBe('easy')
  })

  it('rejects anything else, including impossible dates', () => {
    expect(parseHistoryEntry('yesterday - Good')).toBeNull()
    expect(parseHistoryEntry('2026-10-06 - Good')).toBeNull()
    expect(parseHistoryEntry('2026-10-06 13:45 - Okay')).toBeNull()
    expect(parseHistoryEntry('2026-10-06 13:45 - Goodish')).toBeNull()
    expect(parseHistoryEntry('2026-02-31 13:45 - Good')).toBeNull()
    expect(parseHistoryEntry('2026-10-06 25:00 - Good')).toBeNull()
  })
})

describe('parseHistory', () => {
  it('drops unparseable lines and sorts oldest first, stably', () => {
    const entries = parseHistory([
      '2026-10-09 18:02 - Easy',
      'not an entry',
      '2026-10-06 13:45 - Again',
      '2026-10-06 13:45 - Good',
    ])
    expect(entries.map((e) => e.rating)).toEqual(['again', 'good', 'easy'])
  })
})

describe('labels and formatting', () => {
  it('recognises the history label', () => {
    expect(isHistoryLabel('Review History')).toBe(true)
    expect(isHistoryLabel(' review history ')).toBe(true)
    expect(isHistoryLabel('Review')).toBe(false)
  })

  it('formats entry times the way the server writes them', () => {
    expect(formatEntryTime(new Date(2026, 0, 2, 3, 4))).toBe('2026-01-02 03:04')
  })

  it('summarizes count, last rating and date', () => {
    const now = new Date(2026, 9, 10)
    const entries = parseHistory(['2026-10-06 13:45 - Again', '2026-10-09 18:02 - Easy'])
    const summary = summarizeHistory(entries, now)
    expect(summary.startsWith('Reviewed 2× · last: Easy, ')).toBe(true)
    expect(summary).toContain('9')
    expect(summary).not.toContain('2026')
    expect(summarizeHistory(parseHistory(['2025-01-01 00:00 - Good']), now)).toContain('2025')
    expect(summarizeHistory([], now)).toBe('Review history')
  })
})
