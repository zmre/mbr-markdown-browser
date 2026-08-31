/**
 * Copying the exported review to the clipboard.
 *
 * The first clipboard code in the frontend, so the constraints are worth
 * stating. `navigator.clipboard` is only defined in a **secure context**.
 * `http://127.0.0.1` and `http://localhost` *are* potentially trustworthy
 * origins, so GUI mode and the usual `mbr -s` both get the async API — but
 * `mbr -s --host 0.0.0.0` reached from another machine by IP is not, and there
 * `navigator.clipboard` is simply `undefined`. That is a supported way to run
 * mbr, so the fallback is a real code path rather than defensive decoration.
 *
 * `isSecureContext` is therefore tested **synchronously, first**: an `await` on
 * a rejected promise would put the fallback in a later task, and
 * `document.execCommand('copy')` only works inside the task that began as a
 * user gesture.
 */

/**
 * Copy `text`, returning whether it worked.
 *
 * Callers must have a visible manual fallback for `false` — the panel keeps a
 * read-only textarea holding the same text for exactly that reason. A copy
 * button that silently does nothing is worse than no button.
 */
export async function copyText(text: string): Promise<boolean> {
  if (globalThis.isSecureContext && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      // Permission denied, or a document that lost focus mid-write. Fall
      // through — the legacy path often still works.
    }
  }
  return legacyCopy(text)
}

/**
 * The pre-`navigator.clipboard` route: a hidden textarea, selected and copied.
 *
 * Positioned off-screen rather than `display: none` or `hidden`, because a
 * genuinely invisible element cannot hold a selection and the copy silently
 * produces an empty clipboard.
 */
function legacyCopy(text: string): boolean {
  if (typeof document === 'undefined' || typeof document.execCommand !== 'function') {
    return false
  }

  const area = document.createElement('textarea')
  area.value = text
  area.setAttribute('readonly', '')
  area.setAttribute('aria-hidden', 'true')
  area.style.position = 'fixed'
  area.style.top = '-9999px'
  area.style.left = '-9999px'
  area.style.opacity = '0'

  const previous = document.activeElement
  document.body.appendChild(area)
  try {
    area.select()
    area.setSelectionRange(0, text.length)
    return document.execCommand('copy')
  } catch {
    return false
  } finally {
    area.remove()
    // Restore focus, so a keyboard user is not dumped back at the top of the
    // page by having pressed `c`.
    if (previous instanceof HTMLElement) previous.focus()
  }
}
