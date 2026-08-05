/**
 * In-document task behaviour: clickable checkboxes and the `#mbr-task-N` jump.
 *
 * Two small jobs that both belong to the rendered page rather than to the task
 * panel, and therefore ship in the main bundle:
 *
 * 1. **Toggling.** With editing on, `.mbr-task-check` inputs lose their
 *    `disabled` attribute and one delegated listener on `main#wrapper` turns a
 *    left click into "done ↔ open" and a right click into "canceled ↔ open"
 *    (TASKS_SPEC.md's editing section). One listener, not one per box: a daily
 *    note can carry hundreds.
 * 2. **Jumping.** The task panel's `Enter` navigates to
 *    `{url_path}#mbr-task-42`; this element scrolls that line clear of the
 *    sticky header and flashes it, on load and on every `hashchange`.
 *
 * The jump half runs everywhere, including static builds: `task_checkbox_html`
 * emits the `id` in every mode, so a bookmarked deep link works on a built site
 * even though nothing there can produce one. The toggle half self-gates on
 * `isEditEnabled()`, which is false in a build.
 *
 * # Why the optimistic flip is so small
 *
 * A successful write makes the server broadcast a file change, so
 * `<mbr-live-reload>` re-renders the page within a second and that render is
 * what shows a newly stamped `@done(...)` chip. The flip here exists to cover
 * the round trip, not to reproduce the renderer: it moves the box, the status
 * attribute and the strikethrough, and leaves the chips to the reload. On
 * failure no broadcast happens, no reload comes, and the flip is reverted.
 */
import { LitElement, nothing } from 'lit'
import { customElement } from 'lit/decorators.js'
import { waitForDom } from './dynamic-loader.js'
import { isEditEnabled } from './shared.js'
import {
  applyCheckboxStatus,
  checkboxStatus,
  currentDocumentPath,
  nextCancelStatus,
  nextToggleStatus,
  toggleTask,
} from './task-toggle.js'
import type { TaskStatus } from './tasks/types.js'

/** Class marking a checkbox this element has made interactive. */
const EDITABLE_CLASS = 'mbr-task-editable'

/** Class driving the arrival flash; the animation lives in `theme.css`. */
const FLASH_CLASS = 'mbr-task-flash'

/** Clearance above and below a task before it counts as "on screen". */
const SCROLL_PADDING = 24

/** Belt-and-braces flash cleanup for browsers that skip `animationend`. */
const FLASH_TIMEOUT_MS = 2500

/**
 * The 1-based task line a fragment names, or `null`.
 *
 * Deliberately strict: `#mbr-task-4` is a jump, `#mbr-task-4x` and
 * `#mbr-tasks` are not, so an unrelated anchor never reaches the flash.
 */
export function taskLineFromHash(hash: string): number | null {
  const match = /^#?mbr-task-(\d+)$/.exec(hash)
  if (!match) return null
  const line = Number(match[1])
  return Number.isSafeInteger(line) && line > 0 ? line : null
}

/** Height of the sticky page header, which a jump must not land underneath. */
function stickyHeaderHeight(): number {
  const header = document.querySelector<HTMLElement>('body > header')
  return header?.getBoundingClientRect().height ?? 0
}

/**
 * Scroll `element` clear of the sticky header, doing nothing when it is
 * already comfortably visible.
 *
 * Modelled on `find-in-page.ts`'s `scrollRangeIntoView`, and no-op-when-visible
 * for a specific reason here: the browser performs its own fragment jump on
 * load (correctly, thanks to `scroll-padding-top` in `theme.css`). Scrolling
 * unconditionally would fight that with a second, differently-computed jump.
 */
export function scrollTaskIntoView(element: Element, topInset: number): void {
  const rect = element.getBoundingClientRect()
  // A detached element — and every element under happy-dom — reports zeros.
  if (rect.width === 0 && rect.height === 0) return

  const top = topInset + SCROLL_PADDING
  const bottom = window.innerHeight - SCROLL_PADDING
  if (rect.top >= top && rect.bottom <= bottom) return

  window.scrollTo({ top: Math.max(0, window.scrollY + rect.top - top), behavior: 'auto' })
}

/** The element to highlight for a task: its list item, else the checkbox. */
function flashTarget(input: HTMLElement): HTMLElement {
  return input.closest('li') ?? input
}

/** Which flash owns an element's highlight, so an earlier one cannot end it. */
const flashTokens = new WeakMap<HTMLElement, number>()
let flashCounter = 0

/** Run the arrival flash, restarting it if the same task is re-targeted. */
export function flashTask(element: HTMLElement): void {
  element.classList.remove(FLASH_CLASS)
  // Force a reflow so re-adding the class restarts the animation rather than
  // being coalesced into "no change" — the classic CSS-animation replay.
  void element.offsetWidth
  element.classList.add(FLASH_CLASS)

  const token = ++flashCounter
  flashTokens.set(element, token)
  const clear = () => {
    // Back/forward between two tasks can re-flash this one before the previous
    // run's timeout fires; whoever started last owns the class.
    if (flashTokens.get(element) === token) element.classList.remove(FLASH_CLASS)
  }
  element.addEventListener('animationend', clear, { once: true })
  window.setTimeout(clear, FLASH_TIMEOUT_MS)
}

