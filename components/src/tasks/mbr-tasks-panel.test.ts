import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import './mbr-tasks-panel.js'
import type { MbrTasksPanelElement } from './mbr-tasks-panel.js'
import {
  calendarResponse,
  categoryResponse,
  makeGroup,
  makeHit,
  makeMarker,
  makeResponse,
  markerResponse,
} from './test-fixtures.js'
import type { TaskQueryResponse } from './types.js'

/** Today for every test here; matches the fixture dates. */
const TODAY = new Date(2026, 7, 4, 12, 0, 0)

let fetchMock: ReturnType<typeof vi.fn>

function respondWith(response: TaskQueryResponse) {
  fetchMock.mockResolvedValue({
    ok: true,
    status: 200,
    json: () => Promise.resolve(response),
  })
}

async function flush(element: MbrTasksPanelElement): Promise<void> {
  for (let i = 0; i < 6; i++) {
    await element.updateComplete
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}

async function mount(response: TaskQueryResponse = categoryResponse()) {
  respondWith(response)
  return mountBare(null)
}

/**
 * Mount with a current page, answering each query from `responses` in order —
 * the last one repeats. Opening from a page can cost two queries (the folder
 * scope, then the widened retry), so the tests below need per-call answers.
 */
async function mountFrom(currentPath: string | null, ...responses: TaskQueryResponse[]) {
  for (const response of responses.slice(0, -1)) {
    fetchMock.mockResolvedValueOnce({ ok: true, status: 200, json: () => Promise.resolve(response) })
  }
  respondWith(responses[responses.length - 1])
  return mountBare(currentPath)
}

async function mountBare(currentPath: string | null) {
  const element = document.createElement('mbr-tasks-panel') as MbrTasksPanelElement
  element.today = TODAY
  element.locale = 'en-US'
  element.currentPath = currentPath
  document.body.appendChild(element)
  await flush(element)
  return element
}

function press(key: string, init: KeyboardEventInit = {}): void {
  document.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...init }))
}

function bodyAt(index: number): unknown {
  return JSON.parse((fetchMock.mock.calls[index][1] as RequestInit).body as string)
}

function lastBody(): unknown {
  return bodyAt(fetchMock.mock.calls.length - 1)
}

/** Display name of the selected folder row, or `null` when none is. */
function selectedFolder(element: MbrTasksPanelElement): string | null {
  const row = element.shadowRoot!.querySelector('.tree-row.selected .label-text')
  return row?.textContent?.trim() ?? null
}

function folderLabels(element: MbrTasksPanelElement): string[] {
  return Array.from(element.shadowRoot!.querySelectorAll('.tree-label .label-text')).map(
    (node) => node.textContent?.trim() ?? ''
  )
}

function rowLabels(element: MbrTasksPanelElement): string[] {
  const root = element.shadowRoot!
  return Array.from(root.querySelectorAll('.group-heading, .task-card')).map((node) =>
    node.classList.contains('group-heading')
      ? `H:${node.querySelector('.group-label')?.textContent?.trim()}`
      : `T:${node.querySelector('.task-link')?.textContent?.trim()}`
  )
}

function focusedLabel(element: MbrTasksPanelElement): string | null {
  const focused = element.shadowRoot!.querySelector('.focused')
  if (!focused) return null
  return focused.classList.contains('group-heading')
    ? `H:${focused.querySelector('.group-label')?.textContent?.trim()}`
    : `T:${focused.querySelector('.task-link')?.textContent?.trim()}`
}

