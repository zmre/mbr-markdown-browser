import { LitElement, css, html, nothing, type TemplateResult } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { subscribeSiteNav } from './shared.js'
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
 * Hierarchical tag node for nested tag support (e.g., tech/rust/async).
 */
interface HierarchicalTag {
  name: string;           // Full tag path
  displayName: string;    // Leaf name for display
  count: number;
  children: Map<string, HierarchicalTag>;
}

/**
 * Selection state for middle pane filtering.
 */
type SelectionType = 'tag' | 'folder' | 'frontmatter' | 'recent' | 'shortcuts';

interface Selection {
  type: SelectionType;
  value: string;
  label: string;
}

/**
 * localStorage keys and limits for recent files and shortcuts.
 */
const RECENT_KEY = 'mbr_recent_files';
const SHORTCUTS_KEY = 'mbr_shortcuts';
const MAX_RECENT = 30;
const RECENT_VIEWED_LIMIT = 15;

/**
 * Three-pane navigator component for MBR.
 *
 * When opened, displays:
 * - Left pane: Recent, Shortcuts, Tags tree, Notes tree, Dynamic frontmatter
 * - Middle pane: File list when a selection is made
 * - Right: Content shows through (transparent)
 *
 * Keyboard: - to open, Escape to close, arrows to navigate
 */
@customElement('mbr-browse')
export class MbrBrowseElement extends LitElement {
  // === Visibility State ===
  @state()
  private _isOpen = false;

  @state()
  private _showMiddlePane = false;

  // === Data State ===
  @state()
  private _allFiles: MarkdownFile[] = [];

  @state()
  private _isLoading = true;

  @state()
  private _loadError: string | null = null;

  // === Derived Data ===
  @state()
  private _tagHierarchy: Map<string, HierarchicalTag> = new Map();

  @state()
  private _folderTree: FolderNode | null = null;

  @state()
  private _dynamicFields: Map<string, Set<string>> = new Map();

  // === Selection State ===
  @state()
  private _currentSelection: Selection | null = null;

  @state()
  private _selectedFiles: MarkdownFile[] = [];

  // === UI State ===
  @state()
  private _expandedSections = new Set<string>(['notes']);

  @state()
  private _expandedTags = new Set<string>();

  @state()
  private _expandedFolders = new Set<string>();

  @state()
  private _activePaneIndex = 0;  // 0 = left, 1 = middle

  /** The configured index file name from site.json */
  private _indexFile: string = 'index.md';

  /** Sort configuration from site.json */
  private _sortConfig: SortField[] = DEFAULT_SORT_CONFIG;

  private _keyboardHandler: ((e: KeyboardEvent) => void) | null = null;
  private _unsubscribeSiteNav: (() => void) | null = null;

  // ========================================
  // Lifecycle
  // ========================================

