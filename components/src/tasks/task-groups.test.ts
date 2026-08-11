import { describe, it, expect } from 'vitest'
import {
  UPCOMING_SECTION_KEY,
  buildDisplayGroups,
  buildRows,
  groupRowIndex,
  taskAt,
  taskHref,
} from './task-groups.js'
import { calendarResponse, categoryResponse, makeGroup, makeHit, makeResponse } from './test-fixtures.js'

/** Four files in server (lexicographic) order, so a reorder is visible. */
function fourFiles() {
  return makeResponse({
    groups: ['a', 'b', 'c', 'd'].map((name) =>
      makeGroup({
        key: `/${name}/`,
        label: name.toUpperCase(),
        url_path: `/${name}/`,
        tasks: [makeHit({ text: name, url_path: `/${name}/`, path: `${name}.md` })],
      })
    ),
  })
}

describe('buildDisplayGroups (category mode)', () => {
  it('is one-to-one with the server groups and always shows progress', () => {
    const groups = buildDisplayGroups(categoryResponse(), 'category')
    expect(groups.map((g) => g.key)).toEqual(['/docs/notes/', '/todo/'])
    expect(groups.every((g) => g.showProgress)).toBe(true)
    expect(groups.every((g) => g.level === 0 && g.parentKey === null)).toBe(true)
  })

  it('keeps the server counts verbatim, filters and all', () => {
    // The group shows 2 tasks but reads 3/7: the counts describe the FILE, not
    // the view, and that is computed server-side on purpose.
    const [notes] = buildDisplayGroups(categoryResponse(), 'category')
    expect(notes.tasks).toHaveLength(2)
    expect([notes.done, notes.total]).toEqual([3, 7])
    expect(notes.sublabel).toBe('docs')
    expect(notes.urlPath).toBe('/docs/notes/')
  })

  it('is empty for a null response', () => {
    expect(buildDisplayGroups(null, 'category')).toEqual([])
  })
})

describe('buildDisplayGroups (pinning the current page)', () => {
  it('moves the current file to the front and leaves the rest in order', () => {
    const groups = buildDisplayGroups(fourFiles(), 'category', 'c.md')
    expect(groups.map((g) => g.key)).toEqual(['/c/', '/a/', '/b/', '/d/'])
  })

  it('is a stable reorder: only the pinned group moves', () => {
    const before = buildDisplayGroups(fourFiles(), 'category').map((g) => g.key)
    const after = buildDisplayGroups(fourFiles(), 'category', 'd.md').map((g) => g.key)
    expect(after[0]).toBe('/d/')
    expect(after.slice(1)).toEqual(before.filter((key) => key !== '/d/'))
  })

  it('matches the source path, which the url_path group key does not determine', () => {
    // `docs/index.md` is served at `/docs/`, so the key cannot be reconstructed
    // from the path — the match has to come off the tasks.
    const response = makeResponse({
      groups: [
        makeGroup({ key: '/other/', label: 'Other', tasks: [makeHit({ text: 'x', path: 'other.md' })] }),
        makeGroup({
          key: '/docs/',
          label: 'Docs',
          tasks: [makeHit({ text: 'y', path: 'docs/index.md' })],
        }),
      ],
    })
    expect(buildDisplayGroups(response, 'category', 'docs/index.md').map((g) => g.key)).toEqual([
      '/docs/',
      '/other/',
    ])
  })

  it('leaves the order untouched when the current file is already first', () => {
    const groups = buildDisplayGroups(fourFiles(), 'category', 'a.md')
    expect(groups.map((g) => g.key)).toEqual(['/a/', '/b/', '/c/', '/d/'])
  })

  it('leaves the order untouched when the current file has no tasks in the response', () => {
    const groups = buildDisplayGroups(fourFiles(), 'category', 'nowhere.md')
    expect(groups.map((g) => g.key)).toEqual(['/a/', '/b/', '/c/', '/d/'])
  })

  it('pins nothing in calendar mode, where a group is a date rather than a file', () => {
    const response = calendarResponse()
    // The current page's task sits in the middle bucket: pinning would move
    // "Tomorrow" to the front, and a due date has no business being reordered.
    response.groups[2].tasks[0].path = 'todo.md'
    expect(buildDisplayGroups(response, 'calendar', 'todo.md')[0].key).toBe('overdue')
  })
})

