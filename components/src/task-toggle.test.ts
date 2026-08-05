import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import {
  applyCheckboxStatus,
  checkboxStatus,
  currentDocumentPath,
  nextCancelStatus,
  nextToggleStatus,
  resetTaskToggleState,
  syncDocumentTask,
  toggleTask,
  wasSelfWrite,
} from './task-toggle.js'
import { clearEditToken, isEditTokenRequired, setEditToken } from './edit-token.js'

/** The file every test here patches, and the line it aims at. */
const SOURCE = '# Notes\n\n- [ ] write the report !!\n- [ ] second\n'

let fetchMock: ReturnType<typeof vi.fn>

/** Reply to `/.mbr/raw/...` with SOURCE and to `/.mbr/task` with `taskReply`. */
function routeFetch(taskReply: Partial<Response> & { status: number; ok: boolean }) {
  fetchMock.mockImplementation((url: string) => {
    if (String(url).startsWith('/.mbr/raw/')) {
      return Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(SOURCE),
      })
    }
    return Promise.resolve({
      json: () => Promise.resolve({ line: 3, text: '- [x] write the report !!' }),
      ...taskReply,
    })
  })
}

function taskCalls(): Array<[string, RequestInit]> {
  return fetchMock.mock.calls.filter(
    (call) => String(call[0]) === '/.mbr/task'
  ) as Array<[string, RequestInit]>
}

function lastTaskBody(): unknown {
  const calls = taskCalls()
  return JSON.parse(calls[calls.length - 1][1].body as string)
}

