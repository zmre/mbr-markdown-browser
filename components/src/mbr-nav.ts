import { LitElement, css, html } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { getCanonicalPath, getSiteFolderTree, resolveUrl, siteNav } from './shared.js'
import { scheduleIdleTask } from './dynamic-loader.js'
import { ICON_SM, icon, iconChevronLeft, iconChevronRight } from './icons.ts'
import {
  type FolderNode,
  type MarkdownFile,
  type SortField,
  DEFAULT_SORT_CONFIG,
  getFileName,
  flattenToLinearSequence,
} from './sorting.js'

/**
 * Site navigation data structure.
 */
interface SiteNav {
  markdown_files: MarkdownFile[];
  other_files?: any[];
  sort?: SortField[];
}

/**
 * Upper bound on how long the idle scheduler may defer the prev/next
 * computation; the H/L keyboard shortcuts depend on the rendered links.
 */
const IDLE_TIMEOUT_MS = 500;

/**
 * Previous/Next navigation component.
 *
 * Displays prev/next buttons for linear navigation through the entire site.
 * Navigation follows a depth-first traversal: folder files first (sorted),
 * then child folders (sorted). This creates a "book-like" reading order.
 *
 * Buttons start disabled and enable once site.json is loaded.
 */
@customElement('mbr-nav')
export class MbrNavElement extends LitElement {
  @state()
  private _prevFile: MarkdownFile | null = null;

  @state()
  private _nextFile: MarkdownFile | null = null;

  /** Sort configuration from site.json */
  private _sortConfig: SortField[] = DEFAULT_SORT_CONFIG;

  override connectedCallback() {
    super.connectedCallback();

    // Prev/next lives below the fold, so keep the whole-site sort off the
    // first-paint path. Falls back to setTimeout where requestIdleCallback is
    // unavailable (Safari, happy-dom).
    scheduleIdleTask(() => {
      void this._loadNavigation();
    }, IDLE_TIMEOUT_MS);
  }

  /**
   * Load site navigation data and compute prev/next.
   * Uses the memoized site-wide folder tree from shared.ts rather than
   * rebuilding one per component.
   */
  private async _loadNavigation(): Promise<void> {
    try {
      const nav = (await siteNav) as SiteNav;
      if (!nav?.markdown_files) {
        return;
      }
      // Load sort config if available
      if (nav.sort && Array.isArray(nav.sort) && nav.sort.length > 0) {
        this._sortConfig = nav.sort;
      }
      this._computeNavigation(await getSiteFolderTree());
    } catch {
      // Failed to load site.json - buttons remain disabled
    }
  }

  /**
   * Compute prev/next navigation for the current page using global linear order.
   * This creates a "book-like" navigation through the entire site.
   */
  private _computeNavigation(tree: FolderNode) {
    // site.json stores url_path decoded and without any deployment prefix, so
    // compare against the canonical path, not the raw percent-encoded pathname.
    const currentPath = getCanonicalPath();
    const normalizedCurrent = currentPath.endsWith('/') ? currentPath : currentPath + '/';

    // Flatten the shared folder tree to a linear sequence
    const orderedFiles = flattenToLinearSequence(tree, this._sortConfig);

    // Find current file in global sequence
    const currentIndex = orderedFiles.findIndex(file => {
      const filePath = file.url_path.endsWith('/') ? file.url_path : file.url_path + '/';
      return filePath === normalizedCurrent;
    });

    if (currentIndex === -1) {
      return;
    }

    // Set prev/next from global sequence
    if (currentIndex > 0) {
      this._prevFile = orderedFiles[currentIndex - 1];
    }

    if (currentIndex < orderedFiles.length - 1) {
      this._nextFile = orderedFiles[currentIndex + 1];
    }
  }

  /**
   * Get display title for a file.
   */
  private _getTitle(file: MarkdownFile): string {
    if (file.frontmatter?.title) {
      return file.frontmatter.title;
    }
    return getFileName(file.url_path);
  }

  override render() {
    // Two cards: the direction with its chevron, then the destination title.
    // With no page in a direction the slot keeps a disabled (and invisible)
    // button, so the layout and the "not yet loaded" state are unchanged.
    const card = (file: MarkdownFile | null, dir: 'prev' | 'next') => {
      const label = dir === 'prev'
        ? html`${icon(iconChevronLeft, { size: ICON_SM })}Previous`
        : html`Next${icon(iconChevronRight, { size: ICON_SM })}`;
      return file
        ? html`<a href="${resolveUrl(file.url_path)}" class="nav-button ${dir}">
            <span class="nav-label">${label}</span>
            <span class="nav-title">${this._getTitle(file)}</span>
          </a>`
        : html`<button disabled class="nav-button ${dir}" aria-hidden="true" tabindex="-1">
            <span class="nav-label">${label}</span>
          </button>`;
    };
    return html`
      <nav aria-label="Previous and next page">
        ${card(this._prevFile, 'prev')}
        ${card(this._nextFile, 'next')}
      </nav>
    `;
  }

  static override styles = css`
    :host {
      display: block;
    }

    nav {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 0.75rem;
    }

    .nav-button {
      display: flex;
      flex-direction: column;
      gap: 0.125rem;
      min-width: 0;
      margin: 0;
      padding: 0.75rem 1rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: var(--mbr-control-radius, 8px);
      background: transparent;
      color: var(--pico-color, #333);
      text-decoration: none;
      font: inherit;
      transition: border-color 120ms ease, background-color 120ms ease;
    }

    .nav-button.next {
      align-items: flex-end;
      text-align: right;
    }

    .nav-label {
      display: inline-flex;
      align-items: center;
      gap: 0.25rem;
      font-size: 0.8125rem;
      color: var(--pico-muted-color, #666);
    }

    .nav-title {
      max-width: 100%;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-weight: 600;
      color: var(--pico-primary, #0d6efd);
    }

    a.nav-button:hover {
      border-color: var(--pico-primary, #0d6efd);
      background: var(--mbr-control-hover-bg, rgba(127, 127, 127, 0.06));
    }

    a.nav-button:focus-visible {
      outline: none;
      box-shadow: var(--mbr-focus-ring, 0 0 0 2px var(--pico-primary, #0d6efd));
    }

    .nav-button[disabled] {
      visibility: hidden;
    }
  `;
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-nav': MbrNavElement
  }
}
