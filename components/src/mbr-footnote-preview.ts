/**
 * Reader-side footnote hover previews ("rich popover").
 *
 * On hover-capable (desktop) devices, hovering a footnote reference superscript
 * shows the footnote's rendered content in a floating card near the reference.
 * Clicking is left untouched so the native jump to the definition still works.
 *
 * The card content is cloned from the in-page `.footnote-definition` DOM — no
 * network fetch. A single popover element is reused across all references.
 *
 * Enhancement runs during browser idle time to avoid blocking first paint and
 * is idempotent via the `mbr-footnote-enhanced` marker class. On touch devices
 * the component does nothing: click-to-jump already works there.
 */
import { LitElement, nothing } from 'lit'
import { customElement } from 'lit/decorators.js'
import { waitForDom, scheduleIdleTask } from './dynamic-loader.ts'

const ENHANCED_CLASS = 'mbr-footnote-enhanced'
const POPOVER_CLASS = 'mbr-footnote-popover'
const POPOVER_ID = 'mbr-footnote-popover'
/** Grace period before hiding, so the pointer can travel into the card. */
const HIDE_DELAY_MS = 120
/** Gap (px) between the reference and the popover, and viewport clamp margin. */
const GAP_PX = 8

/**
 * Resolve the footnote definition element a reference anchor points to.
 * Returns null when the target is missing or is not a footnote definition.
 *
 * Exported for unit testing.
 */
export function resolveFootnoteDefinition(
  anchor: HTMLAnchorElement
): HTMLElement | null {
  const href = anchor.getAttribute('href')
  if (!href || !href.startsWith('#')) return null
  let id: string
  try {
    id = decodeURIComponent(href.slice(1))
  } catch {
    // Malformed percent-encoding — fall back to the raw fragment.
    id = href.slice(1)
  }
  if (!id) return null
  const target = document.getElementById(id)
  if (!target || !target.classList.contains('footnote-definition')) return null
  return target
}

/**
 * Build the preview content for a footnote definition: a clone of the
 * definition with its numeric label removed, leaving just the note body
 * (paragraphs, links, and any other inline content).
 *
 * The original definition is not mutated. Exported for unit testing.
 */
export function buildPreviewFragment(def: HTMLElement): DocumentFragment {
  const clone = def.cloneNode(true) as HTMLElement
  clone
    .querySelectorAll('.footnote-definition-label')
    .forEach((el) => el.remove())
  const fragment = document.createDocumentFragment()
  while (clone.firstChild) {
    fragment.appendChild(clone.firstChild)
  }
  return fragment
}

@customElement('mbr-footnote-preview')
export class MbrFootnotePreviewElement extends LitElement {
  private _popover: HTMLDivElement | null = null
  private _hideTimer: number | undefined
  private _activeAnchor: HTMLAnchorElement | null = null

  override connectedCallback() {
    super.connectedCallback()
    // Desktop only: require a hover-capable, fine pointer. On touch the native
    // click-to-jump already works, so previews would just get in the way.
    const mq = window.matchMedia?.('(hover: hover) and (pointer: fine)')
    if (!mq || !mq.matches) return

    waitForDom()
      .then(() => scheduleIdleTask(() => this._enhance()))
      .catch((e) => console.warn('footnote preview enhancement failed:', e))
  }

  private _enhance(): void {
    const anchors = document.querySelectorAll<HTMLAnchorElement>(
      'main sup.footnote-reference > a[href^="#"]'
    )

    anchors.forEach((anchor) => {
      if (anchor.classList.contains(ENHANCED_CLASS)) return
      anchor.classList.add(ENHANCED_CLASS)

      anchor.addEventListener('mouseenter', () => this._show(anchor))
      anchor.addEventListener('mouseleave', () => this._scheduleHide())
      // Keyboard a11y: previews follow focus as well as the pointer.
      anchor.addEventListener('focus', () => this._show(anchor))
      anchor.addEventListener('blur', () => this._scheduleHide())
    })
  }

  private _getPopover(): HTMLDivElement {
    if (this._popover) return this._popover
    const el = document.createElement('div')
    el.className = POPOVER_CLASS
    el.id = POPOVER_ID
    el.setAttribute('role', 'tooltip')
    el.style.display = 'none'
    // Keep the card visible while the pointer is over it, so users can reach
    // links inside long notes.
    el.addEventListener('mouseenter', () => this._cancelHide())
    el.addEventListener('mouseleave', () => this._scheduleHide())
    document.body.appendChild(el)
    this._popover = el
    return el
  }

  private _show(anchor: HTMLAnchorElement): void {
    this._cancelHide()
    const def = resolveFootnoteDefinition(anchor)
    if (!def) return

    const popover = this._getPopover()
    popover.replaceChildren(buildPreviewFragment(def))
    popover.style.display = 'block'
    this._activeAnchor = anchor
    anchor.setAttribute('aria-describedby', POPOVER_ID)
    this._position(anchor, popover)
  }

  /**
   * Position the popover near the reference using fixed coordinates. Prefer
   * placing it above the reference; flip below when there's no room. Clamp
   * horizontally and vertically to the viewport.
   */
  private _position(anchor: HTMLElement, popover: HTMLElement): void {
    const rect = anchor.getBoundingClientRect()
    const vw = window.innerWidth
    const vh = window.innerHeight

    // Measure at a neutral origin first so max-width/height take effect.
    popover.style.left = '0px'
    popover.style.top = '0px'
    const pop = popover.getBoundingClientRect()

    // Horizontal: center on the reference, clamped to the viewport.
    let left = rect.left + rect.width / 2 - pop.width / 2
    left = Math.max(GAP_PX, Math.min(left, vw - pop.width - GAP_PX))

    // Vertical: prefer above; flip below when it would overflow the top.
    let top = rect.top - pop.height - GAP_PX
    if (top < GAP_PX) top = rect.bottom + GAP_PX
    top = Math.max(GAP_PX, Math.min(top, vh - pop.height - GAP_PX))

    popover.style.left = `${left}px`
    popover.style.top = `${top}px`
  }

  private _scheduleHide(): void {
    this._cancelHide()
    this._hideTimer = window.setTimeout(() => this._hide(), HIDE_DELAY_MS)
  }

  private _cancelHide(): void {
    if (this._hideTimer !== undefined) {
      clearTimeout(this._hideTimer)
      this._hideTimer = undefined
    }
  }

  private _hide(): void {
    this._cancelHide()
    if (this._popover) this._popover.style.display = 'none'
    if (this._activeAnchor) {
      this._activeAnchor.removeAttribute('aria-describedby')
      this._activeAnchor = null
    }
  }

  override render() {
    return nothing
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-footnote-preview': MbrFootnotePreviewElement
  }
}