/**
 * Reveal the task named by `hash`. Returns the element it acted on, or `null`
 * when the fragment is not a task link or names a line that is not on the page.
 */
export function revealTaskFromHash(hash: string): HTMLElement | null {
  const line = taskLineFromHash(hash)
  if (line === null) return null
  const input = document.getElementById(`mbr-task-${line}`)
  if (!input) return null

  const target = flashTarget(input)
  scrollTaskIntoView(target, stickyHeaderHeight())
  flashTask(target)
  return target
}

/** Make every rendered checkbox in `root` interactive. Idempotent. */
export function enableTaskCheckboxes(root: ParentNode): void {
  root.querySelectorAll<HTMLInputElement>('input.mbr-task-check').forEach((input) => {
    input.disabled = false
    input.classList.add(EDITABLE_CLASS)
    input.title = 'Click to complete · right-click to cancel'
  })
}

/** The task checkbox an event landed on, or `null`. */
function checkboxFrom(event: Event): HTMLInputElement | null {
  const target = event.target
  return target instanceof HTMLInputElement && target.classList.contains('mbr-task-check')
    ? target
    : null
}

@customElement('mbr-task-doc')
export class MbrTaskDocElement extends LitElement {
  /** Lines with a write in flight, so a double click cannot race itself. */
  private _inFlight = new Set<number>()

  override connectedCallback() {
    super.connectedCallback()
    window.addEventListener('hashchange', this._handleHashChange)
    waitForDom()
      .then(() => this._install())
      .catch((err) => console.warn('Task document setup failed:', err))
  }

  override disconnectedCallback() {
    super.disconnectedCallback()
    window.removeEventListener('hashchange', this._handleHashChange)
    const wrapper = this._wrapper()
    wrapper?.removeEventListener('click', this._handleClick)
    wrapper?.removeEventListener('contextmenu', this._handleContextMenu)
  }

  private _wrapper(): HTMLElement | null {
    return document.querySelector<HTMLElement>('main#wrapper') ?? document.querySelector('main')
  }

  private _install(): void {
    // The browser has already jumped to the fragment by now; this adds the
    // header clearance and the flash on top of that.
    revealTaskFromHash(window.location.hash)

    if (!isEditEnabled()) return
    const wrapper = this._wrapper()
    if (!wrapper) return
    enableTaskCheckboxes(wrapper)
    wrapper.addEventListener('click', this._handleClick)
    wrapper.addEventListener('contextmenu', this._handleContextMenu)
  }

  private _handleHashChange = () => {
    revealTaskFromHash(window.location.hash)
  }

  private _handleClick = (event: Event) => {
    const input = checkboxFrom(event)
    if (!input) return
    // Deliberately NOT `preventDefault()`. Cancelling a checkbox's click
    // restores its pre-click state *after* the listener returns, which would
    // silently undo the optimistic flip made below. Letting the browser's own
    // flip stand costs nothing, because `data-mbr-task-status` — not
    // `checked` — is what the next click reads back, and `_write` restores
    // both whenever it declines or the server refuses.
    void this._write(input, nextToggleStatus(checkboxStatus(input)))
  }

  private _handleContextMenu = (event: MouseEvent) => {
    const input = checkboxFrom(event)
    if (!input) return
    // Right-click cancels (TASKS_SPEC.md:75), so the context menu must not
    // open on top of it.
    event.preventDefault()
    void this._write(input, nextCancelStatus(checkboxStatus(input)))
  }

  private async _write(input: HTMLInputElement, to: TaskStatus): Promise<void> {
    const previous = checkboxStatus(input)
    const path = currentDocumentPath()
    const line = Number(input.dataset.mbrTaskLine)
    // Every decline restores the box: a left click has already moved it (see
    // `_handleClick`), so returning without this would leave the page claiming
    // a status nothing wrote.
    if (!path || !Number.isSafeInteger(line) || line <= 0 || this._inFlight.has(line)) {
      applyCheckboxStatus(input, previous)
      return
    }

    this._inFlight.add(line)
    applyCheckboxStatus(input, to)
    try {
      // No `suppressReload`: the reload this triggers is the authoritative
      // re-render, and the only thing that can draw a new `@done(...)` chip.
      const outcome = await toggleTask({ path, line, to })
      if (!outcome.ok) {
        applyCheckboxStatus(input, previous)
        this._report(outcome.message)
      }
    } finally {
      this._inFlight.delete(line)
    }
  }

  /**
   * Surface a failure. `alert` rather than a toast component on purpose: this
   * fires only when a write was refused, which on a local server means a real
   * misconfiguration the user needs to see rather than dismiss.
   */
  private _report(message: string): void {
    console.warn('Task toggle failed:', message)
    window.alert(message)
  }

  override render() {
    return nothing
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-task-doc': MbrTaskDocElement
  }
}