describe('MbrTasksPanelElement', () => {
  let element: MbrTasksPanelElement | null = null

  beforeEach(() => {
    fetchMock = vi.fn()
    globalThis.fetch = fetchMock as unknown as typeof fetch
  })

  afterEach(() => {
    element?.remove()
    element = null
    vi.restoreAllMocks()
  })

  describe('the request body', () => {
    it('posts the default filter state on open, unscoped without a current page', async () => {
      element = await mount()

      expect(fetchMock).toHaveBeenCalledTimes(1)
      const [url, init] = fetchMock.mock.calls[0]
      expect(url).toBe('/.mbr/tasks')
      expect((init as RequestInit).method).toBe('POST')
      // Every field is spelled out, so this assertion is the guard against
      // drift from `task_query::TaskQuery` — which silently ignores unknown
      // keys rather than erroring.
      expect(lastBody()).toEqual({
        q: '',
        folder: null,
        statuses: ['open'],
        priorities: [],
        due: 'any',
        include: 'all',
        mode: 'category',
        limit: 500,
      })
    })

    it('scopes that same default to the current page’s folder', async () => {
      element = await mountFrom('docs/notes.md', categoryResponse())

      expect(fetchMock).toHaveBeenCalledTimes(1)
      // Spelled out for the same reason: `folder` is the only field that moves,
      // and the rest must not drift along with it.
      expect(lastBody()).toEqual({
        q: '',
        folder: '/docs/',
        statuses: ['open'],
        priorities: [],
        due: 'any',
        include: 'all',
        mode: 'category',
        limit: 500,
      })
    })

    it('carries the full filter state after the popover is used', async () => {
      element = await mount()
      const root = element.shadowRoot!

      ;(root.querySelector('.filter-button') as HTMLButtonElement).click()
      await element.updateComplete

      const checkboxes = Array.from(
        root.querySelectorAll('.filter-popover input[type=checkbox]')
      ) as HTMLInputElement[]
      // Status: Incomplete, Complete, Canceled — then Priority: Normal, High, Urgent.
      checkboxes[1].click() // + done
      await flush(element)
      checkboxes[5].click() // + urgent
      await flush(element)

      // By id, not by position: the popover has two selects now, and reaching
      // for "the first one" would make this test depend on fieldset order.
      const select = root.querySelector('#tasks-due-filter') as HTMLSelectElement
      select.value = 'overdue'
      select.dispatchEvent(new Event('change'))
      await flush(element)

      expect(lastBody()).toEqual({
        q: '',
        folder: null,
        statuses: ['open', 'done'],
        priorities: ['urgent'],
        due: 'overdue',
        include: 'all',
        mode: 'category',
        limit: 500,
      })
    })

    it('refuses to clear the last status, which the server would read as "incomplete"', async () => {
      element = await mount()
      const root = element.shadowRoot!
      ;(root.querySelector('.filter-button') as HTMLButtonElement).click()
      await element.updateComplete

      const openBox = root.querySelector('.filter-popover input[type=checkbox]') as HTMLInputElement
      openBox.click() // unchecking the only selected status
      await flush(element)

      expect(element.requestBody().statuses).toEqual(['open'])
      // The box has to be put back by hand: Lit dirty-checks against the value
      // it last committed, so an unchanged `_statuses` re-renders to nothing.
      expect(openBox.checked).toBe(true)
      expect(fetchMock).toHaveBeenCalledTimes(1) // no pointless refetch either
    })

    it('keeps the surviving status when a different last box is cleared', async () => {
      element = await mount()
      const root = element.shadowRoot!
      ;(root.querySelector('.filter-button') as HTMLButtonElement).click()
      await element.updateComplete

      const boxes = Array.from(
        root.querySelectorAll('.filter-popover input[type=checkbox]')
      ) as HTMLInputElement[]
      boxes[1].click() // + done  -> ['open', 'done']
      await flush(element)
      boxes[0].click() // - open  -> ['done']
      await flush(element)
      expect(element.requestBody().statuses).toEqual(['done'])

      boxes[1].click() // - done  -> refused, 'done' stays
      await flush(element)
      expect(element.requestBody().statuses).toEqual(['done'])
      expect(boxes[1].checked).toBe(true)
    })

    it('scopes to a folder when one is selected, and clears it again on Home', async () => {
      element = await mount()
      const root = element.shadowRoot!

      const labels = Array.from(root.querySelectorAll('.tree-label')) as HTMLButtonElement[]
      expect(labels.map((l) => l.querySelector('.label-text')?.textContent)).toEqual([
        'Home',
        'docs',
      ])

      labels[1].click()
      await flush(element)
      expect((lastBody() as { folder: string }).folder).toBe('/docs/')

      ;(root.querySelectorAll('.tree-label')[0] as HTMLButtonElement).click()
      await flush(element)
      expect((lastBody() as { folder: string | null }).folder).toBeNull()
    })

    it('debounces filter typing into a single request', async () => {
      element = await mount()
      const input = element.shadowRoot!.querySelector('#tasks-filter') as HTMLInputElement

      for (const value of ['r', 're', 'rep']) {
        input.value = value
        input.dispatchEvent(new Event('input'))
      }
      expect(fetchMock).toHaveBeenCalledTimes(1) // still only the mount query

      await new Promise((resolve) => setTimeout(resolve, 250))
      await flush(element)

      expect(fetchMock).toHaveBeenCalledTimes(2)
      expect((lastBody() as { q: string }).q).toBe('rep')
    })

    it('passes a #tag filter through verbatim — the server owns the grammar', async () => {
      element = await mount()
      const input = element.shadowRoot!.querySelector('#tasks-filter') as HTMLInputElement
      input.value = 'report #work'
      input.dispatchEvent(new Event('input'))
      await new Promise((resolve) => setTimeout(resolve, 250))
      await flush(element)

      expect((lastBody() as { q: string }).q).toBe('report #work')
    })
  })

  describe('opening from a page', () => {
    /** One task three folders deep, with the facet chain the server sends for it. */
    function deepResponse(): TaskQueryResponse {
      return makeResponse({
        groups: [
          makeGroup({
            key: '/docs/notes/weekly/',
            label: 'Weekly',
            url_path: '/docs/notes/weekly/',
            tasks: [makeHit({ text: 'deep', path: 'docs/notes/weekly.md' })],
          }),
        ],
        folders: [
          { path: '/', count: 1 },
          { path: '/docs/', count: 1 },
          { path: '/docs/notes/', count: 1 },
        ],
        total_matches: 1,
      })
    }

    /**
     * No matches, but the folder facets the server sends regardless: they are
     * computed ignoring the folder filter, so scoping never empties the tree.
     */
    function emptyResponse(): TaskQueryResponse {
      return makeResponse({ folders: categoryResponse().folders })
    }

    it('selects the current page’s folder in the tree', async () => {
      element = await mountFrom('docs/notes.md', categoryResponse())
      expect(selectedFolder(element)).toBe('docs')
    })

    it('expands the whole ancestor chain, so the scoped folder is on screen', async () => {
      element = await mountFrom('docs/notes/weekly.md', deepResponse())

      // `notes` is only rendered if `/docs/` was expanded; the default set
      // holds `/` alone, which would leave it hidden under a collapsed parent.
      expect(folderLabels(element)).toEqual(['Home', 'docs', 'notes'])
      expect(selectedFolder(element)).toBe('notes')
      expect((lastBody() as { folder: string | null }).folder).toBe('/docs/notes/')
    })

    it('leaves the scope alone for a page at the repository root', async () => {
      element = await mountFrom('todo.md', categoryResponse())
      expect(fetchMock).toHaveBeenCalledTimes(1)
      expect((lastBody() as { folder: string | null }).folder).toBeNull()
      expect(selectedFolder(element)).toBe('Home')
    })

    it('behaves exactly as before without a current page', async () => {
      element = await mount()
      expect(fetchMock).toHaveBeenCalledTimes(1)
      expect((lastBody() as { folder: string | null }).folder).toBeNull()
      expect(selectedFolder(element)).toBe('Home')
      expect(rowLabels(element)).toEqual([
        'H:Weekly notes',
        'T:write the report',
        'T:follow up',
        'H:Todo',
        'T:buy milk',
      ])
    })

    it('pins the current page to the top of the category list', async () => {
      // `todo.md` is at the root, so nothing is scoped: this isolates the pin.
      element = await mountFrom('todo.md', categoryResponse())
      expect(rowLabels(element)).toEqual([
        'H:Todo',
        'T:buy milk',
        'H:Weekly notes',
        'T:write the report',
        'T:follow up',
      ])
    })

    it('pins nothing in calendar mode, where a group is a date', async () => {
      const calendar = calendarResponse()
      // The current page's task in the middle bucket: were the pin to run here,
      // "Tomorrow" would jump the queue.
      calendar.groups[2].tasks[0].path = 'todo.md'
      element = await mountFrom('todo.md', categoryResponse(), calendar)

      element.shadowRoot!.querySelectorAll<HTMLButtonElement>('.mode-tab')[1].click()
      await flush(element)

      const headings = Array.from(element.shadowRoot!.querySelectorAll('.group-label')).map((n) =>
        n.textContent?.trim()
      )
      expect(headings[0]).toBe('Overdue')
    })

    describe('the empty-folder fallback', () => {
      it('widens to the whole repository rather than opening on nothing', async () => {
        element = await mountFrom('docs/notes.md', emptyResponse(), categoryResponse())

        expect(fetchMock).toHaveBeenCalledTimes(2)
        expect((bodyAt(0) as { folder: string | null }).folder).toBe('/docs/')
        expect((bodyAt(1) as { folder: string | null }).folder).toBeNull()
        // The selection follows the scope, and the empty guess is never shown.
        expect(selectedFolder(element)).toBe('Home')
        expect(rowLabels(element)).toContain('H:Weekly notes')
        expect(element.shadowRoot!.querySelector('.results-empty')).toBeNull()
      })

      it('fires at most once, so an empty folder the user picks stays picked', async () => {
        // 1st: empty (widens), 2nd: the full list, 3rd onwards: empty again.
        element = await mountFrom(
          'docs/notes.md',
          emptyResponse(),
          categoryResponse(),
          emptyResponse()
        )
        expect(fetchMock).toHaveBeenCalledTimes(2)

        const docs = element.shadowRoot!.querySelectorAll<HTMLButtonElement>('.tree-label')[1]
        docs.click()
        await flush(element)

        expect(fetchMock).toHaveBeenCalledTimes(3) // no automatic third widening
        expect((lastBody() as { folder: string | null }).folder).toBe('/docs/')
        expect(selectedFolder(element)).toBe('docs')
      })

      it('leaves the folder alone when a typed query is what came back empty', async () => {
        element = await mountFrom('docs/notes.md', categoryResponse(), emptyResponse())
        expect(fetchMock).toHaveBeenCalledTimes(1)

        const input = element.shadowRoot!.querySelector('#tasks-filter') as HTMLInputElement
        input.value = 'nothing matches this'
        input.dispatchEvent(new Event('input'))
        await new Promise((resolve) => setTimeout(resolve, 250))
        await flush(element)

        expect(fetchMock).toHaveBeenCalledTimes(2)
        expect((lastBody() as { folder: string | null }).folder).toBe('/docs/')
      })
    })
  })

  describe('stale responses', () => {
    it('ignores a slow response that a newer query has superseded', async () => {
      // First query never settles until we say so; the second lands first.
      let resolveFirst: (value: unknown) => void = () => {}
      const slow = new Promise((resolve) => {
        resolveFirst = resolve
      })
      fetchMock.mockReturnValueOnce(slow)

      element = document.createElement('mbr-tasks-panel') as MbrTasksPanelElement
      element.today = TODAY
      document.body.appendChild(element)
      await element.updateComplete

      respondWith(categoryResponse())
      element.shadowRoot!.querySelector<HTMLButtonElement>('.mode-tab:last-of-type')!.click()
      await flush(element)
      expect(rowLabels(element)).toContain('H:Weekly notes')

      // Now let the superseded first request finish with different data.
      resolveFirst({
        ok: true,
        status: 200,
        json: () =>
          Promise.resolve(
            makeResponse({
              groups: [makeGroup({ key: '/stale/', label: 'STALE', tasks: [makeHit({ text: 'x' })] })],
            })
          ),
      })
      await flush(element)

      expect(rowLabels(element)).not.toContain('H:STALE')
      expect(rowLabels(element)).toContain('H:Weekly notes')
    })

    it('surfaces a disabled endpoint rather than an empty list', async () => {
      fetchMock.mockResolvedValue({ ok: false, status: 404, json: () => Promise.resolve({}) })
      vi.spyOn(console, 'error').mockImplementation(() => {})

      element = document.createElement('mbr-tasks-panel') as MbrTasksPanelElement
      document.body.appendChild(element)
      await flush(element)

      expect(element.shadowRoot!.querySelector('.results-error')?.textContent).toContain('disabled')
    })
  })

  describe('category mode', () => {
    it('renders one heading per file, with its folder and progress', async () => {
      element = await mount()
      const heading = element.shadowRoot!.querySelector('.group-heading')!

      expect(heading.querySelector('.group-label')?.textContent?.trim()).toBe('Weekly notes')
      expect(heading.querySelector('.group-sublabel')?.textContent?.trim()).toBe('docs')
      // 3/7 even though only two tasks are shown: the counts describe the file.
      expect(heading.querySelector('.group-count')?.textContent?.trim()).toBe('3/7')
      const fill = heading.querySelector('.progress-fill') as HTMLElement
      expect(fill.getAttribute('style')).toContain('43%')
    })

    it('renders cards with the document vocabulary: dot, checkbox, pills, chips', async () => {
      element = await mount()
      const card = element.shadowRoot!.querySelector('.task-card')!

      expect(card.querySelector('.mbr-task-pri.mbr-task-pri-urgent')).not.toBeNull()
      expect(card.querySelector('.mbr-task-tag')?.textContent?.trim()).toBe('#work')
      expect(card.querySelector('.mbr-task-due')?.textContent?.trim()).toBe('Aug 5')
      expect(card.querySelector('.task-link')?.getAttribute('href')).toBe(
        '/docs/notes/#mbr-task-4'
      )
    })

    it('renders the checkbox read-only (toggling is phase 9)', async () => {
      element = await mount()
      const box = element.shadowRoot!.querySelector('.mbr-task-check') as HTMLInputElement
      expect(box.disabled).toBe(true)
      expect(box.getAttribute('data-mbr-task-line')).toBe('4')
    })

    it('indents a subtask by its depth', async () => {
      element = await mount()
      const cards = Array.from(element.shadowRoot!.querySelectorAll('.task-card')) as HTMLElement[]
      expect(cards[0].getAttribute('style')).toContain('0rem')
      expect(cards[1].getAttribute('style')).toContain('0.9rem')
    })

    it('marks an overdue due-chip at runtime, which the server render never does', async () => {
      element = await mount(
        makeResponse({
          groups: [
            makeGroup({
              key: '/n/',
              label: 'N',
              tasks: [
                makeHit({ text: 'late', due: '2026-08-01T00:00:00' }),
                makeHit({ text: 'later', line: 2, due: '2026-08-09T00:00:00' }),
              ],
            }),
          ],
        })
      )
      const chips = Array.from(element.shadowRoot!.querySelectorAll('.mbr-task-due'))
      expect(chips[0].classList.contains('mbr-task-overdue')).toBe(true)
      expect(chips[1].classList.contains('mbr-task-overdue')).toBe(false)
    })
  })

  describe('calendar mode', () => {
    it('shows the buckets in order with a synthesized Upcoming section', async () => {
      element = await mount()
      respondWith(calendarResponse())
      element.shadowRoot!.querySelectorAll<HTMLButtonElement>('.mode-tab')[1].click()
      await flush(element)

      expect((lastBody() as { mode: string }).mode).toBe('calendar')
      const headings = Array.from(element.shadowRoot!.querySelectorAll('.group-label')).map((n) =>
        n.textContent?.trim()
      )
      expect(headings).toEqual([
        'Overdue',
        'Today',
        'Tomorrow',
        'Upcoming',
        'Thu, Aug 20',
        'Fri, Aug 21',
        'No due date',
      ])
    })

    it('draws progress on Today, Tomorrow and Upcoming only', async () => {
      element = await mount()
      respondWith(calendarResponse())
      element.shadowRoot!.querySelectorAll<HTMLButtonElement>('.mode-tab')[1].click()
      await flush(element)

      const withBars = Array.from(element.shadowRoot!.querySelectorAll('.group-heading'))
        .filter((h) => h.querySelector('.progress-track'))
        .map((h) => h.querySelector('.group-label')?.textContent?.trim())
      expect(withBars).toEqual(['Today', 'Tomorrow', 'Upcoming'])
    })

    it('sums the per-date counts onto the Upcoming heading', async () => {
      element = await mount()
      respondWith(calendarResponse())
      element.shadowRoot!.querySelectorAll<HTMLButtonElement>('.mode-tab')[1].click()
      await flush(element)

      const upcoming = Array.from(element.shadowRoot!.querySelectorAll('.group-heading')).find(
        (h) => h.querySelector('.group-label')?.textContent?.trim() === 'Upcoming'
      )!
      expect(upcoming.querySelector('.group-count')?.textContent?.trim()).toBe('1/5')
    })
  })

  describe('collapsing', () => {
    it('hides a group’s tasks when its heading is clicked', async () => {
      element = await mount()
      expect(rowLabels(element)).toEqual([
        'H:Weekly notes',
        'T:write the report',
        'T:follow up',
        'H:Todo',
        'T:buy milk',
      ])

      element.shadowRoot!.querySelector<HTMLButtonElement>('.group-heading')!.click()
      await element.updateComplete
      expect(rowLabels(element)).toEqual(['H:Weekly notes', 'H:Todo', 'T:buy milk'])

      element.shadowRoot!.querySelector<HTMLButtonElement>('.group-heading')!.click()
      await element.updateComplete
      expect(rowLabels(element)).toHaveLength(5)
    })

    it('collapsing Upcoming hides its date headings too', async () => {
      element = await mount()
      respondWith(calendarResponse())
      element.shadowRoot!.querySelectorAll<HTMLButtonElement>('.mode-tab')[1].click()
      await flush(element)

      const upcoming = Array.from(
        element.shadowRoot!.querySelectorAll<HTMLButtonElement>('.group-heading')
      ).find((h) => h.querySelector('.group-label')?.textContent?.trim() === 'Upcoming')!
      upcoming.click()
      await element.updateComplete

      const headings = Array.from(element.shadowRoot!.querySelectorAll('.group-label')).map((n) =>
        n.textContent?.trim()
      )
      expect(headings).toEqual(['Overdue', 'Today', 'Tomorrow', 'Upcoming', 'No due date'])
    })

    it('resets collapse state when the mode changes, since the keys differ', async () => {
      element = await mount()
      element.shadowRoot!.querySelector<HTMLButtonElement>('.group-heading')!.click()
      await element.updateComplete
      expect(rowLabels(element)).toHaveLength(3)

      respondWith(categoryResponse())
      element.shadowRoot!.querySelectorAll<HTMLButtonElement>('.mode-tab')[1].click()
      await flush(element)
      element.shadowRoot!.querySelectorAll<HTMLButtonElement>('.mode-tab')[0].click()
      await flush(element)

      expect(rowLabels(element)).toHaveLength(5)
    })
  })

  describe('keyboard navigation', () => {
    it('walks headings and tasks, crossing group boundaries', async () => {
      element = await mount()
      const seen: (string | null)[] = []
      for (let i = 0; i < 5; i++) {
        press('ArrowDown')
        await element.updateComplete
        seen.push(focusedLabel(element))
      }
      expect(seen).toEqual([
        'H:Weekly notes',
        'T:write the report',
        'T:follow up',
        'H:Todo',
        'T:buy milk',
      ])

      // And stops at the end rather than wrapping.
      press('ArrowDown')
      await element.updateComplete
      expect(focusedLabel(element)).toBe('T:buy milk')
    })

    it('accepts the readline bindings too', async () => {
      element = await mount()
      press('n', { ctrlKey: true })
      press('n', { ctrlKey: true })
      await element.updateComplete
      expect(focusedLabel(element)).toBe('T:write the report')

      press('p', { ctrlKey: true })
      await element.updateComplete
      expect(focusedLabel(element)).toBe('H:Weekly notes')
    })

    it('skips the tasks of a collapsed group', async () => {
      element = await mount()
      element.shadowRoot!.querySelector<HTMLButtonElement>('.group-heading')!.click()
      await element.updateComplete

      press('ArrowDown')
      await element.updateComplete
      expect(focusedLabel(element)).toBe('H:Weekly notes')
      press('ArrowDown')
      await element.updateComplete
      expect(focusedLabel(element)).toBe('H:Todo')
    })

    it('collapses with ← and parks focus on the heading so → can undo it', async () => {
      element = await mount()
      press('ArrowDown')
      press('ArrowDown')
      await element.updateComplete
      expect(focusedLabel(element)).toBe('T:write the report')

      press('ArrowLeft')
      await element.updateComplete
      expect(focusedLabel(element)).toBe('H:Weekly notes')
      expect(rowLabels(element)).toEqual(['H:Weekly notes', 'H:Todo', 'T:buy milk'])

      press('ArrowRight')
      await element.updateComplete
      expect(rowLabels(element)).toHaveLength(5)
      expect(focusedLabel(element)).toBe('H:Weekly notes')
    })

    it('steps → from an expanded heading into its first task', async () => {
      element = await mount()
      press('ArrowDown')
      await element.updateComplete
      press('ArrowRight')
      await element.updateComplete
      expect(focusedLabel(element)).toBe('T:write the report')
    })

    it('leaves ←/→ to the caret while a filter is being typed', async () => {
      element = await mount()
      const input = element.shadowRoot!.querySelector('#tasks-filter') as HTMLInputElement
      input.value = 'report'
      input.dispatchEvent(new Event('input'))
      await element.updateComplete

      press('ArrowDown')
      press('ArrowDown')
      await element.updateComplete
      press('ArrowLeft')
      await element.updateComplete
      // Nothing collapsed: the arrow belonged to the text field.
      expect(rowLabels(element)).toHaveLength(5)
    })

    it('toggles a group with Enter on its heading', async () => {
      element = await mount()
      press('ArrowDown')
      await element.updateComplete
      press('Enter')
      await element.updateComplete
      expect(rowLabels(element)).toEqual(['H:Weekly notes', 'H:Todo', 'T:buy milk'])
    })

    it('navigates to the task deep link with Enter on a task', async () => {
      element = await mount()
      const assign = vi.spyOn(window.location, 'assign').mockImplementation(() => {})

      press('ArrowDown')
      press('ArrowDown')
      await element.updateComplete
      press('Enter')

      expect(assign).toHaveBeenCalledWith('/docs/notes/#mbr-task-4')
    })

    it('leaves the due-range select its own arrow keys', async () => {
      element = await mount()
      element.shadowRoot!.querySelector<HTMLButtonElement>('.filter-button')!.click()
      await element.updateComplete
      const select = element.shadowRoot!.querySelector('#tasks-due-filter') as HTMLSelectElement

      select.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, composed: true })
      )
      await element.updateComplete
      expect(focusedLabel(element)).toBeNull()
    })

    it('leaves a focused button its own Enter, so the popover stays usable', async () => {
      element = await mount()
      // Focus is on a heading button (the user clicked it); Enter must toggle
      // that button natively rather than opening whatever task is focused.
      const assign = vi.spyOn(window.location, 'assign').mockImplementation(() => {})
      press('ArrowDown')
      press('ArrowDown')
      await element.updateComplete

      const heading = element.shadowRoot!.querySelector('.group-heading') as HTMLButtonElement
      heading.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, composed: true })
      )
      expect(assign).not.toHaveBeenCalled()
    })

    it('swaps the active pane with Tab', async () => {
      element = await mount()
      expect(element.shadowRoot!.querySelector('.results-pane.active')).not.toBeNull()

      press('Tab')
      await element.updateComplete
      expect(element.shadowRoot!.querySelector('.folder-pane.active')).not.toBeNull()
      expect(element.shadowRoot!.querySelector('.results-pane.active')).toBeNull()
    })

    it('scrolls the active pane with Ctrl+d/u/f/b', async () => {
      element = await mount()
      const list = element.shadowRoot!.querySelector('.results-list') as HTMLElement
      const scrollBy = vi.fn()
      list.scrollBy = scrollBy as unknown as Element['scrollBy']

      press('d', { ctrlKey: true })
      press('u', { ctrlKey: true })
      press('f', { ctrlKey: true })
      press('b', { ctrlKey: true })
      expect(scrollBy).toHaveBeenCalledTimes(4)
    })

    it('closes on Escape by asking the trigger to close it', async () => {
      element = await mount()
      const onClose = vi.fn()
      element.addEventListener('mbr-tasks-close', onClose)
      press('Escape')
      expect(onClose).toHaveBeenCalledTimes(1)
    })

    it('Escape closes the filter popover before the panel', async () => {
      element = await mount()
      const onClose = vi.fn()
      element.addEventListener('mbr-tasks-close', onClose)
      element.shadowRoot!.querySelector<HTMLButtonElement>('.filter-button')!.click()
      await element.updateComplete

      press('Escape')
      await element.updateComplete
      expect(element.shadowRoot!.querySelector('.filter-popover')).toBeNull()
      expect(onClose).not.toHaveBeenCalled()

      press('Escape')
      expect(onClose).toHaveBeenCalledTimes(1)
    })
  })

  describe('toggling', () => {
    /** A panel with a stub toggler already wired in, per `editEnabled`. */
    async function mountEditable(
      toggler: ReturnType<typeof vi.fn>,
      editEnabled = true,
      response: TaskQueryResponse = categoryResponse()
    ) {
      respondWith(response)
      const el = document.createElement('mbr-tasks-panel') as MbrTasksPanelElement
      el.today = TODAY
      el.locale = 'en-US'
      el.editEnabled = editEnabled
      el.toggleTask = toggler as unknown as MbrTasksPanelElement['toggleTask']
      document.body.appendChild(el)
      await flush(el)
      return el
    }

    function checkboxes(el: MbrTasksPanelElement): HTMLInputElement[] {
      return Array.from(el.shadowRoot!.querySelectorAll('.task-card .mbr-task-check'))
    }

    it('leaves the checkboxes inert and the keys unbound when editing is off', async () => {
      const toggler = vi.fn()
      element = await mountEditable(toggler, false)

      expect(checkboxes(element).every((box) => box.disabled)).toBe(true)

      press('ArrowDown') // heading
      press('ArrowDown') // first task
      press(' ')
      press('x')
      await flush(element)
      expect(toggler).not.toHaveBeenCalled()
    })

    it('Space completes the focused task with its filesystem path', async () => {
      const toggler = vi.fn().mockResolvedValue({ ok: true })
      element = await mountEditable(toggler)

      press('ArrowDown')
      press('ArrowDown')
      await element.updateComplete
      expect(focusedLabel(element)).toBe('T:write the report')

      press(' ')
      await flush(element)
      // `path`, not `url_path`: `/docs/notes/` is not a file.
      expect(toggler).toHaveBeenCalledWith({ path: 'docs/notes.md', line: 4, to: 'done' })
    })

    it('x cancels the focused task, and cancels back to open', async () => {
      const toggler = vi.fn().mockResolvedValue({ ok: true })
      const canceled = categoryResponse()
      canceled.groups[0].tasks[0].status = 'canceled'
      element = await mountEditable(toggler, true, canceled)

      press('ArrowDown')
      press('ArrowDown')
      press('x')
      await flush(element)
      expect(toggler).toHaveBeenCalledWith({ path: 'docs/notes.md', line: 4, to: 'open' })
    })

    it('leaves Space and x to the filter field until a task is focused', async () => {
      const toggler = vi.fn().mockResolvedValue({ ok: true })
      element = await mountEditable(toggler)

      // Nothing focused: the keys are the field's, so they must not be eaten.
      const space = new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true })
      document.dispatchEvent(space)
      expect(space.defaultPrevented).toBe(false)
      expect(toggler).not.toHaveBeenCalled()

      // A heading is focused, not a task: still the field's.
      press('ArrowDown')
      const onHeading = new KeyboardEvent('keydown', { key: 'x', bubbles: true, cancelable: true })
      document.dispatchEvent(onHeading)
      expect(onHeading.defaultPrevented).toBe(false)
      expect(toggler).not.toHaveBeenCalled()
    })

    it('clicking a checkbox toggles instead of opening the file', async () => {
      const toggler = vi.fn().mockResolvedValue({ ok: true })
      const assign = vi.spyOn(window.location, 'assign').mockImplementation(() => {})
      element = await mountEditable(toggler)

      checkboxes(element)[0].click()
      await flush(element)

      expect(toggler).toHaveBeenCalledWith({ path: 'docs/notes.md', line: 4, to: 'done' })
      expect(assign).not.toHaveBeenCalled()
    })

    it('right-clicking a checkbox cancels and suppresses the context menu', async () => {
      const toggler = vi.fn().mockResolvedValue({ ok: true })
      element = await mountEditable(toggler)

      const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
      checkboxes(element)[0].dispatchEvent(event)
      await flush(element)

      expect(event.defaultPrevented).toBe(true)
      expect(toggler).toHaveBeenCalledWith({ path: 'docs/notes.md', line: 4, to: 'canceled' })
    })

    it('flips the card immediately and re-queries once the write lands', async () => {
      let resolve!: (outcome: { ok: true }) => void
      const toggler = vi.fn().mockReturnValue(
        new Promise<{ ok: true }>((r) => {
          resolve = r
        })
      )
      element = await mountEditable(toggler)
      expect(fetchMock).toHaveBeenCalledTimes(1)

      checkboxes(element)[0].click()
      await element.updateComplete
      // Optimistic: the write has not returned yet. `checked` is asserted as
      // well as the attribute because the card cancels the browser's own flip
      // and relies on Lit re-committing the PROPERTY — an `?checked` attribute
      // binding would stop reflecting the moment the user interacted.
      expect(checkboxes(element)[0].dataset.mbrTaskStatus).toBe('done')
      expect(checkboxes(element)[0].checked).toBe(true)

      resolve({ ok: true })
      await flush(element)
      expect(fetchMock).toHaveBeenCalledTimes(2)
    })

    it('reverts the flip and explains a failure without re-querying', async () => {
      const toggler = vi
        .fn()
        .mockResolvedValue({ ok: false, kind: 'other', message: 'The task could not be saved.' })
      element = await mountEditable(toggler)

      checkboxes(element)[0].click()
      await flush(element)

      expect(checkboxes(element)[0].dataset.mbrTaskStatus).toBe('open')
      expect(checkboxes(element)[0].checked).toBe(false)
      expect(element.shadowRoot!.querySelector('.results-notice')?.textContent).toContain(
        'could not be saved'
      )
      // A plain failure leaves the view alone: only the write was refused.
      expect(fetchMock).toHaveBeenCalledTimes(1)
    })

    it('a 409 surfaces a message and refreshes the stale view', async () => {
      const toggler = vi
        .fn()
        .mockResolvedValue({ ok: false, kind: 'conflict', message: 'That line changed on disk.' })
      element = await mountEditable(toggler)

      checkboxes(element)[0].click()
      await flush(element)

      expect(element.shadowRoot!.querySelector('.results-notice')?.textContent).toContain(
        'changed on disk'
      )
      expect(checkboxes(element)[0].dataset.mbrTaskStatus).toBe('open')
      expect(fetchMock).toHaveBeenCalledTimes(2)
    })

    it('keeps focus on the task across the refresh, so Space can repeat', async () => {
      const toggler = vi.fn().mockResolvedValue({ ok: true })
      element = await mountEditable(toggler)

      press('ArrowDown')
      press('ArrowDown')
      await element.updateComplete
      press(' ')
      await flush(element)

      // The stub fetch replays the same response, so the same task is still
      // there — and still focused, rather than reset to nothing.
      expect(focusedLabel(element)).toBe('T:write the report')
      press(' ')
      await flush(element)
      expect(toggler).toHaveBeenCalledTimes(2)
    })

    it('ignores a second press while the first write is in flight', async () => {
      const toggler = vi.fn().mockReturnValue(new Promise(() => {}))
      element = await mountEditable(toggler)

      checkboxes(element)[0].click()
      await element.updateComplete
      checkboxes(element)[0].click()
      await element.updateComplete

      expect(toggler).toHaveBeenCalledTimes(1)
    })
  })

  describe('markers', () => {
    /**
     * The marker fixture, with editing on and a stub toggler wired in — so a
     * marker that declines a write is declining it on its own account rather
     * than because nothing could have written anyway.
     */
    async function mountWithMarkers(toggler = vi.fn().mockResolvedValue({ ok: true })) {
      respondWith(markerResponse())
      const el = document.createElement('mbr-tasks-panel') as MbrTasksPanelElement
      el.today = TODAY
      el.locale = 'en-US'
      el.editEnabled = true
      el.toggleTask = toggler as unknown as MbrTasksPanelElement['toggleTask']
      document.body.appendChild(el)
      await flush(el)
      return el
    }

    /** `[task card, marker card]` — the fixture's source order. */
    function cards(el: MbrTasksPanelElement): HTMLElement[] {
      return Array.from(el.shadowRoot!.querySelectorAll('.task-card'))
    }

    function includeSelect(el: MbrTasksPanelElement): HTMLSelectElement {
      return el.shadowRoot!.querySelector('#tasks-include-filter') as HTMLSelectElement
    }

    async function openFilters(el: MbrTasksPanelElement): Promise<void> {
      el.shadowRoot!.querySelector<HTMLButtonElement>('.filter-button')!.click()
      await el.updateComplete
    }

    it('renders a spacer instead of a checkbox, while its neighbour keeps one', async () => {
      element = await mountWithMarkers()
      const [task, marker] = cards(element)

      // Absent, not disabled: `data-mbr-task-line` / `-status` are exactly what
      // `task-toggle.ts` reads back, and markup that is not there cannot be
      // mistargeted.
      expect(marker.querySelector('.mbr-task-check')).toBeNull()
      expect(marker.querySelector('.mbr-task-check-spacer')).not.toBeNull()
      // The task beside it proves the branch rather than merely its absence.
      expect(task.querySelector('.mbr-task-check')).not.toBeNull()
      expect(task.querySelector('.mbr-task-check-spacer')).toBeNull()
    })

    it('draws no chips, and keeps the priority rail with a spacer', async () => {
      element = await mountWithMarkers()
      const marker = cards(element)[1]

      expect(marker.querySelector('.task-chips')).toBeNull()
      expect(marker.querySelector('.mbr-task-pri')).toBeNull()
      expect(marker.querySelector('.mbr-task-pri-spacer')).not.toBeNull()
      // The text is the whole source line, marker word included.
      expect(marker.querySelector('.task-link')?.textContent?.trim()).toBe(
        'The market fell 10% (source: TK).'
      )
    })

    it('washes only the marker word, leaving the rest of the line untouched', async () => {
      element = await mountWithMarkers()
      const [task, marker] = cards(element)
      const link = marker.querySelector('.task-link')!

      const washed = link.querySelectorAll('.task-marker')
      expect(washed.length).toBe(1)
      expect(washed[0].textContent).toBe('TK')
      // The line still reads verbatim: the span splits the text, it does not
      // rewrite it. Highlighting the whole card would say the sentence is
      // unfinished rather than pointing at the word that says so.
      expect(link.textContent).toBe('The market fell 10% (source: TK).')
      // The task beside it proves the branch rather than merely its absence.
      expect(task.querySelector('.task-marker')).toBeNull()
    })

    it('never washes a checkbox task, even one whose own text says TODO', async () => {
      // A task carries no span, so the word is prose. The card keys off the
      // server's range, not off the string — an `indexOf` here would wash this.
      respondWith(
        makeResponse({
          groups: [
            makeGroup({
              key: '/notes/',
              url_path: '/notes/',
              total: 1,
              tasks: [makeHit({ text: 'rename the TODO list page', line: 3 })],
            }),
          ],
          total_matches: 1,
        })
      )
      element = await mountBare(null)

      const link = element.shadowRoot!.querySelector('.task-link')!
      expect(link.querySelector('.task-marker')).toBeNull()
      expect(link.textContent).toBe('rename the TODO list page')
    })

    it('degrades an unusable span to plain text instead of mis-slicing', async () => {
      // Three ways the range can be wrong — past the end of a text that got
      // shorter, inverted, and absent on a hit that still claims to be a
      // marker. A missing wash is invisible; a mis-sliced one corrupts the
      // words the reader came here to find.
      const line = 'The market fell 10% (source: TK).'
      respondWith(
        makeResponse({
          groups: [
            makeGroup({
              key: '/notes/',
              url_path: '/notes/',
              tasks: [
                makeMarker({ text: line, line: 1, marker_end: line.length + 40 }),
                makeMarker({ text: line, line: 2, marker_start: 31, marker_end: 29 }),
                makeMarker({ text: line, line: 3, marker_start: null, marker_end: null }),
              ],
            }),
          ],
          total_matches: 3,
        })
      )
      element = await mountBare(null)

      const links = Array.from(element.shadowRoot!.querySelectorAll('.task-link'))
      expect(links.length).toBe(3)
      for (const link of links) {
        expect(link.querySelector('.task-marker')).toBeNull()
        expect(link.textContent).toBe(line)
      }
    })

    it('deep links to #mbr-marker-N, which is a different id from a task’s', async () => {
      element = await mountWithMarkers()
      const [task, marker] = cards(element)

      expect(marker.querySelector('.task-link')?.getAttribute('href')).toBe('/notes/#mbr-marker-9')
      expect(task.querySelector('.task-link')?.getAttribute('href')).toBe('/notes/#mbr-task-3')
    })

    it('navigates a click to that same href, not to a #mbr-task-N that does not exist', async () => {
      element = await mountWithMarkers()
      const assign = vi.spyOn(window.location, 'assign').mockImplementation(() => {})

      cards(element)[1].click()

      // The card's own navigation and the rendered href come from one function;
      // a second hand-built fragment here is what this catches.
      expect(assign).toHaveBeenCalledWith('/notes/#mbr-marker-9')
    })

    it('leaves Space and x to the filter field when a marker is focused', async () => {
      const toggler = vi.fn().mockResolvedValue({ ok: true })
      element = await mountWithMarkers(toggler)

      press('ArrowDown') // heading
      press('ArrowDown') // the real task
      press('ArrowDown') // the marker
      await element.updateComplete
      expect(focusedLabel(element)).toBe('T:The market fell 10% (source: TK).')

      // Not prevented: the guard has to return BEFORE `preventDefault()`, so
      // the key falls back to the field exactly as it does on a heading — a
      // swallowed keystroke would cost the user a character for nothing.
      const space = new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true })
      document.dispatchEvent(space)
      const cancel = new KeyboardEvent('keydown', { key: 'x', bubbles: true, cancelable: true })
      document.dispatchEvent(cancel)
      await flush(element)

      expect(space.defaultPrevented).toBe(false)
      expect(cancel.defaultPrevented).toBe(false)
      expect(toggler).not.toHaveBeenCalled()
    })

    it('still toggles the real task in the same list', async () => {
      const toggler = vi.fn().mockResolvedValue({ ok: true })
      element = await mountWithMarkers(toggler)

      press('ArrowDown')
      press('ArrowDown')
      await element.updateComplete
      press(' ')
      await flush(element)

      expect(toggler).toHaveBeenCalledWith({ path: 'notes.md', line: 3, to: 'done' })
    })

    it('sends the Show choice, defaulting to all', async () => {
      element = await mountWithMarkers()
      expect((lastBody() as { include: string }).include).toBe('all')

      await openFilters(element)
      const select = includeSelect(element)
      select.value = 'markers'
      select.dispatchEvent(new Event('change'))
      await flush(element)

      expect((lastBody() as { include: string }).include).toBe('markers')
    })

    it('marks the initial option selected, which is what survives the first render', async () => {
      element = await mountWithMarkers()
      await openFilters(element)

      // The `?selected` half of the double binding, asserted as the ATTRIBUTE
      // it actually sets. On the first render Lit commits the <select>'s
      // `.value` PropertyPart before the options ChildPart exists, so in a real
      // browser `.value` is dropped and this attribute is the only thing
      // carrying the initial selection. (happy-dom's <select> is more forgiving
      // about `.value` without options, so this is asserted directly rather
      // than through `select.value`.)
      const options = Array.from(includeSelect(element).querySelectorAll('option'))
      expect(options.map((o) => o.getAttribute('value'))).toEqual(['all', 'tasks', 'markers'])
      expect(options.filter((o) => o.hasAttribute('selected')).map((o) => o.value)).toEqual(['all'])
    })

    it('pins Show to tasks in By Due, and restores the choice on the way back', async () => {
      element = await mountWithMarkers()
      await openFilters(element)

      const select = includeSelect(element)
      select.value = 'markers'
      select.dispatchEvent(new Event('change'))
      await flush(element)
      expect(select.disabled).toBe(false)

      // A marker has no due date, so no bucket could hold one and "Markers
      // only" would ask for a guaranteed-empty list.
      element.shadowRoot!.querySelectorAll<HTMLButtonElement>('.mode-tab')[1].click()
      await flush(element)
      expect((lastBody() as { include: string }).include).toBe('tasks')
      expect(includeSelect(element).disabled).toBe(true)
      // The `.value` half of the double binding: `?selected` alone would not
      // move a selection that is already committed.
      expect(includeSelect(element).value).toBe('tasks')

      element.shadowRoot!.querySelectorAll<HTMLButtonElement>('.mode-tab')[0].click()
      await flush(element)
      // Derived, not assigned: the user's category-mode choice survived.
      expect((lastBody() as { include: string }).include).toBe('markers')
      expect(includeSelect(element).disabled).toBe(false)
      expect(includeSelect(element).value).toBe('markers')
    })

    it('explains the pin on the fieldset, where a disabled control cannot', async () => {
      element = await mountWithMarkers()
      await openFilters(element)
      const fieldset = includeSelect(element).closest('fieldset')!

      expect(fieldset.getAttribute('title')).toBeNull()

      element.shadowRoot!.querySelectorAll<HTMLButtonElement>('.mode-tab')[1].click()
      await flush(element)
      // On the fieldset, not the <select>: a disabled control suppresses
      // pointer events, so its own tooltip would never render.
      expect(fieldset.getAttribute('title')).toContain('no due date')
    })
  })

  describe('empty and partial states', () => {
    it('says so when nothing matches', async () => {
      element = await mount(makeResponse())
      expect(element.shadowRoot!.querySelector('.results-empty')?.textContent).toContain(
        'No tasks match'
      )
    })

    it('reports a scan still in progress, so partial results are explained', async () => {
      element = await mount(makeResponse({ scan_in_progress: true }))
      expect(element.shadowRoot!.querySelector('.scanning')).not.toBeNull()
    })

    it('stops listening for keys once removed from the DOM', async () => {
      element = await mount()
      const onClose = vi.fn()
      element.addEventListener('mbr-tasks-close', onClose)
      element.remove()
      press('Escape')
      expect(onClose).not.toHaveBeenCalled()
    })
  })
})
