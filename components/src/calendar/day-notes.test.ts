import { describe, expect, it } from 'vitest'
import type { MarkdownFile } from '../sorting.ts'
import { bucketByDate, groupDayNotes, noteTitle } from './day-notes.ts'

const file = (raw_path: string, extra: Partial<MarkdownFile> = {}): MarkdownFile => ({
  url_path: `/${raw_path.replace(/\.md$/, '')}/`,
  raw_path,
  created: 0,
  modified: 0,
  frontmatter: null,
  ...extra,
})

const FILES: MarkdownFile[] = [
  file('daily/2026-10-31.md', { calendar: { date: '2026-10-31', source: 'filename', period: '2026-10-31' } }),
  file('weekly/2026-W44.md', { calendar: { date: '2026-10-26', source: 'filename', period: '2026-W44' } }),
  file('meetings/2026-10-31 Acme sync.md', {
    calendar: { date: '2026-10-31', source: 'filename' },
    frontmatter: { type: 'meeting', title: 'Acme sync' },
  }),
  file('journal/rainy.md', { calendar: { date: '2026-10-31', source: 'frontmatter' }, frontmatter: { type: 'journal' } }),
  file('notes/idea.md', { calendar: { date: '2026-10-31', source: 'frontmatter' } }),
  file('meetings/b.md', { calendar: { date: '2026-10-31', source: 'frontmatter' }, frontmatter: { type: 'Meeting', title: 'Board' } }),
]

describe('bucketByDate', () => {
  it('buckets by note date, leaving out non-day periodic notes', () => {
    const buckets = bucketByDate(FILES)
    expect(buckets.get('2026-10-31')!.map((f) => f.raw_path)).toEqual([
      'daily/2026-10-31.md',
      'meetings/2026-10-31 Acme sync.md',
      'journal/rainy.md',
      'notes/idea.md',
      'meetings/b.md',
    ])
    expect(buckets.has('2026-10-26')).toBe(false)
  })
})

describe('groupDayNotes', () => {
  it('groups by type, meetings first, untyped last, without the day note', () => {
    const groups = groupDayNotes(bucketByDate(FILES).get('2026-10-31')!, '/daily/2026-10-31/')
    expect(groups.map((g) => [g.type, g.notes.map((n) => [n.title, n.folder])])).toEqual([
      ['meeting', [['Acme sync', 'meetings']]],
      ['Meeting', [['Board', 'meetings']]],
      ['journal', [['rainy', 'journal']]],
      [null, [['idea', 'notes']]],
    ])
  })

  it('is one untyped group when nothing has a type', () => {
    const groups = groupDayNotes([file('b.md'), file('a.md')], null)
    expect(groups).toEqual([
      {
        type: null,
        notes: [
          { title: 'a', url: '/a/', folder: '' },
          { title: 'b', url: '/b/', folder: '' },
        ],
      },
    ])
  })
})

describe('noteTitle', () => {
  it('prefers the frontmatter title', () => {
    expect(noteTitle(file('x/y.md', { frontmatter: { title: '  Why ' } }))).toBe('Why')
    expect(noteTitle(file('x/y.md'))).toBe('y')
  })
})
