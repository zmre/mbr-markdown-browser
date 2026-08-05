/**
 * The optional edit token: one in-memory value, shared by everything that
 * writes to a file.
 *
 * Every write endpoint answers to the same `check_edit_access` policy
 * (`src/server.rs`): `X-MBR-Edit: 1` on every request, plus
 * `Authorization: Bearer <token>` whenever the server was started with an
 * `edit_token_hash`, is being reached from something other than loopback, or
 * sets `edit_require_token_on_loopback`.
 *
 * # Why a variable and not `sessionStorage`
 *
 * mbr renders arbitrary markdown, and markdown may contain raw HTML, so
 * same-origin script execution is a real vector here rather than a theoretical
 * one. This variable dies when the page does. A `sessionStorage` entry instead
 * sits readable for the whole life of the tab, which turns "may write files
 * while this page is open" into a durable credential that can be exfiltrated
 * and replayed long afterwards. The token therefore never touches
 * `localStorage` or `sessionStorage`, and `edit-token.test.ts` asserts that
 * directly rather than trusting this comment.
 *
 * # Crossing the bundle boundary
 *
 * The editor prompts for the token in its own footer, and the editor is a
 * separate chunk — it must not import main-bundle state (see the chunk rule in
 * `vite.graph.config.ts`), and a module-level variable over there would be
 * invisible here anyway. It does not need to: `openEditor()` receives the
 * current token and hands new ones back through its `onToken` callback, and
 * `<mbr-editor>` — main bundle — does that plumbing.
 *
 * # Why an in-memory token is enough for a checkbox
 *
 * A task write makes the server broadcast a file change, and a live reload
 * would take this variable with it — 401ing the very next click on a
 * token-protected server. It does not, because every task write suppresses its
 * own reload and patches the document in place instead (see `task-toggle.ts`).
 * A token entered once therefore covers every toggle until the page is
 * navigated or reloaded for some other reason.
 */
import { isEditEnabled } from './shared.js'

/** The token for this page load. Never persisted anywhere. */
let token = ''

/** Whether a write has already been refused on this page for want of a token. */
let demanded = false

/**
 * Whether this page may hold token state at all, forgetting it if not.
 *
 * `edit_enabled` is server-rendered per page (`shared.ts`), so it can only
 * change across a navigation — which clears this module anyway. The check is
 * here so that "no token state outlives `edit_enabled`" holds by construction
 * rather than by luck, and so that a page with editing off cannot accumulate a
 * credential it has no use for.
 */
function editable(): boolean {
  if (isEditEnabled()) return true
  clearEditToken()
  return false
}

/** The edit token for this page, or `''` when there is none. */
export function getEditToken(): string {
  return editable() ? token : ''
}

/** Remember (or, for an empty string, forget) the edit token for this page. */
export function setEditToken(next: string): void {
  token = editable() ? next.trim() : ''
}

/** Forget the token and everything inferred from it. */
export function clearEditToken(): void {
  token = ''
  demanded = false
}

/**
 * Record that the server refused a write for want of a token.
 *
 * The editor's token field is hidden until there is a reason to show it. With
 * the token living only as long as the page, a fresh load of a token-protected
 * server has no token and no 401 yet, so "open the editor and enter it" would
 * otherwise point at a field that is not there. A refused task write sets this,
 * and `<mbr-editor>` passes it to `openEditor` so the field is waiting.
 */
export function noteEditTokenRequired(): void {
  demanded = true
}

/** Whether a write has been refused for want of a token on this page. */
export function isEditTokenRequired(): boolean {
  return editable() && demanded
}

/**
 * Headers for a write request: the CSRF marker, plus the bearer token when one
 * is known. Mirrors `authHeaders` in `editor-crepe.ts`.
 */
export function editAuthHeaders(extra?: Record<string, string>): Record<string, string> {
  const headers: Record<string, string> = { 'X-MBR-Edit': '1', ...extra }
  const current = getEditToken()
  if (current) headers['Authorization'] = `Bearer ${current}`
  return headers
}
