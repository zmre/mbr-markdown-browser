import { LitElement, css, html, nothing, type TemplateResult } from 'lit'
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
 * Folder node in the hierarchical tree structure.
 */
interface FolderNode {
  name: string;
  path: string;
  children: Map<string, FolderNode>;
  files: MarkdownFile[];
}

/**
 * Tag information with count and source field.
 */
interface TagInfo {
  name: string;
  count: number;
  field: string;
}

/**
 * Browse panel component for MBR.
 *
 * Displays a slide-in panel with:
 * - Tag filtering (multiple tags = AND logic)
 * - Hierarchical folder tree with expand/collapse
 * - Current page highlighting
 * - Keyboard navigation (- to open, Escape to close, arrows to navigate)
 */
@customElement('mbr-browse')
export class MbrBrowseElement extends LitElement {
  @state()
  private _isOpen = false;

  @state()
  private _allFiles: MarkdownFile[] = [];

  @state()
  private _allTags: TagInfo[] = [];

  @state()
  private _selectedTags: Set<string> = new Set();

  @state()
  private _expandedFolders: Set<string> = new Set();

  @state()
  private _tagsExpanded = false;

  @state()
  private _selectedPath: string | null = null;

  private _keyboardHandler: ((e: KeyboardEvent) => void) | null = null;

  override connectedCallback() {
    super.connectedCallback();

    // Load site navigation data
    siteNav.then((nav: SiteNav) => {
      if (nav?.markdown_files) {
        this._allFiles = nav.markdown_files;
        this._allTags = this._extractTags(nav.markdown_files);

        // Auto-expand folders in the current path
        const currentPath = window.location.pathname;
        this._autoExpandCurrentPath(currentPath);
      }
    });

    // Setup keyboard event listener
    this._keyboardHandler = (e: KeyboardEvent) => {
      // Open with '-' key (when not in input field)
      if (e.key === '-' && !this._isInputTarget(e.target)) {
        e.preventDefault();
        this.open();
      }

      // Open with F2 key
      if (e.key === 'F2') {
        e.preventDefault();
        this.open();
      }

      // Close with Escape key
      if (e.key === 'Escape' && this._isOpen) {
        e.preventDefault();
        this.close();
      }

      // Handle keyboard navigation when open
      if (this._isOpen) {
        this._handlePanelKeydown(e);
      }
    };

    document.addEventListener('keydown', this._keyboardHandler);
  }

  /**
   * Handle keyboard events when the panel is open.
   */
  private _handlePanelKeydown(e: KeyboardEvent) {
    // Handle Ctrl key combinations for scrolling
    if (e.ctrlKey) {
      const panelContent = this.shadowRoot?.querySelector('.panel-content');
      if (!panelContent) return;

      const halfPage = panelContent.clientHeight / 2;
      const fullPage = panelContent.clientHeight - 50;

      switch (e.key.toLowerCase()) {
        case 'd': // Ctrl+d - half page down
          e.preventDefault();
          panelContent.scrollBy({ top: halfPage, behavior: 'smooth' });
          return;
        case 'u': // Ctrl+u - half page up
          e.preventDefault();
          panelContent.scrollBy({ top: -halfPage, behavior: 'smooth' });
          return;
        case 'f': // Ctrl+f - full page down
          e.preventDefault();
          panelContent.scrollBy({ top: fullPage, behavior: 'smooth' });
          return;
        case 'b': // Ctrl+b - full page up
          e.preventDefault();
          panelContent.scrollBy({ top: -fullPage, behavior: 'smooth' });
          return;
      }
    }

    // Don't handle navigation keys when in input
    if (this._isInputTarget(e.target)) return;

    const visibleItems = this._getVisibleItems();
    const currentIndex = this._selectedPath
      ? visibleItems.findIndex(item => item.path === this._selectedPath)
      : -1;

    switch (e.key) {
      case 'ArrowDown':
      case 'j':
        e.preventDefault();
        if (visibleItems.length > 0) {
          const nextIndex = Math.min(currentIndex + 1, visibleItems.length - 1);
          this._selectedPath = visibleItems[nextIndex].path;
          this._scrollSelectedIntoView();
        }
        break;

      case 'ArrowUp':
      case 'k':
        e.preventDefault();
        if (visibleItems.length > 0) {
          const prevIndex = Math.max(currentIndex - 1, 0);
          this._selectedPath = visibleItems[prevIndex].path;
          this._scrollSelectedIntoView();
        }
        break;

      case 'Enter':
      case 'l': // vim-style: enter/expand
        if (this._selectedPath) {
          e.preventDefault();
          const item = visibleItems.find(i => i.path === this._selectedPath);
          if (item) {
            if (item.type === 'folder') {
              // Toggle folder expansion
              this._toggleFolder(item.path);
            } else {
              // Navigate to file
              window.location.href = item.url;
            }
          }
        }
        break;

      case 'h': // vim-style: collapse/go to parent
        if (this._selectedPath) {
          e.preventDefault();
          const item = visibleItems.find(i => i.path === this._selectedPath);
          if (item && item.type === 'folder' && this._expandedFolders.has(item.path)) {
            // Collapse if expanded
            this._toggleFolder(item.path);
          } else {
            // Go to parent folder
            const parentPath = this._getParentPath(this._selectedPath);
            if (parentPath && parentPath !== '/') {
              this._selectedPath = parentPath;
              this._scrollSelectedIntoView();
            }
          }
        }
        break;

      case 'o': // Open in new tab
        if (this._selectedPath) {
          e.preventDefault();
          const item = visibleItems.find(i => i.path === this._selectedPath);
          if (item) {
            window.open(item.url, '_blank');
          }
        }
        break;
    }
  }

