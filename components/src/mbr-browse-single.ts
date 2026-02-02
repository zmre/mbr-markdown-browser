import { LitElement, css, html, nothing, type TemplateResult } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { subscribeSiteNav, resolveUrl, getTagSources, getCanonicalPath, type TagSourceConfig } from './shared.js'
import {
  type MarkdownFile,
  type SortField,
  type FolderNode,
  DEFAULT_SORT_CONFIG,
  sortFiles,
  sortFolders,
  buildFolderTree,
} from './sorting.js'

/**
 * Tag group containing tags from a single source.
 */
interface TagGroup {
  source: string;
  label: string;
  labelPlural: string;
  urlSource: string;
  tags: Map<string, number>;  // tag value -> count
}

/**
 * Default sidebar items limit per section.
 */
const DEFAULT_MAX_ITEMS = 100;

/**
 * Single-column sidebar navigation component for MBR.
 *
 * Provides persistent sidebar navigation similar to documentation sites.
 * Supports two modes based on viewport width:
 * - Desktop: Inline sidebar beside content
 * - Mobile: Overlay drawer triggered by hamburger button
 *
 * Features:
 * - Root index link at top
 * - Expandable folder tree (bold folders, normal-weight files)
 * - Root-level files section
 * - Tag groups with badge pills
 * - Pagination for large repos
 * - Keyboard navigation
 *
 * Keyboard shortcuts:
 * - `-` or F2: Toggle sidebar (via mbr-keys)
 * - Escape: Close drawer (mobile)
 * - Arrow keys / j/k / Ctrl+N/P: Navigate items
 * - h/l: Collapse/expand folders
 * - Enter: Navigate to item
 */
@customElement('mbr-browse-single')
export class MbrBrowseSingleElement extends LitElement {
  // === Mode State ===
  @state()
  private _isOverlayMode = false;

  @state()
  private _isDrawerOpen = false;

  // === Data State ===
  @state()
  private _allFiles: MarkdownFile[] = [];

  @state()
  private _isLoading = true;

  @state()
  private _loadError: string | null = null;

  // === Derived Data ===
  @state()
  private _folderTree: FolderNode | null = null;

  @state()
  private _rootFiles: MarkdownFile[] = [];

  @state()
  private _tagGroups: Map<string, TagGroup> = new Map();

  // === UI State ===
  @state()
  private _expandedFolders = new Set<string>();

  @state()
  private _focusedPath: string | null = null;

  @state()
  private _paginationState = new Map<string, number>();

  // === Config ===
  private _breakpoint = 1024;
  private _maxItems = DEFAULT_MAX_ITEMS;
  private _indexFile = 'index.md';
  private _sortConfig: SortField[] = DEFAULT_SORT_CONFIG;
  private _tagSources: TagSourceConfig[] = [];

  // === Internal ===
  private _resizeObserver: ResizeObserver | null = null;
  private _mediaQuery: MediaQueryList | null = null;
  private _keyboardHandler: ((e: KeyboardEvent) => void) | null = null;
  private _toggleHandler: ((e: Event) => void) | null = null;
  private _unsubscribeSiteNav: (() => void) | null = null;
  private _slidesStartHandler: (() => void) | null = null;
  private _focusedIndex = -1;
  private _flatItems: Array<{ type: 'folder' | 'file' | 'tag'; path: string; depth: number }> = [];

  // ========================================
  // Lifecycle
  // ========================================

  override connectedCallback() {
    super.connectedCallback();

    // Read CSS variable for breakpoint
    this._readBreakpointFromCSS();

    // Read max items from data attribute
    const maxItemsAttr = this.getAttribute('max-items');
    if (maxItemsAttr) {
      const parsed = parseInt(maxItemsAttr, 10);
      if (!isNaN(parsed) && parsed > 0) {
        this._maxItems = parsed;
      }
    }

    // Subscribe to site navigation data
    this._unsubscribeSiteNav = subscribeSiteNav((state) => {
      this._isLoading = state.isLoading;
      this._loadError = state.error;

      if (state.data?.markdown_files) {
        this._allFiles = state.data.markdown_files;

        if (state.data.index_file) {
          this._indexFile = state.data.index_file;
        }

        if (state.data.sort && Array.isArray(state.data.sort) && state.data.sort.length > 0) {
          this._sortConfig = state.data.sort;
        }

        // Build derived data structures
        this._folderTree = buildFolderTree(this._allFiles, this._indexFile);
        this._rootFiles = this._extractRootFiles(this._allFiles);
        this._tagSources = getTagSources();
        this._tagGroups = this._buildTagGroups(this._allFiles);

        // Auto-expand current path (use canonical path for static mode)
        const currentPath = getCanonicalPath();
        this._autoExpandCurrentPath(currentPath);
      }
    });

    // Setup responsive behavior
    this._setupResponsive();

    // Setup keyboard handler
    this._setupKeyboardHandler();

    // Listen for toggle events from mbr-sidebar-trigger
    this._toggleHandler = () => this.toggle();
    window.addEventListener('mbr-sidebar-toggle', this._toggleHandler);

    // Close drawer when slides presentation starts
    this._slidesStartHandler = () => this.close();
    window.addEventListener('mbr-slides-start', this._slidesStartHandler);
  }

