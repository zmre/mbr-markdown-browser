/**
 * Writing a single task's status: the one implementation of `POST /.mbr/task`.
 *
 * Both callers go through here — the in-document checkboxes (`mbr-task-doc.ts`)
 * and the task panel, which receives {@link toggleTask} as an injected property
 * because it lives in a lazy chunk and this module is stateful.
 *
 * # Sourcing `expected`
 *
 * The endpoint takes the line's *exact current text* rather than a whole-file
 * hash, so a click has to know what is on disk. It reads it from
 * `/.mbr/raw/<path>` and caches the file's lines for the page's lifetime,
 * because the alternative — trusting the rendered HTML — cannot work: the
 * renderer strips `@due(...)`, `#tag` and `!!` out of the display text on
 * purpose, so nothing on the page reproduces the source line.
 *
 * The cache is kept honest by the response: a successful write returns the new
 * line, which is written straight back into the cache so a second click needs
 * no second fetch. A `409` drops the file entirely, since the copy we hold is
 * provably wrong.
 *
 * # Live reload
 *
 * A successful write makes the server broadcast a file change, and
 * `<mbr-live-reload>` would reload the page for it. **Every** task write
 * suppresses that reload and patches the document instead — see
 * {@link wasSelfWrite} — for two reasons:
 *
 * - The edit token lives in memory for the life of the page (`edit-token.ts`),
 *   so a reload after each write would 401 the next click on a token-protected
 *   server. Not reloading is what makes an in-memory token workable.
 * - A reload would tear down the task panel's overlay, losing the user's
 *   filters, for a view the panel has already refreshed by re-querying.
 *
 * What the reload used to buy — a freshly stamped `@done(...)` chip — is drawn
 * from the response's `text` instead (`task-chips.ts`), which is the only part
 * of a task's markup a status change alters beyond the box itself.
 */
import { editAuthHeaders, noteEditTokenRequired } from './edit-token.js'
import { syncDoneChip } from './task-chips.js'
import type { TaskStatus, TaskToggleOutcome, TaskToggleTarget } from './tasks/types.js'

/** Endpoint for a single-line task patch (`server.rs::task_toggle_handler`). */
const TASK_ENDPOINT = '/.mbr/task'

/** Prefix for raw markdown reads (`server.rs::raw_markdown_handler`). */
const RAW_PREFIX = '/.mbr/raw/'

/**
 * How long a write keeps reloads for that file suppressed.
 *
 * One write produces **several** reload-worthy events, not one: the handler
 * broadcasts explicitly before it responds, and then the watcher sees the
 * atomic rename and broadcasts again — twice, on macOS, measured about 7ms
 * later. So the suppression is a window rather than a single token to be
 * consumed; consuming one entry would swallow the handler's event and let the
 * watcher's echo reload the page anyway, which is the failure this window
 * exists to prevent.
 *
 * The window is short because the only thing it costs is a genuine external
 * edit landing on the same file within it, which then waits for the next
 * navigation to show up. A second and a half is two orders of magnitude more
 * than the observed echo and still short enough to be invisible.
 */
const SELF_WRITE_TTL_MS = 1500

export type { TaskToggleOutcome, TaskToggleTarget } from './tasks/types.js'

/** Cached source lines per repo-relative path, without line terminators. */
const lineCache = new Map<string, string[]>()

/** The outcome of reading a file's source, with the status when it failed. */
type SourceRead = { ok: true; lines: string[] } | { ok: false; status: number }

/** In-flight raw reads, so two quick clicks on one file fetch it once. */
const pendingReads = new Map<string, Promise<SourceRead>>()

/** Repo-relative paths this page wrote recently, keyed to their timestamps. */
const selfWrites = new Map<string, number>()

/** Normalizes a repo-relative path for comparison against a watcher event. */
function normalizePath(path: string): string {
  return path.replace(/\\/g, '/').replace(/^\/+/, '')
}

/**
 * True when `relativePath` names a file this page wrote within the last
 * {@link SELF_WRITE_TTL_MS}.
 *
 * Deliberately does **not** consume the entry: a single write is echoed by the
 * watcher as well as announced by the handler, and every one of those events
 * describes the change this page already made and already drew.
 */
export function wasSelfWrite(relativePath: string): boolean {
  const key = normalizePath(relativePath)
  const at = selfWrites.get(key)
  if (at === undefined) return false
  if (Date.now() - at < SELF_WRITE_TTL_MS) return true
  selfWrites.delete(key)
  return false
}

