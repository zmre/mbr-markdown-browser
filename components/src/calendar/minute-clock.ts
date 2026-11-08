/**
 * The one minute tick of a bundle (the calendar chunk; the QuickLook
 * preview): everything that redraws "now" — the companion's today and
 * current-time line, the page's now/next agenda marks — subscribes here, so a
 * page runs a single interval however many of them are on screen.
 *
 * Skipped while the page is hidden; a page coming back fires at once. The
 * interval exists only while someone listens.
 */

/** How often listeners run. */
export const MINUTE_MS = 60_000

const listeners = new Set<() => void>()
let timer: ReturnType<typeof setInterval> | null = null

function fire(): void {
  if (typeof document !== 'undefined' && document.hidden) return
  for (const listener of [...listeners]) listener()
}

/** Calls `listener` every minute while the page is visible; returns the unsubscribe. */
export function onMinute(listener: () => void): () => void {
  listeners.add(listener)
  if (timer === null) {
    timer = setInterval(fire, MINUTE_MS)
    document.addEventListener('visibilitychange', fire)
  }
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0 && timer !== null) {
      clearInterval(timer)
      timer = null
      document.removeEventListener('visibilitychange', fire)
    }
  }
}