  override disconnectedCallback() {
    super.disconnectedCallback();

    if (this._unsubscribeSiteNav) {
      this._unsubscribeSiteNav();
    }
    if (this._resizeObserver) {
      this._resizeObserver.disconnect();
    }
    if (this._mediaQuery) {
      this._mediaQuery.removeEventListener('change', this._handleMediaChange);
    }
    if (this._keyboardHandler) {
      document.removeEventListener('keydown', this._keyboardHandler);
    }
    if (this._toggleHandler) {
      window.removeEventListener('mbr-sidebar-toggle', this._toggleHandler);
    }
    if (this._slidesStartHandler) {
      window.removeEventListener('mbr-slides-start', this._slidesStartHandler);
    }

    // Remove body class
    document.body.classList.remove('mbr-has-sidebar');
  }

  // ========================================
  // Public Methods
  // ========================================

  public open() {
    this._isDrawerOpen = true;
    // Focus the sidebar for keyboard nav
    requestAnimationFrame(() => {
      const sidebar = this.shadowRoot?.querySelector('.sidebar');
      if (sidebar instanceof HTMLElement) {
        sidebar.focus();
      }
    });
  }

  public close() {
    this._isDrawerOpen = false;
  }

  public toggle() {
    if (this._isDrawerOpen) {
      this.close();
    } else {
      this.open();
    }
  }

  // ========================================
  // Responsive Setup
  // ========================================

  private _readBreakpointFromCSS() {
    // Read --mbr-hide-nav-bp from CSS (defaults to 1024px)
    const style = getComputedStyle(document.documentElement);
    const bpValue = style.getPropertyValue('--mbr-hide-nav-bp').trim();
    if (bpValue) {
      const parsed = parseInt(bpValue, 10);
      if (!isNaN(parsed) && parsed > 0) {
        this._breakpoint = parsed;
      }
    }
  }

  private _setupResponsive() {
    // Use matchMedia for responsive behavior
    this._mediaQuery = window.matchMedia(`(max-width: ${this._breakpoint}px)`);
    this._handleMediaChange = this._handleMediaChange.bind(this);
    this._mediaQuery.addEventListener('change', this._handleMediaChange);
    this._handleMediaChange(this._mediaQuery);
  }

  private _handleMediaChange = (e: MediaQueryListEvent | MediaQueryList) => {
    this._isOverlayMode = e.matches;

    // Update body class for CSS grid layout
    if (this._isOverlayMode) {
      document.body.classList.remove('mbr-has-sidebar');
    } else {
      document.body.classList.add('mbr-has-sidebar');
      // Close drawer when switching to desktop mode
      this._isDrawerOpen = false;
    }
  };

  // ========================================
  // Keyboard Handler
  // ========================================

  private _setupKeyboardHandler() {
    this._keyboardHandler = (e: KeyboardEvent) => {
      // Handle - and F2 for toggle (in overlay mode)
      if (this._isOverlayMode && !this._isInputTarget(e.target)) {
        if (e.key === '-' || e.key === 'F2') {
          e.preventDefault();
          this.toggle();
          return;
        }
      }

      // Handle Escape to close drawer
      if (e.key === 'Escape' && this._isDrawerOpen) {
        e.preventDefault();
        this.close();
        return;
      }

      // Only handle other keys when focused on sidebar
      if (!this._isSidebarFocused()) return;

      this._handleSidebarKeydown(e);
    };

    document.addEventListener('keydown', this._keyboardHandler);
  }

  private _isSidebarFocused(): boolean {
    const activeElement = this.shadowRoot?.activeElement || document.activeElement;
    return this.contains(activeElement as Node);
  }