/**
 * Register a write so its own events do not reload the page.
 *
 * Called *before* the request goes out, not after it returns: the handler
 * broadcasts the change before it writes the response, so the WebSocket frame
 * can reach this page while the `fetch` promise is still pending. Registering
 * afterwards would lose that race for the first — and most likely — event.
 *
 * A write that then fails leaves the window standing rather than clearing it.
 * A failed write triggers no broadcast, so the window suppresses nothing of
 * ours; withdrawing it would have to identify *which* registration to withdraw,
 * and getting that wrong drops a successful sibling write's suppression, which
 * is the expensive mistake of the two.
 */
function noteSelfWrite(path: string): void {
  const now = Date.now()
  for (const [key, at] of selfWrites) {
    if (now - at >= SELF_WRITE_TTL_MS) selfWrites.delete(key)
  }
  selfWrites.set(path, now)
}

/** Test hook: forget every cached source line and pending suppression. */
export function resetTaskToggleState(): void {
  lineCache.clear()
  pendingReads.clear()
  selfWrites.clear()
}

/** `/.mbr/raw/...` URL for a repo-relative path, each segment encoded. */
function rawUrl(path: string): string {
  return RAW_PREFIX + normalizePath(path).split('/').map(encodeURIComponent).join('/')
}

/**
 * Read and split `path`'s source.
 *
 * Split on `\n` only, and never on a lone `\r`, because that is how the server
 * numbers lines (`tasks::line_span` uses `split_inclusive('\n')`). Splitting on
 * `\r` as well would silently shift every line number in an old Mac-format
 * file. A trailing `\r` is trimmed instead, matching the endpoint's
 * terminator-insensitive comparison.
 */
async function fetchSourceLines(path: string): Promise<SourceRead> {
  const response = await fetch(rawUrl(path), {
    headers: editAuthHeaders(),
    credentials: 'same-origin',
  })
  if (!response.ok) return { ok: false, status: response.status }
  const lines = (await response.text()).split('\n').map((line) => line.replace(/\r$/, ''))
  return { ok: true, lines }
}

/** The source lines of `path`, fetched at most once per page and per read. */
async function sourceLines(path: string): Promise<SourceRead> {
  const cached = lineCache.get(path)
  if (cached) return { ok: true, lines: cached }

  // Single-flight: two boxes in one file clicked in quick succession would
  // otherwise each fetch it, and the slower read — holding bytes from before
  // the faster one's write — would land last and overwrite the fresher copy.
  let pending = pendingReads.get(path)
  if (!pending) {
    pending = fetchSourceLines(path)
    pendingReads.set(path, pending)
    void pending.catch(() => {}).finally(() => pendingReads.delete(path))
  }
  const fetched = await pending

  // Same hazard across a longer gap: a write may have cached a newer copy
  // while this read was in flight. What it holds always wins.
  const current = lineCache.get(path)
  if (current) return { ok: true, lines: current }
  if (fetched.ok) lineCache.set(path, fetched.lines)
  return fetched
}

/**
 * What to tell the user when a token is missing.
 *
 * Points at the editor because that is where the field is. `edit-token.ts`
 * remembers the refusal so the field is actually visible when they get there.
 */
const TOKEN_MESSAGE = 'Editing needs a token — open the editor (e) and enter it first.'

/** Human-readable reason for a failed write, by status. */
function describeFailure(status: number): { kind: 'conflict' | 'auth' | 'other'; message: string } {
  switch (status) {
    case 409:
      return {
        kind: 'conflict',
        message: 'That line changed on disk, so nothing was written.',
      }
    case 401:
      return { kind: 'auth', message: TOKEN_MESSAGE }
    case 403:
      return { kind: 'auth', message: 'Editing is not enabled on this server.' }
    case 400:
      // The endpoint returns 400 for several line-level refusals (not a task,
      // no such line, unreadable file); the common one by far is the first.
      return { kind: 'other', message: 'That line could not be updated as a task.' }
    default:
      return { kind: 'other', message: `The task could not be saved (${status}).` }
  }
}

/**
 * Human-readable reason for a failed *read* of the source.
 *
 * Separate from {@link describeFailure} because the same statuses mean
 * different things here: `/.mbr/raw` is behind the identical `check_edit_access`
 * policy, so a token-protected server refuses the read before the write is ever
 * attempted, and reporting that as a generic "could not read the file" hides
 * the one thing the user can act on.
 */
function describeReadFailure(status: number): {
  kind: 'conflict' | 'auth' | 'other'
  message: string
} {
  switch (status) {
    case 401:
      return { kind: 'auth', message: TOKEN_MESSAGE }
    case 403:
      return { kind: 'auth', message: 'Editing is not enabled on this server.' }
    default:
      return { kind: 'other', message: 'Could not read the file to update it.' }
  }
}

/**
 * Set one task's status.
 *
 * Resolves rather than rejects on failure: every caller has a revert to run and
 * a message to show, and a rejected promise would make both easy to forget.
 */
