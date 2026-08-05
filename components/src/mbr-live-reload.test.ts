import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import './mbr-live-reload.ts'
import type { MbrLiveReloadElement } from './mbr-live-reload.ts'
import { resetTaskToggleState, toggleTask } from './task-toggle.ts'

/**
 * Tests for <mbr-live-reload>, the server-mode element that watches
 * /.mbr/ws/changes and reloads the page when the file behind it changes.
 *
 * Everything here is driven through a fake WebSocket global: the element is
 * mounted for real, and the test plays the part of the server (open, message,
 * close). That covers the four things worth guaranteeing — the URL it dials,
 * what it does per message type, how it backs off, and what it leaves behind
 * when it is torn down.
 *
 * Several tests are marked DEFECT: they pin down CURRENT, WRONG behaviour so
 * the bug is visible and a fix is a deliberate, test-updating act. Each says
 * what the right behaviour would be.
 */

// ============================================================================
// Fake WebSocket
// ============================================================================

type Handler<E> = ((event: E) => void) | null

/** Every socket the component has constructed, oldest first. */
const sockets: FakeWebSocket[] = []

/**
 * Minimal stand-in for the WebSocket the component constructs.
 *
 * `close()` delivers the close event on a later tick rather than inline,
 * because that is what browsers do: close() starts the closing handshake and
 * the `close` event follows. The component's reconnect logic hangs off that
 * event, so collapsing it to a synchronous call would hide the teardown bugs
 * below rather than reproduce them.
 */
class FakeWebSocket {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3

  readyState: number = FakeWebSocket.CONNECTING
  /** How many times the component asked to close this socket. */
  closeCalls = 0

  onopen: Handler<Event> = null
  onmessage: Handler<{ data: string }> = null
  onerror: Handler<Event> = null
  onclose: Handler<Event> = null

  constructor(readonly url: string) {
    sockets.push(this)
  }

  close(): void {
    this.closeCalls++
    if (this.readyState === FakeWebSocket.CLOSED || this.readyState === FakeWebSocket.CLOSING) return
    this.readyState = FakeWebSocket.CLOSING
    setTimeout(() => this.emitClose(), 0)
  }

  // -- server-side driving ---------------------------------------------------

  emitOpen(): void {
    this.readyState = FakeWebSocket.OPEN
    this.onopen?.(new Event('open'))
  }

  /** Send a frame. Objects are JSON-encoded; strings are sent verbatim. */
  emitMessage(payload: unknown): void {
    this.onmessage?.({ data: typeof payload === 'string' ? payload : JSON.stringify(payload) })
  }

  emitError(): void {
    this.onerror?.(new Event('error'))
  }

  emitClose(): void {
    this.readyState = FakeWebSocket.CLOSED
    this.onclose?.(new Event('close'))
  }
}

/** The delays the component schedules: 1000ms * 1.5^(attempt - 1), five tries. */
const BACKOFF_MS = [1000, 1500, 2250, 3375, 5062.5]

/** How long the element waits after showing the notification before reloading. */
const RELOAD_DELAY_MS = 500

// ============================================================================
// Harness
// ============================================================================

let reload: ReturnType<typeof vi.fn>
let element: MbrLiveReloadElement | null = null

interface LocationOverrides {
  protocol?: string
  host?: string
  pathname?: string
}

function setLocation(overrides: LocationOverrides = {}): void {
  vi.stubGlobal('location', {
    protocol: 'http:',
    host: 'localhost:5200',
    pathname: '/docs/guide/',
    reload,
    ...overrides,
  })
}

function latestSocket(): FakeWebSocket {
  expect(sockets.length, 'expected the component to have opened a WebSocket').toBeGreaterThan(0)
  return sockets[sockets.length - 1]
}

async function mount(): Promise<MbrLiveReloadElement> {
  const el = document.createElement('mbr-live-reload')
  document.body.appendChild(el)
  await el.updateComplete
  element = el
  return el
}