  /**
   * Get parent path of a given path.
   */
  private _getParentPath(path: string): string | null {
    const normalized = path.endsWith('/') ? path.slice(0, -1) : path;
    const lastSlash = normalized.lastIndexOf('/');
    if (lastSlash <= 0) return '/';
    return normalized.slice(0, lastSlash) + '/';
  }

  /**
   * Get a flat list of visible items in tree order.
   */
  private _getVisibleItems(): Array<{ type: 'folder' | 'file'; path: string; url: string }> {
    const items: Array<{ type: 'folder' | 'file'; path: string; url: string }> = [];
    const filteredFiles = this._getFilteredFiles();
    const tree = this._buildFolderTree(filteredFiles);

    const traverse = (node: FolderNode) => {
      // Add folder (skip root)
      if (node.name !== '') {
        items.push({ type: 'folder', path: node.path, url: node.path });
      }

      // If expanded or root, add children
      if (this._expandedFolders.has(node.path) || node.name === '') {
        // Add child folders (sorted)
        const sortedChildren = Array.from(node.children.values())
          .sort((a, b) => a.name.localeCompare(b.name));
        for (const child of sortedChildren) {
          traverse(child);
        }

        // Add files (sorted)
        const sortedFiles = [...node.files].sort((a, b) => {
          const titleA = a.frontmatter?.title || a.url_path;
          const titleB = b.frontmatter?.title || b.url_path;
          return titleA.localeCompare(titleB);
        });
        for (const file of sortedFiles) {
          items.push({ type: 'file', path: file.url_path, url: file.url_path });
        }
      }
    };

    traverse(tree);
    return items;
  }

  /**
   * Scroll the selected item into view.
   */
  private _scrollSelectedIntoView() {
    this.updateComplete.then(() => {
      const selectedEl = this.shadowRoot?.querySelector('.tree-folder.keyboard-selected, .tree-file.keyboard-selected');
      if (selectedEl) {
        selectedEl.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
      }
    });
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    if (this._keyboardHandler) {
      document.removeEventListener('keydown', this._keyboardHandler);
    }
  }

  /**
   * Check if the event target is an input element.
   */
  private _isInputTarget(target: EventTarget | null): boolean {
    if (!target || !(target instanceof HTMLElement)) return false;
    const tagName = target.tagName.toLowerCase();
    return tagName === 'input' || tagName === 'textarea' || target.isContentEditable;
  }

  /**
   * Auto-expand folders in the current URL path.
   */
  private _autoExpandCurrentPath(path: string) {
    const parts = path.split('/').filter(p => p.length > 0);
    let accumulated = '';

    for (const part of parts) {
      accumulated += '/' + part;
      this._expandedFolders.add(accumulated);
    }

    // Also expand root
    this._expandedFolders.add('/');
    this._expandedFolders = new Set(this._expandedFolders);
  }

