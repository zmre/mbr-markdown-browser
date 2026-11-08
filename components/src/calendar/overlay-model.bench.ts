/**
 * The calendar overlay's open path at the design budget (design/calendar.md
 * §5.4): 20 000 notes → bucket + the counts and filter a first paint reads,
 * in well under 50 ms on a laptop.
 */
import { describe, test } from 'vitest'
import type { MarkdownFile } from '../sorting'
import * as model from './overlay-model'

const { bucketNotes, daysBetween, matchesFilter, nextMatch, parseFilter } = model

function generate(count: number): MarkdownFile[] {
  return Array.from({ length: count }, (_, i) => {
    const day = new Date(Date.UTC(2020, 0, 1 + (i % 2400)))
    const date = day.toISOString().slice(0, 10)
    return {
      url_path: `/folder_${i % 40}/note_${i}/`,
      raw_path: `folder_${i % 40}/note_${i}.md`,
      created: 1_600_000_000 + i * 3_600,
      modified: 1_650_000_000 + i * 1_800,
      frontmatter: { title: `Note ${i}`, type: ['meeting', 'journal', 'blog', 'person'][i % 4], tags: [`tag${i % 25}`] },
      calendar: i % 3 === 0 ? { date, source: 'filename' } : undefined,
    }
  })
}

const FILES = generate(20_000)
const OPTIONS = { tagFields: ['tags'], dateFields: ['date', 'created'] }

describe('calendar overlay, 20 000 notes', () => {
  test('open path', async ({ bench }) => {
    await bench.compare(
      bench('bucket by note date', () => {
        bucketNotes(FILES, 'date', OPTIONS)
      }),
      bench('bucket + first month (counts over 31 days, filtered)', () => {
        const buckets = bucketNotes(FILES, 'modified', OPTIONS)
        const terms = parseFilter('meeting #tag1')
        let shown = 0
        for (const date of daysBetween('2023-05-01', '2023-05-31')) {
          for (const e of buckets.days.get(date) ?? []) if (matchesFilter(e, terms)) shown++
        }
        nextMatch(buckets, '2023-05-31', terms)
        if (shown < 0) throw new Error('unreachable')
      })
    )
  })
})