describe('buildDisplayGroups (calendar mode)', () => {
  it('splices a synthetic Upcoming header in front of the date buckets', () => {
    const groups = buildDisplayGroups(calendarResponse(), 'calendar')
    expect(groups.map((g) => g.key)).toEqual([
      'overdue',
      'today',
      'tomorrow',
      UPCOMING_SECTION_KEY,
      'upcoming:2026-08-20',
      'upcoming:2026-08-21',
      'none',
    ])
  })

  it('sums the per-date counts onto the Upcoming header', () => {
    const groups = buildDisplayGroups(calendarResponse(), 'calendar')
    const upcoming = groups.find((g) => g.key === UPCOMING_SECTION_KEY)!
    // 1/2 + 0/3 across the two date buckets.
    expect([upcoming.done, upcoming.total]).toEqual([1, 5])
    expect(upcoming.tasks).toEqual([])
  })

  it('draws progress on Today, Tomorrow and Upcoming only', () => {
    const groups = buildDisplayGroups(calendarResponse(), 'calendar')
    const withProgress = groups.filter((g) => g.showProgress).map((g) => g.key)
    // Not Overdue (sent as 0/0 — a missed-deadline backlog has no denominator),
    // not the individual dates, and not the no-due-date bucket.
    expect(withProgress).toEqual(['today', 'tomorrow', UPCOMING_SECTION_KEY])
  })

  it('nests the date buckets under the Upcoming header', () => {
    const groups = buildDisplayGroups(calendarResponse(), 'calendar')
    const dates = groups.filter((g) => g.key.startsWith('upcoming:'))
    expect(dates).toHaveLength(2)
    expect(dates.every((g) => g.level === 1 && g.parentKey === UPCOMING_SECTION_KEY)).toBe(true)
  })

  it('omits the Upcoming header entirely when no date bucket came back', () => {
    const response = calendarResponse()
    response.groups = response.groups.filter((g) => !g.key.startsWith('upcoming:'))
    const groups = buildDisplayGroups(response, 'calendar')
    expect(groups.map((g) => g.key)).toEqual(['overdue', 'today', 'tomorrow', 'none'])
  })
})

describe('buildRows', () => {
  it('emits a heading followed by its tasks', () => {
    const groups = buildDisplayGroups(categoryResponse(), 'category')
    expect(buildRows(groups, new Set())).toEqual([
      { kind: 'group', groupIndex: 0 },
      { kind: 'task', groupIndex: 0, taskIndex: 0 },
      { kind: 'task', groupIndex: 0, taskIndex: 1 },
      { kind: 'group', groupIndex: 1 },
      { kind: 'task', groupIndex: 1, taskIndex: 0 },
    ])
  })

  it('keeps a collapsed group heading but drops its tasks', () => {
    const groups = buildDisplayGroups(categoryResponse(), 'category')
    expect(buildRows(groups, new Set(['/docs/notes/']))).toEqual([
      { kind: 'group', groupIndex: 0 },
      { kind: 'group', groupIndex: 1 },
      { kind: 'task', groupIndex: 1, taskIndex: 0 },
    ])
  })

  it('hides a nested date bucket entirely when its section is collapsed', () => {
    const groups = buildDisplayGroups(calendarResponse(), 'calendar')
    const rows = buildRows(groups, new Set([UPCOMING_SECTION_KEY]))
    const visibleGroups = rows
      .filter((row) => row.kind === 'group')
      .map((row) => groups[row.groupIndex].key)
    // The dates are gone, headings included — not just their tasks.
    expect(visibleGroups).toEqual(['overdue', 'today', 'tomorrow', UPCOMING_SECTION_KEY, 'none'])
  })

  it('is empty for no groups', () => {
    expect(buildRows([], new Set())).toEqual([])
  })
})

describe('groupRowIndex', () => {
  it('finds a heading row so focus can park on it after a collapse', () => {
    const groups = buildDisplayGroups(categoryResponse(), 'category')
    const rows = buildRows(groups, new Set(['/docs/notes/']))
    expect(groupRowIndex(rows, 0)).toBe(0)
    expect(groupRowIndex(rows, 1)).toBe(1)
  })

  it('reports -1 for a group that is not visible', () => {
    const groups = buildDisplayGroups(calendarResponse(), 'calendar')
    const rows = buildRows(groups, new Set([UPCOMING_SECTION_KEY]))
    const dateIndex = groups.findIndex((g) => g.key === 'upcoming:2026-08-20')
    expect(groupRowIndex(rows, dateIndex)).toBe(-1)
  })
})

describe('taskAt', () => {
  it('resolves a task row and rejects a heading row', () => {
    const groups = buildDisplayGroups(categoryResponse(), 'category')
    expect(taskAt(groups, { kind: 'task', groupIndex: 0, taskIndex: 1 })?.text).toBe('follow up')
    expect(taskAt(groups, { kind: 'group', groupIndex: 0 })).toBeNull()
    expect(taskAt(groups, undefined)).toBeNull()
  })
})

describe('taskHref', () => {
  it('deep links to the rendered checkbox id', () => {
    const hit = makeHit({ text: 'x', line: 42, url_path: '/docs/guide/' })
    expect(taskHref(hit, (p) => p)).toBe('/docs/guide/#mbr-task-42')
  })

  it('routes the page path through the injected resolver (static base paths)', () => {
    const hit = makeHit({ text: 'x', line: 7, url_path: '/docs/guide/' })
    expect(taskHref(hit, (p) => `../..${p}`)).toBe('../../docs/guide/#mbr-task-7')
  })
})
