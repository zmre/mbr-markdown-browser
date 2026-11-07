import { describe, expect, it } from 'vitest'
import type { MarkdownFile } from '../sorting.ts'
import {
  OVERLAY_SHORTCUTS,
  bucketNotes,
  cellLines,
  defaultView,
  filesInFolder,
  matchesFilter,
  moveDay,
  nextMatch,
  nextPlotBy,
  overflowSplit,
  overlayCommand,
  pageFolder,
  parseFilter,
  parseJump,
  prevMatch,
  typeColor,
  viewRange,
  type OverlayCommand,
} from './overlay-model.ts'

/** Local-time unix seconds, so the expected dates do not depend on the test machine's zone. */
const local = (y: number, m: number, d: number, h = 12, min = 0): number => new Date(y, m - 1, d, h, min).getTime() / 1000

const note = (raw: string, extra: Partial<MarkdownFile> = {}): MarkdownFile => ({
  url_path: `/${raw.replace(/\.md$/, '')}/`,
  raw_path: raw,
  created: local(2026, 1, 5),
  modified: local(2026, 2, 6, 9, 30),
  frontmatter: null,
  ...extra,
})

const OPTIONS = { tagFields: ['tags'], dateFields: ['date', 'created'] }

const FILES: MarkdownFile[] = [
  note('daily/2026-10-31.md', { calendar: { date: '2026-10-31', source: 'filename', period: '2026-10-31' } }),
  note('weekly/2026-W44.md', { calendar: { date: '2026-10-26', source: 'filename', period: '2026-W44' } }),
  note('meetings/2026-10-31 Acme sync.md', {
    frontmatter: { type: 'Meeting', title: 'Acme sync', tags: ['clients/acme', 'sales'] },
    calendar: { date: '2026-10-31', source: 'filename' },
  }),
  note('blog/launch.md', {
    frontmatter: { title: 'Launch post', date: '2026-10-31T14:05:00-07:00', tags: 'writing, drafts' },
    calendar: { date: '2026-10-31', source: 'frontmatter' },
  }),
  note('blog/aardvark.md', {
    frontmatter: { title: 'Aardvark', date: '2026-10-31' },
    calendar: { date: '2026-10-31', source: 'frontmatter' },
  }),
  note('inbox/undated.md'),
]

describe('typeColor', () => {
  // Pinned (independently computed FNV-1a over UTF-8, mod 8): changing the
  // hash recolours every calendar that has ever been looked at.
  it.each([
    ['meeting', 2],
    ['Meeting', 2],
    [' journal ', 0],
    ['person', 0],
    ['blog', 1],
    ['café', 1],
  ])('%j → %i', (type, slot) => expect(typeColor(type)).toBe(slot))
})

describe('bucketNotes', () => {
  it('plots by note date: day note first and flagged, untimed by title, then timed; period notes left out', () => {
    const b = bucketNotes(FILES, 'date', OPTIONS)
    const day = b.days.get('2026-10-31')!
    expect(day.map((e) => e.title)).toEqual(['2026-10-31', 'Aardvark', 'Acme sync', 'Launch post'])
    expect(day[0].daily).toBe(true)
    expect(day.slice(1).every((e) => !e.daily)).toBe(true)
    expect(day[day.length - 1].minutes).toBe(14 * 60 + 5)
    expect(b.days.has('2026-10-26')).toBe(false)
    // Undated: the reader's local date of birthtime, with its time.
    expect(b.days.get('2026-01-05')!.map((e) => e.minutes)).toEqual([12 * 60])
    expect(b.keys).toEqual(['2026-01-05', '2026-10-31'])
  })

  it('plots by modified and created as plain entries, period notes included', () => {
    const modified = bucketNotes(FILES, 'modified', OPTIONS)
    expect(modified.keys).toEqual(['2026-02-06'])
    const all = modified.days.get('2026-02-06')!
    expect(all).toHaveLength(FILES.length)
    expect(all.every((e) => !e.daily && e.minutes === 9 * 60 + 30)).toBe(true)
    expect(bucketNotes(FILES, 'created', OPTIONS).keys).toEqual(['2026-01-05'])
  })

  it('gives each entry its folder, type colour, tags and haystack once', () => {
    const acme = bucketNotes(FILES, 'date', OPTIONS)
      .days.get('2026-10-31')!
      .find((e) => e.title === 'Acme sync')!
    expect(acme.folder).toBe('meetings')
    expect(acme.type).toBe('Meeting')
    expect(acme.color).toBe(2)
    expect(acme.tags).toEqual(['clients/acme', 'sales'])
    const launch = bucketNotes(FILES, 'date', OPTIONS)
      .days.get('2026-10-31')!
      .find((e) => e.title === 'Launch post')!
    expect(launch.tags).toEqual(['writing', 'drafts'])
    expect(launch.color).toBeNull()
  })

  it('flags only the first of two day notes for one day', () => {
    const dupes = [
      note('b/2026-10-31.md', { calendar: { date: '2026-10-31', source: 'filename', period: '2026-10-31' } }),
      note('a/20261031.md', { calendar: { date: '2026-10-31', source: 'filename', period: '2026-10-31' } }),
    ]
    const day = bucketNotes(dupes, 'date', OPTIONS).days.get('2026-10-31')!
    expect(day.map((e) => [e.folder, e.daily])).toEqual([
      ['a', true],
      ['b', false],
    ])
  })
})