  /**
   * Extract tags from all markdown files' frontmatter.
   * Checks multiple fields: tags, tag, keywords, category, categories, taxonomy.tags, taxonomy.categories
   */
  private _extractTags(files: MarkdownFile[]): TagInfo[] {
    const tagMap = new Map<string, { count: number; field: string }>();

    for (const file of files) {
      if (!file.frontmatter) continue;

      const tagFields = [
        { key: 'tags', value: file.frontmatter.tags },
        { key: 'tag', value: file.frontmatter.tag },
        { key: 'keywords', value: file.frontmatter.keywords },
        { key: 'category', value: file.frontmatter.category },
        { key: 'categories', value: file.frontmatter.categories },
        { key: 'taxonomy.tags', value: file.frontmatter.taxonomy?.tags },
        { key: 'taxonomy.categories', value: file.frontmatter.taxonomy?.categories },
      ];

      for (const { key, value } of tagFields) {
        if (!value) continue;

        // Handle string (comma-separated) or array
        const tags = Array.isArray(value)
          ? value
          : String(value).split(',').map(t => t.trim()).filter(t => t.length > 0);

        for (const tag of tags) {
          const normalized = tag.trim();
          if (normalized.length === 0) continue;

          const existing = tagMap.get(normalized);
          if (existing) {
            existing.count += 1;
          } else {
            tagMap.set(normalized, { count: 1, field: key });
          }
        }
      }
    }

    // Convert to array and sort by count (descending), then by name
    return Array.from(tagMap.entries())
      .map(([name, { count, field }]) => ({ name, count, field }))
      .sort((a, b) => {
        if (b.count !== a.count) return b.count - a.count;
        return a.name.localeCompare(b.name);
      });
  }

  /**
   * Build hierarchical folder tree from markdown files.
   */
  private _buildFolderTree(files: MarkdownFile[]): FolderNode {
    const root: FolderNode = {
      name: '',
      path: '/',
      children: new Map(),
      files: [],
    };

    for (const file of files) {
      const parts = file.url_path.split('/').filter(p => p.length > 0);
      let currentNode = root;

      // Navigate/create folder structure (all parts except last are folders)
      for (let i = 0; i < parts.length - 1; i++) {
        const part = parts[i];
        const folderPath = '/' + parts.slice(0, i + 1).join('/') + '/';

        if (!currentNode.children.has(part)) {
          currentNode.children.set(part, {
            name: part,
            path: folderPath,
            children: new Map(),
            files: [],
          });
        }

        currentNode = currentNode.children.get(part)!;
      }

      // Add file to the current folder
      currentNode.files.push(file);
    }

    return root;
  }

  /**
   * Get filtered files based on selected tags (AND logic).
   */
  private _getFilteredFiles(): MarkdownFile[] {
    if (this._selectedTags.size === 0) {
      return this._allFiles;
    }

    return this._allFiles.filter(file => {
      if (!file.frontmatter) return false;

      // Collect all tags from all fields for this file
      const fileTags = new Set<string>();

      const tagFields = [
        file.frontmatter.tags,
        file.frontmatter.tag,
        file.frontmatter.keywords,
        file.frontmatter.category,
        file.frontmatter.categories,
        file.frontmatter.taxonomy?.tags,
        file.frontmatter.taxonomy?.categories,
      ];

      for (const value of tagFields) {
        if (!value) continue;

        const tags = Array.isArray(value)
          ? value
          : String(value).split(',').map(t => t.trim());

        for (const tag of tags) {
          const normalized = tag.trim();
          if (normalized.length > 0) {
            fileTags.add(normalized);
          }
        }
      }

      // Check if file has ALL selected tags (AND logic)
      for (const selectedTag of this._selectedTags) {
        if (!fileTags.has(selectedTag)) {
          return false;
        }
      }

      return true;
    });
  }

  /**
   * Toggle tag selection.
   */
  private _toggleTag(tagName: string) {
    const newSelection = new Set(this._selectedTags);

    if (newSelection.has(tagName)) {
      newSelection.delete(tagName);
    } else {
      newSelection.add(tagName);
    }

    this._selectedTags = newSelection;
  }