  private _handleSidebarKeydown(e: KeyboardEvent) {
    if (this._isInputTarget(e.target)) return;

    // Build flat list of navigable items for arrow key nav
    if (this._flatItems.length === 0) {
      this._buildFlatItems();
    }

    switch (e.key) {
      case 'ArrowDown':
      case 'j':
        if (e.ctrlKey && e.key.toLowerCase() === 'n') {
          e.preventDefault();
          this._focusNext();
        } else if (!e.ctrlKey) {
          e.preventDefault();
          this._focusNext();
        }
        break;

      case 'ArrowUp':
      case 'k':
        if (e.ctrlKey && e.key.toLowerCase() === 'p') {
          e.preventDefault();
          this._focusPrev();
        } else if (!e.ctrlKey) {
          e.preventDefault();
          this._focusPrev();
        }
        break;

      case 'ArrowRight':
      case 'l':
        e.preventDefault();
        this._expandOrEnter();
        break;

      case 'ArrowLeft':
      case 'h':
        e.preventDefault();
        this._collapseOrParent();
        break;

      case 'Enter':
        e.preventDefault();
        this._activateFocused();
        break;

      case 'Home':
        e.preventDefault();
        this._focusFirst();
        break;

      case 'End':
        e.preventDefault();
        this._focusLast();
        break;
    }

    // Ctrl+N / Ctrl+P for navigation
    if (e.ctrlKey) {
      if (e.key === 'n' || e.key === 'N') {
        e.preventDefault();
        this._focusNext();
      } else if (e.key === 'p' || e.key === 'P') {
        e.preventDefault();
        this._focusPrev();
      }
    }
  }

  private _buildFlatItems() {
    this._flatItems = [];

    // Add root/home item
    this._flatItems.push({ type: 'folder', path: '/', depth: 0 });

    // Add folder tree items recursively
    if (this._folderTree) {
      this._addFolderItemsRecursive(this._folderTree, 0, true);
    }

    // Add root files
    for (const file of this._rootFiles) {
      this._flatItems.push({ type: 'file', path: file.url_path, depth: 0 });
    }

    // Add tag items
    for (const [, group] of this._tagGroups) {
      for (const [tag] of group.tags) {
        this._flatItems.push({ type: 'tag', path: `${group.urlSource}/${tag}`, depth: 0 });
      }
    }
  }

  private _addFolderItemsRecursive(node: FolderNode, depth: number, isRoot: boolean) {
    const isExpanded = this._expandedFolders.has(node.path) || isRoot;

    // Add child folders
    if (isExpanded) {
      const sortedChildren = sortFolders([...node.children.values()], this._sortConfig);
      for (const child of sortedChildren) {
        this._flatItems.push({ type: 'folder', path: child.path, depth: depth + 1 });
        this._addFolderItemsRecursive(child, depth + 1, false);
      }

      // Add files in this folder
      const sortedFiles = sortFiles(node.files, this._sortConfig);
      for (const file of sortedFiles) {
        const fileName = file.raw_path.split('/').pop() || '';
        if (fileName !== this._indexFile) {
          this._flatItems.push({ type: 'file', path: file.url_path, depth: depth + 1 });
        }
      }
    }
  }

  /**
   * Initialize focus index to current page when keyboard navigation starts.
   * Called on first navigation keypress when _focusedIndex is still -1.
   */
  private _initializeFocusIfNeeded() {
    if (this._focusedIndex >= 0) return;

    // Find current page in flat items (use canonical path for static mode)
    const currentPath = getCanonicalPath();
    const normalizedCurrent = currentPath.endsWith('/') ? currentPath : currentPath + '/';

    const currentIndex = this._flatItems.findIndex(item => {
      const normalizedItem = item.path.endsWith('/') ? item.path : item.path + '/';
      return normalizedItem === normalizedCurrent;
    });

    // Start at current page if found, otherwise at beginning
    this._focusedIndex = currentIndex >= 0 ? currentIndex : 0;
  }

  private _focusNext() {
    this._initializeFocusIfNeeded();
    this._focusedIndex = Math.min(this._focusedIndex + 1, this._flatItems.length - 1);
    this._updateFocusedPath();
  }

  private _focusPrev() {
    this._initializeFocusIfNeeded();
    this._focusedIndex = Math.max(this._focusedIndex - 1, 0);
    this._updateFocusedPath();
  }

  private _focusFirst() {
    this._focusedIndex = 0;
    this._updateFocusedPath();
  }

