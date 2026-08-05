import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import './mbr-tasks-panel.js'
import type { MbrTasksPanelElement } from './mbr-tasks-panel.js'
import { calendarResponse, categoryResponse, makeGroup, makeHit, makeResponse } from './test-fixtures.js'
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
  const element = document.createElement('mbr-tasks-panel') as MbrTasksPanelElement
  element.today = TODAY
  element.locale = 'en-US'
  document.body.appendChild(element)
  await flush(element)
  return element
}

function press(key: string, init: KeyboardEventInit = {}): void {
  document.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...init }))
}

function lastBody(): unknown {
  const call = fetchMock.mock.calls[fetchMock.mock.calls.length - 1]
  return JSON.parse((call[1] as RequestInit).body as string)
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
    it('posts the default filter state on open', async () => {
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

      const select = root.querySelector('.filter-popover select') as HTMLSelectElement
      select.value = 'overdue'
      select.dispatchEvent(new Event('change'))
      await flush(element)

      expect(lastBody()).toEqual({
        q: '',
        folder: null,
        statuses: ['open', 'done'],
        priorities: ['urgent'],
        due: 'overdue',
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
      const select = element.shadowRoot!.querySelector('.filter-popover select') as HTMLSelectElement

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
