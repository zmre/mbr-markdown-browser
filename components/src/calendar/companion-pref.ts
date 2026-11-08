/**
 * The reader's "hide the docked companion" preference (main bundle).
 *
 * `localStorage[companionHiddenKey(repoId)] = "1"` while the docked daily-note
 * companion is dismissed (✕ or `C`). The key is scoped to the served
 * repository the way `review-store.ts` scopes review notes, because several
 * repos served from one origin (the GUI's fixed port, a reused dev port) would
 * otherwise share one preference. `_head.html` reads the same key in an inline
 * script — from `window.__MBR_CONFIG__.repoId`, set by the script just above
 * it — and sets `html.mbr-companion-hidden` before first paint, so theme.css
 * never reserves the column only to collapse it; after that the class is the
 * source of truth, and this module keeps both in step.
 *
 * The repo id is a parameter rather than `shared.ts`'s `getRepoId()` so this
 * module stays free of stateful imports; `<mbr-calendar>` passes it in.
 *
 * A UI preference, not a credential: one flag, nothing from the page. It is on
 * `edit-token.test.ts`'s `localStorage` allowlist for that reason, and calls
 * `localStorage` itself (a wrapper would hide it from that scan).
 */

/** The storage key's base; `_head.html`'s pre-paint script names it too. */
export const COMPANION_HIDDEN_KEY = 'mbr_calendar_companion_hidden'

/** The class on `<html>` that collapses the docked column (theme.css). */
export const COMPANION_HIDDEN_CLASS = 'mbr-companion-hidden'

/**
 * The key for one repository: `COMPANION_HIDDEN_KEY:<repoId>`, or the bare
 * base when there is no id (static builds and error pages, where a site is
 * one repository per origin anyway). `_head.html` computes the same string.
 */
export function companionHiddenKey(repoId: string): string {
  return repoId === '' ? COMPANION_HIDDEN_KEY : `${COMPANION_HIDDEN_KEY}:${repoId}`
}

/** True while the reader has the docked companion dismissed. */
export function isCompanionHidden(): boolean {
  return document.documentElement.classList.contains(COMPANION_HIDDEN_CLASS)
}

/** Dismisses (or restores) the docked companion, remembered for this repository. */
export function setCompanionHidden(hidden: boolean, repoId: string): void {
  document.documentElement.classList.toggle(COMPANION_HIDDEN_CLASS, hidden)
  const key = companionHiddenKey(repoId)
  try {
    if (hidden) localStorage.setItem(key, '1')
    else localStorage.removeItem(key)
  } catch {
    // Storage refused (private mode, file://): the class still applies to
    // this page.
  }
}