describe('the filter', () => {
  const day = bucketNotes(FILES, 'date', OPTIONS).days.get('2026-10-31')!
  const titles = (q: string) => day.filter((e) => matchesFilter(e, parseFilter(q))).map((e) => e.title)

  it('parses tags, types and plain terms', () => {
    expect(parseFilter('  Acme #Sales type:Meeting ')).toEqual([
      { kind: 'text', value: 'acme' },
      { kind: 'tag', value: 'sales' },
      { kind: 'type', value: 'meeting' },
    ])
    expect(parseFilter('# type:')).toEqual([
      { kind: 'text', value: '#' },
      { kind: 'text', value: 'type:' },
    ])
  })

  it('ANDs substrings of title, path, type and tags, case-insensitively', () => {
    expect(titles('ACME')).toEqual(['Acme sync'])
    expect(titles('blog')).toEqual(['Aardvark', 'Launch post'])
    expect(titles('blog launch')).toEqual(['Launch post'])
    expect(titles('meeting')).toEqual(['Acme sync'])
    expect(titles('drafts')).toEqual(['Launch post'])
    expect(titles('nothing-like-this')).toEqual([])
  })

  it('matches #tag by prefix and type: exactly', () => {
    expect(titles('#client')).toEqual(['Acme sync'])
    expect(titles('#acme')).toEqual([])
    expect(titles('type:meeting')).toEqual(['Acme sync'])
    expect(titles('type:meet')).toEqual([])
  })
})

describe('next and previous match', () => {
  const b = bucketNotes(FILES, 'date', OPTIONS)
  it('walks the sorted days from the visible range', () => {
    expect(nextMatch(b, '2026-01-31', [])).toBe('2026-10-31')
    expect(nextMatch(b, '2026-10-31', [])).toBeNull()
    expect(nextMatch(b, '2025-12-31', parseFilter('acme'))).toBe('2026-10-31')
    expect(prevMatch(b, '2026-11-01', parseFilter('undated'))).toBe('2026-01-05')
    expect(prevMatch(b, '2026-01-05', [])).toBeNull()
  })
})

describe('views and ranges', () => {
  it('defaults to List at phone widths, Month above', () => {
    expect(defaultView(390)).toBe('list')
    expect(defaultView(600)).toBe('list')
    expect(defaultView(601)).toBe('month')
  })

  it('covers the month, or the week from the week start', () => {
    expect(viewRange('month', '2026-02-14', 1)).toEqual({ from: '2026-02-01', to: '2026-02-28' })
    expect(viewRange('list', '2024-02-14', 0)).toEqual({ from: '2024-02-01', to: '2024-02-29' })
    // 2026-10-31 is a Saturday.
    expect(viewRange('week', '2026-10-31', 1)).toEqual({ from: '2026-10-26', to: '2026-11-01' })
    expect(viewRange('week', '2026-10-31', 0)).toEqual({ from: '2026-10-25', to: '2026-10-31' })
    expect(viewRange('week', '2026-10-31', 6)).toEqual({ from: '2026-10-31', to: '2026-11-06' })
  })

  it('cycles plot-by', () => {
    expect([nextPlotBy('date'), nextPlotBy('modified'), nextPlotBy('created')]).toEqual(['modified', 'created', 'date'])
  })
})