describe('toggleTask', () => {
  beforeEach(() => {
    resetTaskToggleState()
    clearEditToken()
    // The token module is inert on a page with editing off, which is every
    // page that has no `__MBR_CONFIG__` at all.
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: true }
    fetchMock = vi.fn()
    globalThis.fetch = fetchMock as unknown as typeof fetch
    routeFetch({ ok: true, status: 200 })
  })

  afterEach(() => {
    resetTaskToggleState()
    clearEditToken()
    window.__MBR_CONFIG__ = undefined
    vi.restoreAllMocks()
  })

  it('sends exactly the body the endpoint expects', async () => {
    const outcome = await toggleTask({ path: 'notes.md', line: 3, to: 'done' })

    // The new source line comes back with the outcome: with the live reload
    // suppressed, it is the only thing that can redraw a `@done(...)` chip.
    expect(outcome).toEqual({ ok: true, text: '- [x] write the report !!' })
    // The whole body is spelled out: `TaskToggleRequest` is `#[serde(default)]`
    // on the Rust side, so a misspelled key degrades silently rather than
    // erroring, and this assertion is the only guard against that.
    expect(lastTaskBody()).toEqual({
      path: 'notes.md',
      line: 3,
      // Sourced from /.mbr/raw, terminator stripped — the exact source line,
      // annotations and all, which the rendered page no longer contains.
      expected: '- [ ] write the report !!',
      to: 'done',
    })

    const [, init] = taskCalls()[0]
    expect(init.method).toBe('POST')
    expect(init.credentials).toBe('same-origin')
    expect(init.headers).toMatchObject({
      'X-MBR-Edit': '1',
      'Content-Type': 'application/json',
    })
  })

  it('reads the raw source once per file and keeps it current from the response', async () => {
    await toggleTask({ path: 'notes.md', line: 3, to: 'done' })
    await toggleTask({ path: 'notes.md', line: 3, to: 'open' })

    const rawCalls = fetchMock.mock.calls.filter((call) => String(call[0]).startsWith('/.mbr/raw/'))
    expect(rawCalls).toHaveLength(1)
    // The second request's `expected` is the line the first write returned,
    // not the stale one it started from.
    expect(lastTaskBody()).toMatchObject({ expected: '- [x] write the report !!' })
  })

  it('reads a file once for two toggles started together', async () => {
    // Two boxes in one file, clicked in quick succession. Without
    // single-flighting, the slower read holds bytes from before the faster
    // one's write and lands last, poisoning the cache with a stale line.
    await Promise.all([
      toggleTask({ path: 'notes.md', line: 3, to: 'done' }),
      toggleTask({ path: 'notes.md', line: 4, to: 'done' }),
    ])

    const rawCalls = fetchMock.mock.calls.filter((call) => String(call[0]).startsWith('/.mbr/raw/'))
    expect(rawCalls).toHaveLength(1)
    expect(taskCalls()).toHaveLength(2)
    const sent = taskCalls().map((call) => JSON.parse(call[1].body as string).expected)
    expect(sent).toEqual(['- [ ] write the report !!', '- [ ] second'])
  })

  it('percent-encodes each path segment of the raw URL', async () => {
    await toggleTask({ path: 'my notes/a b.md', line: 3, to: 'done' })

    expect(fetchMock.mock.calls[0][0]).toBe('/.mbr/raw/my%20notes/a%20b.md')
  })

  it('carries the bearer token when one is known', async () => {
    setEditToken('s3cret')
    await toggleTask({ path: 'notes.md', line: 3, to: 'done' })

    expect(taskCalls()[0][1].headers).toMatchObject({ Authorization: 'Bearer s3cret' })
  })

  it('reports a 409 as a conflict and re-reads the file next time', async () => {
    routeFetch({ ok: false, status: 409 })
    const conflict = await toggleTask({ path: 'notes.md', line: 3, to: 'done' })
    expect(conflict).toEqual({
      ok: false,
      kind: 'conflict',
      message: 'That line changed on disk, so nothing was written.',
    })

    routeFetch({ ok: true, status: 200 })
    await toggleTask({ path: 'notes.md', line: 3, to: 'done' })
    const rawCalls = fetchMock.mock.calls.filter((call) => String(call[0]).startsWith('/.mbr/raw/'))
    expect(rawCalls).toHaveLength(2)
  })

  it('classifies auth failures separately from everything else', async () => {
    routeFetch({ ok: false, status: 403 })
    expect(await toggleTask({ path: 'notes.md', line: 3, to: 'done' })).toMatchObject({
      ok: false,
      kind: 'auth',
    })

    resetTaskToggleState()
    routeFetch({ ok: false, status: 500 })
    expect(await toggleTask({ path: 'notes.md', line: 3, to: 'done' })).toMatchObject({
      ok: false,
      kind: 'other',
    })
  })

  it('refuses to guess `expected` when the source cannot be read', async () => {
    fetchMock.mockImplementation((url: string) =>
      String(url).startsWith('/.mbr/raw/')
        ? Promise.resolve({ ok: false, status: 404, text: () => Promise.resolve('') })
        : Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve({}) })
    )

    const outcome = await toggleTask({ path: 'notes.md', line: 3, to: 'done' })
    expect(outcome).toMatchObject({ ok: false, kind: 'other' })
    expect(taskCalls()).toHaveLength(0)
  })

  it('reports a token-refused raw read as an auth problem, not a read problem', async () => {
    // `/.mbr/raw` sits behind the same `check_edit_access` policy as the write,
    // so on a token-protected server the FIRST thing a click does is 401 —
    // before `/.mbr/task` is ever reached. Reporting that as "could not read
    // the file" would hide the one thing the user can do about it.
    fetchMock.mockImplementation(() =>
      Promise.resolve({ ok: false, status: 401, text: () => Promise.resolve('') })
    )

    const outcome = await toggleTask({ path: 'notes.md', line: 3, to: 'done' })

    expect(outcome).toEqual({
      ok: false,
      kind: 'auth',
      message: 'Editing needs a token — open the editor (e) and enter it first.',
    })
    expect(taskCalls()).toHaveLength(0)
    // ...and the editor will have its token field waiting when they get there.
    expect(isEditTokenRequired()).toBe(true)
  })

  it('reports a token-refused write as an auth problem too', async () => {
    routeFetch({ ok: false, status: 401 })

    const outcome = await toggleTask({ path: 'notes.md', line: 3, to: 'done' })

    expect(outcome).toMatchObject({ ok: false, kind: 'auth' })
    expect(isEditTokenRequired()).toBe(true)
  })

  it('still reports success when the response body is unusable, minus the text', async () => {
    routeFetch({
      ok: true,
      status: 200,
      json: () => Promise.reject(new Error('not JSON')),
    } as never)

    // The status code already confirmed the write; only the chip is lost.
    expect(await toggleTask({ path: 'notes.md', line: 3, to: 'done' })).toEqual({ ok: true })

    // And the cached line was dropped, so the next click re-reads.
    routeFetch({ ok: true, status: 200 })
    await toggleTask({ path: 'notes.md', line: 3, to: 'done' })
    const rawCalls = fetchMock.mock.calls.filter((call) => String(call[0]).startsWith('/.mbr/raw/'))
    expect(rawCalls).toHaveLength(2)
  })

  it('refuses a line the file does not have', async () => {
    const outcome = await toggleTask({ path: 'notes.md', line: 99, to: 'done' })
    expect(outcome).toMatchObject({ ok: false, kind: 'other' })
    expect(taskCalls()).toHaveLength(0)
  })

  it('numbers lines the way the server does, ignoring a lone CR', async () => {
    fetchMock.mockImplementation((url: string) =>
      String(url).startsWith('/.mbr/raw/')
        ? Promise.resolve({
            ok: true,
            status: 200,
            // One `\n`, so the server sees two lines; the `\r` inside line 1 is
            // part of its content, not a line break.
            text: () => Promise.resolve('- [ ] a\ra-tail\n- [ ] b\r\n'),
          })
        : Promise.resolve({
            ok: true,
            status: 200,
            json: () => Promise.resolve({ line: 2, text: '- [x] b' }),
          })
    )

    await toggleTask({ path: 'notes.md', line: 2, to: 'done' })
    // Line 2 is `- [ ] b`, with its trailing CR stripped like the server does.
    expect(lastTaskBody()).toMatchObject({ expected: '- [ ] b' })
  })
})

