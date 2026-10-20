import { LitElement, css, html } from 'lit'
import { customElement } from 'lit/decorators.js'
import { ICON_BUTTON_CSS } from './controls.ts'
import { icon, iconPanelLeft } from './icons.ts'

/**
 * Trigger button for the sidebar navigation drawer.
 *
 * This component renders the `panel-left` ghost button (the same glyph as
 * `<mbr-browse>`'s, so both sidebar styles look alike) that dispatches a
 * custom event to toggle the sidebar. It's designed to be placed in the nav
 * bar and is hidden on desktop (where the sidebar is always visible inline).
 *
 * Usage:
 * <mbr-sidebar-trigger></mbr-sidebar-trigger>
 *
 * Listens for: (none)
 * Dispatches: 'mbr-sidebar-toggle' on window when clicked
 */
@customElement('mbr-sidebar-trigger')
export class MbrSidebarTriggerElement extends LitElement {
  static override styles = [
    ICON_BUTTON_CSS,
    css`
      :host {
        display: inline-flex;
        align-items: center;
      }

      /* Hide on desktop - sidebar is always visible inline */
      @media (min-width: 1024px) {
        :host {
          display: none;
        }
      }
    `,
  ];

  private _handleClick() {
    window.dispatchEvent(new CustomEvent('mbr-sidebar-toggle'));
  }

  override render() {
    return html`
      <button class="mbr-icon-btn" @click=${this._handleClick} aria-label="Toggle navigation" title="Browse (−)">
        ${icon(iconPanelLeft)}
      </button>
    `;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-sidebar-trigger': MbrSidebarTriggerElement
  }
}