describe('the jump picker', () => {
  it.each([
    ['2026-10', '2026-10-01', 'month'],
    ['2026-10-31', '2026-10-31', 'day'],
    ['2026-W44', '2026-10-26', 'week'],
    [' 2026-w01 ', '2025-12-29', 'week'],
    ['2026-q4', '2026-10-01', 'quarter'],
    ['2026', '2026-01-01', 'year'],
  ])('%j → %s (%s)', (input, date, kind) => expect(parseJump(input)).toEqual({ date, kind }))

  it.each(['', 'october', '2026-13', '2026-02-30', '2026-W54', '2026-Q5', '26-10'])('rejects %j', (input) =>
    expect(parseJump(input)).toBeNull()
  )
})

describe('month cell overflow', () => {
  it('fits at least two lines', () => {
    expect(cellLines(0, 22, 19)).toBe(2)
    expect(cellLines(Number.NaN, 22, 19)).toBe(2)
    expect(cellLines(100, 22, 19)).toBe(4)
  })

  it('replaces the last line with +N only when the entries do not fit', () => {
    expect(overflowSplit(0, 3)).toEqual({ shown: 0, more: 0 })
    expect(overflowSplit(3, 3)).toEqual({ shown: 3, more: 0 })
    expect(overflowSplit(4, 3)).toEqual({ shown: 2, more: 2 })
    expect(overflowSplit(10, 2)).toEqual({ shown: 1, more: 9 })
    expect(overflowSplit(5, 1)).toEqual({ shown: 1, more: 4 })
  })
})

describe('the key map', () => {
  it.each<[string, OverlayCommand]>([
    ['ArrowLeft', 'left'],
    ['h', 'left'],
    ['l', 'right'],
    ['k', 'up'],
    ['j', 'down'],
    ['[', 'prev'],
    ['PageDown', 'next'],
    ['t', 'today'],
    ['m', 'month'],
    ['w', 'week'],
    ['a', 'list'],
    ['g', 'jump'],
    ['p', 'plot'],
    ['/', 'filter'],
    ['Enter', 'open'],
    ['Escape', 'escape'],
    ['?', 'help'],
  ])('%s → %s', (key, command) => expect(overlayCommand(key)).toBe(command))

  it('ignores other keys, including inherited property names', () => {
    for (const key of ['x', 'C', 'constructor', 'toString', ' ']) expect(overlayCommand(key)).toBeNull()
  })

  it('moves by day and week; pages by month, or week in Week view', () => {
    expect(moveDay('left', 'month', '2026-03-01')).toBe('2026-02-28')
    expect(moveDay('down', 'month', '2026-10-31')).toBe('2026-11-07')
    expect(moveDay('next', 'month', '2026-01-31')).toBe('2026-02-28')
    expect(moveDay('prev', 'week', '2026-10-31')).toBe('2026-10-24')
    expect(moveDay('open', 'month', '2026-10-31')).toBeNull()
  })

  it('lists every command in the help', () => {
    const help = OVERLAY_SHORTCUTS.map((s) => s.keys).join(' ')
    for (const key of ['h', 'j', '[', 'PageUp', 't', 'm', 'w', 'a', 'g', 'p', '/', 'Enter', 'Esc']) {
      expect(help).toContain(key)
    }
  })
})

describe('folders', () => {
  it('finds the page folder from a note, else from the listing path', () => {
    expect(pageFolder(FILES, '/meetings/2026-10-31 Acme sync/')).toBe('meetings')
    expect(pageFolder(FILES, '/blog/')).toBe('blog')
    expect(pageFolder(FILES, '/')).toBe('')
  })

  it('takes a folder with its subfolders, not a name prefix', () => {
    const files = [note('blog/a.md'), note('blog/old/b.md'), note('blogroll/c.md')]
    expect(filesInFolder(files, 'blog').map((f) => f.raw_path)).toEqual(['blog/a.md', 'blog/old/b.md'])
  })
})
