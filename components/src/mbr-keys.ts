import { LitElement, css, html, nothing } from 'lit'
import { customElement, state } from 'lit/decorators.js'

/**
 * Scroll amount constants.
 */
const SCROLL_LINE = 40;  // j/k scroll amount (roughly one line)
const SCROLL_HALF_PAGE = () => window.innerHeight / 2;
const SCROLL_FULL_PAGE = () => window.innerHeight - 100; // Leave some context

/**
 * Check if the event target is an input element where we shouldn't intercept keys.
 */
function isInputTarget(target: EventTarget | null): boolean {
  if (!target || !(target instanceof HTMLElement)) return false;
  const tagName = target.tagName.toLowerCase();
  return tagName === 'input' || tagName === 'textarea' || target.isContentEditable;
}

/**
 * Check if any modal/popup is currently open (excluding help overlay).
 */
function isModalOpen(): boolean {
  // Check for mbr-search modal
  const search = document.querySelector('mbr-search');
  if (search && (search as any)._isOpen) return true;

  // Check for mbr-browse panel
  const browse = document.querySelector('mbr-browse');
  if (browse && (browse as any)._isOpen) return true;

  return false;
}

/**
 * Get the scrollable element - either a modal/panel content or the document.
 */
function getScrollTarget(): Element | Window {
  // If search modal is open, scroll its results container
  const search = document.querySelector('mbr-search');
  if (search && (search as any)._isOpen) {
    const resultsContainer = search.shadowRoot?.querySelector('.results-container');
    if (resultsContainer) return resultsContainer;
  }

  // If browse panel is open, scroll its panel content
  const browse = document.querySelector('mbr-browse');
  if (browse && (browse as any)._isOpen) {
    const panelContent = browse.shadowRoot?.querySelector('.panel-content');
    if (panelContent) return panelContent;
  }

  // Default to window/document scrolling
  return window;
}

/**
 * Scroll a target element or window by a given amount.
 */
function scrollBy(target: Element | Window, amount: number) {
  if (target instanceof Window) {
    target.scrollBy({ top: amount, behavior: 'smooth' });
  } else {
    target.scrollBy({ top: amount, behavior: 'smooth' });
  }
}

/**
 * Keyboard shortcut definition.
 */
interface Shortcut {
  keys: string;
  description: string;
}

/**
 * Shortcut category.
 */
interface ShortcutCategory {
  title: string;
  shortcuts: Shortcut[];
}

/**
 * All keyboard shortcuts organized by category.
 */
const SHORTCUTS: ShortcutCategory[] = [
  {
    title: 'Navigation',
    shortcuts: [
      { keys: 'j / k', description: 'Scroll down / up' },
      { keys: 'Ctrl+d / Ctrl+u', description: 'Half page down / up' },
      { keys: 'Ctrl+f / Ctrl+b', description: 'Full page down / up' },
      { keys: 'g g', description: 'Go to top' },
      { keys: 'G', description: 'Go to bottom' },
      { keys: 'H / L', description: 'Previous / next page' },
    ],
  },
  {
    title: 'Panels',
    shortcuts: [
      { keys: '/', description: 'Open search' },
      { keys: '- or F2', description: 'Open file browser' },
      { keys: 'Esc', description: 'Close panel' },
    ],
  },
  {
    title: 'Search (when open)',
    shortcuts: [
      { keys: 'j / k', description: 'Navigate results' },
      { keys: 'Enter', description: 'Open selected result' },
      { keys: 'Ctrl+d / Ctrl+u', description: 'Scroll results' },
    ],
  },
  {
    title: 'File Browser (when open)',
    shortcuts: [
      { keys: 'j / k', description: 'Navigate tree' },
      { keys: 'h', description: 'Collapse / go to parent' },
      { keys: 'l or Enter', description: 'Expand / open' },
      { keys: 'o', description: 'Open in new tab' },
      { keys: 'Ctrl+d / Ctrl+u', description: 'Scroll panel' },
    ],
  },
  {
    title: 'Help',
    shortcuts: [
      { keys: '?', description: 'Toggle this help' },
    ],
  },
];

/**
 * Global keyboard shortcuts component for vim-like navigation.
 *
 * Provides:
 * - `/` to open search
 * - `-` or F2 to open browse panel
 * - `j/k` for line scrolling (page or active modal)
 * - `Ctrl+d/u` for half-page scrolling
 * - `Ctrl+f/b` for full-page scrolling
 * - `H/L` for prev/next page navigation (when mbr-nav is present)
 * - `g g` for scroll to top, `G` for scroll to bottom
 * - `?` for help overlay
 */
