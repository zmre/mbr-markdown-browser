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
 * # Which version of the file
 *
 * `expected` proves the line still *reads* the same, not that it is the same
 * task: a line added above a run of identical `- [ ] call back` items moves a
 * different one onto the number the page knows. So every write also sends
 * `base_hash`, the fingerprint of the version its line numbers come from, and
 * the server refuses any other version with a `409`.
 *
 * - For the page's own note (`markdown_source`) the line numbers come from the
 *   *render*, which carries its fingerprint (`markdown_source_hash`). The
 *   first `/.mbr/raw` read is checked against it, and a different version is a
 *   conflict at once, before anything is sent: the lines just read are not the
 *   ones the page numbered.
 * - For any other file (the task panel's) the line numbers come from a query
 *   newer than any render, so the first read is trusted as before; its
 *   fingerprint then guards every later write from this page.
 *
 * Each successful write answers with the new fingerprint, which replaces the
 * held one, so a page's own consecutive writes — task toggles and flashcard
 * reviews alike — chain. Writes to one file are serialized ({@link
 * serializeWrite}): two quick clicks would otherwise both send the version
 * before either write, and the second would be refused.
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
import { NOTE_CHANGED_MESSAGE, staleKindForRead, staleKindForWrite, staleNoteMessage } from './stale-note.js'
import { syncDoneChip } from './task-chips.js'
import type { TaskStatus, TaskToggleFailure, TaskToggleOutcome, TaskToggleTarget } from './tasks/types.js'

/** Endpoint for a single-line task patch (`server.rs::task_toggle_handler`). */
const TASK_ENDPOINT = '/.mbr/task'

/** Prefix for raw markdown reads (`server.rs::raw_markdown_handler`). */
const RAW_PREFIX = '/.mbr/raw/'

/**
 * How long a write keeps reloads for that file suppressed outright.
 *
 * One write produces **several** reload-worthy events, not one: the handler
 * broadcasts explicitly before it responds, and then the watcher sees the
 * atomic rename and broadcasts again — twice, on macOS, measured about 7ms
 * later. So the suppression is a window rather than a single token to be
 * consumed; consuming one entry would swallow the handler's event and let the
 * watcher's echo reload the page anyway, which is the failure this window
 * exists to prevent.
 *
 * **The window is not enough on its own.** The handler's broadcast is prompt,
 * but the watcher's echo is whatever FSEvents decides: measured on an idle
 * macOS `/tmp`, one echo came ~950ms after its request, and a run of later
 * writes produced *no* echo until one coalesced event arrived more than two
 * minutes afterwards. An echo that lands outside the window used to reload the
 * page — closing an open flashcard deck mid-review — so a late event for a
 * file this page wrote is settled by content instead: see
 * {@link isOwnWriteEcho}. The window stays because it is free (no request) and
 * because it covers the moment a write is in flight, before the cache holds
 * the new text.
 */
const SELF_WRITE_TTL_MS = 1500

export type { TaskToggleOutcome, TaskToggleTarget } from './tasks/types.js'

/** Response header carrying a file's fingerprint (`server.rs::raw_markdown_handler`). */
const HASH_HEADER = 'x-mbr-content-hash'

/**
 * A file's source as this page holds it: its lines, without terminators, and
 * the fingerprint of the version they are (`null` when the server sent none,
 * which leaves the line check as the only one).
 */
interface HeldSource {
  lines: string[]
  hash: string | null
}

/** Cached source per repo-relative path. */
const lineCache = new Map<string, HeldSource>()

/**
 * The outcome of reading a file's source, with the status when it failed.
 *
 * `lines` is the cache's own array, so a writer that knows what changed can
 * patch it in place; `hash` is the fingerprint at the time of the read, sent
 * as the write's `base_hash`.
 */
export type SourceRead = { ok: true; lines: string[]; hash: string | null } | { ok: false; status: number }

/**
 * The version of the page's own note this page shows, once a write has moved
 * it on from the rendered one (`markdown_source_hash`). `undefined` until then.
 */
let documentHash: string | null | undefined

/** Per-file chains of in-flight writes; see {@link serializeWrite}. */
const writeChains = new Map<string, Promise<unknown>>()

/** In-flight raw reads, so two quick clicks on one file fetch it once. */
const pendingReads = new Map<string, Promise<SourceRead>>()

/** Repo-relative paths this page wrote recently, keyed to their timestamps. */
const selfWrites = new Map<string, number>()

/**
 * Every repo-relative path this page has written, for the page's lifetime.
 *
 * The window in {@link selfWrites} covers the events a write produces
 * *promptly*; this set is what lets {@link isOwnWriteEcho} recognise the ones
 * that arrive late.
 */
const writtenPaths = new Set<string>()

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
 *
 * Exported for every other in-place line write (`flashcards/review-writer.ts`,
 * which receives it from `<mbr-flashcards>` as a service): there
 * is one suppression window per page, and a second registry would be invisible
 * to `<mbr-live-reload>`, which consults only {@link wasSelfWrite} and
 * {@link isOwnWriteEcho}.
 */
export function noteSelfWrite(path: string): void {
  const now = Date.now()
  const key = normalizePath(path)
  for (const [written, at] of selfWrites) {
    if (now - at >= SELF_WRITE_TTL_MS) selfWrites.delete(written)
  }
  selfWrites.set(key, now)
  writtenPaths.add(key)
}

/**
 * True when a reload for `relativePath` might be an echo of this page's own
 * writes — it wrote that file at some point — so {@link isOwnWriteEcho} is
 * worth a request. Synchronous so that every other event decides at once.
 */
export function mayBeOwnWriteEcho(relativePath: string): boolean {
  return writtenPaths.has(normalizePath(relativePath))
}

/**
 * True when the file on disk is exactly what this page already holds and has
 * drawn: the event is an echo of its own writes, however late it arrived.
 *
 * The comparison is against the source-line cache, which every write keeps in
 * step from the server's answer (`toggleTask` patches the line, the flashcard
 * writer splices in the inserted ones). Anything that makes the cache
 * untrustworthy — a 409, an unreadable response — drops it, and with no cache
 * this answers false and the page reloads as it always did. So does a failed
 * read (a 401 on a token-protected server, the server gone): reloading is the
 * safe default, because a page that silently stops following real external
 * edits is the worse failure.
 *
 * Reads the file fresh, bypassing the cache and its single-flight, since the
 * whole question is whether the cache is still current.
 */
export async function isOwnWriteEcho(relativePath: string): Promise<boolean> {
  const key = normalizePath(relativePath)
  if (!writtenPaths.has(key) || !lineCache.has(key)) return false
  let disk: SourceRead
  try {
    disk = await fetchSourceLines(key)
  } catch {
    return false
  }
  if (!disk.ok) return false
  // A write that started while the file was being read is still in flight, and
  // its line is not in the cache yet: the window is the authority for it.
  if (wasSelfWrite(key)) return true
  const held = lineCache.get(key)?.lines
  return held !== undefined && held.length === disk.lines.length && held.every((line, i) => line === disk.lines[i])
}

/** Test hook: forget every cached source line and pending suppression. */
export function resetTaskToggleState(): void {
  lineCache.clear()
  pendingReads.clear()
  selfWrites.clear()
  writtenPaths.clear()
  writeChains.clear()
  documentHash = undefined
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
  const hash = response.headers?.get(HASH_HEADER) ?? null
  const lines = (await response.text()).split('\n').map((line) => line.replace(/\r$/, ''))
  return { ok: true, lines, hash }
}

/**
 * The version of `path` the page was rendered from — or has written since —
 * when `path` is the page's own note; `null` for any other file, or a page
 * that carries no fingerprint.
 */
function renderedVersion(path: string): string | null {
  if (path !== currentDocumentPath()) return null
  if (documentHash !== undefined) return documentHash
  const rendered = window.frontmatter?.['markdown_source_hash']
  return typeof rendered === 'string' && rendered.length > 0 ? rendered : null
}

/**
 * The source lines of `path`, fetched at most once per page and per read.
 *
 * Exported as {@link readSourceLines} for the review feature's suggestion
 * prefill. Sharing this rather than opening a second reader is not tidiness:
 * a second cache would reintroduce the interleaved-read hazard the two comments
 * below exist to close, and it would miss the invalidation a task write does.
 */
async function sourceLines(path: string): Promise<SourceRead> {
  const cached = lineCache.get(path)
  if (cached) return { ok: true, ...cached }

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
  if (current) return { ok: true, ...current }
  if (!fetched.ok) return fetched

  // The page's own note, read for the first time: its line numbers are the
  // render's, so a file that is no longer the rendered version is a conflict
  // — what is on line N now may be a different task. Not cached, so every
  // attempt says the same until the page is reloaded.
  const rendered = renderedVersion(path)
  if (rendered !== null && fetched.hash !== null && fetched.hash !== rendered) {
    return { ok: false, status: 409 }
  }
  lineCache.set(path, { lines: fetched.lines, hash: fetched.hash })
  return fetched
}

/**
 * Record the version a successful write left `path` at — the response's
 * `hash` — so the next write from this page names it. For the page's own note
 * it is also the version the page now shows. `null` (no hash in the response)
 * stops checking versions for that file rather than guessing one.
 */
export function noteSourceHash(path: string, hash: string | null): void {
  const key = normalizePath(path)
  const held = lineCache.get(key)
  if (held) held.hash = hash
  if (key === currentDocumentPath()) documentHash = hash
}

/**
 * Run `write` after every earlier write to `path` from this page has settled.
 *
 * Each write sends the version its predecessor left (`base_hash`), so two
 * writes to one file in flight at once would both name the version before
 * either, and the server would refuse the second. Shared by every in-place
 * writer — the flashcard deck receives it as a service — because a chain per
 * writer would not see the other's writes.
 */
export function serializeWrite<T>(path: string, write: () => Promise<T>): Promise<T> {
  const key = normalizePath(path)
  const previous = writeChains.get(key) ?? Promise.resolve()
  const result = previous.then(write)
  const settled = result.catch(() => {})
  writeChains.set(key, settled)
  void settled.then(() => {
    if (writeChains.get(key) === settled) writeChains.delete(key)
  })
  return result
}

/**
 * What to tell the user when a token is missing.
 *
 * Points at the editor because that is where the field is. `edit-token.ts`
 * remembers the refusal so the field is actually visible when they get there.
 */
export const TOKEN_MESSAGE = 'Editing needs a token — open the editor (e) and enter it first.'

/**
 * What to say on a 403. Not "editing is not enabled": nothing here writes
 * unless the page says editing is on, and the server says so only for a
 * `Host` `check_edit_access` accepts — so a 403 means the server's config
 * changed under an open page.
 */
const REFUSED_MESSAGE = 'This server is not accepting edits from this page.'

/** A failed toggle, as {@link toggleTask} resolves it. */
type ToggleFailure = { kind: TaskToggleFailure; message: string }

/** The failure for a note that changed elsewhere, or is still downloading. */
function staleFailure(kind: 'changed' | 'downloading'): ToggleFailure {
  return { kind: kind === 'changed' ? 'conflict' : 'downloading', message: staleNoteMessage(kind) }
}

/** Human-readable reason for a failed write, by status. */
function describeFailure(status: number): ToggleFailure {
  const stale = staleKindForWrite(status)
  if (stale) return staleFailure(stale)
  switch (status) {
    case 401:
      return { kind: 'auth', message: TOKEN_MESSAGE }
    case 403:
      return { kind: 'auth', message: REFUSED_MESSAGE }
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
 * the one thing the user can act on. A `404` means the note is no longer where
 * this page found it, and a `503` that it is still downloading (the iOS app,
 * after an edit on another device made the local copy a placeholder).
 */
function describeReadFailure(status: number): ToggleFailure {
  const stale = staleKindForRead(status)
  if (stale) return staleFailure(stale)
  switch (status) {
    case 401:
      return { kind: 'auth', message: TOKEN_MESSAGE }
    case 403:
      return { kind: 'auth', message: REFUSED_MESSAGE }
    case 0:
      return { kind: 'other', message: 'The server could not be reached.' }
    default:
      return { kind: 'other', message: `Could not read the note to update it (${status}).` }
  }
}

/**
 * Set one task's status.
 *
 * Resolves rather than rejects on failure: every caller has a revert to run and
 * a message to show, and a rejected promise would make both easy to forget.
 */
export function toggleTask(target: TaskToggleTarget): Promise<TaskToggleOutcome> {
  return serializeWrite(target.path, () => writeTask(target))
}

/** {@link toggleTask}, once the file has no other write from this page in flight. */
async function writeTask(target: TaskToggleTarget): Promise<TaskToggleOutcome> {
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
    return { ok: false, kind: 'conflict', message: NOTE_CHANGED_MESSAGE }
  }

  // Before the request, not after it: see `noteSelfWrite`.
  noteSelfWrite(path)

  let response: Response
  try {
    response = await fetch(TASK_ENDPOINT, {
      method: 'POST',
      headers: editAuthHeaders({ 'Content-Type': 'application/json' }),
      credentials: 'same-origin',
      body: JSON.stringify({
        path,
        line: target.line,
        expected,
        to: target.to,
        base_hash: read.hash ?? undefined,
      }),
    })
  } catch (err) {
    console.warn('Task toggle request failed:', err)
    return { ok: false, kind: 'other', message: 'The server could not be reached.' }
  }

  if (!response.ok) {
    const failure = describeFailure(response.status)
    // Our copy of the line is what the server just disagreed with; drop it so
    // the retry re-reads rather than repeating the same rejected request. A
    // note still downloading is about to change under us too.
    if (failure.kind === 'conflict' || failure.kind === 'downloading') lineCache.delete(path)
    if (response.status === 401) noteEditTokenRequired()
    return { ok: false, ...failure }
  }

  // Keep the cache current from the authoritative new text, so a second click
  // on the same box needs no round trip to `/.mbr/raw`. The same text is handed
  // back to the caller, which is the only place a new `@done(...)` can come
  // from now that the write no longer reloads the page.
  try {
    const body = (await response.json()) as { line: number; text: string; hash?: unknown }
    if (typeof body.text !== 'string') throw new Error('no text in the response')
    read.lines[target.line - 1] = body.text
    noteSourceHash(path, typeof body.hash === 'string' ? body.hash : null)
    return { ok: true, text: body.text }
  } catch {
    // A response we cannot parse says nothing about the write, which the
    // status code already confirmed. Re-read the file next time instead — and
    // stop checking the page's version, which moved to one we never learned.
    lineCache.delete(path)
    noteSourceHash(path, null)
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

/**
 * Read a file's source lines through this module's cache.
 *
 * The review feature's suggestion prefill needs the *source* of the lines a
 * note is anchored to, which is the same read a task toggle makes to source its
 * `expected`. One reader, one cache, one single-flight — see {@link sourceLines}.
 *
 * `{ ok: false, status: 403 }` is the ordinary answer on a server started
 * without `--edit`, not an error: `/.mbr/raw` sits behind `check_edit_access`.
 * Callers are expected to degrade rather than report a failure.
 */
export function readSourceLines(path: string): Promise<SourceRead> {
  return sourceLines(path)
}

/**
 * Drop `path`'s cached lines, so the next read fetches the file again.
 *
 * For writers outside this module that learn the copy is wrong — a `409` from
 * `POST /.mbr/flashcard-review` — and have no authoritative text to patch in.
 */
export function forgetSourceLines(path: string): void {
  lineCache.delete(normalizePath(path))
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
