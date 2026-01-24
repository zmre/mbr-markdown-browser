import { LitElement, css, html } from 'lit'
import { customElement } from 'lit/decorators.js'

/**
 * Trigger button for the sidebar navigation drawer.
 *
 * This component renders a hamburger button that dispatches a custom event
 * to toggle the sidebar. It's designed to be placed in the nav bar and is
 * hidden on desktop (where the sidebar is always visible inline).
 *
 * Usage:
 * <mbr-sidebar-trigger></mbr-sidebar-trigger>
 *
 * Listens for: (none)
 * Dispatches: 'mbr-sidebar-toggle' on window when clicked
 */
@customElement('mbr-sidebar-trigger')
export class MbrSidebarTriggerElement extends LitElement {
  static override styles = css`
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

    button {
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 0.5rem;
      background: transparent;
      border: none;
      cursor: pointer;
      border-radius: 4px;
      transition: background 0.15s ease;
      margin-left: 1px;
      margin-right: 1px;
    }

    button:hover {
      border: 1px solid var(--pico-contrast-hover-border, rgba(0, 0, 0, 0.05));
      margin-left: 0px;
      margin-right: 0px;
    }

    .hamburger {
      display: flex;
      flex-direction: column;
      justify-content: space-between;
      width: 18px;
      height: 14px;
    }

    .hamburger span {
      display: block;
      height: 2px;
      background: var(--pico-color, currentColor);
      border-radius: 1px;
      transition: all 0.2s ease;
    }
  `;

  private _handleClick() {
    window.dispatchEvent(new CustomEvent('mbr-sidebar-toggle'));
  }

  override render() {
    return html`
      <button @click=${this._handleClick} aria-label="Toggle navigation" title="Browse (-)">
        <div class="hamburger">
          <span></span>
          <span></span>
          <span></span>
        </div>
      </button>
    `;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-sidebar-trigger': MbrSidebarTriggerElement
  }
}