@customElement('mbr-keys')
export class MbrKeysElement extends LitElement {
  @state()
  private _helpOpen = false;

  private _lastKeyTime = 0;
  private _lastKey = '';

  override connectedCallback() {
    super.connectedCallback();
    document.addEventListener('keydown', this._handleKeydown);
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener('keydown', this._handleKeydown);
  }

  private _closeHelp() {
    this._helpOpen = false;
  }

  private _handleKeydown = (e: KeyboardEvent) => {
    // Handle ? for help (works even in inputs, with shift)
    if (e.key === '?' && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      this._helpOpen = !this._helpOpen;
      return;
    }

    // Close help with Escape
    if (e.key === 'Escape' && this._helpOpen) {
      e.preventDefault();
      this._helpOpen = false;
      return;
    }

    // Don't process other keys when help is open
    if (this._helpOpen) {
      return;
    }

    // Don't intercept when typing in inputs (unless it's a ctrl/cmd combo)
    if (isInputTarget(e.target) && !e.ctrlKey && !e.metaKey) {
      return;
    }

    // Handle ctrl/cmd key combinations first
    if (e.ctrlKey || e.metaKey) {
      switch (e.key.toLowerCase()) {
        case 'd': // Ctrl+d - half page down
          if (!e.metaKey) { // Don't override Cmd+D (bookmark)
            e.preventDefault();
            scrollBy(getScrollTarget(), SCROLL_HALF_PAGE());
          }
          return;

        case 'u': // Ctrl+u - half page up
          e.preventDefault();
          scrollBy(getScrollTarget(), -SCROLL_HALF_PAGE());
          return;

        case 'f': // Ctrl+f - full page down (but not Cmd+F which is find)
          if (!e.metaKey && !isModalOpen()) {
            e.preventDefault();
            scrollBy(getScrollTarget(), SCROLL_FULL_PAGE());
          }
          return;

        case 'b': // Ctrl+b - full page up
          if (!e.metaKey) {
            e.preventDefault();
            scrollBy(getScrollTarget(), -SCROLL_FULL_PAGE());
          }
          return;
      }
      return;
    }

    // Don't intercept plain keys when in input
    if (isInputTarget(e.target)) {
      return;
    }

    // Handle gg (go to top) - track double key press
    const now = Date.now();
    if (e.key === 'g' && !e.shiftKey) {
      if (this._lastKey === 'g' && now - this._lastKeyTime < 500) {
        e.preventDefault();
        const target = getScrollTarget();
        if (target instanceof Window) {
          target.scrollTo({ top: 0, behavior: 'smooth' });
        } else {
          target.scrollTo({ top: 0, behavior: 'smooth' });
        }
        this._lastKey = '';
        return;
      }
      this._lastKey = 'g';
      this._lastKeyTime = now;
      return;
    }
    this._lastKey = e.key;
    this._lastKeyTime = now;

    switch (e.key) {
      case '/': // Open search
        if (!isModalOpen()) {
          e.preventDefault();
          const search = document.querySelector('mbr-search');
          if (search && typeof (search as any)._openSearch === 'function') {
            (search as any)._openSearch();
          }
        }
        break;

      case '-': // Open browse (already handled in mbr-browse, but ensure it works)
        // mbr-browse handles this itself
        break;

      case 'F2': // Open browse
        e.preventDefault();
        const browse = document.querySelector('mbr-browse');
        if (browse && typeof (browse as any).open === 'function') {
          (browse as any).open();
        }
        break;

      case 'j': // Scroll down / next item
        if (!isModalOpen()) {
          e.preventDefault();
          scrollBy(getScrollTarget(), SCROLL_LINE);
        }
        // Modal-specific j/k is handled by the respective components
        break;

      case 'k': // Scroll up / prev item
        if (!isModalOpen()) {
          e.preventDefault();
          scrollBy(getScrollTarget(), -SCROLL_LINE);
        }
        break;

      case 'G': // Go to bottom (shift+g)
        if (e.shiftKey && !isModalOpen()) {
          e.preventDefault();
          const target = getScrollTarget();
          if (target instanceof Window) {
            target.scrollTo({ top: document.body.scrollHeight, behavior: 'smooth' });
          } else {
            target.scrollTo({ top: target.scrollHeight, behavior: 'smooth' });
          }
        }
        break;

      case 'H': // Previous page (shift+h)
        if (e.shiftKey) {
          e.preventDefault();
          const nav = document.querySelector('mbr-nav');
          if (nav) {
            const prevLink = nav.shadowRoot?.querySelector('a.nav-button.prev') as HTMLAnchorElement;
            if (prevLink) {
              prevLink.click();
            }
          }
        }
        break;

      case 'L': // Next page (shift+l)
        if (e.shiftKey) {
          e.preventDefault();
          const nav = document.querySelector('mbr-nav');
          if (nav) {
            const nextLink = nav.shadowRoot?.querySelector('a.nav-button.next') as HTMLAnchorElement;
            if (nextLink) {
              nextLink.click();
            }
          }
        }
        break;
    }
  };

