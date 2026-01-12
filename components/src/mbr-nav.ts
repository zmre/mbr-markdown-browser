import { LitElement, css, html } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { siteNav } from './shared.js'
import {
  type MarkdownFile,
  type SortField,
  DEFAULT_SORT_CONFIG,
  getFileName,
  buildFolderTree,
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

    // Load site navigation data and compute prev/next
    siteNav.then((nav: SiteNav) => {
      if (nav?.markdown_files) {
        // Load sort config if available
        if (nav.sort && Array.isArray(nav.sort) && nav.sort.length > 0) {
          this._sortConfig = nav.sort;
        }
        this._computeNavigation(nav.markdown_files);
      }
    }).catch(() => {
      // Failed to load site.json - buttons remain disabled
    });
  }

  /**
   * Compute prev/next navigation for the current page using global linear order.
   * This creates a "book-like" navigation through the entire site.
   */
  private _computeNavigation(allFiles: MarkdownFile[]) {
    const currentPath = window.location.pathname;
    const normalizedCurrent = currentPath.endsWith('/') ? currentPath : currentPath + '/';

    // Build folder tree and flatten to linear sequence
    const tree = buildFolderTree(allFiles);
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
    return html`
      <nav>
        <ul>
          <li>
            ${this._prevFile ? html`
              <a href="${this._prevFile.url_path}" class="nav-button prev" title="${this._getTitle(this._prevFile)}">
                &lt; Previous
              </a>
            ` : html`
              <button disabled class="nav-button prev">&lt; Previous</button>
            `}
          </li>
        </ul>
        <ul>
          <li>
            ${this._nextFile ? html`
              <a href="${this._nextFile.url_path}" class="nav-button next" title="${this._getTitle(this._nextFile)}">
                Next &gt;
              </a>
            ` : html`
              <button disabled class="nav-button next">Next &gt;</button>
            `}
          </li>
        </ul>
      </nav>
    `;
  }

  static override styles = css`
    :host {
      display: block;
    }

    nav {
      display: flex;
      justify-content: space-between;
    }

    ul {
      list-style: none;
      margin: 0;
      padding: 0;
    }

    li {
      display: inline-block;
    }

    .nav-button {
      display: inline-block;
      padding: 0.5rem 1rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
      text-decoration: none;
      font-size: 0.9rem;
      cursor: pointer;
      transition: all 0.15s ease;
    }

    .nav-button:hover:not([disabled]) {
      border-color: var(--pico-primary, #0d6efd);
      color: var(--pico-primary, #0d6efd);
    }

    .nav-button[disabled] {
      opacity: 0.5;
      cursor: not-allowed;
    }

    a.nav-button {
      cursor: pointer;
    }
  `;
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-nav': MbrNavElement
  }
}
