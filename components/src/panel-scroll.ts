/**
 * Scroll keys for a side panel that owns the keyboard while open (the info
 * panel), with the same vocabulary as the page and the other panels:
 * `j`/`k` and `↓`/`↑` a line, `Ctrl+d`/`Ctrl+u` half a pane, `Ctrl+f`/`Ctrl+b`
 * a full pane less 50px of context (`Ctrl+f` on macOS only, see
 * `isMacPlatform` in mbr-keys.ts: elsewhere it is the browser's find).
 *
 * MAIN BUNDLE, pure: no DOM beyond reading the event's modifier fields, so the
 * mapping is testable without layout.
 */

/** One line, the same step `mbr-keys` scrolls the page by for `j`/`k`. */
export const PANEL_SCROLL_LINE = 40

/** Context kept on a full-pane scroll, matching mbr-browse / mbr-search / tasks. */
const FULL_PAGE_CONTEXT = 50

type ScrollKeyEvent = Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>

/**
 * How far (px, positive = down) a key scrolls a pane `paneHeight` tall, or
 * `null` when the key is not a scroll key. `mac` decides whether `Ctrl+f`
 * scrolls.
 */
export function panelScrollAmount(e: ScrollKeyEvent, paneHeight: number, mac: boolean): number | null {
  if (e.metaKey || e.altKey) return null
  const full = Math.max(paneHeight - FULL_PAGE_CONTEXT, 0)
  if (e.ctrlKey) {
    if (e.shiftKey) return null
    switch (e.key.toLowerCase()) {
      case 'd':
        return paneHeight / 2
      case 'u':
        return -paneHeight / 2
      case 'f':
        return mac ? full : null
      case 'b':
        return -full
      default:
        return null
    }
  }
  switch (e.key) {
    case 'j':
    case 'ArrowDown':
      return PANEL_SCROLL_LINE
    case 'k':
    case 'ArrowUp':
      return -PANEL_SCROLL_LINE
    default:
      return null
  }
}