/** Deliver a file-change frame exactly as watcher.rs broadcasts it. */
async function fileChanged(
  relativePath: string,
  kind: 'modified' | 'created' | 'deleted' = 'modified',
): Promise<void> {
  latestSocket().emitMessage({
    path: `/Users/someone/notes/${relativePath}`,
    relative_path: relativePath,
    event: kind,
  })
  await element?.updateComplete
}

/** Assert the page reloads, and only after the notification delay. */
function expectReload(): void {
  expect(reload, 'reload must not be synchronous — the notification needs a frame').not.toHaveBeenCalled()
  vi.advanceTimersByTime(RELOAD_DELAY_MS - 1)
  expect(reload).not.toHaveBeenCalled()
  vi.advanceTimersByTime(1)
  expect(reload).toHaveBeenCalledTimes(1)
}

/** Assert the page never reloads, however long we wait. */
function expectNoReload(): void {
  vi.advanceTimersByTime(60_000)
  expect(reload).not.toHaveBeenCalled()
}

beforeEach(() => {
  vi.useFakeTimers()
  sockets.length = 0
  reload = vi.fn()
  window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
  window.frontmatter = { markdown_source: 'docs/guide.md' }
  setLocation()
  vi.stubGlobal('WebSocket', FakeWebSocket)
  // The component narrates every step to the console; keep the suite readable.
  vi.spyOn(console, 'log').mockImplementation(() => {})
  vi.spyOn(console, 'error').mockImplementation(() => {})
})

afterEach(() => {
  element?.remove()
  element = null
  document.body.innerHTML = ''
  vi.useRealTimers()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
  window.__MBR_CONFIG__ = undefined
  window.frontmatter = undefined
})

// ============================================================================
// Connecting
// ============================================================================

describe('MbrLiveReloadElement connection', () => {
  it('dials the live-reload endpoint over ws:// on an http page', async () => {
    await mount()

    expect(sockets).toHaveLength(1)
    expect(sockets[0].url).toBe('ws://localhost:5200/.mbr/ws/changes')
  })

  it('upgrades to wss:// on an https page', async () => {
    setLocation({ protocol: 'https:', host: 'notes.example.com' })

    await mount()

    expect(latestSocket().url).toBe('wss://notes.example.com/.mbr/ws/changes')
  })

  it('keeps the page host and port, which is what the server is bound to', async () => {
    // mbr picks a non-default port routinely (--port), so a hardcoded :5200
    // would connect to the wrong server or nothing at all.
    setLocation({ host: '127.0.0.1:5873' })

    await mount()

    expect(latestSocket().url).toBe('ws://127.0.0.1:5873/.mbr/ws/changes')
  })

  it('opens no socket in a static build', async () => {
    // A generated site has no server to talk to; a socket here would retry
    // against whatever host the site is deployed on.
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false }

    await mount()

    expect(sockets).toHaveLength(0)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('opens no socket when the page carries no config', async () => {
    window.__MBR_CONFIG__ = undefined

    await mount()

    expect(sockets).toHaveLength(0)
  })

  it('tears down cleanly when it never connected', async () => {
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false }
    const el = await mount()

    expect(() => el.remove()).not.toThrow()
  })
})

// ============================================================================
// Message handling
// ============================================================================