  /**
   * Toggle folder expanded/collapsed state.
   */
  private _toggleFolder(path: string) {
    const newExpanded = new Set(this._expandedFolders);

    if (newExpanded.has(path)) {
      newExpanded.delete(path);
    } else {
      newExpanded.add(path);
    }

    this._expandedFolders = newExpanded;
  }

  /**
   * Check if a path is the current page.
   */
  private _isCurrentPath(path: string): boolean {
    const currentPath = window.location.pathname;

    // Normalize both paths for comparison
    const normalizedCurrent = currentPath.endsWith('/') ? currentPath : currentPath + '/';
    const normalizedPath = path.endsWith('/') ? path : path + '/';

    return normalizedCurrent === normalizedPath;
  }

  /**
   * Public method to open the browse panel.
   */
  public open() {
    this._isOpen = true;
  }

  /**
   * Public method to close the browse panel.
   */
  public close() {
    this._isOpen = false;
  }

  /**
   * Render tag pills with filtering.
   */
  private _renderTags() {
    if (this._allTags.length === 0) {
      return nothing;
    }

    const maxVisibleTags = 20;
    const visibleTags = this._tagsExpanded
      ? this._allTags
      : this._allTags.slice(0, maxVisibleTags);
    const hasMoreTags = this._allTags.length > maxVisibleTags;

    return html`
      <div class="tags-section">
        <div class="tags-header">
          <h3>Filter by Tags</h3>
          ${this._selectedTags.size > 0 ? html`
            <button
              class="clear-tags-button"
              @click=${() => { this._selectedTags = new Set(); }}
              title="Clear all tag filters"
            >
              Clear
            </button>
          ` : nothing}
        </div>
        <div class="tags-container">
          ${visibleTags.map(tag => html`
            <button
              class="tag-pill ${this._selectedTags.has(tag.name) ? 'selected' : ''}"
              @click=${() => this._toggleTag(tag.name)}
              title="${tag.field}: ${tag.name}"
            >
              ${tag.name}
              <span class="tag-count">${tag.count}</span>
            </button>
          `)}
        </div>
        ${hasMoreTags ? html`
          <button
            class="expand-tags-button"
            @click=${() => { this._tagsExpanded = !this._tagsExpanded; }}
          >
            ${this._tagsExpanded ? 'Show fewer tags' : `Show ${this._allTags.length - maxVisibleTags} more tags`}
          </button>
        ` : nothing}
      </div>
    `;
  }

  /**
   * Render folder tree recursively.
   */
  private _renderFolderNode(node: FolderNode, depth: number = 0): TemplateResult {
    const isExpanded = this._expandedFolders.has(node.path);
    const hasChildren = node.children.size > 0;
    const isCurrent = this._isCurrentPath(node.path);
    const isKeyboardSelected = this._selectedPath === node.path;

    return html`
      ${node.name !== '' ? html`
        <div
          class="tree-folder ${isCurrent ? 'current' : ''} ${isKeyboardSelected ? 'keyboard-selected' : ''}"
          style="padding-left: ${depth * 1}rem"
          @click=${() => { this._selectedPath = node.path; }}
        >
          <button
            class="folder-toggle"
            @click=${(e: Event) => { e.stopPropagation(); this._toggleFolder(node.path); }}
            ?disabled=${!hasChildren}
          >
            ${hasChildren ? (isExpanded ? '▼' : '▶') : ''}
          </button>
          <a href="${node.path}" class="folder-link">
            📁 ${node.name}
          </a>
        </div>
      ` : nothing}

      ${isExpanded || node.name === '' ? html`
        ${Array.from(node.children.values())
          .sort((a, b) => a.name.localeCompare(b.name))
          .map(child => this._renderFolderNode(child, depth + 1))}

        ${node.files
          .sort((a, b) => {
            const titleA = a.frontmatter?.title || a.url_path;
            const titleB = b.frontmatter?.title || b.url_path;
            return titleA.localeCompare(titleB);
          })
          .map(file => this._renderFile(file, depth + 1))}
      ` : nothing}
    `;
  }

