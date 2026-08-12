/**
 * Response fixtures shared by the task-panel tests.
 *
 * Test-only, and not reachable from `tasks/index.ts`, so it never reaches the
 * shipped chunk. The shapes here are transcribed from the assertions in
 * `src/task_query.rs`'s test module, so a wire-format change breaks these
 * fixtures rather than silently passing.
 */
import type { TaskGroup, TaskHit, TaskQueryResponse } from './types.js'

export function makeHit(overrides: Partial<TaskHit> & { text: string }): TaskHit {
  return {
    kind: 'task',
    line: 1,
    depth: 0,
    status: 'open',
    priority: 'normal',
    tags: [],
    marker_start: null,
    marker_end: null,
    due: null,
    due_has_time: false,
    done: null,
    done_has_time: false,
    moved_to: null,
    url_path: '/notes/',
    path: 'notes.md',
    ...overrides,
  }
}

/**
 * The default `incomplete_markers`, longest-first, as `MarkerRule` sorts them.
 * Only for locating the word in a fixture's text — see {@link markerSpan}.
 */
const DEFAULT_MARKERS = ['FIXME', 'TODO', 'XXX', 'TK']

/**
 * Where the marker word sits in a fixture's text, as `parse_marker_line` would
 * report it.
 *
 * A plain search, and deliberately **not** a second implementation of the
 * grammar: fixture text is chosen to have exactly one unambiguous marker, so
 * this only has to agree with the server on those strings. Computing it beats
 * hand-counting columns, which is the sort of arithmetic that goes stale the
 * first time somebody edits a fixture's wording.
 */
function markerSpan(text: string): { marker_start: number; marker_end: number } | null {
  for (const marker of DEFAULT_MARKERS) {
    const at = text.indexOf(marker)
    if (at !== -1) return { marker_start: at, marker_end: at + marker.length }
  }
  return null
}

/**
 * One incomplete-marker hit, shaped exactly as `parse_marker_line` emits it.
 *
 * Every field the server pins for a marker is pinned here too — open, normal,
 * no tags, no dates — so a test that reaches for one of them is asserting
 * against the real wire shape rather than a convenient fiction. `text` is the
 * whole source line, marker word and all, and the marker span points at the
 * word inside it, so a marker fixture exercises the card's highlight instead of
 * quietly falling through its "no usable span" branch.
 */
export function makeMarker(overrides: Partial<TaskHit> & { text: string }): TaskHit {
  return makeHit({
    kind: 'marker',
    status: 'open',
    priority: 'normal',
    tags: [],
    ...markerSpan(overrides.text),
    due: null,
    due_has_time: false,
    done: null,
    done_has_time: false,
    moved_to: null,
    ...overrides,
  })
}

export function makeGroup(overrides: Partial<TaskGroup> & { key: string }): TaskGroup {
  return {
    label: overrides.key,
    sublabel: '',
    url_path: null,
    date: null,
    done: 0,
    total: 0,
    tasks: [],
    ...overrides,
  }
}

export function makeResponse(overrides: Partial<TaskQueryResponse> = {}): TaskQueryResponse {
  return {
    groups: [],
    folders: [],
    total_matches: 0,
    duration_ms: 1,
    scan_in_progress: false,
    ...overrides,
  }
}

/** Two files with three tasks between them — the category-mode default view. */
export function categoryResponse(): TaskQueryResponse {
  return makeResponse({
    groups: [
      makeGroup({
        key: '/docs/notes/',
        label: 'Weekly notes',
        sublabel: 'docs',
        url_path: '/docs/notes/',
        done: 3,
        total: 7,
        tasks: [
          makeHit({
            text: 'write the report',
            line: 4,
            priority: 'urgent',
            tags: ['work'],
            due: '2026-08-05T00:00:00',
            url_path: '/docs/notes/',
            path: 'docs/notes.md',
          }),
          makeHit({
            text: 'follow up',
            line: 5,
            depth: 1,
            url_path: '/docs/notes/',
            path: 'docs/notes.md',
          }),
        ],
      }),
      makeGroup({
        key: '/todo/',
        label: 'Todo',
        sublabel: '',
        url_path: '/todo/',
        done: 0,
        total: 1,
        tasks: [makeHit({ text: 'buy milk', line: 2, url_path: '/todo/', path: 'todo.md' })],
      }),
    ],
    folders: [
      { path: '/', count: 3 },
      { path: '/docs/', count: 2 },
    ],
    total_matches: 3,
  })
}

/**
 * One file holding a checkbox task **and** an incomplete marker, in source
 * order — the `include: 'all'` view.
 *
 * Deliberately its own fixture rather than an extra entry in
 * {@link categoryResponse}: at least six tests assert that response's rows
 * positionally, and quietly lengthening it would break them for a reason that
 * has nothing to do with what they are testing. Having both kinds in one group
 * is the point here — it lets a marker assertion prove the branch by showing
 * the task beside it behaving differently.
 *
 * `done`/`total` count the task alone, as the server counts them.
 */
export function markerResponse(): TaskQueryResponse {
  return makeResponse({
    groups: [
      makeGroup({
        key: '/notes/',
        label: 'Notes',
        url_path: '/notes/',
        done: 0,
        total: 1,
        tasks: [
          makeHit({ text: 'a real task', line: 3 }),
          makeMarker({ text: 'The market fell 10% (source: TK).', line: 9 }),
        ],
      }),
    ],
    folders: [{ path: '/', count: 2 }],
    total_matches: 2,
  })
}

/** Every calendar bucket the server can emit, in its wire order. */
export function calendarResponse(): TaskQueryResponse {
  return makeResponse({
    groups: [
      makeGroup({
        key: 'overdue',
        label: 'Overdue',
        done: 0,
        total: 0,
        tasks: [makeHit({ text: 'late', line: 1, due: '2026-08-01T00:00:00' })],
      }),
      makeGroup({
        key: 'today',
        label: 'Today',
        date: '2026-08-04',
        done: 2,
        total: 3,
        tasks: [makeHit({ text: 'now', line: 2, due: '2026-08-04T00:00:00' })],
      }),
      makeGroup({
        key: 'tomorrow',
        label: 'Tomorrow',
        date: '2026-08-05',
        done: 0,
        total: 1,
        tasks: [makeHit({ text: 'soon', line: 3, due: '2026-08-05T00:00:00' })],
      }),
      makeGroup({
        key: 'upcoming:2026-08-20',
        label: '2026-08-20',
        date: '2026-08-20',
        done: 1,
        total: 2,
        tasks: [makeHit({ text: 'far', line: 4, due: '2026-08-20T00:00:00' })],
      }),
      makeGroup({
        key: 'upcoming:2026-08-21',
        label: '2026-08-21',
        date: '2026-08-21',
        done: 0,
        total: 3,
        tasks: [makeHit({ text: 'farther', line: 5, due: '2026-08-21T00:00:00' })],
      }),
      makeGroup({
        key: 'none',
        label: 'No due date',
        done: 0,
        total: 1,
        tasks: [makeHit({ text: 'whenever', line: 6 })],
      }),
    ],
    total_matches: 6,
  })
}