describe('MbrLiveReloadElement message handling', () => {
  it('treats the connection-confirmation frame as a no-op', async () => {
    const el = await mount()
    const ws = latestSocket()
    ws.emitOpen()

    ws.emitMessage({ status: 'connected' })
    await el.updateComplete

    expect(ws.closeCalls).toBe(0)
    expectNoReload()
  })

  it('survives a frame that is not JSON', async () => {
    const el = await mount()
    const ws = latestSocket()
    ws.emitOpen()

    expect(() => ws.emitMessage('<html>502 Bad Gateway</html>')).not.toThrow()
    await el.updateComplete

    expect(ws.closeCalls).toBe(0)
    expectNoReload()
  })

  it('ignores a frame with no relative_path', async () => {
    const el = await mount()
    latestSocket().emitOpen()

    latestSocket().emitMessage({ event: 'modified' })
    await el.updateComplete

    expectNoReload()
  })

  it('closes the socket when the server reports an error', async () => {
    await mount()
    const ws = latestSocket()
    ws.emitOpen()

    ws.emitMessage({ error: 'file watcher failed' })

    expect(ws.closeCalls).toBe(1)
  })

  it('reloads when the markdown file behind this page changes', async () => {
    await mount()
    latestSocket().emitOpen()

    await fileChanged('docs/guide.md')

    expectReload()
  })

  it('skips a reload this page asked to be skipped, once', async () => {
    // The task panel writes with `suppressReload`, because a reload would tear
    // down the open overlay for a view it has already refreshed itself.
    resetTaskToggleState()
    globalThis.fetch = vi.fn().mockImplementation((url: string) =>
      String(url).startsWith('/.mbr/raw/')
        ? Promise.resolve({ ok: true, status: 200, text: () => Promise.resolve('- [ ] a\n') })
        : Promise.resolve({
            ok: true,
            status: 200,
            json: () => Promise.resolve({ line: 1, text: '- [x] a' }),
          }),
    ) as unknown as typeof fetch
    await toggleTask({ path: 'docs/guide.md', line: 1, to: 'done' }, { suppressReload: true })

    await mount()
    latestSocket().emitOpen()

    await fileChanged('docs/guide.md')
    expectNoReload()

    // Only the one event is swallowed: the watcher's own later event for the
    // same file is indistinguishable from somebody else's edit.
    await fileChanged('docs/guide.md')
    expectReload()
    resetTaskToggleState()
  })

  it('shows a notification before navigating away', async () => {
    const el = await mount()
    latestSocket().emitOpen()
    expect(el.shadowRoot?.querySelector('.notification')).toBeNull()

    await fileChanged('docs/guide.md')

    expect(el.shadowRoot?.querySelector('.notification')?.textContent).toContain('Reloading page')
  })

  // One `it` per case so each gets a fresh element and a fresh timer queue.
  for (const kind of ['created', 'deleted'] as const) {
    it(`reloads on a ${kind} event, not just a modification`, async () => {
      await mount()
      latestSocket().emitOpen()

      await fileChanged('docs/guide.md', kind)

      expectReload()
    })
  }

  it('ignores a markdown file this page does not render', async () => {
    await mount()
    latestSocket().emitOpen()

    await fileChanged('docs/other-note.md')

    expectNoReload()
  })

  // Templates, styles and scripts are global to the render, so a change to any
  // of them invalidates every page regardless of where it lives.
  for (const path of ['.mbr/index.html', '.mbr/theme.css', '.mbr/components/mbr-components.min.js']) {
    it(`reloads when ${path} changes`, async () => {
      await mount()
      latestSocket().emitOpen()

      await fileChanged(path)

      expectReload()
    })
  }

  it('reloads when the sibling index.md of the current note changes', async () => {
    // The folder landing page contributes to this page's navigation.
    await mount()
    latestSocket().emitOpen()

    await fileChanged('docs/index.md')

    expectReload()
  })

  it('ignores an index.md in an unrelated folder', async () => {
    await mount()
    latestSocket().emitOpen()

    await fileChanged('recipes/index.md')

    expectNoReload()
  })

  it('matches the current file regardless of a leading slash', async () => {
    // markdown_source is relative (server.rs) but the URL fallback produces a
    // leading slash, so both spellings have to compare equal.
    window.frontmatter = { markdown_source: '/docs/guide.md' }
    await mount()
    latestSocket().emitOpen()

    await fileChanged('docs/guide.md')

    expectReload()
  })

  it('falls back to the URL path when the page has no markdown_source', async () => {
    window.frontmatter = undefined
    setLocation({ pathname: '/docs/guide/' })
    await mount()
    latestSocket().emitOpen()

    await fileChanged('docs/guide.md')

    expectReload()
  })

  it('maps the site root to index.md', async () => {
    window.frontmatter = undefined
    setLocation({ pathname: '/' })
    await mount()
    latestSocket().emitOpen()

    await fileChanged('index.md')

    expectReload()
  })

  /**
   * DEFECT (characterised, not fixed): on a directory listing the page has no
   * `markdown_source`, so `_detectCurrentMarkdownFile()` guesses by appending
   * ".md" to the URL — `/docs/` becomes `docs.md`, a file that does not exist.
   * Nothing under `docs/` can ever match it, so section pages never live-reload
   * when their contents change.
   *
   * `_shouldReloadForFile` has a branch for exactly this case (`!current ||
   * current.endsWith('/')`), but the guess above guarantees it is unreachable.
   *
   * Fix: return null from `_detectCurrentMarkdownFile()` for a directory URL
   * (or keep the trailing slash) so the directory-listing branch can fire.
   */
  it('DEFECT: a directory listing does not reload when a file inside it changes', async () => {
    window.frontmatter = undefined
    setLocation({ pathname: '/docs/' })
    await mount()
    latestSocket().emitOpen()

    await fileChanged('docs/index.md')
    expectNoReload()

    await fileChanged('docs/guide.md')
    expectNoReload()
  })
})