  /**
   * Render individual file in tree.
   */
  private _renderFile(file: MarkdownFile, depth: number): TemplateResult {
    const title = file.frontmatter?.title || file.url_path.split('/').filter(p => p).pop() || 'Untitled';
    const description = file.frontmatter?.description || file.frontmatter?.summary || '';
    const isCurrent = this._isCurrentPath(file.url_path);
    const isKeyboardSelected = this._selectedPath === file.url_path;

    return html`
      <div
        class="tree-file ${isCurrent ? 'current' : ''} ${isKeyboardSelected ? 'keyboard-selected' : ''}"
        style="padding-left: ${depth * 1}rem"
        @click=${() => { this._selectedPath = file.url_path; }}
      >
        <a href="${file.url_path}" class="file-link">
          <div class="file-title">📄 ${title}</div>
          ${description ? html`
            <div class="file-description">${description}</div>
          ` : nothing}
        </a>
      </div>
    `;
  }

  /**
   * Render the browse panel.
   */
  private _renderPanel() {
    if (!this._isOpen) return nothing;

    const filteredFiles = this._getFilteredFiles();
    const tree = this._buildFolderTree(filteredFiles);

    return html`
      <div class="panel-backdrop" @click=${this.close}>
        <aside class="browse-panel" @click=${(e: Event) => e.stopPropagation()}>
          <div class="panel-header">
            <h2>Browse Files</h2>
            <button class="close-button" @click=${this.close} aria-label="Close panel">
              ✕
            </button>
          </div>

          <div class="panel-content">
            ${this._renderTags()}

            <div class="tree-section">
              <h3>Folder Tree</h3>
              ${filteredFiles.length === 0 ? html`
                <div class="no-results">No files match the selected tags</div>
              ` : html`
                <div class="tree-container">
                  ${this._renderFolderNode(tree)}
                </div>
              `}
            </div>
          </div>
        </aside>
      </div>
    `;
  }

  override render() {
    return this._renderPanel();
  }

