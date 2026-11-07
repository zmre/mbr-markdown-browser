/**
 * The shared prefilter table (`tests/fixtures/agenda-prefilter-cases.json`):
 * each case's HTML is what QuickLook renders for its markdown (kept current by
 * mbr-ffi's `test_agenda_prefilter_is_a_superset_of_the_grammar`, which also
 * holds `mbr_core::agenda::may_have_agenda` to accepting every case with an
 * entry). Here the one grammar, `agenda-parse.ts`, counts the entries.
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { agendaEntries } from '../calendar/agenda-parse.ts'

interface Case {
  readonly name: string
  readonly markdown: string
  readonly html: string
  readonly entries: number
  readonly prefilter?: boolean
}

const CASES: readonly Case[] = JSON.parse(
  readFileSync(join(process.cwd(), '..', 'tests', 'fixtures', 'agenda-prefilter-cases.json'), 'utf8')
)

describe('agenda prefilter cases', () => {
  it('has cases with entries and cases without', () => {
    expect(CASES.some((c) => c.entries > 0)).toBe(true)
    expect(CASES.some((c) => c.entries === 0)).toBe(true)
  })

  for (const c of CASES) {
    it(`${c.name}: ${c.entries} entr${c.entries === 1 ? 'y' : 'ies'}`, () => {
      const main = document.createElement('main')
      main.id = 'wrapper'
      main.innerHTML = c.html
      expect(agendaEntries(main)).toHaveLength(c.entries)
      // A prefilter answer pinned as "no" is only sound when nothing parses.
      if (c.prefilter === false) expect(c.entries).toBe(0)
    })
  }
})