// ============================================================================
// Reconnecting
// ============================================================================

describe('MbrLiveReloadElement reconnection', () => {
  it('reconnects to the same endpoint after the server drops the connection', async () => {
    await mount()
    const first = latestSocket()
    first.emitOpen()

    first.emitClose()
    expect(sockets).toHaveLength(1)
    vi.advanceTimersByTime(999)
    expect(sockets, 'must not reconnect before the backoff elapses').toHaveLength(1)
    vi.advanceTimersByTime(1)

    expect(sockets).toHaveLength(2)
    expect(sockets[1].url).toBe(first.url)
  })

  it('backs off by 1.5x on every consecutive failure', async () => {
    await mount()

    BACKOFF_MS.forEach((delay, i) => {
      latestSocket().emitClose()
      vi.advanceTimersByTime(delay - 1)
      expect(sockets, `attempt ${i + 1} reconnected before ${delay}ms`).toHaveLength(i + 1)
      vi.advanceTimersByTime(1)
      expect(sockets, `attempt ${i + 1} did not reconnect at ${delay}ms`).toHaveLength(i + 2)
    })
  })

  it('gives up after five attempts instead of retrying forever', async () => {
    await mount()
    for (const delay of BACKOFF_MS) {
      latestSocket().emitClose()
      vi.advanceTimersByTime(delay)
    }
    expect(sockets).toHaveLength(1 + BACKOFF_MS.length)

    latestSocket().emitClose()
    vi.advanceTimersByTime(600_000)

    expect(sockets, 'a sixth attempt was scheduled').toHaveLength(1 + BACKOFF_MS.length)
    expect(vi.getTimerCount(), 'a timer outlived the give-up path').toBe(0)
  })

  it('restores the full retry budget once a connection succeeds', async () => {
    await mount()
    for (const delay of BACKOFF_MS.slice(0, 3)) {
      latestSocket().emitClose()
      vi.advanceTimersByTime(delay)
    }
    expect(sockets).toHaveLength(4)

    // A server restart: this attempt lands, so the next drop starts over at 1s
    // rather than continuing to grow.
    latestSocket().emitOpen()
    latestSocket().emitClose()
    vi.advanceTimersByTime(999)
    expect(sockets).toHaveLength(4)
    vi.advanceTimersByTime(1)

    expect(sockets).toHaveLength(5)
  })

  it('does not reconnect merely because a socket errored', async () => {
    // onerror is informational; the close that follows drives the retry, and
    // reacting to both would halve the backoff.
    await mount()
    const ws = latestSocket()
    ws.emitOpen()

    ws.emitError()
    vi.advanceTimersByTime(60_000)

    expect(sockets).toHaveLength(1)
  })

  /**
   * DEFECT (characterised, not fixed): the error frame calls `_disconnect()`,
   * whose `close()` fires `onclose`, which schedules a reconnect — and because
   * the new socket opens successfully, `onopen` resets `_reconnectAttempts` to
   * 0. The five-attempt ceiling therefore never applies: a server that keeps
   * reporting an error is polled once a second forever.
   *
   * Fix: set a "stopped" flag in `_disconnect()` and check it in
   * `_attemptReconnect()`, so an explicit disconnect stays disconnected.
   */
  it('DEFECT: reconnects forever after a server error instead of standing down', async () => {
    await mount()

    for (let round = 0; round < 4; round++) {
      const ws = latestSocket()
      ws.emitOpen()
      ws.emitMessage({ error: 'file watcher failed' })
      vi.advanceTimersByTime(0) // the close event that close() queued
      vi.advanceTimersByTime(1000) // ... and the reconnect it triggered
      expect(sockets, `round ${round + 1} did not reconnect`).toHaveLength(round + 2)
    }
  })
})

