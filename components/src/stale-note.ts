/**
 * What to tell the reader when an in-place write finds the note is not the
 * one on screen any more — changed on another device, or (in the iOS app over
 * iCloud) still downloading — and the one way every writer offers a reload.
 *
 * Shared by the in-document task checkboxes (`mbr-task-doc.ts`), the task
 * panel, the flashcard deck's review writer and the editor. Stateless — no
 * fetches, no caches — so the lazy chunks may import it.
 *
 * The statuses come from the edit endpoints (`server.rs`):
 *
 * | Status | Where | Meaning |
 * |--------|-------|---------|
 * | `409` | write | The line (or the file's hash) no longer matches what this page read |
 * | `404` | `/.mbr/raw` | The note is gone from where the page found it (moved or deleted elsewhere) |
 * | `503` | read or write | The note is not downloaded yet; the server asked the provider for it |
 *
 * A line the page names that is past the end of the file is the same story as
 * a `409`: the file is shorter than the page thinks.
 */

/** Why a note-level write could not go ahead because the page is stale. */
export type StaleNoteKind = 'changed' | 'downloading'

/** Statement for a note that changed elsewhere; nothing was written. */
export const NOTE_CHANGED_MESSAGE =
  'This note was changed somewhere else since it was opened, so nothing was written.'

/** The question {@link offerReload} adds to a statement. */
export const RELOAD_QUESTION = 'Reload it to get the latest version?'

/** Statement for a note the server is still fetching from the cloud. */
export const NOTE_DOWNLOADING_MESSAGE =
  'This note is still downloading from the cloud. Try again in a moment.'

/**
 * The stale-note reading of a failed `/.mbr/raw` read, or `null` when the
 * status means something else (auth, a server error).
 */
export function staleKindForRead(status: number): StaleNoteKind | null {
  switch (status) {
    case 404:
    case 409:
      return 'changed'
    case 503:
      return 'downloading'
    default:
      return null
  }
}

/**
 * The stale-note reading of a failed write (`/.mbr/task`,
 * `/.mbr/flashcard-review`, `/.mbr/edit`). A `404` here is not included: the
 * write's own resolution would have failed the same way the read did, so it
 * says nothing the read did not.
 */
export function staleKindForWrite(status: number): StaleNoteKind | null {
  switch (status) {
    case 409:
      return 'changed'
    case 503:
      return 'downloading'
    default:
      return null
  }
}

/** The statement for `kind`. */
export function staleNoteMessage(kind: StaleNoteKind): string {
  return kind === 'changed' ? NOTE_CHANGED_MESSAGE : NOTE_DOWNLOADING_MESSAGE
}

/** Injection points for {@link offerReload}; the defaults are the page's own. */
export interface ReloadHooks {
  readonly confirm?: (question: string) => boolean
  readonly reload?: () => void
}

/**
 * Ask whether to reload the page for the latest version of the note, and do
 * it if the reader agrees. Returns whether the page is reloading.
 *
 * `confirm` rather than a component of our own: it is modal, needs no styles
 * in any of the shadow roots involved, and the iOS app implements it natively
 * (`DialogModel` in `apple/ios`). `statement` is what happened; the question
 * is appended.
 */
export function offerReload(statement: string = NOTE_CHANGED_MESSAGE, hooks: ReloadHooks = {}): boolean {
  const ask = hooks.confirm ?? ((question: string) => window.confirm(question))
  const reload = hooks.reload ?? (() => window.location.reload())
  if (!ask(`${statement} ${RELOAD_QUESTION}`)) return false
  reload()
  return true
}
