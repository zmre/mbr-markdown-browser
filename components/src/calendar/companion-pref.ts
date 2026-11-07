/**
 * The reader's "hide the docked companion" preference (main bundle).
 *
 * `localStorage["mbr_calendar_companion_hidden"] = "1"` while the docked
 * daily-note companion is dismissed (✕ or `C`). `_head.html` reads the same
 * key in an inline script and sets `html.mbr-companion-hidden` before first
 * paint, so theme.css never reserves the column only to collapse it; after
 * that the class is the source of truth, and this module keeps both in step.
 *
 * A UI preference, not a credential: one flag, nothing from the page. It is on
 * `edit-token.test.ts`'s `localStorage` allowlist for that reason, and calls
 * `localStorage` itself (a wrapper would hide it from that scan).
 */

/** The storage key; `_head.html`'s pre-paint script names it too. */
export const COMPANION_HIDDEN_KEY = 'mbr_calendar_companion_hidden'

/** The class on `<html>` that collapses the docked column (theme.css). */
export const COMPANION_HIDDEN_CLASS = 'mbr-companion-hidden'

/** True while the reader has the docked companion dismissed. */
export function isCompanionHidden(): boolean {
  return document.documentElement.classList.contains(COMPANION_HIDDEN_CLASS)
}

/** Dismisses (or restores) the docked companion, remembered across pages. */
export function setCompanionHidden(hidden: boolean): void {
  document.documentElement.classList.toggle(COMPANION_HIDDEN_CLASS, hidden)
  try {
    if (hidden) localStorage.setItem(COMPANION_HIDDEN_KEY, '1')
    else localStorage.removeItem(COMPANION_HIDDEN_KEY)
  } catch {
    // Storage refused (private mode, file://): the class still applies to
    // this page.
  }
}
