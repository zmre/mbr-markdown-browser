import { describe, expect, it } from 'vitest'
import {
  facetOf,
  filterByDate,
  formatMatchedDate,
  matchDateFacets,
  queryTokens,
  splitDateFacets,
  type DatedFile,
} from './date-facets.js'
import { localDateOf } from './note-date.js'

const TODAY = '2026-11-06'

/** Local noon on `day`, unix seconds: a timestamp whose local date is `day`. */
const noon = (day: string): number => new Date(`${day}T12:00:00`).getTime() / 1000

describe('queryTokens / facetOf (mirror of search.rs parse_query)', () => {
  it('keeps a quoted facet value whole', () => {
    expect(queryTokens('a type:"Meeting Notes"  when:-7d')).toEqual(['a', 'type:"Meeting Notes"', 'when:-7d'])
    expect(queryTokens('a "b c')).toEqual(['a', '"b', 'c'])
  })

  it('reads facets like the server does', () => {
    expect(facetOf('when:-7d')).toEqual({ key: 'when', value: '-7d' })
    expect(facetOf('type:"Meeting Notes"')).toEqual({ key: 'type', value: 'Meeting Notes' })
    expect(facetOf('https://x.y/z')).toBeNull()
    expect(facetOf(':x')).toBeNull()
    expect(facetOf('x:')).toBeNull()
    expect(facetOf('x:""')).toBeNull()
  })
})

describe('splitDateFacets', () => {
  it('takes out date facets and keeps everything else as typed', () => {
    const split = splitDateFacets('standup When:-7d type:"Meeting Notes" date:draft created:2026-10', TODAY)
    expect(split.text).toBe('standup type:"Meeting Notes" date:draft')
    expect(split.facets).toEqual([
      { key: 'when', range: { from: '2026-10-31', to: TODAY } },
      { key: 'created', range: { from: '2026-10-01', to: '2026-10-31' } },
    ])
  })

  it('leaves a query without date facets alone', () => {
    expect(splitDateFacets('just text', TODAY)).toEqual({ text: 'just text', facets: [] })
  })
})

describe('matchDateFacets', () => {
  const byFilename: DatedFile = {
    url_path: '/daily/2026-10-31/',
    created: noon('2026-11-01'),
    modified: noon('2026-11-02'),
    calendar: { date: '2026-10-31', source: 'filename', period: '2026-10-31' },
  }
  const byFrontmatter: DatedFile = {
    url_path: '/post/',
    created: noon('2026-11-01'),
    modified: noon('2026-10-20'),
    calendar: { date: '2026-09-15', source: 'frontmatter' },
  }
  const byBirthtime: DatedFile = { url_path: '/undated/', created: noon('2026-10-05'), modified: noon('2026-11-03') }
  const facets = (q: string) => splitDateFacets(q, TODAY).facets

  it('date: is the note date: filename, frontmatter, then birthtime', () => {
    expect(matchDateFacets(byFilename, facets('date:2026-10'))).toEqual({ date: '2026-10-31', field: 'date' })
    expect(matchDateFacets(byFrontmatter, facets('date:2026-09'))).toEqual({ date: '2026-09-15', field: 'date' })
    expect(matchDateFacets(byBirthtime, facets('date:2026-10'))).toEqual({ date: '2026-10-05', field: 'date' })
    expect(matchDateFacets(byFrontmatter, facets('date:2026-10'))).toBeNull()
  })

  it('created: and modified: read the timestamps in local time', () => {
    expect(matchDateFacets(byFilename, facets('created:2026-11-01'))).toEqual({ date: '2026-11-01', field: 'created' })
    expect(matchDateFacets(byFilename, facets('modified:2026-11'))).toEqual({ date: '2026-11-02', field: 'modified' })
    expect(localDateOf(byBirthtime.created)).toBe('2026-10-05')
  })

  it('when: is the note date, else modified', () => {
    expect(matchDateFacets(byFrontmatter, facets('when:2026-10'))).toEqual({ date: '2026-10-20', field: 'modified' })
    expect(matchDateFacets(byFrontmatter, facets('when:2026-09'))).toEqual({ date: '2026-09-15', field: 'date' })
  })

  it('several facets AND, and the first one names the date', () => {
    expect(matchDateFacets(byFrontmatter, facets('modified:2026-10 date:2026'))).toEqual({
      date: '2026-10-20',
      field: 'modified',
    })
    expect(matchDateFacets(byFrontmatter, facets('modified:2026-10 date:2025'))).toBeNull()
  })

  it('filterByDate sorts newest first, ties by URL', () => {
    const hits = filterByDate([byBirthtime, byFrontmatter, byFilename], facets('when:2026-10'))
    expect(hits.map((h) => [h.file.url_path, h.matched.date])).toEqual([
      ['/daily/2026-10-31/', '2026-10-31'],
      ['/post/', '2026-10-20'],
      ['/undated/', '2026-10-05'],
    ])
  })
})

describe('formatMatchedDate', () => {
  it('formats in the given locale and names a non-note date', () => {
    expect(formatMatchedDate({ date: '2026-10-31', field: 'date' }, 'en-US')).toBe('Oct 31, 2026')
    expect(formatMatchedDate({ date: '2026-10-31', field: 'modified' }, 'en-US')).toBe('modified Oct 31, 2026')
    expect(formatMatchedDate({ date: '2026-10-31', field: 'created' }, 'de-DE')).toBe('created 31.10.2026')
  })
})