// ============================================================================
// Teardown
// ============================================================================

describe('MbrLiveReloadElement teardown', () => {
  it('closes its socket when the element is removed', async () => {
    const el = await mount()
    const ws = latestSocket()
    ws.emitOpen()

    el.remove()

    expect(ws.closeCalls).toBe(1)
  })

  /**
   * DEFECT (characterised, not fixed): `_disconnect()` closes the socket, the
   * browser answers with a `close` event, and `onclose` unconditionally calls
   * `_attemptReconnect()`. Removing the element therefore opens a NEW socket a
   * second later — for an element that is no longer in the document. Once that
   * socket opens, `onopen` resets the attempt counter, so the connection (and
   * the element it retains) lives on indefinitely.
   *
   * Fix: same "stopped" flag as above; `disconnectedCallback` must make the
   * teardown final.
   */
  it('DEFECT: opens a fresh socket after the element has been removed', async () => {
    const el = await mount()
    latestSocket().emitOpen()

    el.remove()
    vi.advanceTimersByTime(0) // close event
    vi.advanceTimersByTime(1000) // reconnect backoff

    expect(sockets).toHaveLength(2)
    expect(sockets[1].readyState).toBe(FakeWebSocket.CONNECTING)
  })

  /**
   * DEFECT (characterised, not fixed): `_attemptReconnect()` throws away the
   * `setTimeout` handle, so `_disconnect()` has nothing to cancel. A reconnect
   * scheduled before removal still fires afterwards and opens a socket.
   *
   * Fix: keep the handle in a field and `clearTimeout` it in `_disconnect()`.
   */
  it('DEFECT: a reconnect scheduled before removal still fires afterwards', async () => {
    const el = await mount()
    latestSocket().emitClose() // server dropped us; a retry is now pending

    el.remove()

    expect(vi.getTimerCount(), 'the pending reconnect was cancelled after all').toBeGreaterThan(0)
    vi.advanceTimersByTime(1000)
    expect(sockets).toHaveLength(2)
  })

  /**
   * DEFECT (characterised, not fixed): `connectedCallback` registers an inline
   * `beforeunload` closure and nothing ever removes it. The closure captures
   * the element, so every mount leaks a listener and pins the element for the
   * lifetime of the page.
   *
   * Fix: store the bound handler and remove it in `disconnectedCallback`.
   */
  it('DEFECT: never removes the beforeunload listener it registers', async () => {
    const add = vi.spyOn(window, 'addEventListener')
    const remove = vi.spyOn(window, 'removeEventListener')

    const el = await mount()
    el.remove()

    expect(add.mock.calls.filter((c) => c[0] === 'beforeunload')).toHaveLength(1)
    expect(remove.mock.calls.filter((c) => c[0] === 'beforeunload')).toHaveLength(0)
  })

  /**
   * DEFECT (characterised, not fixed): the user-visible consequence of the two
   * defects above. A detached element rebuilds its socket and keeps honouring
   * file changes, so it can still navigate the document it no longer belongs
   * to. Whatever else leaks, a removed element must not reload the page.
   */
  it('DEFECT: a removed element still reloads the page when a file changes', async () => {
    const el = await mount()
    latestSocket().emitOpen()

    el.remove()
    vi.advanceTimersByTime(0) // close event
    vi.advanceTimersByTime(1000) // reconnect backoff

    const revived = latestSocket()
    revived.emitOpen()
    revived.emitMessage({ relative_path: 'docs/guide.md', event: 'modified' })
    vi.advanceTimersByTime(RELOAD_DELAY_MS)

    expect(reload).toHaveBeenCalledTimes(1)
  })
})