export async function toggleTask(target: TaskToggleTarget): Promise<TaskToggleOutcome> {
  const path = normalizePath(target.path)
  let read: SourceRead
  try {
    read = await sourceLines(path)
  } catch (err) {
    console.warn('Failed to read the task source:', err)
    read = { ok: false, status: 0 }
  }
  if (!read.ok) {
    lineCache.delete(path)
    if (read.status === 401) noteEditTokenRequired()
    return { ok: false, ...describeReadFailure(read.status) }
  }

  const expected = read.lines[target.line - 1]
  if (expected === undefined) {
    // No such line means no `expected`, and sending a guess would either be
    // rejected as a conflict or — worse — match a line we never saw.
    lineCache.delete(path)
    return { ok: false, kind: 'other', message: 'Could not read the file to update it.' }
  }

  // Before the request, not after it: see `noteSelfWrite`.
  noteSelfWrite(path)

  let response: Response
  try {
    response = await fetch(TASK_ENDPOINT, {
      method: 'POST',
      headers: editAuthHeaders({ 'Content-Type': 'application/json' }),
      credentials: 'same-origin',
      body: JSON.stringify({ path, line: target.line, expected, to: target.to }),
    })
  } catch (err) {
    console.warn('Task toggle request failed:', err)
    return { ok: false, kind: 'other', message: 'The server could not be reached.' }
  }

  if (!response.ok) {
    const failure = describeFailure(response.status)
    // Our copy of the line is what the server just disagreed with; drop it so
    // the retry re-reads rather than repeating the same rejected request.
    if (failure.kind === 'conflict') lineCache.delete(path)
    if (response.status === 401) noteEditTokenRequired()
    return { ok: false, ...failure }
  }

  // Keep the cache current from the authoritative new text, so a second click
  // on the same box needs no round trip to `/.mbr/raw`. The same text is handed
  // back to the caller, which is the only place a new `@done(...)` can come
  // from now that the write no longer reloads the page.
  try {
    const body = (await response.json()) as { line: number; text: string }
    if (typeof body.text !== 'string') throw new Error('no text in the response')
    read.lines[target.line - 1] = body.text
    return { ok: true, text: body.text }
  } catch {
    // A response we cannot parse says nothing about the write, which the
    // status code already confirmed. Re-read the file next time instead.
    lineCache.delete(path)
    return { ok: true }
  }
}

// ============================================================================
// In-document checkbox state
// ============================================================================

/** The next status for a left click / `Space`: complete ↔ reopen. */
export function nextToggleStatus(current: TaskStatus): TaskStatus {
  return current === 'done' ? 'open' : 'done'
}

/** The next status for a right click / `x`: cancel ↔ reopen. */
export function nextCancelStatus(current: TaskStatus): TaskStatus {
  return current === 'canceled' ? 'open' : 'canceled'
}

/** The status a rendered checkbox currently claims. */
export function checkboxStatus(input: HTMLInputElement): TaskStatus {
  const status = input.dataset.mbrTaskStatus
  return status === 'done' || status === 'canceled' ? status : 'open'
}

/**
 * Write a status onto a rendered checkbox and its text.
 *
 * Only the parts of the markup a status decides on its own: the box, the data
 * attribute the next click reads back, and the strikethrough. The `@done(...)`
 * chip needs the server's answer and is applied separately by
 * {@link syncDoneChip} once the write returns.
 */
export function applyCheckboxStatus(input: HTMLInputElement, status: TaskStatus): void {
  input.dataset.mbrTaskStatus = status
  input.checked = status === 'done'
  const text = input.parentElement?.querySelector<HTMLElement>('.mbr-task-text')
  text?.classList.toggle('mbr-task-canceled', status === 'canceled')
}

/** Repo-relative source path of the page being viewed, if it is a markdown page. */
export function currentDocumentPath(): string | null {
  const source = window.frontmatter?.['markdown_source']
  return typeof source === 'string' && source.length > 0 ? normalizePath(source) : null
}

/**
 * Reflect a write into the page behind an overlay.
 *
 * The task panel writes to files it is not showing, so when it happens to write
 * to the one on screen it has to keep that copy honest itself — nothing reloads
 * the page any more. `sourceLine` is the server's new text for the line, which
 * is what redraws the `@done(...)` chip; without it only the box moves.
 *
 * A no-op for every other file, and for pages with no such line.
 */
export function syncDocumentTask(
  path: string,
  line: number,
  status: TaskStatus,
  sourceLine?: string
): void {
  if (normalizePath(path) !== currentDocumentPath()) return
  const input = document.getElementById(`mbr-task-${line}`)
  if (!(input instanceof HTMLInputElement)) return
  applyCheckboxStatus(input, status)
  if (sourceLine !== undefined) syncDoneChip(input, sourceLine)
}