  private _focusLast() {
    this._focusedIndex = this._flatItems.length - 1;
    this._updateFocusedPath();
  }

  private _updateFocusedPath() {
    if (this._focusedIndex >= 0 && this._focusedIndex < this._flatItems.length) {
      this._focusedPath = this._flatItems[this._focusedIndex].path;

      // Scroll focused item into view
      requestAnimationFrame(() => {
        const focused = this.shadowRoot?.querySelector('.nav-item.focused');
        if (focused) {
          focused.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
        }
      });
    }
  }

  private _expandOrEnter() {
    if (this._focusedIndex < 0) return;
    const item = this._flatItems[this._focusedIndex];

    if (item.type === 'folder') {
      if (!this._expandedFolders.has(item.path)) {
        this._toggleFolder(item.path);
        this._buildFlatItems();
      } else {
        // Already expanded, navigate to it
        window.location.href = resolveUrl(item.path);
      }
    } else if (item.type === 'file') {
      window.location.href = resolveUrl(item.path);
    } else if (item.type === 'tag') {
      window.location.href = resolveUrl(`/${item.path}/`);
    }
  }

  private _collapseOrParent() {
    if (this._focusedIndex < 0) return;
    const item = this._flatItems[this._focusedIndex];

    if (item.type === 'folder' && this._expandedFolders.has(item.path)) {
      this._toggleFolder(item.path);
      this._buildFlatItems();
    } else {
      // Move to parent
      const parentPath = this._getParentPath(item.path);
      if (parentPath) {
        const parentIndex = this._flatItems.findIndex(i => i.path === parentPath);
        if (parentIndex >= 0) {
          this._focusedIndex = parentIndex;
          this._updateFocusedPath();
        }
      }
    }
  }

  private _getParentPath(path: string): string | null {
    const parts = path.split('/').filter(p => p);
    if (parts.length === 0) return null;
    parts.pop();
    return parts.length === 0 ? '/' : '/' + parts.join('/') + '/';
  }

  private _activateFocused() {
    if (this._focusedIndex < 0) return;
    const item = this._flatItems[this._focusedIndex];

    if (item.type === 'folder') {
      window.location.href = resolveUrl(item.path);
    } else if (item.type === 'file') {
      window.location.href = resolveUrl(item.path);
    } else if (item.type === 'tag') {
      window.location.href = resolveUrl(`/${item.path}/`);
    }
  }

  // ========================================
  // Data Building
  // ========================================

  private _extractRootFiles(files: MarkdownFile[]): MarkdownFile[] {
    return files.filter(f => {
      const parts = f.url_path.split('/').filter(p => p);
      return parts.length === 1;  // Single part = root level file
    });
  }

  private _buildTagGroups(files: MarkdownFile[]): Map<string, TagGroup> {
    const groups = new Map<string, TagGroup>();

    for (const source of this._tagSources) {
      const tagCounts = new Map<string, number>();

      for (const file of files) {
        const tags = this._extractTagsFromFile(file, source.field);
        for (const tag of tags) {
          tagCounts.set(tag, (tagCounts.get(tag) || 0) + 1);
        }
      }

      if (tagCounts.size > 0) {
        groups.set(source.field, {
          source: source.field,
          label: source.label,
          labelPlural: source.labelPlural,
          urlSource: source.urlSource,
          tags: tagCounts,
        });
      }
    }

    return groups;
  }

  private _extractTagsFromFile(file: MarkdownFile, field: string): string[] {
    if (!file.frontmatter) return [];

    // Support dot-notation for nested fields
    const parts = field.split('.');
    let value: any = file.frontmatter;

    for (const part of parts) {
      if (value && typeof value === 'object' && part in value) {
        value = value[part];
      } else {
        return [];
      }
    }

    if (!value) return [];

    // Handle array or comma-separated string
    if (Array.isArray(value)) {
      return value.map(v => String(v).trim()).filter(v => v);
    } else {
      return String(value).split(',').map(t => t.trim()).filter(t => t);
    }
  }

  // ========================================
  // Navigation Helpers
  // ========================================

  private _autoExpandCurrentPath(path: string) {
    const parts = path.split('/').filter(p => p);
    let accumulated = '';

    for (const part of parts) {
      accumulated += '/' + part;
      this._expandedFolders.add(accumulated + '/');
    }

    this._expandedFolders.add('/');
    this._expandedFolders = new Set(this._expandedFolders);

    // Build flat items for keyboard navigation (but don't set focused path -
    // that should only happen on actual keyboard navigation, not page load)
    this._buildFlatItems();
  }

