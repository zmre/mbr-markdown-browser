import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import './mbr-task-doc.js'
import { flashTask, revealTaskFromHash, taskLineFromHash } from './mbr-task-doc.js'
import { resetTaskToggleState, wasSelfWrite } from './task-toggle.js'

/**
 * Rendered markup for one open and one completed task, as `html.rs` emits it.
 *
 * The first task carries a `@due(...)` chip so the completion chip added by a
 * toggle has a neighbour to be ordered against; the second is already done and
 * stamped, so reopening it has a chip to take away.
 */
const DOCUMENT = `
  <main id="wrapper">
    <ul>
      <li><input type="checkbox" class="mbr-task-check" id="mbr-task-3"
                 data-mbr-task-line="3" data-mbr-task-status="open" disabled>
        <span class="mbr-task-text">write the report</span> <time class="mbr-task-due" datetime="2026-08-05">Aug 5</time></li>
      <li><input type="checkbox" class="mbr-task-check" id="mbr-task-4"
                 data-mbr-task-line="4" data-mbr-task-status="done" checked disabled>
        <span class="mbr-task-text">second</span> <time class="mbr-task-completed" datetime="2026-08-01">Aug 1</time></li>
    </ul>
  </main>
`

const SOURCE =
  '# Notes\n\n- [ ] write the report !! @due(2026-08-05)\n- [x] second @done(2026-08-01)\n'

/** What the server writes back for each line it patches, keyed by line. */
const PATCHED: Record<number, string> = {
  3: '- [x] write the report !! @due(2026-08-05) @done(2026-08-04 22:16)',
  4: '- [ ] second',
}

let fetchMock: ReturnType<typeof vi.fn>
let alertMock: ReturnType<typeof vi.fn>

/** Raw reads always succeed; `/.mbr/task` answers with `taskStatus`. */
function routeFetch(taskStatus = 200) {
  fetchMock.mockImplementation((url: string, init?: RequestInit) => {
    if (String(url).startsWith('/.mbr/raw/')) {
      return Promise.resolve({ ok: true, status: 200, text: () => Promise.resolve(SOURCE) })
    }
    const line = (JSON.parse(String(init?.body ?? '{}')) as { line?: number }).line ?? 3
    return Promise.resolve({
      ok: taskStatus >= 200 && taskStatus < 300,
      status: taskStatus,
      json: () => Promise.resolve({ line, text: PATCHED[line] ?? '' }),
    })
  })
}

/** The completion chip beside a task, if it has one. */
function doneChip(line: number): Element | null {
  return checkbox(line).parentElement!.querySelector('.mbr-task-completed')
}

/** Mount the element and let its `waitForDom()` promise settle. */
async function mount(): Promise<HTMLElement> {
  const element = document.createElement('mbr-task-doc')
  document.body.appendChild(element)
  await flush()
  return element
}

async function flush(): Promise<void> {
  for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0))
}

function checkbox(line: number): HTMLInputElement {
  return document.getElementById(`mbr-task-${line}`) as HTMLInputElement
}

function taskCalls(): Array<[string, RequestInit]> {
  return fetchMock.mock.calls.filter(
    (call) => String(call[0]) === '/.mbr/task'
  ) as Array<[string, RequestInit]>
}

describe('taskLineFromHash', () => {
  it('accepts only a whole `mbr-task-<n>` fragment', () => {
    expect(taskLineFromHash('#mbr-task-42')).toBe(42)
    expect(taskLineFromHash('mbr-task-1')).toBe(1)
    // Anything else belongs to some other anchor and must not be hijacked.
    expect(taskLineFromHash('#mbr-task-42x')).toBeNull()
    expect(taskLineFromHash('#mbr-tasks')).toBeNull()
    expect(taskLineFromHash('#my-section')).toBeNull()
    expect(taskLineFromHash('')).toBeNull()
    expect(taskLineFromHash('#mbr-task-0')).toBeNull()
  })
})

describe('the jump-to-task fragment handler', () => {
  beforeEach(() => {
    document.body.innerHTML = DOCUMENT
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
  })

  afterEach(() => {
    document.body.innerHTML = ''
    window.__MBR_CONFIG__ = undefined
    window.location.hash = ''
  })

  it('flashes the list item containing the addressed task', () => {
    const target = revealTaskFromHash('#mbr-task-4')

    expect(target).toBe(checkbox(4).closest('li'))
    expect(target!.classList.contains('mbr-task-flash')).toBe(true)
    // The other task is untouched.
    expect(checkbox(3).closest('li')!.classList.contains('mbr-task-flash')).toBe(false)
  })

  it('no-ops on a fragment that is not a task, or names a missing line', () => {
    expect(revealTaskFromHash('#some-heading')).toBeNull()
    expect(revealTaskFromHash('#mbr-task-999')).toBeNull()
    expect(document.querySelectorAll('.mbr-task-flash')).toHaveLength(0)
  })

  it('runs on mount and again on hashchange', async () => {
    window.location.hash = '#mbr-task-3'
    const element = await mount()
    expect(checkbox(3).closest('li')!.classList.contains('mbr-task-flash')).toBe(true)

    window.location.hash = '#mbr-task-4'
    window.dispatchEvent(new HashChangeEvent('hashchange'))
    expect(checkbox(4).closest('li')!.classList.contains('mbr-task-flash')).toBe(true)

    element.remove()
  })

  it('clears the flash when the animation ends', () => {
    const item = checkbox(3).closest('li')!
    flashTask(item)
    expect(item.classList.contains('mbr-task-flash')).toBe(true)

    item.dispatchEvent(new Event('animationend'))
    expect(item.classList.contains('mbr-task-flash')).toBe(false)
  })

  it('a re-flash outlives the previous run’s cleanup timeout', () => {
    vi.useFakeTimers()
    try {
      const item = checkbox(3).closest('li')!
      flashTask(item)
      vi.advanceTimersByTime(1000)
      // Back/forward between two tasks lands here, restarting the animation.
      flashTask(item)

      // The first flash's fallback timeout now fires. It must not cut the
      // second one short.
      vi.advanceTimersByTime(1600)
      expect(item.classList.contains('mbr-task-flash')).toBe(true)

      // The second one's own timeout still ends it.
      vi.advanceTimersByTime(1000)
      expect(item.classList.contains('mbr-task-flash')).toBe(false)
    } finally {
      vi.useRealTimers()
    }
  })
})

