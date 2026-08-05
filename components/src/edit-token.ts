/**
 * The optional edit token, shared by everything that writes to a file.
 *
 * Every write endpoint answers to the same `check_edit_access` policy
 * (`src/server.rs`): `X-MBR-Edit: 1` on every request, plus
 * `Authorization: Bearer <token>` whenever the server was started with an
 * `edit_token_hash`, is being reached from something other than loopback, or
 * sets `edit_require_token_on_loopback`.
 *
 * The editor prompts for that token in its own footer — but the editor is a
 * separate bundle, so a module-level variable there is invisible to the main
 * bundle and vice versa. `sessionStorage` is the seam: per-tab, cleared when
 * the tab closes, readable from both. It buys two things a checkbox needs — a
 * toggle works on a token-protected server, and the token survives the live
 * reload that a save triggers.
 *
 * Exposure is unchanged by this: the editor already holds the token in a
 * variable on the same origin, and anything that can read `sessionStorage`
 * could equally read that.
 */

const TOKEN_KEY = 'mbr_edit_token'

/** The stored edit token, or `''` when there is none. */
export function getEditToken(): string {
  try {
    return window.sessionStorage?.getItem(TOKEN_KEY) ?? ''
  } catch {
    // Storage access throws outright under some privacy settings and on
    // `file://` origins. A missing token is the same as an empty one.
    return ''
  }
}

/** Store (or, for an empty string, forget) the edit token. */
export function setEditToken(token: string): void {
  try {
    if (token) {
      window.sessionStorage?.setItem(TOKEN_KEY, token)
    } else {
      window.sessionStorage?.removeItem(TOKEN_KEY)
    }
  } catch {
    // See getEditToken: callers keep their own in-memory copy, so a failure
    // here costs cross-bundle sharing, not the current operation.
  }
}

/**
 * Headers for a write request: the CSRF marker, plus the bearer token when one
 * is known. Mirrors `authHeaders` in `editor-crepe.ts`.
 */
export function editAuthHeaders(extra?: Record<string, string>): Record<string, string> {
  const headers: Record<string, string> = { 'X-MBR-Edit': '1', ...extra }
  const token = getEditToken()
  if (token) headers['Authorization'] = `Bearer ${token}`
  return headers
}