describe('self-write suppression', () => {
  beforeEach(() => {
    resetTaskToggleState()
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, editEnabled: true }
    fetchMock = vi.fn()
    globalThis.fetch = fetchMock as unknown as typeof fetch
    routeFetch({ ok: true, status: 200 })
  })

  afterEach(() => {
    resetTaskToggleState()
    window.__MBR_CONFIG__ = undefined
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  it('suppresses every event a single write produces, not just the first', async () => {
    await toggleTask({ path: 'notes.md', line: 3, to: 'done' })

    // One write is announced three times on macOS: the handler broadcasts
    // before it responds, then the watcher sees the atomic rename. Consuming a
    // single entry would let the echo reload the page — taking the in-memory
    // edit token with it — which is the whole failure this guards.
    // The path arrives from the watcher, which may lead with a slash.
    expect(wasSelfWrite('/notes.md')).toBe(true)
    expect(wasSelfWrite('notes.md')).toBe(true)
    expect(wasSelfWrite('notes.md')).toBe(true)
  })

  it('registers before the request, because the broadcast can beat the response', async () => {
    // The handler sends the WebSocket frame before it writes the response, so
    // an event can reach the page while the fetch is still pending. Anything
    // that registered afterwards would lose that race every time.
    let seenDuringFlight: boolean | null = null
    fetchMock.mockImplementation((url: string) => {
      if (String(url) === '/.mbr/task') {
        seenDuringFlight = wasSelfWrite('notes.md')
        return Promise.resolve({
          ok: true,
          status: 200,
          json: () => Promise.resolve({ line: 3, text: '- [x] write the report !!' }),
        })
      }
      return Promise.resolve({ ok: true, status: 200, text: () => Promise.resolve(SOURCE) })
    })

    await toggleTask({ path: 'notes.md', line: 3, to: 'done' })

    expect(seenDuringFlight).toBe(true)
  })

  it('stops suppressing once the window has passed', async () => {
    await toggleTask({ path: 'notes.md', line: 3, to: 'done' })
    expect(wasSelfWrite('notes.md')).toBe(true)

    // Somebody else's edit, seconds later, is a real change and must reload.
    vi.useFakeTimers()
    vi.setSystemTime(Date.now() + 5000)
    expect(wasSelfWrite('notes.md')).toBe(false)
  })

  it('never suppresses a change to a file this page did not write', async () => {
    await toggleTask({ path: 'notes.md', line: 3, to: 'done' })

    expect(wasSelfWrite('other.md')).toBe(false)
  })

  it('leaves a failed write’s window standing rather than risk withdrawing another', async () => {
    // A failed write triggers no broadcast, so the window suppresses nothing of
    // ours; withdrawing it would have to identify which registration to drop,
    // and dropping a successful sibling's is the expensive mistake.
    routeFetch({ ok: false, status: 409 })
    await toggleTask({ path: 'notes.md', line: 3, to: 'done' })

    expect(wasSelfWrite('notes.md')).toBe(true)
  })
})

describe('checkbox status helpers', () => {
  function checkbox(status: string): HTMLInputElement {
    const wrapper = document.createElement('li')
    wrapper.innerHTML =
      `<input type="checkbox" class="mbr-task-check" id="mbr-task-3" ` +
      `data-mbr-task-line="3" data-mbr-task-status="${status}">` +
      `<span class="mbr-task-text">write the report</span>`
    document.body.appendChild(wrapper)
    return wrapper.querySelector('input')!
  }

  afterEach(() => {
    document.body.innerHTML = ''
    window.frontmatter = undefined
  })

  it('reads the rendered status, defaulting to open', () => {
    expect(checkboxStatus(checkbox('done'))).toBe('done')
    expect(checkboxStatus(checkbox('canceled'))).toBe('canceled')
    expect(checkboxStatus(checkbox('nonsense'))).toBe('open')
  })

  it('cycles the two independent pairs', () => {
    expect(nextToggleStatus('open')).toBe('done')
    expect(nextToggleStatus('done')).toBe('open')
    // A canceled task completed rather than reopened: one key per pair.
    expect(nextToggleStatus('canceled')).toBe('done')
    expect(nextCancelStatus('open')).toBe('canceled')
    expect(nextCancelStatus('canceled')).toBe('open')
    expect(nextCancelStatus('done')).toBe('canceled')
  })

  it('moves the box, the attribute and the strikethrough together', () => {
    const input = checkbox('open')
    const text = input.parentElement!.querySelector('.mbr-task-text')!

    applyCheckboxStatus(input, 'done')
    expect(input.checked).toBe(true)
    expect(input.dataset.mbrTaskStatus).toBe('done')
    expect(text.classList.contains('mbr-task-canceled')).toBe(false)

    applyCheckboxStatus(input, 'canceled')
    expect(input.checked).toBe(false)
    expect(text.classList.contains('mbr-task-canceled')).toBe(true)

    applyCheckboxStatus(input, 'open')
    expect(text.classList.contains('mbr-task-canceled')).toBe(false)
  })

  it('syncs the document only for the file on screen', () => {
    const input = checkbox('open')
    window.frontmatter = { markdown_source: 'notes.md' }
    expect(currentDocumentPath()).toBe('notes.md')

    syncDocumentTask('other.md', 3, 'done')
    expect(input.checked).toBe(false)

    syncDocumentTask('notes.md', 3, 'done')
    expect(input.checked).toBe(true)

    // A line with no checkbox on this page is simply not there.
    expect(() => syncDocumentTask('notes.md', 99, 'done')).not.toThrow()
  })

  it('draws the panel’s write into the page behind it, chip and all', () => {
    const input = checkbox('open')
    window.frontmatter = { markdown_source: 'notes.md' }

    syncDocumentTask('notes.md', 3, 'done', '- [x] write the report @done(2026-08-04 22:16)')

    expect(input.checked).toBe(true)
    const chip = input.parentElement!.querySelector('.mbr-task-completed')!
    expect(chip.getAttribute('datetime')).toBe('2026-08-04T22:16')
    expect(chip.textContent).toBe('Aug 4, 10:16 PM')

    // ...and takes it away again on a reopen.
    syncDocumentTask('notes.md', 3, 'open', '- [ ] write the report')
    expect(input.parentElement!.querySelector('.mbr-task-completed')).toBeNull()
  })
})
