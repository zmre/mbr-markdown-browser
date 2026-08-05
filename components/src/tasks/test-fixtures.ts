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
    line: 1,
    depth: 0,
    status: 'open',
    priority: 'normal',
    tags: [],
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
