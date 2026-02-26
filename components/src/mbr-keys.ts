import { LitElement, css, html, nothing } from 'lit'
import { customElement, state } from 'lit/decorators.js'

/**
 * Scroll amount constants.
 */
const SCROLL_LINE = 40;  // j/k scroll amount (roughly one line)
const SCROLL_HALF_PAGE = () => window.innerHeight / 2;
const SCROLL_FULL_PAGE = () => window.innerHeight - 100; // Leave some context

/**
 * Font size adjustment constants (percentage-based, matching Pico CSS breakpoint increments).
 */
const FONT_SIZE_STEP = 6.25;   // % step (matches Pico breakpoint increments)
const FONT_SIZE_MIN = 62.5;    // minimum %
const FONT_SIZE_MAX = 250;     // maximum %

/**
 * Check if the event target is an input element where we shouldn't intercept keys.
 * Uses composedPath to correctly identify inputs inside shadow DOMs.
 */
function isInputTarget(e: KeyboardEvent): boolean {
  // Use composedPath to get the actual target, even inside shadow DOMs
  const target = e.composedPath()[0];
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

  // Check for mbr-browse-single drawer (in overlay mode)
  const browseSingle = document.querySelector('mbr-browse-single');
  if (browseSingle && (browseSingle as any)._isDrawerOpen) return true;

  // Check for mbr-fuzzy-nav modal
  const fuzzyNav = document.querySelector('mbr-fuzzy-nav');
  if (fuzzyNav && (fuzzyNav as any)._isOpen) return true;

  // Check for info panel
  const infoPanel = document.getElementById('info-panel-toggle') as HTMLInputElement | null;
  if (infoPanel?.checked) return true;

  return false;
}

/**
 * Toggle the info panel open/closed.
 */