  private _toggleFolder(path: string) {
    const newExpanded = new Set(this._expandedFolders);
    if (newExpanded.has(path)) {
      newExpanded.delete(path);
    } else {
      newExpanded.add(path);
    }
    this._expandedFolders = newExpanded;
    this._flatItems = [];  // Reset flat items to rebuild
  }

  private _isCurrentPath(path: string): boolean {
    // Use canonical path to handle static mode with subdirectory deployment
    const currentPath = getCanonicalPath();
    const normalizedCurrent = currentPath.endsWith('/') ? currentPath : currentPath + '/';
    const normalizedPath = path.endsWith('/') ? path : path + '/';
    return normalizedCurrent === normalizedPath;
  }

  private _isInputTarget(target: EventTarget | null): boolean {
    if (!target || !(target instanceof HTMLElement)) return false;
    const tagName = target.tagName.toLowerCase();
    return tagName === 'input' || tagName === 'textarea' || target.isContentEditable;
  }

  // ========================================
  // Pagination
  // ========================================

  private _getPageItems<T>(items: T[], sectionKey: string): T[] {
    const page = this._paginationState.get(sectionKey) || 0;
    const limit = (page + 1) * this._maxItems;
    return items.slice(0, limit);
  }

  private _hasMoreItems<T>(items: T[], sectionKey: string): boolean {
    const page = this._paginationState.get(sectionKey) || 0;
    const limit = (page + 1) * this._maxItems;
    return items.length > limit;
  }

  private _showMore(sectionKey: string) {
    const current = this._paginationState.get(sectionKey) || 0;
    this._paginationState.set(sectionKey, current + 1);
    this._paginationState = new Map(this._paginationState);
  }

  // ========================================
  // Render Methods
  // ========================================

  override render() {
    // In desktop mode, always show sidebar inline
    // In overlay mode, show drawer only when open (trigger is in nav bar via mbr-sidebar-trigger)
    if (this._isOverlayMode) {
      return this._isDrawerOpen ? this._renderDrawer() : nothing;
    } else {
      return this._renderSidebar();
    }
  }

  private _renderDrawer(): TemplateResult {
    return html`
      <div class="drawer-backdrop" @click=${this.close}>
        <div class="drawer" @click=${(e: Event) => e.stopPropagation()}>
          ${this._renderSidebarContent()}
        </div>
      </div>
    `;
  }

  private _renderSidebar(): TemplateResult {
    return html`
      <aside class="sidebar" tabindex="-1">
        ${this._renderSidebarContent()}
      </aside>
    `;
  }

  private _renderSidebarContent(): TemplateResult {
    return html`
      <div class="sidebar-header">
        <h2>Navigate</h2>
        ${this._isOverlayMode ? html`
          <button class="close-button" @click=${this.close} aria-label="Close">
            <span aria-hidden="true">&times;</span>
          </button>
        ` : nothing}
      </div>
      <nav class="sidebar-nav" aria-label="Site navigation">
        ${this._isLoading ? this._renderLoading() :
          this._loadError ? this._renderError() : html`
            ${this._renderHomeLink()}
            ${this._renderFolderTree()}
            ${this._renderRootFiles()}
            ${this._renderTagGroups()}
          `}
      </nav>
    `;
  }

  private _renderHomeLink(): TemplateResult {
    const isCurrent = this._isCurrentPath('/');
    const isFocused = this._focusedPath === '/';
    const homeTitle = this._folderTree?.title || 'Home';

    return html`
      <div class="nav-section home-section">
        <a
          href="${resolveUrl('/')}"
          class="nav-item home-link ${isCurrent ? 'current' : ''} ${isFocused ? 'focused' : ''}"
        >
          <span class="nav-label">${homeTitle}</span>
        </a>
      </div>
    `;
  }

  private _renderFolderTree(): TemplateResult | typeof nothing {
    if (!this._folderTree || this._folderTree.children.size === 0) {
      return nothing;
    }

    const sortedFolders = sortFolders([...this._folderTree.children.values()], this._sortConfig);
    const pagedFolders = this._getPageItems(sortedFolders, 'folders');

    return html`
      <div class="nav-section folder-section">
        ${pagedFolders.map(folder => this._renderFolderNode(folder, 0))}
        ${this._hasMoreItems(sortedFolders, 'folders') ? html`
          <button class="show-more" @click=${() => this._showMore('folders')}>
            Show more folders...
          </button>
        ` : nothing}
      </div>
    `;
  }

