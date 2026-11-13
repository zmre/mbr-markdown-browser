/**
 * `<mbr-selection-actions>`: the small toolbar that follows a selection in the
 * note — **Copy markdown** (or **Copy text** where the page cannot map the
 * selection to its source, see `copy-content.ts`), **Copy rich**, and whatever
 * other actions are registered (review's **Add note**).
 *
 * Main bundle, emitted by `_footer.html` in every mode (server, GUI, static
 * build), because copying needs no server. It used to be review's own
 * "Add note" button; review now registers that button here
 * ({@link registerSelectionAction}), so there is one toolbar, one selection
 * tracker and one position, whether review is on or off.
 *
 * Also installs `window.mbrCopyPage` ({@link installCopyPageGlobal}), the
 * function the GUI's Edit menu calls.
 */

import { LitElement, css, html, nothing, type TemplateResult } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import {
  copyOutcomeMessage,
  copySelection,
  installCopyPageGlobal,
  prefetchMarkdownSource,
  selectionCopyKind,
  type CopyFormat,
} from './copy-content.ts'
import { isModalOpen } from './mbr-keys.js'
import { showToast } from './toast.ts'

/** An extra button in the selection toolbar. */
export interface SelectionAction {
  /** Button text. */
  label: string
  /** Tooltip, e.g. naming the shortcut. */
  title: string
  /**
   * Whether to offer the action for the current selection. Called on every
   * measured selection change, after the toolbar has checked the selection is
   * a non-empty one inside the note — so it should be cheap.
   */
  isAvailable: () => boolean
  /** Do it. The toolbar hides itself afterwards. */
  run: () => void
}

const actions = new Set<SelectionAction>()
const listeners = new Set<() => void>()

/**
 * Add `action` to the toolbar, before the copy buttons. Returns the
 * unregister function.
 */
export function registerSelectionAction(action: SelectionAction): () => void {
  actions.add(action)
  listeners.forEach((fn) => fn())
  return () => {
    actions.delete(action)
    listeners.forEach((fn) => fn())
  }
}

if (typeof window !== 'undefined') {
  installCopyPageGlobal((outcome) => showToast(copyOutcomeMessage(outcome, 'page')))
}

/** Approximate height of the toolbar, for placing it above the selection. */
const TOOLBAR_OFFSET = 40

@customElement('mbr-selection-actions')
export class MbrSelectionActions extends LitElement {
  /** Viewport rect of the live selection, when there is a copyable one. */
  @state() private _rect: DOMRect | null = null
  /** Registered actions available for the current selection. */
  @state() private _available: SelectionAction[] = []
  /** What the first copy button copies for the current selection. */
  @state() private _copyKind: 'markdown' | 'text' = 'text'

  private _frame = 0

  override connectedCallback(): void {
    super.connectedCallback()
    document.addEventListener('selectionchange', this._onSelectionChange)
    window.addEventListener('scroll', this._onViewportChange, { passive: true })
    window.addEventListener('resize', this._onViewportChange)
    listeners.add(this._onActionsChanged)
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    document.removeEventListener('selectionchange', this._onSelectionChange)
    window.removeEventListener('scroll', this._onViewportChange)
    window.removeEventListener('resize', this._onViewportChange)
    listeners.delete(this._onActionsChanged)
    if (this._frame) cancelAnimationFrame(this._frame)
  }

  /**
   * Coalesced through `requestAnimationFrame`: `selectionchange` fires for
   * every pointer move of a drag-select.
   */
  private _onSelectionChange = (): void => {
    if (this._frame) cancelAnimationFrame(this._frame)
    this._frame = requestAnimationFrame(() => {
      this._frame = 0
      this.measure()
    })
  }

  private _onViewportChange = (): void => {
    if (this._rect !== null) this.measure()
  }

  private _onActionsChanged = (): void => this.measure()

  /** Re-read the selection. Public for tests and for a host that changed state. */
  public measure(): void {
    const selection = window.getSelection()
    const kind = isModalOpen() ? null : selectionCopyKind(selection)
    const rect = kind === null ? null : (selection?.getRangeAt(0).getBoundingClientRect() ?? null)
    // happy-dom, and a detached range, report all zeros.
    this._rect = rect === null || (rect.width === 0 && rect.height === 0) ? null : rect
    this._available = this._rect === null ? [] : [...actions].filter((a) => a.isAvailable())
    this._copyKind = kind ?? 'text'
    // Warm the source cache now, so the click can write synchronously.
    if (this._rect !== null && kind === 'markdown') prefetchMarkdownSource()
  }

  /** Hide until the selection changes again. */
  private _hide(): void {
    this._rect = null
    this._available = []
  }

  private async _copy(format: CopyFormat): Promise<void> {
    showToast(copyOutcomeMessage(await copySelection(format), 'selection'))
  }

  override updated(): void {
    // Keep the whole toolbar on screen; its width depends on its buttons.
    const bar = this.renderRoot.querySelector<HTMLElement>('.selection-toolbar')
    if (!bar) return
    const max = window.innerWidth - bar.offsetWidth - 8
    if (bar.offsetLeft > max && max > 8) bar.style.left = `${max}px`
  }

  override render(): TemplateResult | typeof nothing {
    const rect = this._rect
    if (rect === null) return nothing
    // Above the selection when there is room, otherwise below it.
    const top = rect.top > TOOLBAR_OFFSET + 4 ? rect.top - TOOLBAR_OFFSET : rect.bottom + 8
    const left = Math.max(8, rect.left)
    // `mousedown` default would collapse the selection before `click`.
    const keep = (e: Event): void => e.preventDefault()
    return html`
      <div class="selection-toolbar" role="toolbar" aria-label="Selection" style="top:${top}px;left:${left}px">
        ${this._available.map(
          (action) => html`
            <button
              @mousedown=${keep}
              @click=${() => {
                this._hide()
                action.run()
              }}
              title=${action.title}
            >
              ${action.label}
            </button>
          `
        )}
        <button
          class="copy-markdown"
          data-kind=${this._copyKind}
          @mousedown=${keep}
          @click=${() => void this._copy('markdown')}
          title=${this._copyKind === 'markdown' ? 'Copy the markdown source of the selection' : 'Copy the selection as plain text'}
        >
          ${this._copyKind === 'markdown' ? 'Copy markdown' : 'Copy text'}
        </button>
        <button class="copy-rich" @mousedown=${keep} @click=${() => void this._copy('rich')} title="Copy the selection with its formatting">
          Copy rich
        </button>
      </div>
    `
  }

  static override styles = css`
    :host {
      display: contents;
    }

    .selection-toolbar {
      position: fixed;
      z-index: 850;
      display: flex;
      gap: 1px;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 6px;
      overflow: hidden;
      background: var(--pico-muted-border-color, #ccc);
      box-shadow: 0 6px 18px -8px rgba(0, 0, 0, 0.45);
    }

    button {
      margin: 0;
      padding: 0.3rem 0.7rem;
      font-size: 0.85rem;
      line-height: 1.2;
      border: none;
      border-radius: 0;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
      cursor: pointer;
      white-space: nowrap;
    }

    button:hover,
    button:focus-visible {
      background: var(--mbr-control-hover-bg, rgba(127, 127, 127, 0.15));
    }

    @media print {
      .selection-toolbar {
        display: none !important;
      }
    }
  `
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-selection-actions': MbrSelectionActions
  }
}