function toggleInfoPanel(): void {
  const checkbox = document.getElementById('info-panel-toggle') as HTMLInputElement | null;
  if (checkbox) {
    checkbox.checked = !checkbox.checked;
  }
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
 * Get the current --pico-font-size as a percentage number, defaulting to 100.
 */
function getCurrentFontSizePercent(): number {
  const raw = getComputedStyle(document.documentElement)
    .getPropertyValue('--pico-font-size')
    .trim();
  if (raw.endsWith('%')) {
    const parsed = parseFloat(raw);
    if (!isNaN(parsed)) return parsed;
  }
  return 100;
}

/**
 * Set --pico-font-size on :root, clamped to [FONT_SIZE_MIN, FONT_SIZE_MAX].
 */
function setFontSizePercent(percent: number): void {
  const clamped = Math.min(FONT_SIZE_MAX, Math.max(FONT_SIZE_MIN, percent));
  document.documentElement.style.setProperty('--pico-font-size', `${clamped}%`);
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
      { keys: 'H / L', description: 'Previous / next sibling' },
      { keys: 'Ctrl+o / Ctrl+i', description: 'History back / forward' },
    ],
  },
  {
    title: 'Panels',
    shortcuts: [
      { keys: '/', description: 'Open search' },
      { keys: '=', description: 'Open media browser' },
      { keys: '- or F2', description: 'Open file browser' },
      { keys: 'Ctrl+g', description: 'Toggle info panel' },
      { keys: 'Esc', description: 'Close panel' },
    ],
  },
  {
    title: 'Quick Navigation',
    shortcuts: [
      { keys: 'f', description: 'Open links out' },
      { keys: 'F', description: 'Open links in (backlinks)' },
      { keys: 'T', description: 'Open table of contents' },
    ],
  },
  {
    title: 'Search (when open)',
    shortcuts: [
      { keys: 'Ctrl+n / Ctrl+p', description: 'Navigate results' },
      { keys: '↑ / ↓', description: 'Navigate results' },
      { keys: 'Enter', description: 'Open selected result' },
      { keys: 'Ctrl+d / Ctrl+u', description: 'Scroll results' },
    ],
  },
  {
    title: 'File Browser (when open)',
    shortcuts: [
      { keys: 'j / k / ↑ / ↓', description: 'Navigate tree' },
      { keys: 'Ctrl+n / Ctrl+p', description: 'Navigate tree' },
      { keys: 'h', description: 'Collapse / go to parent' },
      { keys: 'l or Enter', description: 'Expand / open' },
      { keys: 'o', description: 'Open in new tab' },
      { keys: 'Ctrl+d / Ctrl+u', description: 'Scroll panel' },
    ],
  },
  {
    title: 'Display',
    shortcuts: [
      { keys: 'Ctrl++ / Cmd++', description: 'Increase font size' },
      { keys: 'Ctrl+- / Cmd+-', description: 'Decrease font size' },
      { keys: 'Ctrl+0 / Cmd+0', description: 'Reset font size' },
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

    // Close info panel with Escape
    if (e.key === 'Escape') {
      const infoPanel = document.getElementById('info-panel-toggle') as HTMLInputElement | null;
      if (infoPanel?.checked) {
        e.preventDefault();
        infoPanel.checked = false;
        return;
      }
    }

    // Don't process other keys when help is open
    if (this._helpOpen) {
      return;
    }

    // Don't intercept when typing in inputs (unless it's a ctrl/cmd combo)
    if (isInputTarget(e) && !e.ctrlKey && !e.metaKey) {
      return;
    }

    // Handle Ctrl+o/i for history navigation (works regardless of modal state)
    // Handle Ctrl+g for info panel toggle
    if (e.ctrlKey && !e.metaKey) {
      switch (e.key.toLowerCase()) {
        case 'o': // Ctrl+o - history back (vim jump list style)
          e.preventDefault();
          history.back();
          return;

        case 'i': // Ctrl+i - history forward (vim jump list style)
          e.preventDefault();
          history.forward();
          return;

        case 'g': // Ctrl+g - toggle info panel (vim file info style)
          e.preventDefault();
          toggleInfoPanel();
          return;
      }
    }

    // Handle Ctrl/Cmd +/-/0 for font size adjustment (works regardless of modal state)
    if (e.ctrlKey || e.metaKey) {
      if (e.key === '=' || e.key === '+') {
        e.preventDefault();
        setFontSizePercent(getCurrentFontSizePercent() + FONT_SIZE_STEP);
        return;
      }
      if (e.key === '-') {
        e.preventDefault();
        setFontSizePercent(getCurrentFontSizePercent() - FONT_SIZE_STEP);
        return;
      }
      if (e.key === '0') {
        e.preventDefault();
        document.documentElement.style.removeProperty('--pico-font-size');
        return;
      }
    }

    // Handle ctrl/cmd key combinations for page scrolling (only when no modal is open)
    // Modals handle their own Ctrl+d/u/f/b scrolling
    if ((e.ctrlKey || e.metaKey) && !isModalOpen()) {
      switch (e.key.toLowerCase()) {
        case 'd': // Ctrl+d - half page down
          if (!e.metaKey) { // Don't override Cmd+D (bookmark)
            e.preventDefault();
            scrollBy(window, SCROLL_HALF_PAGE());
          }
          return;

        case 'u': // Ctrl+u - half page up
          if (!e.metaKey) {
            e.preventDefault();
            scrollBy(window, -SCROLL_HALF_PAGE());
          }
          return;

        case 'f': // Ctrl+f - full page down (but not Cmd+F which is find)
          if (!e.metaKey) {
            e.preventDefault();
            scrollBy(window, SCROLL_FULL_PAGE());
          }
          return;

        case 'b': // Ctrl+b - full page up
          if (!e.metaKey) {
            e.preventDefault();
            scrollBy(window, -SCROLL_FULL_PAGE());
          }
          return;
      }
      return;
    }

    // Don't intercept plain keys when in input
    if (isInputTarget(e)) {
      return;
    }

    // Handle gg (go to top) - track double key press (only when no modal is open)
    const now = Date.now();
    if (e.key === 'g' && !e.shiftKey && !isModalOpen()) {
      if (this._lastKey === 'g' && now - this._lastKeyTime < 500) {
        e.preventDefault();
        window.scrollTo({ top: 0, behavior: 'smooth' });
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

      case '=': // Open media browser
        if (!isModalOpen()) {
          e.preventDefault();
          const searchForMedia = document.querySelector('mbr-search');
          if (searchForMedia && typeof (searchForMedia as any)._openMediaBrowser === 'function') {
            (searchForMedia as any)._openMediaBrowser();
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

      case 'j': // Scroll down (main page only, modals handle their own j/k)
        if (!isModalOpen()) {
          e.preventDefault();
          scrollBy(window, SCROLL_LINE);
        }
        break;

      case 'k': // Scroll up (main page only)
        if (!isModalOpen()) {
          e.preventDefault();
          scrollBy(window, -SCROLL_LINE);
        }
        break;

      case 'G': // Go to bottom (shift+g, main page only)
        if (e.shiftKey && !isModalOpen()) {
          e.preventDefault();
          window.scrollTo({ top: document.body.scrollHeight, behavior: 'smooth' });
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

      case 'f': // Open fuzzy nav - links out (lowercase f)
        if (!e.shiftKey && !isModalOpen()) {
          e.preventDefault();
          const fuzzyNavOut = document.querySelector('mbr-fuzzy-nav');
          if (fuzzyNavOut && typeof (fuzzyNavOut as any).open === 'function') {
            (fuzzyNavOut as any).open('links-out');
          }
        }
        break;

      case 'F': // Open fuzzy nav - links in (Shift+f)
        if (e.shiftKey && !isModalOpen()) {
          e.preventDefault();
          const fuzzyNavIn = document.querySelector('mbr-fuzzy-nav');
          if (fuzzyNavIn && typeof (fuzzyNavIn as any).open === 'function') {
            (fuzzyNavIn as any).open('links-in');
          }
        }
        break;

      case 'T': // Open fuzzy nav - table of contents (Shift+t)
        if (e.shiftKey && !isModalOpen()) {
          e.preventDefault();
          const fuzzyNavToc = document.querySelector('mbr-fuzzy-nav');
          if (fuzzyNavToc && typeof (fuzzyNavToc as any).open === 'function') {
            (fuzzyNavToc as any).open('toc');
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
