/**
 * A transient, non-blocking message ("No daily notes found").
 *
 * One `<div class="mbr-toast" role="status">` on `<body>`, replaced by the
 * next toast and removed after `durationMs`. Styled by theme.css's "Toast"
 * section, so a repository theme can restyle it. Pure DOM, no module state
 * beyond the element itself: safe for any bundle.
 */

const TOAST_CLASS = 'mbr-toast'

/** Shows `message` for `durationMs` and returns the element. */
export function showToast(message: string, durationMs = 2500): HTMLElement {
  document.querySelectorAll(`.${TOAST_CLASS}`).forEach((el) => el.remove())
  const toast = document.createElement('div')
  toast.className = TOAST_CLASS
  toast.setAttribute('role', 'status')
  toast.setAttribute('aria-live', 'polite')
  toast.textContent = message
  document.body.appendChild(toast)
  window.setTimeout(() => toast.remove(), durationMs)
  return toast
}