describe('in-document checkbox toggling', () => {
  let element: HTMLElement | null = null

  beforeEach(() => {
    resetTaskToggleState()
    document.body.innerHTML = DOCUMENT
    window.frontmatter = { markdown_source: 'notes.md' }
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: true }
    fetchMock = vi.fn()
    globalThis.fetch = fetchMock as unknown as typeof fetch
    routeFetch()
    alertMock = vi.fn()
    window.alert = alertMock as unknown as typeof window.alert
  })

  afterEach(() => {
    element?.remove()
    element = null
    resetTaskToggleState()
    document.body.innerHTML = ''
    window.frontmatter = undefined
    window.__MBR_CONFIG__ = undefined
    vi.restoreAllMocks()
  })

  it('enables the rendered checkboxes when editing is on', async () => {
    element = await mount()

    expect(checkbox(3).disabled).toBe(false)
    expect(checkbox(3).classList.contains('mbr-task-editable')).toBe(true)
  })

  it('leaves the checkboxes inert and sends nothing when editing is off', async () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: false }
    element = await mount()

    expect(checkbox(3).disabled).toBe(true)
    checkbox(3).click()
    await flush()
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('left click completes the task and posts the line it came from', async () => {
    element = await mount()

    checkbox(3).click()
    // The flip is immediate: the write has not returned yet.
    expect(checkbox(3).checked).toBe(true)
    expect(checkbox(3).dataset.mbrTaskStatus).toBe('done')
    await flush()

    expect(taskCalls()).toHaveLength(1)
    expect(JSON.parse(taskCalls()[0][1].body as string)).toEqual({
      path: 'notes.md',
      line: 3,
      expected: '- [ ] write the report !! @due(2026-08-05)',
      to: 'done',
    })
  })

  it('left click on a completed task reopens it', async () => {
    element = await mount()

    checkbox(4).click()
    await flush()

    expect(JSON.parse(taskCalls()[0][1].body as string)).toMatchObject({ line: 4, to: 'open' })
  })

  it('draws the freshly stamped @done(...) instead of reloading for it', async () => {
    element = await mount()

    checkbox(3).click()
    await flush()

    // The page is not going to be re-rendered: this element has to produce the
    // chip the reload used to, in the position `task_annotations_html` puts it.
    const chip = doneChip(3)!
    expect(chip.getAttribute('datetime')).toBe('2026-08-04T22:16')
    expect(chip.textContent).toBe('Aug 4, 10:16 PM')
    expect(chip.previousElementSibling!.className).toBe('mbr-task-due')

    // ...and the reload it would otherwise trigger is suppressed, which is what
    // keeps the in-memory edit token alive for the next click.
    expect(wasSelfWrite('notes.md')).toBe(true)
  })

  it('takes the stamp away again when a task is reopened', async () => {
    element = await mount()
    expect(doneChip(4)).not.toBeNull()

    checkbox(4).click()
    await flush()

    expect(checkbox(4).checked).toBe(false)
    expect(doneChip(4)).toBeNull()
  })

  it('leaves the chips alone when the write is refused', async () => {
    routeFetch(500)
    element = await mount()

    checkbox(3).click()
    await flush()

    expect(doneChip(3)).toBeNull()
    expect(checkbox(3).dataset.mbrTaskStatus).toBe('open')
  })

  it('right click cancels and suppresses the browser context menu', async () => {
    element = await mount()

    const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
    checkbox(3).dispatchEvent(event)

    expect(event.defaultPrevented).toBe(true)
    expect(checkbox(3).dataset.mbrTaskStatus).toBe('canceled')
    expect(
      checkbox(3).parentElement!.querySelector('.mbr-task-text')!.classList.contains(
        'mbr-task-canceled'
      )
    ).toBe(true)
    await flush()

    expect(JSON.parse(taskCalls()[0][1].body as string)).toMatchObject({ line: 3, to: 'canceled' })
  })

  it('reverts the optimistic flip when the write fails', async () => {
    routeFetch(500)
    element = await mount()

    checkbox(3).click()
    expect(checkbox(3).checked).toBe(true)
    await flush()

    expect(checkbox(3).checked).toBe(false)
    expect(checkbox(3).dataset.mbrTaskStatus).toBe('open')
    expect(alertMock).toHaveBeenCalledOnce()
  })

  it('reverts and explains a 409', async () => {
    routeFetch(409)
    element = await mount()

    checkbox(3).click()
    await flush()

    expect(checkbox(3).dataset.mbrTaskStatus).toBe('open')
    expect(alertMock.mock.calls[0][0]).toMatch(/changed on disk/)
  })

  it('binds one listener on the wrapper rather than one per checkbox', async () => {
    const wrapper = document.querySelector('main#wrapper')!
    const addSpy = vi.spyOn(wrapper, 'addEventListener')
    element = await mount()

    expect(addSpy.mock.calls.map((call) => call[0]).sort()).toEqual(['click', 'contextmenu'])
  })
})