  private _renderFolderNode(node: FolderNode, depth: number): TemplateResult {
    const isExpanded = this._expandedFolders.has(node.path);
    const hasChildren = node.children.size > 0;
    const isCurrent = this._isCurrentPath(node.path);
    const isFocused = this._focusedPath === node.path;
    const folderTitle = node.title || node.name;

    // Get files in this folder (excluding index)
    const filesInFolder = sortFiles(node.files, this._sortConfig)
      .filter(f => {
        const fileName = f.raw_path.split('/').pop() || '';
        return fileName !== this._indexFile;
      });

    const sortedChildren = sortFolders([...node.children.values()], this._sortConfig);

    return html`
      <div class="folder-tree-item" style="--depth: ${depth}">
        <div class="nav-item folder-item ${isCurrent ? 'current' : ''} ${isFocused ? 'focused' : ''}">
          ${hasChildren || filesInFolder.length > 0 ? html`
            <button
              class="expand-toggle ${isExpanded ? 'expanded' : ''}"
              @click=${(e: Event) => { e.preventDefault(); e.stopPropagation(); this._toggleFolder(node.path); }}
              aria-expanded=${isExpanded}
              aria-label="${isExpanded ? 'Collapse' : 'Expand'} ${folderTitle}"
            >
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
                <path d="M4 2L8 6L4 10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            </button>
          ` : html`<span class="expand-spacer"></span>`}
          <a href="${resolveUrl(node.path)}" class="folder-link">
            <span class="nav-label">${folderTitle}</span>
          </a>
        </div>
        ${isExpanded ? html`
          <div class="folder-children">
            ${sortedChildren.map(child => this._renderFolderNode(child, depth + 1))}
            ${filesInFolder.map(file => this._renderFileItem(file, depth + 1))}
          </div>
        ` : nothing}
      </div>
    `;
  }

  private _renderFileItem(file: MarkdownFile, depth: number): TemplateResult {
    const isCurrent = this._isCurrentPath(file.url_path);
    const isFocused = this._focusedPath === file.url_path;
    const title = file.frontmatter?.title ||
      file.url_path.split('/').filter(p => p).pop() || 'Untitled';

    return html`
      <a
        href="${resolveUrl(file.url_path)}"
        class="nav-item file-item ${isCurrent ? 'current' : ''} ${isFocused ? 'focused' : ''}"
        style="--depth: ${depth}"
      >
        <span class="expand-spacer"></span>
        <span class="nav-label">${title}</span>
      </a>
    `;
  }

  private _renderRootFiles(): TemplateResult | typeof nothing {
    // Filter out any files that are index files (already shown as home)
    const rootFilesExcludingIndex = this._rootFiles.filter(f => {
      const fileName = f.raw_path.split('/').pop() || '';
      return fileName !== this._indexFile;
    });

    if (rootFilesExcludingIndex.length === 0) {
      return nothing;
    }

    const sortedFiles = sortFiles(rootFilesExcludingIndex, this._sortConfig);
    const pagedFiles = this._getPageItems(sortedFiles, 'root-files');

    return html`
      <div class="nav-section root-files-section">
        ${pagedFiles.map(file => this._renderFileItem(file, 0))}
        ${this._hasMoreItems(sortedFiles, 'root-files') ? html`
          <button class="show-more" @click=${() => this._showMore('root-files')}>
            Show more files...
          </button>
        ` : nothing}
      </div>
    `;
  }

  private _renderTagGroups(): TemplateResult | typeof nothing {
    if (this._tagGroups.size === 0) {
      return nothing;
    }

    return html`
      ${[...this._tagGroups.entries()].map(([field, group]) => html`
        <div class="nav-section tag-section" data-tag-source="${field}">
          <h3 class="section-header">
            <a href="${resolveUrl(`/${group.urlSource}/`)}">${group.labelPlural}</a>
          </h3>
          <div class="tag-pills">
            ${this._renderTagPills(group)}
          </div>
        </div>
      `)}
    `;
  }

