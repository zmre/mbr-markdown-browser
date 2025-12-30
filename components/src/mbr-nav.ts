import { LitElement, css, html } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { siteNav } from './shared.js'

/**
 * Markdown file metadata from site.json.
 */
interface MarkdownFile {
  url_path: string;
  raw_path: string;
  created: number;
  modified: number;
  frontmatter: Record<string, any> | null;
}

/**
 * Site navigation data structure.
 */
interface SiteNav {
  markdown_files: MarkdownFile[];
  other_files?: any[];
}

/**
 * Previous/Next navigation component.
 *
 * Displays prev/next buttons that navigate to adjacent files in the same folder,
 * sorted alphabetically by URL path.
 *
 * Buttons start disabled and enable once site.json is loaded and siblings are found.
 */
@customElement('mbr-nav')
export class MbrNavElement extends LitElement {
  @state()
  private _prevFile: MarkdownFile | null = null;

  @state()
  private _nextFile: MarkdownFile | null = null;

  override connectedCallback() {
    super.connectedCallback();

    // Load site navigation data and compute prev/next
    siteNav.then((nav: SiteNav) => {
      if (nav?.markdown_files) {
        this._computeSiblings(nav.markdown_files);
      }
    }).catch(() => {
      // Failed to load site.json - buttons remain disabled
    });
  }

  /**
   * Get the parent folder path from a URL path.
   * e.g., "/docs/guide/intro/" -> "/docs/guide/"
   *       "/README/" -> "/"
   */
  private _getParentPath(urlPath: string): string {
    // Remove trailing slash for processing
    const normalized = urlPath.endsWith('/') ? urlPath.slice(0, -1) : urlPath;
    const lastSlash = normalized.lastIndexOf('/');

    if (lastSlash <= 0) {
      return '/';
    }

    return normalized.slice(0, lastSlash + 1);
  }

  /**
   * Get the filename part of a URL path for sorting.
   * e.g., "/docs/guide/intro/" -> "intro"
   */
  private _getFileName(urlPath: string): string {
    const normalized = urlPath.endsWith('/') ? urlPath.slice(0, -1) : urlPath;
    const lastSlash = normalized.lastIndexOf('/');
    return normalized.slice(lastSlash + 1);
  }

  /**
   * Compute prev/next siblings for the current page.
   */
  private _computeSiblings(allFiles: MarkdownFile[]) {
    const currentPath = window.location.pathname;
    const normalizedCurrent = currentPath.endsWith('/') ? currentPath : currentPath + '/';
    const parentPath = this._getParentPath(normalizedCurrent);

    // Find all files in the same parent folder
    const siblings = allFiles.filter(file => {
      const fileParent = this._getParentPath(file.url_path);
      return fileParent === parentPath;
    });

    // Sort alphabetically by filename (case-insensitive)
    siblings.sort((a, b) => {
      const nameA = this._getFileName(a.url_path).toLowerCase();
      const nameB = this._getFileName(b.url_path).toLowerCase();
      return nameA.localeCompare(nameB);
    });

    // Find current file index
    const currentIndex = siblings.findIndex(file => {
      const filePath = file.url_path.endsWith('/') ? file.url_path : file.url_path + '/';
      return filePath === normalizedCurrent;
    });

    if (currentIndex === -1) {
      return;
    }

    // Set prev/next
    if (currentIndex > 0) {
      this._prevFile = siblings[currentIndex - 1];
    }

    if (currentIndex < siblings.length - 1) {
      this._nextFile = siblings[currentIndex + 1];
    }
  }

  /**
   * Get display title for a file.
   */
  private _getTitle(file: MarkdownFile): string {
    if (file.frontmatter?.title) {
      return file.frontmatter.title;
    }
    return this._getFileName(file.url_path);
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
