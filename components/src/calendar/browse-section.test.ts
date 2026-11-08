import { describe, expect, it } from 'vitest'
import { calendarSectionModel } from './browse-section.ts'
import { buildPeriodicIndex } from './periodic.ts'

const p = (period: string) => ({ url_path: `/n/${period}/`, calendar: { period } })

describe('calendarSectionModel', () => {
  it('is null without periodic notes', () => {
    expect(calendarSectionModel(buildPeriodicIndex([{ url_path: '/x/' }]), '2026-10-31', 'en-US')).toBeNull()
  })

  it('lists only kinds the repository has, linking the current note', () => {
    const index = buildPeriodicIndex([p('2026-10-31'), p('2026-W44'), p('2026-10')])
    const model = calendarSectionModel(index, '2026-10-31', 'en-US')!
    expect(model.rows).toEqual([
      { kind: 'day', label: 'Today', url: '/n/2026-10-31/' },
      { kind: 'week', label: 'This week', url: '/n/2026-W44/' },
      { kind: 'month', label: 'This month', url: '/n/2026-10/' },
    ])
  })

  it('falls back to the latest note before the current period, saying so', () => {
    const index = buildPeriodicIndex([p('2026-10-30'), p('2026-11-04'), p('2026-Q3'), p('2027')])
    const model = calendarSectionModel(index, '2026-10-31', 'en-US')!
    expect(model.rows).toEqual([
      { kind: 'day', label: 'Today', url: '/n/2026-10-30/', latest: 'Oct 30' },
      { kind: 'quarter', label: 'This quarter', url: '/n/2026-Q3/', latest: '2026-Q3' },
      // Nothing before: the earliest one.
      { kind: 'year', label: 'This year', url: '/n/2027/', latest: '2027' },
    ])
  })

  it('lists the last three day notes up to today, newest first', () => {
    const days = Array.from({ length: 10 }, (_, i) => p(`2026-10-${String(20 + i).padStart(2, '0')}`))
    const model = calendarSectionModel(buildPeriodicIndex([...days, p('2026-11-02')]), '2026-10-28', 'en-US')!
    expect(model.recent.map((r) => r.url)).toEqual(
      ['28', '27', '26'].map((d) => `/n/2026-10-${d}/`)
    )
    expect(model.recent[0].label).toBe('Wed, Oct 28')
  })
})