  private _renderTagPills(group: TagGroup): TemplateResult {
    const sortedTags = [...group.tags.entries()]
      .sort((a, b) => a[0].localeCompare(b[0]));
    const pagedTags = this._getPageItems(sortedTags, `tags-${group.source}`);

    return html`
      ${pagedTags.map(([tag, count]) => {
        const tagUrl = `/${group.urlSource}/${encodeURIComponent(tag)}/`;
        const isFocused = this._focusedPath === `${group.urlSource}/${tag}`;
        return html`
          <a
            href="${resolveUrl(tagUrl)}"
            class="tag-pill ${isFocused ? 'focused' : ''}"
            title="${tag} (${count})"
          >
            <span class="tag-name">${tag}</span>
            <span class="tag-count">${count}</span>
          </a>
        `;
      })}
      ${this._hasMoreItems(sortedTags, `tags-${group.source}`) ? html`
        <button class="show-more-tags" @click=${() => this._showMore(`tags-${group.source}`)}>
          +${sortedTags.length - pagedTags.length} more
        </button>
      ` : nothing}
    `;
  }

  private _renderLoading(): TemplateResult {
    return html`
      <div class="loading" aria-busy="true">
        <p>Loading navigation...</p>
      </div>
    `;
  }

  private _renderError(): TemplateResult {
    return html`
      <div class="error">
        <p>Failed to load navigation</p>
        <p class="error-detail">${this._loadError}</p>
      </div>
    `;
  }

  // ========================================
  // Styles
  // ========================================

