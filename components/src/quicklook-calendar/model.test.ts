import { describe, expect, it } from 'vitest'
import { COLUMN_MIN_WIDTH, placementFor, readPayload } from './model.ts'

describe('readPayload', () => {
  it('reads what quicklook.rs writes', () => {
    expect(
      readPayload({ date: '2026-11-03', week_start: 'sunday', week_numbers: true, agenda_chips: false })
    ).toEqual({ date: '2026-11-03', weekStart: 'sunday', weekNumbers: true, agendaChips: false })
  })

  it('defaults the settings key by key (week_numbers null = off)', () => {
    expect(readPayload({ date: '2026-11-03', week_numbers: null, week_start: 'fortnight' })).toEqual({
      date: '2026-11-03',
      weekStart: 'locale',
      weekNumbers: false,
      agendaChips: true,
    })
  })

  it('refuses a missing payload or one without a valid date', () => {
    for (const raw of [undefined, null, 'x', {}, { date: '2026-02-30' }, { date: 20261103 }]) {
      expect(readPayload(raw), JSON.stringify(raw)).toBeNull()
    }
  })
})

describe('placementFor', () => {
  it('is a right column on a wide panel and above the note on a narrow one', () => {
    expect(placementFor(1400)).toBe('column')
    expect(placementFor(COLUMN_MIN_WIDTH)).toBe('column')
    expect(placementFor(COLUMN_MIN_WIDTH - 1)).toBe('top')
    expect(placementFor(390)).toBe('top')
  })
})