  static override styles = css`
    :host {
      display: contents;
    }

    /* Panel backdrop */
    .panel-backdrop {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.5);
      z-index: 1000;
      animation: fadeIn 0.2s ease;
    }

    @keyframes fadeIn {
      from { opacity: 0; }
      to { opacity: 1; }
    }

    /* Browse panel */
    .browse-panel {
      position: fixed;
      left: 0;
      top: 0;
      height: 100%;
      width: 100%;
      max-width: 100vw;
      background: var(--pico-background-color, #fff);
      box-shadow: 2px 0 8px rgba(0, 0, 0, 0.1);
      display: flex;
      flex-direction: column;
      overflow: hidden;
      animation: slideIn 0.3s ease;
    }

    @media (min-width: 768px) {
      .browse-panel {
        width: clamp(320px, 40vw, 600px);
      }
    }

    @keyframes slideIn {
      from { transform: translateX(-100%); }
      to { transform: translateX(0); }
    }

    /* Panel header */
    .panel-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 1rem 1.25rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    .panel-header h2 {
      margin: 0;
      font-size: 1.25rem;
      color: var(--pico-color, #333);
    }

    .close-button {
      background: transparent;
      border: none;
      font-size: 1.5rem;
      color: var(--pico-muted-color, #999);
      cursor: pointer;
      padding: 0.25rem 0.5rem;
      line-height: 1;
      transition: color 0.15s ease;
    }

    .close-button:hover {
      color: var(--pico-color, #333);
    }

    /* Panel content */
    .panel-content {
      flex: 1;
      overflow-y: auto;
      padding: 1rem 0;
    }

    /* Tags section */
    .tags-section {
      padding: 0 1.25rem;
      margin-bottom: 1.5rem;
    }

    .tags-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-bottom: 0.75rem;
    }

    .tags-header h3 {
      margin: 0;
      font-size: 0.875rem;
      font-weight: 600;
      color: var(--pico-muted-color, #666);
      text-transform: uppercase;
      letter-spacing: 0.05em;
    }

    .clear-tags-button {
      background: transparent;
      border: none;
      color: var(--pico-primary, #0d6efd);
      cursor: pointer;
      font-size: 0.8rem;
      padding: 0.25rem 0.5rem;
      text-decoration: underline;
    }

    .clear-tags-button:hover {
      color: var(--pico-primary-hover, #0b5ed7);
    }

    .tags-container {
      display: flex;
      flex-wrap: wrap;
      gap: 0.5rem;
    }

    .tag-pill {
      display: inline-flex;
      align-items: center;
      gap: 0.35rem;
      padding: 0.4rem 0.75rem;
      border: 1px solid var(--pico-muted-border-color, #ddd);
      border-radius: 16px;
      background: var(--pico-secondary-background, #f5f5f5);
      color: var(--pico-color, #333);
      font-size: 0.8rem;
      cursor: pointer;
      transition: all 0.15s ease;
    }

    .tag-pill:hover {
      border-color: var(--pico-primary, #0d6efd);
      background: var(--pico-primary-background, #e3f2fd);
    }

    .tag-pill.selected {
      border-color: var(--pico-primary, #0d6efd);
      background: var(--pico-primary, #0d6efd);
      color: white;
    }

    .tag-count {
      font-size: 0.7rem;
      opacity: 0.8;
    }

    .expand-tags-button {
      margin-top: 0.75rem;
      width: 100%;
      padding: 0.5rem;
      background: transparent;
      border: 1px dashed var(--pico-muted-border-color, #ddd);
      border-radius: 6px;
      color: var(--pico-muted-color, #666);
      cursor: pointer;
      font-size: 0.8rem;
      transition: all 0.15s ease;
    }

    .expand-tags-button:hover {
      border-color: var(--pico-primary, #0d6efd);
      color: var(--pico-primary, #0d6efd);
    }

    /* Tree section */
    .tree-section {
      padding: 0 1.25rem;
    }

    .tree-section h3 {
      margin: 0 0 0.75rem 0;
      font-size: 0.875rem;
      font-weight: 600;
      color: var(--pico-muted-color, #666);
      text-transform: uppercase;
      letter-spacing: 0.05em;
    }

    .tree-container {
      display: flex;
      flex-direction: column;
      gap: 0.25rem;
    }

    .no-results {
      padding: 1.5rem;
      text-align: center;
      color: var(--pico-muted-color, #666);
      font-size: 0.9rem;
    }

    /* Tree folder */
    .tree-folder {
      display: flex;
      align-items: center;
      gap: 0.35rem;
      padding: 0.4rem 0.5rem;
      border-radius: 6px;
      transition: background 0.15s ease;
    }

    .tree-folder:hover {
      background: var(--pico-secondary-background, #f5f5f5);
    }

    .tree-folder.current {
      background: var(--pico-primary-background, #e3f2fd);
      border-left: 3px solid var(--pico-primary, #0d6efd);
    }

    .tree-folder.keyboard-selected,
    .tree-file.keyboard-selected {
      outline: 2px solid var(--pico-primary, #0d6efd);
      outline-offset: -2px;
    }

    .folder-toggle {
      background: transparent;
      border: none;
      color: var(--pico-muted-color, #999);
      cursor: pointer;
      padding: 0;
      width: 1.25rem;
      height: 1.25rem;
      display: flex;
      align-items: center;
      justify-content: center;
      font-size: 0.75rem;
      flex-shrink: 0;
    }

    .folder-toggle:disabled {
      cursor: default;
      visibility: hidden;
    }

    .folder-link {
      color: var(--pico-color, #333);
      text-decoration: none;
      font-weight: 500;
      flex: 1;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .folder-link:hover {
      color: var(--pico-primary, #0d6efd);
    }

    /* Tree file */
    .tree-file {
      padding: 0.4rem 0.5rem;
      border-radius: 6px;
      transition: background 0.15s ease;
    }

    .tree-file:hover {
      background: var(--pico-secondary-background, #f5f5f5);
    }

    .tree-file.current {
      background: var(--pico-primary-background, #e3f2fd);
      border-left: 3px solid var(--pico-primary, #0d6efd);
    }

    .file-link {
      color: var(--pico-color, #333);
      text-decoration: none;
      display: block;
    }

    .file-title {
      font-size: 0.9rem;
      margin-bottom: 0.15rem;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .file-link:hover .file-title {
      color: var(--pico-primary, #0d6efd);
    }

    .file-description {
      font-size: 0.75rem;
      color: var(--pico-muted-color, #666);
      line-height: 1.4;
      display: -webkit-box;
      -webkit-line-clamp: 2;
      -webkit-box-orient: vertical;
      overflow: hidden;
    }
  `;
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-browse': MbrBrowseElement
  }
}