  static override styles = css`
    :host {
      display: block;
    }

    /* ==================== Drawer (Mobile Overlay) ==================== */

    .drawer-backdrop {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.4);
      z-index: 1000;
      animation: fadeIn 0.2s ease;
    }

    @keyframes fadeIn {
      from { opacity: 0; }
      to { opacity: 1; }
    }

    .drawer {
      position: fixed;
      left: 0;
      top: 0;
      height: 100%;
      width: min(85vw, 320px);
      background: var(--pico-background-color, #fff);
      box-shadow: 4px 0 12px rgba(0, 0, 0, 0.15);
      animation: slideIn 0.25s ease;
      display: flex;
      flex-direction: column;
      overflow: hidden;
    }

    @keyframes slideIn {
      from { transform: translateX(-100%); }
      to { transform: translateX(0); }
    }

    /* ==================== Sidebar (Desktop Inline) ==================== */

    .sidebar {
      width: var(--mbr-sidebar-width, 280px);
      height: 100%;
      background: var(--pico-background-color, #fff);
      border-right: 1px solid var(--pico-muted-border-color, #eee);
      display: flex;
      flex-direction: column;
      overflow: hidden;
      position: sticky;
      top: 0;
      align-self: start;
      max-height: 100vh;
    }

    .sidebar:focus {
      outline: none;
    }

    /* ==================== Sidebar Header ==================== */

    .sidebar-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0.75rem 1rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    .sidebar-header h2 {
      margin: 0;
      font-size: 0.9rem;
      font-weight: 600;
      color: var(--pico-muted-color, #666);
      text-transform: uppercase;
      letter-spacing: 0.05em;
    }

    .close-button {
      background: transparent;
      border: none;
      font-size: 1.5rem;
      color: var(--pico-muted-color, #999);
      cursor: pointer;
      padding: 0.25rem;
      line-height: 1;
    }

    .close-button:hover {
      color: var(--pico-color, #333);
    }

    /* ==================== Sidebar Navigation ==================== */

    .sidebar-nav {
      flex: 1;
      overflow-y: auto;
      padding: 0.5rem 0;
    }

    /* ==================== Navigation Sections ==================== */

    .nav-section {
      padding: 0.25rem 0;
    }

    .nav-section + .nav-section {
      border-top: 1px solid var(--pico-muted-border-color, #eee);
      margin-top: 0.5rem;
      padding-top: 0.75rem;
    }

    .section-header {
      font-size: 0.75rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      color: var(--pico-muted-color, #666);
      padding: 0.5rem 1rem;
      margin: 0;
    }

    .section-header a {
      color: inherit;
      text-decoration: none;
    }

    .section-header a:hover {
      color: var(--pico-primary, #0d6efd);
    }

    /* ==================== Navigation Items (picocss.com style) ==================== */

    .nav-item {
      display: flex;
      align-items: center;
      padding: 0.35rem 0.75rem;
      padding-left: calc(0.75rem + var(--depth, 0) * 1rem);
      color: var(--pico-secondary, rgb(93, 107, 137));
      text-decoration: none;
      font-size: 0.875rem;
      transition: color 0.15s ease, border-color 0.15s ease;
      position: relative;
      gap: 0.25rem;
      border-left: 1px solid transparent;
      margin-left: 0.5rem;
    }

    .nav-item:hover {
      color: var(--pico-secondary-hover, #4a5568);
      border-left-color: var(--pico-secondary-hover, #4a5568);
    }

    .nav-item.current {
      color: var(--pico-primary, #0d6efd);
      border-left-color: var(--pico-primary, #0d6efd);
    }

    .nav-item.focused {
      outline: 2px solid var(--pico-primary, #0d6efd);
      outline-offset: -2px;
    }

    /* ==================== Folder Items ==================== */

    .folder-tree-item {
      display: flex;
      flex-direction: column;
    }

    .folder-item {
      padding-left: calc(0.25rem + var(--depth, 0) * 1rem);
    }

    .expand-toggle {
      width: 1.25rem;
      height: 1.25rem;
      background: transparent;
      border: none;
      cursor: pointer;
      color: var(--pico-muted-color, #999);
      display: flex;
      align-items: center;
      justify-content: center;
      flex-shrink: 0;
      padding: 0;
      transition: transform 0.15s ease, color 0.15s ease;
    }

    .expand-toggle:hover {
      color: var(--pico-primary, #0d6efd);
    }

    .expand-toggle.expanded {
      transform: rotate(90deg);
    }

    .expand-toggle svg {
      width: 12px;
      height: 12px;
    }

    .expand-spacer {
      width: 1.25rem;
      flex-shrink: 0;
    }

    .folder-link {
      display: flex;
      align-items: center;
      gap: 0.25rem;
      color: inherit;
      text-decoration: none;
      flex: 1;
      min-width: 0;
    }

    .folder-link:hover {
      color: var(--pico-primary, #0d6efd);
    }

    .nav-label {
      flex: 1;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    /* Folders (have children) are bold */
    .folder-item .nav-label,
    .folder-link .nav-label {
      font-weight: 600;
    }

    /* Files/pages (no children) are normal weight */
    .file-item .nav-label {
      font-weight: 400;
    }

    .folder-children {
      display: flex;
      flex-direction: column;
    }

    /* ==================== File Items ==================== */

    .file-item {
      padding-left: calc(0.25rem + var(--depth, 0) * 1rem);
    }

    /* ==================== Home Link ==================== */

    .home-link {
      font-weight: 600;
    }

    /* ==================== Tag Pills ==================== */

    .tag-section {
      padding: 0.5rem 1rem;
    }

    .tag-pills {
      display: flex;
      flex-wrap: wrap;
      gap: 0.35rem;
    }

    .tag-pill {
      display: inline-flex;
      align-items: center;
      gap: 0.25rem;
      padding: 0.2rem 0.5rem;
      background: var(--pico-card-background-color, #f8f9fa);
      border: 1px solid var(--pico-muted-border-color, #e0e0e0);
      border-radius: 1rem;
      font-size: 0.75rem;
      color: var(--pico-color, #333);
      text-decoration: none;
      max-width: 200px;
      transition: background 0.15s ease, color 0.15s ease, border-color 0.15s ease;
    }

    .tag-pill:hover {
      background: var(--pico-primary-background, #e3f2fd);
      color: var(--pico-primary, #0d6efd);
    }

    .tag-pill.focused {
      outline: 2px solid var(--pico-primary, #0d6efd);
      outline-offset: -1px;
    }

    .tag-name {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .tag-count {
      font-size: 0.65rem;
      opacity: 0.7;
    }

    /* ==================== Show More Buttons ==================== */

    .show-more,
    .show-more-tags {
      display: block;
      width: 100%;
      padding: 0.5rem 1rem;
      background: transparent;
      border: none;
      cursor: pointer;
      text-align: left;
      color: var(--pico-primary, #0d6efd);
      font-size: 0.8rem;
    }

    .show-more:hover,
    .show-more-tags:hover {
      text-decoration: underline;
    }

    .show-more-tags {
      display: inline;
      width: auto;
      padding: 0.2rem 0.5rem;
      background: var(--pico-muted-border-color, #e0e0e0);
      border-radius: 1rem;
      font-size: 0.7rem;
    }

    /* ==================== Loading / Error States ==================== */

    .loading,
    .error {
      padding: 2rem 1rem;
      text-align: center;
    }

    .loading p {
      color: var(--pico-muted-color, #666);
    }

    .error p {
      color: var(--pico-del-color, #dc3545);
      margin-bottom: 0.5rem;
    }

    .error-detail {
      color: var(--pico-muted-color, #666);
      font-size: 0.875rem;
    }
  `;
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-browse-single': MbrBrowseSingleElement
  }
}
