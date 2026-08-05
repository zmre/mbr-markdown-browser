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
 * `<mbr-live-reload>` reloads the page for it. That is what we want for an
 * in-document click — the reload is the authoritative re-render, and it is the
 * only thing that can show a freshly stamped `@done(...)` chip. It is *not*
 * what we want for a panel toggle: the reload would tear down the open panel
 * and lose the user's filters, for a view the panel has already refreshed by
 * re-querying. So a panel write registers itself here and
 * {@link wasSelfWrite} tells the live-reload element to skip that one event.
 */
import { editAuthHeaders } from './edit-token.js'
import type { TaskStatus, TaskToggleOutcome, TaskToggleTarget } from './tasks/types.js'

/** Endpoint for a single-line task patch (`server.rs::task_toggle_handler`). */
const TASK_ENDPOINT = '/.mbr/task'

/** Prefix for raw markdown reads (`server.rs::raw_markdown_handler`). */
const RAW_PREFIX = '/.mbr/raw/'

/**
 * How long a self-write stays suppressed.
 *
 * The broadcast is sent before the response is written, so the WebSocket event
 * has normally already arrived by the time the caller resolves. The window only
 * has to cover a slow event loop, not a debounce — the watcher's own (much
 * later) duplicate event is a genuine external change as far as we can tell,
 * and reloading for it is the safe default.
 */
const SELF_WRITE_TTL_MS = 4000

export type { TaskToggleOutcome, TaskToggleTarget } from './tasks/types.js'

/** Options for one write. */
export interface TaskToggleOptions {
  /**
   * Skip the live reload this write will trigger. Set by the task panel, whose
   * overlay a reload would destroy; left off for in-document clicks, which
   * want the re-render.
   */
  suppressReload?: boolean
}

/** Cached source lines per repo-relative path, without line terminators. */
const lineCache = new Map<string, string[]>()

/** In-flight raw reads, so two quick clicks on one file fetch it once. */
const pendingReads = new Map<string, Promise<string[] | null>>()

/** Repo-relative paths this page wrote recently, keyed to their timestamps. */
const selfWrites = new Map<string, number>()

/** Normalizes a repo-relative path for comparison against a watcher event. */
function normalizePath(path: string): string {
  return path.replace(/\\/g, '/').replace(/^\/+/, '')
}

/**
 * True when `relativePath` names a file this page just wrote and asked not to
 * be reloaded for. Consumes the entry: only the first event is suppressed.
 */
export function wasSelfWrite(relativePath: string): boolean {
  const key = normalizePath(relativePath)
  const at = selfWrites.get(key)
  if (at === undefined) return false
  selfWrites.delete(key)
  return Date.now() - at < SELF_WRITE_TTL_MS
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
async function fetchSourceLines(path: string): Promise<string[] | null> {
  const response = await fetch(rawUrl(path), {
    headers: editAuthHeaders(),
    credentials: 'same-origin',
  })
  if (!response.ok) return null
  return (await response.text()).split('\n').map((line) => line.replace(/\r$/, ''))
}

/** The source lines of `path`, fetched at most once per page and per read. */
async function sourceLines(path: string): Promise<string[] | null> {
  const cached = lineCache.get(path)
  if (cached) return cached

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
  if (current) return current
  if (fetched) lineCache.set(path, fetched)
  return fetched
}

/** Human-readable reason for a failed write, by status. */
function describeFailure(status: number): { kind: 'conflict' | 'auth' | 'other'; message: string } {
  switch (status) {
    case 409:
      return {
        kind: 'conflict',
        message: 'That line changed on disk, so nothing was written.',
      }
    case 401:
      return {
        kind: 'auth',
        message: 'Editing needs a token — open the editor (e) and enter it first.',
      }
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
 * Set one task's status.
 *
 * Resolves rather than rejects on failure: every caller has a revert to run and
 * a message to show, and a rejected promise would make both easy to forget.
 */
export async function toggleTask(
  target: TaskToggleTarget,
  options: TaskToggleOptions = {}
): Promise<TaskToggleOutcome> {
  const path = normalizePath(target.path)
  let expected: string | undefined
  try {
    const lines = await sourceLines(path)
    expected = lines?.[target.line - 1]
  } catch (err) {
    console.warn('Failed to read the task source:', err)
  }
  if (expected === undefined) {
    // No source means no `expected`, and sending a guess would either be
    // rejected as a conflict or — worse — match a line we never saw.
    lineCache.delete(path)
    return { ok: false, kind: 'other', message: 'Could not read the file to update it.' }
  }

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
    return { ok: false, ...failure }
  }

  // Keep the cache current from the authoritative new text, so a second click
  // on the same box needs no round trip to `/.mbr/raw`.
  try {
    const body = (await response.json()) as { line: number; text: string }
    const lines = lineCache.get(path)
    if (lines && typeof body.text === 'string') lines[target.line - 1] = body.text
  } catch {
    // A response we cannot parse says nothing about the write, which the
    // status code already confirmed. Re-read the file next time instead.
    lineCache.delete(path)
  }

  if (options.suppressReload) selfWrites.set(path, Date.now())
  return { ok: true }
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
 * Only the parts of the markup that a status decides: the box, the data
 * attribute the next click reads back, and the strikethrough. The annotation
 * chips are left alone — a `@done(...)` stamp appears when the page next
 * renders, which for an in-document click is the live reload seconds away.
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
 * The task panel suppresses the reload that would otherwise refresh the
 * document, so when it writes to the file on screen it has to keep that copy
 * honest itself. A no-op for every other file, and for pages with no such line.
 */
export function syncDocumentTask(path: string, line: number, status: TaskStatus): void {
  if (normalizePath(path) !== currentDocumentPath()) return
  const input = document.getElementById(`mbr-task-${line}`)
  if (input instanceof HTMLInputElement) applyCheckboxStatus(input, status)
}