  override connectedCallback() {
    super.connectedCallback();

    // Track current page as recently viewed
    const currentPath = window.location.pathname;
    if (currentPath && currentPath !== '/') {
      this._addToRecent(currentPath);
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

        // Load sort configuration if available
        if (state.data.sort && Array.isArray(state.data.sort) && state.data.sort.length > 0) {
          this._sortConfig = state.data.sort;
        }

        // Build derived data structures
        this._tagHierarchy = this._buildTagHierarchy(this._allFiles);
        this._folderTree = buildFolderTree(this._allFiles, this._indexFile);
        this._dynamicFields = this._detectDynamicFields(this._allFiles);

        // Auto-expand current path in folder tree
        this._autoExpandCurrentPath(currentPath);
      }
    });

    // Setup keyboard event listener
    this._keyboardHandler = (e: KeyboardEvent) => {
      // Open with '-' key (when not in input field)
      if (e.key === '-' && !this._isInputTarget(e.target)) {
        e.preventDefault();
        this.toggle();
        return;
      }

      // Open with F2 key
      if (e.key === 'F2') {
        e.preventDefault();
        this.toggle();
        return;
      }

      // Handle keys when open
      if (this._isOpen) {
        // Close with Escape - middle pane first, then navigator
        if (e.key === 'Escape') {
          e.preventDefault();
          if (this._showMiddlePane) {
            this._clearSelection();
          } else {
            this.close();
          }
          return;
        }

        this._handlePanelKeydown(e);
      }
    };

    document.addEventListener('keydown', this._keyboardHandler);
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    if (this._keyboardHandler) {
      document.removeEventListener('keydown', this._keyboardHandler);
    }
    if (this._unsubscribeSiteNav) {
      this._unsubscribeSiteNav();
    }
  }

  // ========================================
  // Public Methods
  // ========================================

  public open() {
    this._isOpen = true;
  }

  public close() {
    this._isOpen = false;
    this._showMiddlePane = false;
    this._currentSelection = null;
  }

  public toggle() {
    if (this._isOpen) {
      this.close();
    } else {
      this.open();
    }
  }

  // ========================================
  // localStorage Helpers
  // ========================================

  private _getRecentViewed(): string[] {
    try {
      const stored = localStorage.getItem(RECENT_KEY);
      return stored ? JSON.parse(stored) : [];
    } catch {
      return [];
    }
  }

  private _addToRecent(urlPath: string) {
    try {
      const recent = this._getRecentViewed().filter(p => p !== urlPath);
      recent.unshift(urlPath);
      localStorage.setItem(RECENT_KEY, JSON.stringify(recent.slice(0, MAX_RECENT)));
    } catch {
      // Ignore localStorage errors
    }
  }

  private _getShortcuts(): string[] {
    try {
      const stored = localStorage.getItem(SHORTCUTS_KEY);
      return stored ? JSON.parse(stored) : [];
    } catch {
      return [];
    }
  }

  // ========================================
  // Data Building
  // ========================================

  /**
   * Build hierarchical tag structure from files.
   * Tags with "/" are split into hierarchy (e.g., "tech/rust" -> tech > rust).
   */
  private _buildTagHierarchy(files: MarkdownFile[]): Map<string, HierarchicalTag> {
    const root = new Map<string, HierarchicalTag>();
    const tagCounts = new Map<string, number>();

    // First pass: count all tags
    for (const file of files) {
      const tags = this._extractFileTags(file);
      for (const tag of tags) {
        tagCounts.set(tag, (tagCounts.get(tag) || 0) + 1);
      }
    }

    // Second pass: build hierarchy
    for (const [fullTag, count] of tagCounts) {
      const parts = fullTag.split('/').filter(p => p.length > 0);
      let currentLevel = root;
      let currentPath = '';

      for (let i = 0; i < parts.length; i++) {
        const part = parts[i];
        currentPath = currentPath ? `${currentPath}/${part}` : part;
        const isLeaf = i === parts.length - 1;

        if (!currentLevel.has(part)) {
          currentLevel.set(part, {
            name: currentPath,
            displayName: part,
            count: 0,
            children: new Map(),
          });
        }

        const node = currentLevel.get(part)!;
        if (isLeaf) {
          node.count = count;
        }

        currentLevel = node.children;
      }
    }

    return root;
  }

  /**
   * Detect frontmatter fields that appear commonly with enumerable values.
   */
  private _detectDynamicFields(files: MarkdownFile[]): Map<string, Set<string>> {
    const fieldStats = new Map<string, { count: number; values: Map<string, number> }>();
    const skipFields = new Set(['title', 'description', 'summary', 'tags', 'tag', 'keywords',
      'date', 'created', 'modified', 'draft', 'published', 'aliases', 'slug', 'weight']);

    for (const file of files) {
      if (!file.frontmatter) continue;

      for (const [key, value] of Object.entries(file.frontmatter)) {
        if (skipFields.has(key)) continue;
        if (typeof value === 'object' && value !== null) continue;
        if (value === null || value === undefined) continue;

        const stringValue = String(value);
        if (stringValue.length > 100) continue;

        if (!fieldStats.has(key)) {
          fieldStats.set(key, { count: 0, values: new Map() });
        }

        const stats = fieldStats.get(key)!;
        stats.count++;
        stats.values.set(stringValue, (stats.values.get(stringValue) || 0) + 1);
      }
    }

    // Filter: count >= 10, 2 <= unique values < 50
    const result = new Map<string, Set<string>>();

    for (const [field, stats] of fieldStats) {
      if (stats.count < 10) continue;
      if (stats.values.size < 2 || stats.values.size >= 50) continue;

      // Skip if average value length is too long (likely free-form text)
      const avgLength = [...stats.values.keys()].reduce((sum, v) => sum + v.length, 0) / stats.values.size;
      if (avgLength > 50) continue;

      result.set(field, new Set(stats.values.keys()));
    }

    return result;
  }

  /**
   * Extract all tags from a file's frontmatter.
   */
  private _extractFileTags(file: MarkdownFile): string[] {
    if (!file.frontmatter) return [];

    const tags: string[] = [];
    const tagFields = [
      file.frontmatter.tags,
      file.frontmatter.tag,
      file.frontmatter.keywords,
      file.frontmatter.category,
      file.frontmatter.categories,
    ];

    for (const value of tagFields) {
      if (!value) continue;
      const parsed = Array.isArray(value)
        ? value
        : String(value).split(',').map(t => t.trim());

      for (const tag of parsed) {
        const normalized = tag.trim();
        if (normalized.length > 0 && !tags.includes(normalized)) {
          tags.push(normalized);
        }
      }
    }

    return tags;
  }

  // ========================================
  // Selection Handlers
  // ========================================

  private _selectTag(tagName: string, label: string) {
    this._currentSelection = { type: 'tag', value: tagName, label };
    this._showMiddlePane = true;
    this._updateSelectedFiles();
    this._activePaneIndex = 1;
  }

  private _selectFolder(folderPath: string, label: string) {
    this._currentSelection = { type: 'folder', value: folderPath, label };
    this._showMiddlePane = true;
    this._updateSelectedFiles();
    this._activePaneIndex = 1;
  }

  private _selectFrontmatter(field: string, value: string) {
    this._currentSelection = {
      type: 'frontmatter',
      value: `${field}:${value}`,
      label: `${this._formatFieldName(field)}: ${value}`
    };
    this._showMiddlePane = true;
    this._updateSelectedFiles();
    this._activePaneIndex = 1;
  }

  private _selectRecent() {
    this._currentSelection = { type: 'recent', value: '', label: 'Recent Files' };
    this._showMiddlePane = true;
    this._updateSelectedFiles();
    this._activePaneIndex = 1;
  }

  private _clearSelection() {
    this._currentSelection = null;
    this._showMiddlePane = false;
    this._selectedFiles = [];
    this._activePaneIndex = 0;
  }

  private _updateSelectedFiles() {
    if (!this._currentSelection) {
      this._selectedFiles = [];
      return;
    }

    let files: MarkdownFile[] = [];

    switch (this._currentSelection.type) {
      case 'tag': {
        const selectedTag = this._currentSelection.value;
        files = this._allFiles.filter(f => {
          const tags = this._extractFileTags(f);
          // Match exact tag or tag hierarchy (e.g., "tech" matches "tech/rust")
          return tags.some(t => t === selectedTag || t.startsWith(selectedTag + '/'));
        });
        break;
      }

      case 'folder': {
        const folderPath = this._currentSelection.value;
        // Filter to only direct children of this folder (not descendants)
        files = this._allFiles.filter(f => {
          if (!f.url_path.startsWith(folderPath)) return false;
          // Get the path relative to the folder
          const relativePath = f.url_path.slice(folderPath.length);
          const parts = relativePath.split('/').filter(p => p.length > 0);

          // parts.length === 0: This folder's own index file (show it)
          // parts.length === 1: Direct child file or subfolder's index
          if (parts.length > 1) return false;  // Too deep, not direct child

          if (parts.length === 1) {
            // Direct child - but check if it's a subfolder's index file
            const fileName = f.raw_path.split('/').pop() || '';
            if (fileName === this._indexFile) {
              // This is a subfolder's index file - don't show as file
              // (it's represented by the folder in the tree)
              return false;
            }
          }

          return true;
        });
        break;
      }

      case 'frontmatter': {
        const [field, value] = this._currentSelection.value.split(':');
        files = this._allFiles.filter(f =>
          f.frontmatter && String(f.frontmatter[field]) === value
        );
        break;
      }

      case 'recent': {
        files = this._getBlendedRecent();
        break;
      }

      case 'shortcuts': {
        const shortcuts = this._getShortcuts();
        files = shortcuts
          .map(url => this._allFiles.find(f => f.url_path === url))
          .filter((f): f is MarkdownFile => f !== undefined);
        break;
      }
    }

    // Sort files using configurable sort order (except recent which is already sorted)
    if (this._currentSelection.type !== 'recent') {
      files = sortFiles(files, this._sortConfig);
    }

    this._selectedFiles = files;
  }

  /**
   * Get blended recent files from localStorage viewed + site.json modified.
   */
  private _getBlendedRecent(): MarkdownFile[] {
    const viewedUrls = this._getRecentViewed();
    const viewedFiles = viewedUrls
      .map(url => this._allFiles.find(f => f.url_path === url))
      .filter((f): f is MarkdownFile => f !== undefined)
      .slice(0, RECENT_VIEWED_LIMIT);

    const seenUrls = new Set(viewedFiles.map(f => f.url_path));

    const modifiedRecent = [...this._allFiles]
      .sort((a, b) => b.modified - a.modified)
      .filter(f => !seenUrls.has(f.url_path))
      .slice(0, RECENT_VIEWED_LIMIT);

    return [...viewedFiles, ...modifiedRecent].slice(0, MAX_RECENT);
  }

  // ========================================
  // Utility Methods
  // ========================================

  private _formatFieldName(field: string): string {
    return field.split(/[_-]/).map(w =>
      w.charAt(0).toUpperCase() + w.slice(1)
    ).join(' ');
  }

  private _getParentFolder(urlPath: string): string {
    const parts = urlPath.split('/').filter(p => p.length > 0);
    if (parts.length <= 1) return '/';
    return parts[parts.length - 2];
  }

  private _formatDate(timestamp: number): string {
    return new Date(timestamp * 1000).toLocaleDateString('en-US', {
      month: 'short',
      day: 'numeric',
      year: 'numeric'
    });
  }

  private _isInputTarget(target: EventTarget | null): boolean {
    if (!target || !(target instanceof HTMLElement)) return false;
    const tagName = target.tagName.toLowerCase();
    return tagName === 'input' || tagName === 'textarea' || target.isContentEditable;
  }

  private _autoExpandCurrentPath(path: string) {
    const parts = path.split('/').filter(p => p.length > 0);
    let accumulated = '';

    // Expand all parent folders
    for (const part of parts) {
      accumulated += '/' + part;
      this._expandedFolders.add(accumulated);
    }

    this._expandedFolders.add('/');
    this._expandedFolders = new Set(this._expandedFolders);

    // Auto-select the current folder (path already ends with / for folders)
    const folderPath = path.endsWith('/') ? path : path + '/';

    // Find the folder node to get its title
    let folderTitle = 'Home';
    if (this._folderTree) {
      if (folderPath === '/') {
        folderTitle = this._folderTree.title || 'Home';
      } else {
        const node = this._findFolderNode(this._folderTree, folderPath);
        if (node) {
          folderTitle = node.title || node.name;
        }
      }
    }

    this._currentSelection = {
      type: 'folder',
      value: folderPath,
      label: folderTitle,
    };
  }

  private _findFolderNode(node: FolderNode, path: string): FolderNode | null {
    if (node.path === path) {
      return node;
    }
    for (const child of node.children.values()) {
      const found = this._findFolderNode(child, path);
      if (found) return found;
    }
    return null;
  }

  private _toggleSection(section: string) {
    const newExpanded = new Set(this._expandedSections);
    if (newExpanded.has(section)) {
      newExpanded.delete(section);
    } else {
      newExpanded.add(section);
    }
    this._expandedSections = newExpanded;
  }

  private _toggleTag(tagPath: string) {
    const newExpanded = new Set(this._expandedTags);
    if (newExpanded.has(tagPath)) {
      newExpanded.delete(tagPath);
    } else {
      newExpanded.add(tagPath);
    }
    this._expandedTags = newExpanded;
  }

  private _toggleFolder(path: string) {
    const newExpanded = new Set(this._expandedFolders);
    if (newExpanded.has(path)) {
      newExpanded.delete(path);
    } else {
      newExpanded.add(path);
    }
    this._expandedFolders = newExpanded;
  }

  private _isCurrentPath(path: string): boolean {
    const currentPath = window.location.pathname;
    const normalizedCurrent = currentPath.endsWith('/') ? currentPath : currentPath + '/';
    const normalizedPath = path.endsWith('/') ? path : path + '/';
    return normalizedCurrent === normalizedPath;
  }

  // ========================================
  // Keyboard Navigation
  // ========================================

  private _handlePanelKeydown(e: KeyboardEvent) {
    if (this._isInputTarget(e.target)) return;

    // Tab switches between panes
    if (e.key === 'Tab' && this._showMiddlePane) {
      e.preventDefault();
      this._activePaneIndex = this._activePaneIndex === 0 ? 1 : 0;
      return;
    }

    // Ctrl key combinations for scrolling
    if (e.ctrlKey) {
      const key = e.key.toLowerCase();
      const panelContent = this.shadowRoot?.querySelector(
        this._activePaneIndex === 0 ? '.left-pane' : '.middle-pane'
      );
      if (!panelContent) return;

      const halfPage = panelContent.clientHeight / 2;
      const fullPage = panelContent.clientHeight - 50;

      switch (key) {
        case 'd':
          e.preventDefault();
          panelContent.scrollBy({ top: halfPage, behavior: 'smooth' });
          return;
        case 'u':
          e.preventDefault();
          panelContent.scrollBy({ top: -halfPage, behavior: 'smooth' });
          return;
        case 'f':
          e.preventDefault();
          panelContent.scrollBy({ top: fullPage, behavior: 'smooth' });
          return;
        case 'b':
          e.preventDefault();
          panelContent.scrollBy({ top: -fullPage, behavior: 'smooth' });
          return;
      }
    }
  }

  // ========================================
  // Render Methods
  // ========================================

  override render() {
    if (!this._isOpen) return nothing;

    return html`
      <div class="navigator-backdrop" @click=${this.close}>
        <div class="navigator-container" @click=${(e: Event) => e.stopPropagation()}>
          ${this._renderLeftPane()}
          ${this._showMiddlePane ? this._renderMiddlePane() : nothing}
        </div>
      </div>
    `;
  }

  private _renderLeftPane(): TemplateResult {
    return html`
      <aside class="left-pane">
        <div class="pane-header">
          <h2>Navigate</h2>
          <button class="close-button" @click=${this.close} aria-label="Close">
            ✕
          </button>
        </div>

        <div class="pane-content">
          ${this._isLoading ? this._renderLoading() :
            this._loadError ? this._renderError() : html`
            ${this._renderRecentSection()}
            ${this._renderShortcutsSection()}
            ${this._renderTagsSection()}
            ${this._renderNotesSection()}
            ${this._renderDynamicSections()}
          `}
        </div>
      </aside>
    `;
  }

  private _renderRecentSection(): TemplateResult {
    const isExpanded = this._expandedSections.has('recent');
    const recentFiles = this._getBlendedRecent();

    return html`
      <div class="nav-section">
        <button
          class="section-header"
          @click=${() => this._toggleSection('recent')}
          aria-expanded=${isExpanded}
        >
          <span class="toggle-icon">${isExpanded ? '▼' : '▶'}</span>
          <span class="section-title">Recent</span>
          <span class="section-count">${recentFiles.length}</span>
        </button>
        ${isExpanded ? html`
          <div class="section-content">
            ${recentFiles.slice(0, 5).map(file => this._renderCompactFile(file))}
            ${recentFiles.length > 5 ? html`
              <button class="show-more" @click=${() => this._selectRecent()}>
                Show all ${recentFiles.length}...
              </button>
            ` : nothing}
          </div>
        ` : nothing}
      </div>
    `;
  }

  private _renderShortcutsSection(): TemplateResult | typeof nothing {
    const isExpanded = this._expandedSections.has('shortcuts');
    const shortcuts = this._getShortcuts();

    if (shortcuts.length === 0) {
      return nothing;
    }

    const shortcutFiles = shortcuts
      .map(url => this._allFiles.find(f => f.url_path === url))
      .filter((f): f is MarkdownFile => f !== undefined);

    return html`
      <div class="nav-section">
        <button
          class="section-header"
          @click=${() => this._toggleSection('shortcuts')}
          aria-expanded=${isExpanded}
        >
          <span class="toggle-icon">${isExpanded ? '▼' : '▶'}</span>
          <span class="section-title">Shortcuts</span>
          <span class="section-count">${shortcutFiles.length}</span>
        </button>
        ${isExpanded ? html`
          <div class="section-content">
            ${shortcutFiles.map(file => this._renderCompactFile(file))}
          </div>
        ` : nothing}
      </div>
    `;
  }

  private _renderTagsSection(): TemplateResult | typeof nothing {
    // Hide section if no tags exist
    if (this._tagHierarchy.size === 0) {
      return nothing;
    }

    const isExpanded = this._expandedSections.has('tags');

    return html`
      <div class="nav-section">
        <button
          class="section-header"
          @click=${() => this._toggleSection('tags')}
          aria-expanded=${isExpanded}
        >
          <span class="toggle-icon">${isExpanded ? '▼' : '▶'}</span>
          <span class="section-title">Tags</span>
        </button>
        ${isExpanded ? html`
          <div class="section-content tree-content">
            ${this._renderTagTree(this._tagHierarchy, 0)}
          </div>
        ` : nothing}
      </div>
    `;
  }

  private _renderTagTree(tags: Map<string, HierarchicalTag>, depth: number): TemplateResult[] {
    const sorted = [...tags.values()].sort((a, b) =>
      a.displayName.localeCompare(b.displayName)
    );

    return sorted.map(tag => {
      const hasChildren = tag.children.size > 0;
      const isExpanded = this._expandedTags.has(tag.name);
      const isSelected = this._currentSelection?.type === 'tag' &&
                         this._currentSelection?.value === tag.name;

      return html`
        <div class="tree-item tag-item">
          <div
            class="tree-row ${isSelected ? 'selected' : ''}"
            style="padding-left: ${depth * 0.75 + 0.25}rem"
          >
            ${hasChildren ? html`
              <button
                class="tree-toggle"
                @click=${(e: Event) => { e.stopPropagation(); this._toggleTag(tag.name); }}
              >
                ${isExpanded ? '▼' : '▶'}
              </button>
            ` : html`<span class="tree-spacer"></span>`}
            <button
              class="tree-label"
              @click=${() => this._selectTag(tag.name, tag.displayName)}
            >
              <span class="label-text">${tag.displayName}</span>
              <span class="label-count">${tag.count}</span>
            </button>
          </div>
          ${hasChildren && isExpanded ? html`
            <div class="tree-children">
              ${this._renderTagTree(tag.children, depth + 1)}
            </div>
          ` : nothing}
        </div>
      `;
    });
  }

  private _renderNotesSection(): TemplateResult {
    const isExpanded = this._expandedSections.has('notes');

    return html`
      <div class="nav-section">
        <button
          class="section-header"
          @click=${() => this._toggleSection('notes')}
          aria-expanded=${isExpanded}
        >
          <span class="toggle-icon">${isExpanded ? '▼' : '▶'}</span>
          <span class="section-title">Notes</span>
          <span class="section-count">${this._folderTree?.fileCount || 0}</span>
        </button>
        ${isExpanded && this._folderTree ? html`
          <div class="section-content tree-content">
            ${this._renderFolderTree(this._folderTree, 0, true)}
          </div>
        ` : nothing}
      </div>
    `;
  }

  private _renderFolderTree(node: FolderNode, depth: number, isRoot: boolean = false): TemplateResult {
    const hasChildren = node.children.size > 0;
    const isExpanded = this._expandedFolders.has(node.path) || isRoot;
    const isSelected = this._currentSelection?.type === 'folder' &&
                       this._currentSelection?.value === node.path;
    const isCurrent = this._isCurrentPath(node.path);

    // For root, render "Home" entry with children indented under it
    if (isRoot) {
      const homeLabel = node.title || 'Home';
      return html`
        <div class="tree-item folder-item">
          <div
            class="tree-row ${isSelected ? 'selected' : ''} ${isCurrent ? 'current' : ''}"
            style="padding-left: 0.25rem"
          >
            ${hasChildren ? html`
              <button
                class="tree-toggle"
                @click=${(e: Event) => { e.stopPropagation(); this._toggleFolder(node.path); }}
              >
                ${isExpanded ? '▼' : '▶'}
              </button>
            ` : html`<span class="tree-spacer"></span>`}
            <button
              class="tree-label"
              @click=${() => this._selectFolder(node.path, homeLabel)}
            >
              <span class="folder-icon">📁</span>
              <span class="label-text">${homeLabel}</span>
              <span class="label-count">${node.fileCount}</span>
            </button>
          </div>
        </div>
        ${isExpanded ? html`
          ${sortFolders([...node.children.values()], this._sortConfig)
            .map(child => this._renderFolderTree(child, 1))}
        ` : nothing}
      `;
    }

    // Non-root folder rendering
    return html`
      <div class="tree-item folder-item">
        <div
          class="tree-row ${isSelected ? 'selected' : ''} ${isCurrent ? 'current' : ''}"
          style="padding-left: ${depth * 0.75 + 0.25}rem"
        >
          ${hasChildren ? html`
            <button
              class="tree-toggle"
              @click=${(e: Event) => { e.stopPropagation(); this._toggleFolder(node.path); }}
            >
              ${isExpanded ? '▼' : '▶'}
            </button>
          ` : html`<span class="tree-spacer"></span>`}
          <button
            class="tree-label"
            @click=${() => this._selectFolder(node.path, node.title || node.name)}
          >
            <span class="folder-icon">📁</span>
            <span class="label-text">${node.title || node.name}</span>
            <span class="label-count">${node.fileCount}</span>
          </button>
        </div>
      </div>
      ${isExpanded ? html`
        ${sortFolders([...node.children.values()], this._sortConfig)
          .map(child => this._renderFolderTree(child, depth + 1))}
      ` : nothing}
    `;
  }

  private _renderDynamicSections(): TemplateResult | typeof nothing {
    if (this._dynamicFields.size === 0) return nothing;

    return html`
      ${[...this._dynamicFields.entries()].map(([fieldName, values]) => {
        const sectionKey = `fm_${fieldName}`;
        const isExpanded = this._expandedSections.has(sectionKey);
        const sortedValues = [...values].sort();

        return html`
          <div class="nav-section">
            <button
              class="section-header"
              @click=${() => this._toggleSection(sectionKey)}
              aria-expanded=${isExpanded}
            >
              <span class="toggle-icon">${isExpanded ? '▼' : '▶'}</span>
              <span class="section-title">${this._formatFieldName(fieldName)}</span>
            </button>
            ${isExpanded ? html`
              <div class="section-content">
                ${sortedValues.map(value => {
                  const count = this._allFiles.filter(f =>
                    f.frontmatter && String(f.frontmatter[fieldName]) === value
                  ).length;
                  const isSelected = this._currentSelection?.type === 'frontmatter' &&
                                     this._currentSelection?.value === `${fieldName}:${value}`;
                  return html`
                    <button
                      class="frontmatter-value ${isSelected ? 'selected' : ''}"
                      @click=${() => this._selectFrontmatter(fieldName, value)}
                    >
                      <span class="value-name">${value}</span>
                      <span class="value-count">${count}</span>
                    </button>
                  `;
                })}
              </div>
            ` : nothing}
          </div>
        `;
      })}
    `;
  }

  private _renderCompactFile(file: MarkdownFile): TemplateResult {
    const title = file.frontmatter?.title ||
      file.url_path.split('/').filter(p => p).pop() || 'Untitled';

    return html`
      <a href="${file.url_path}" class="compact-file">
        <span class="compact-title">${title}</span>
      </a>
    `;
  }

  private _renderMiddlePane(): TemplateResult {
    return html`
      <div class="middle-pane">
        <div class="pane-header">
          <h3>${this._currentSelection?.label || 'Files'}</h3>
          <button class="close-button" @click=${() => this._clearSelection()} aria-label="Close">
            ✕
          </button>
        </div>

        <div class="pane-content file-list">
          ${this._selectedFiles.length === 0 ? html`
            <div class="no-results">No files found</div>
          ` : this._selectedFiles.map(file => this._renderFileCard(file))}
        </div>
      </div>
    `;
  }

  private _renderFileCard(file: MarkdownFile): TemplateResult {
    const filename = file.raw_path.split('/').pop() || '';
    const title = file.frontmatter?.title ||
      file.url_path.split('/').filter(p => p).pop() || 'Untitled';
    const description = file.frontmatter?.description ||
      file.frontmatter?.summary || '';
    const tags = this._extractFileTags(file);
    const modifiedDate = this._formatDate(file.modified);
    const parentFolder = this._getParentFolder(file.url_path);
    const isCurrent = this._isCurrentPath(file.url_path);

    return html`
      <a href="${file.url_path}" class="file-card ${isCurrent ? 'current' : ''}">
        <div class="file-filename">${filename}</div>
        <div class="file-title">${title}</div>
        ${description ? html`
          <div class="file-description">${description}</div>
        ` : nothing}
        ${tags.length > 0 ? html`
          <div class="file-tags">
            ${tags.slice(0, 5).map(tag => html`
              <span class="tag-pill">${tag}</span>
            `)}
            ${tags.length > 5 ? html`
              <span class="tag-more">+${tags.length - 5}</span>
            ` : nothing}
          </div>
        ` : nothing}
        <div class="file-meta">
          <span class="file-date">${modifiedDate}</span>
          <span class="file-folder">${parentFolder}</span>
        </div>
      </a>
    `;
  }

  private _renderLoading(): TemplateResult {
    return html`
      <div class="loading-container" aria-busy="true">
        <p class="loading-text">Loading site data...</p>
      </div>
    `;
  }

  private _renderError(): TemplateResult {
    return html`
      <div class="error-container">
        <p class="error-text">Failed to load site data</p>
        <p class="error-detail">${this._loadError}</p>
      </div>
    `;
  }

  // ========================================
  // Styles
  // ========================================

  static override styles = css`
    :host {
      display: contents;
    }

    /* Backdrop */
    .navigator-backdrop {
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

    /* Container */
    .navigator-container {
      position: fixed;
      left: 0;
      top: 0;
      height: 100%;
      display: flex;
      animation: slideIn 0.25s ease;
    }

    @keyframes slideIn {
      from { transform: translateX(-100%); }
      to { transform: translateX(0); }
    }

    /* Left Pane */
    .left-pane {
      width: 280px;
      height: 100%;
      background: var(--pico-background-color, #fff);
      border-right: 1px solid var(--pico-muted-border-color, #eee);
      display: flex;
      flex-direction: column;
      overflow: hidden;
    }

    /* Middle Pane */
    .middle-pane {
      width: 320px;
      height: 100%;
      background: var(--pico-card-background-color, #f8f9fa);
      border-right: 1px solid var(--pico-muted-border-color, #eee);
      display: flex;
      flex-direction: column;
      overflow: hidden;
      animation: slideInMiddle 0.2s ease;
    }

    @keyframes slideInMiddle {
      from { opacity: 0; transform: translateX(-20px); }
      to { opacity: 1; transform: translateX(0); }
    }

    /* Pane Header */
    .pane-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0.75rem 1rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    .pane-header h2,
    .pane-header h3 {
      margin: 0;
      font-size: 1rem;
      font-weight: 600;
      color: var(--pico-color, #333);
      flex: 1;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .close-button {
      background: transparent;
      border: none;
      font-size: 1.25rem;
      color: var(--pico-muted-color, #999);
      cursor: pointer;
      padding: 0.25rem;
      line-height: 1;
    }

    .close-button:hover {
      color: var(--pico-color, #333);
    }

    /* Pane Content */
    .pane-content {
      flex: 1;
      overflow-y: auto;
      padding: 0.5rem 0;
    }

    /* Navigation Sections */
    .nav-section {
      margin-bottom: 0.25rem;
    }

    .section-header {
      display: flex;
      align-items: center;
      width: 100%;
      padding: 0.5rem 1rem;
      background: transparent;
      border: none;
      cursor: pointer;
      text-align: left;
      color: var(--pico-color, #333);
      transition: background 0.15s ease;
    }

    .section-header:hover {
      background: var(--pico-secondary-background, #f5f5f5);
    }

    .toggle-icon {
      width: 1rem;
      font-size: 0.65rem;
      color: var(--pico-muted-color, #999);
    }

    .section-title {
      flex: 1;
      font-weight: 600;
      font-size: 0.8rem;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      color: var(--pico-muted-color, #666);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .section-count {
      font-size: 0.75rem;
      color: var(--pico-muted-color, #999);
      background: var(--pico-secondary-background, #f0f0f0);
      padding: 0.1rem 0.4rem;
      border-radius: 10px;
    }

    .section-content {
      padding: 0.25rem 0;
    }

    .tree-content {
      padding-left: 0.5rem;
    }

    /* Tree Items */
    .tree-item {
      display: flex;
      flex-direction: column;
    }

    .tree-row {
      display: flex;
      align-items: center;
      padding: 0.35rem 0.5rem;
      border-radius: 4px;
      transition: background 0.15s ease;
      min-width: 0;  /* Allow flex children to shrink for ellipsis */
    }

    .tree-row:hover {
      background: var(--pico-secondary-background, #f5f5f5);
    }

    .tree-row.selected {
      background: var(--pico-primary-background, #e3f2fd);
    }

    .tree-row.current {
      border-left: 3px solid var(--pico-primary, #0d6efd);
    }

    .tree-toggle {
      width: 1rem;
      height: 1rem;
      background: transparent;
      border: none;
      cursor: pointer;
      font-size: 0.6rem;
      color: var(--pico-muted-color, #999);
      display: flex;
      align-items: center;
      justify-content: center;
      flex-shrink: 0;
    }

    .tree-spacer {
      width: 1rem;
      flex-shrink: 0;
    }

    .tree-label {
      flex: 1;
      display: flex;
      align-items: center;
      gap: 0.35rem;
      background: transparent;
      border: none;
      cursor: pointer;
      text-align: left;
      padding: 0;
      color: var(--pico-color, #333);
      font-size: 0.875rem;
      min-width: 0;  /* Allow text truncation */
      overflow: hidden;
    }

    .tree-label:hover .label-text {
      color: var(--pico-primary, #0d6efd);
    }

    .folder-icon {
      font-size: 0.9rem;
    }

    .label-text {
      flex: 1;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .label-count {
      font-size: 0.7rem;
      color: var(--pico-muted-color, #999);
      padding-right: 0.25rem;
    }

    /* Compact File Link */
    .compact-file {
      display: block;
      padding: 0.35rem 1rem 0.35rem 1.5rem;
      color: var(--pico-color, #333);
      text-decoration: none;
      font-size: 0.875rem;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .compact-file:hover {
      background: var(--pico-secondary-background, #f5f5f5);
      color: var(--pico-primary, #0d6efd);
    }

    .show-more {
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

    .show-more:hover {
      text-decoration: underline;
    }

    /* Frontmatter Values */
    .frontmatter-value {
      display: flex;
      align-items: center;
      width: 100%;
      padding: 0.35rem 1rem 0.35rem 1.5rem;
      background: transparent;
      border: none;
      cursor: pointer;
      text-align: left;
      color: var(--pico-color, #333);
      font-size: 0.875rem;
    }

    .frontmatter-value:hover {
      background: var(--pico-secondary-background, #f5f5f5);
    }

    .frontmatter-value.selected {
      background: var(--pico-primary-background, #e3f2fd);
    }

    .value-name {
      flex: 1;
    }

    .value-count {
      font-size: 0.7rem;
      color: var(--pico-muted-color, #999);
    }

    /* File List */
    .file-list {
      padding: 0.5rem;
    }

    .file-card {
      display: block;
      padding: 0.75rem;
      margin-bottom: 0.5rem;
      background: var(--pico-background-color, #fff);
      border: 1px solid var(--pico-muted-border-color, #eee);
      border-radius: 6px;
      text-decoration: none;
      color: var(--pico-color, #333);
      transition: border-color 0.15s ease, box-shadow 0.15s ease;
    }

    .file-card:hover {
      border-color: var(--pico-primary, #0d6efd);
      box-shadow: 0 2px 8px rgba(0, 0, 0, 0.08);
    }

    .file-card.current {
      border-left: 3px solid var(--pico-primary, #0d6efd);
      background: var(--pico-primary-background, #e3f2fd);
    }

    .file-filename {
      font-size: 0.7rem;
      color: var(--pico-muted-color, #999);
      margin-bottom: 0.25rem;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .file-title {
      font-size: 0.95rem;
      font-weight: 600;
      margin-bottom: 0.35rem;
      line-height: 1.3;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .file-description {
      font-size: 0.8rem;
      color: var(--pico-muted-color, #666);
      line-height: 1.4;
      margin-bottom: 0.5rem;
      display: -webkit-box;
      -webkit-line-clamp: 2;
      -webkit-box-orient: vertical;
      overflow: hidden;
    }

    .file-tags {
      display: flex;
      flex-wrap: wrap;
      gap: 0.3rem;
      margin-bottom: 0.5rem;
    }

    .tag-pill {
      display: inline-block;
      padding: 0.15rem 0.5rem;
      background: var(--pico-secondary-background, #f0f0f0);
      border-radius: 12px;
      font-size: 0.7rem;
      color: var(--pico-muted-color, #666);
    }

    .tag-more {
      font-size: 0.7rem;
      color: var(--pico-muted-color, #999);
      padding: 0.15rem 0.25rem;
    }

    .file-meta {
      display: flex;
      justify-content: space-between;
      font-size: 0.7rem;
      color: var(--pico-muted-color, #999);
    }

    /* Loading/Error States */
    .loading-container,
    .error-container {
      padding: 2rem 1rem;
      text-align: center;
    }

    .loading-text {
      color: var(--pico-muted-color, #666);
    }

    .error-text {
      color: var(--pico-del-color, #dc3545);
      font-weight: 500;
      margin-bottom: 0.5rem;
    }

    .error-detail {
      color: var(--pico-muted-color, #666);
      font-size: 0.875rem;
    }

    .no-results {
      padding: 2rem;
      text-align: center;
      color: var(--pico-muted-color, #666);
    }

    /* Responsive - Mobile */
    @media (max-width: 768px) {
      .left-pane {
        width: 100%;
        max-width: 100vw;
      }

      .middle-pane {
        position: fixed;
        left: 0;
        top: 0;
        width: 100%;
        max-width: 100vw;
        z-index: 1001;
      }
    }

    /* Responsive - Large screens */
    @media (min-width: 1200px) {
      .left-pane {
        width: 320px;
      }

      .middle-pane {
        width: 380px;
      }
    }

    /* Responsive - Extra large screens */
    @media (min-width: 1400px) {
      .left-pane {
        width: 360px;
      }

      .middle-pane {
        width: 440px;
      }
    }

    /* Responsive - Ultra wide screens */
    @media (min-width: 1800px) {
      .left-pane {
        width: 400px;
      }

      .middle-pane {
        width: 500px;
      }
    }
  `;
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-browse': MbrBrowseElement
  }
}