  override render() {
    if (!this._helpOpen) return nothing;

    return html`
      <div class="help-backdrop" @click=${this._closeHelp}>
        <div class="help-modal" @click=${(e: Event) => e.stopPropagation()}>
          <div class="help-header">
            <h2>Keyboard Shortcuts</h2>
            <button class="close-button" @click=${this._closeHelp} aria-label="Close">
              <kbd>?</kbd> or <kbd>Esc</kbd>
            </button>
          </div>
          <div class="help-content">
            ${SHORTCUTS.map(category => html`
              <div class="shortcut-category">
                <h3>${category.title}</h3>
                <dl class="shortcut-list">
                  ${category.shortcuts.map(shortcut => html`
                    <div class="shortcut-item">
                      <dt><kbd>${shortcut.keys}</kbd></dt>
                      <dd>${shortcut.description}</dd>
                    </div>
                  `)}
                </dl>
              </div>
            `)}
          </div>
        </div>
      </div>
    `;
  }

  static override styles = css`
    :host {
      display: contents;
    }

    .help-backdrop {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.6);
      display: flex;
      align-items: center;
      justify-content: center;
      z-index: 10000;
      animation: fadeIn 0.15s ease;
    }

    @keyframes fadeIn {
      from { opacity: 0; }
      to { opacity: 1; }
    }

    .help-modal {
      background: var(--pico-background-color, #fff);
      border-radius: 12px;
      box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.25);
      max-width: 700px;
      max-height: 80vh;
      width: 90vw;
      display: flex;
      flex-direction: column;
      overflow: hidden;
      animation: slideUp 0.2s ease;
    }

    @keyframes slideUp {
      from {
        opacity: 0;
        transform: translateY(20px);
      }
      to {
        opacity: 1;
        transform: translateY(0);
      }
    }

    .help-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 1rem 1.5rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    .help-header h2 {
      margin: 0;
      font-size: 1.25rem;
      color: var(--pico-color, #333);
    }

    .close-button {
      background: transparent;
      border: none;
      color: var(--pico-muted-color, #666);
      cursor: pointer;
      font-size: 0.85rem;
      padding: 0.25rem 0.5rem;
      display: flex;
      align-items: center;
      gap: 0.25rem;
    }

    .close-button:hover {
      color: var(--pico-color, #333);
    }

    .help-content {
      padding: 1rem 1.5rem;
      overflow-y: auto;
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
      gap: 1.5rem;
    }

    .shortcut-category h3 {
      margin: 0 0 0.75rem 0;
      font-size: 0.8rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      color: var(--pico-primary, #0d6efd);
    }

    .shortcut-list {
      margin: 0;
      padding: 0;
    }

    .shortcut-item {
      display: flex;
      align-items: baseline;
      gap: 0.75rem;
      padding: 0.35rem 0;
    }

    .shortcut-item dt {
      flex-shrink: 0;
      min-width: 100px;
    }

    .shortcut-item dd {
      margin: 0;
      color: var(--pico-muted-color, #666);
      font-size: 0.9rem;
    }

    kbd {
      display: inline-block;
      padding: 0.15rem 0.4rem;
      font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
      font-size: 0.8rem;
      color: var(--pico-color, #333);
      background: var(--pico-secondary-background, #f5f5f5);
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      box-shadow: 0 1px 0 var(--pico-muted-border-color, #ccc);
    }

    /* Dark mode adjustments */
    @media (prefers-color-scheme: dark) {
      kbd {
        box-shadow: 0 1px 0 rgba(255, 255, 255, 0.1);
      }
    }
  `;
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-keys': MbrKeysElement
  }
}
