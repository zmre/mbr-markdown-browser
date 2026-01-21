import { LitElement, html, css, nothing, type TemplateResult } from 'lit';
import { customElement, state, query } from 'lit/decorators.js';
import { resolveUrl } from './shared.js';

/**
 * Heading from window.headings (injected by template).
 */
interface Heading {
  level: number;
  id: string;
  text: string;
}

/**
 * Outbound link from links.json.
 */
interface OutboundLink {
  to: string;
  text: string;
  anchor?: string;
  internal: boolean;
}

/**
 * Inbound link from links.json.
 */
interface InboundLink {
  from: string;
  text: string;
  anchor?: string;
}

/**
 * Combined page links response from links.json.
 */
interface PageLinks {
  inbound: InboundLink[];
  outbound: OutboundLink[];
}

/**
 * Navigation item for display in the fuzzy nav modal.
 */
interface NavItem {
  id: string;
  text: string;
  url: string;
  type: 'link-out' | 'link-in' | 'heading';
  level?: number;
  anchor?: string;
  isVisible: boolean;
  yPosition?: number;
  isInternal?: boolean;
}

/**
 * Tab options for the navigation modal.
 */
type NavTab = 'links-out' | 'links-in' | 'toc';

declare global {
  interface Window {
    headings?: Heading[];
  }
  interface HTMLElementTagNameMap {
    'mbr-fuzzy-nav': MbrFuzzyNavElement;
  }
}

/**
 * Fuzzy search navigation modal component.
 *
 * Provides quick navigation via three tabs:
 * - Links Out: Outbound links from the current page
 * - Links In: Inbound links (backlinks) to the current page
 * - Table of Contents: Headings in the current document
 *
 * Features:
 * - Fuzzy search filtering
 * - Keyboard navigation (Ctrl+N/P, Arrow keys)
 * - Tab cycling with Tab key
 * - Escape to close
 * - Visible items prioritized in sorting
 */
@customElement('mbr-fuzzy-nav')
export class MbrFuzzyNavElement extends LitElement {
  // ========================================
  // State
  // ========================================

  @state()
  private _isOpen = false;

  @state()
  private _activeTab: NavTab = 'links-out';

  @state()
  private _searchQuery = '';

  @state()
  private _selectedIndex = 0;

  @state()
  private _links: PageLinks | null = null;

  @state()
  private _linksLoading = false;

  @state()
  private _linksError: string | null = null;

  @state()
  private _headings: Heading[] = [];

  @state()
  private _visibleHeadingIds = new Set<string>();

  @query('#fuzzy-search-input')
  private _searchInput!: HTMLInputElement;

  private _observer: IntersectionObserver | null = null;
  private _linksCache: PageLinks | null = null;

  // ========================================
  // Lifecycle
  // ========================================

  override connectedCallback() {
    super.connectedCallback();
    this._headings = window.headings || [];
    this._setupIntersectionObserver();
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    this._cleanupIntersectionObserver();
  }

  // ========================================
  // Public Methods (called from mbr-keys)
  // ========================================

  public open(tab: NavTab = 'links-out') {
    this._isOpen = true;
    this._activeTab = tab;
    this._searchQuery = '';
    this._selectedIndex = 0;

    // Load links if not cached
    if (!this._linksCache && !this._linksLoading) {
      this._loadLinks();
    } else if (this._linksCache) {
      this._links = this._linksCache;
    }

    // Focus input after render
    this.updateComplete.then(() => {
      this._searchInput?.focus();
    });
  }

  public close() {
    this._isOpen = false;
    this._searchQuery = '';
    this._selectedIndex = 0;
  }

  public get isOpen(): boolean {
    return this._isOpen;
  }

  // ========================================
  // Links Loading
  // ========================================

  private async _loadLinks() {
    this._linksLoading = true;
    this._linksError = null;

    try {
      const currentPath = window.location.pathname;
      const normalizedPath = currentPath.endsWith('/') ? currentPath : currentPath + '/';
      const linksUrl = normalizedPath + 'links.json';

      const response = await fetch(linksUrl);

      if (!response.ok) {
        if (response.status === 404) {
          // Link tracking disabled
          this._links = { inbound: [], outbound: [] };
          this._linksCache = this._links;
          return;
        }
        throw new Error(`Failed to load links: ${response.status}`);
      }

      this._links = await response.json() as PageLinks;
      this._linksCache = this._links;
    } catch (error) {
      console.warn('Failed to load links:', error);
      this._linksError = error instanceof Error ? error.message : 'Unknown error';
      this._links = { inbound: [], outbound: [] };
    } finally {
      this._linksLoading = false;
    }
  }

  // ========================================
  // Intersection Observer for Visibility
  // ========================================

  private _setupIntersectionObserver() {
    if (typeof IntersectionObserver === 'undefined') return;

    this._observer = new IntersectionObserver(
      (entries) => {
        const newVisible = new Set(this._visibleHeadingIds);
        for (const entry of entries) {
          const id = entry.target.id;
          if (entry.isIntersecting) {
            newVisible.add(id);
          } else {
            newVisible.delete(id);
          }
        }
        this._visibleHeadingIds = newVisible;
      },
      {
        rootMargin: '0px',
        threshold: 0.1,
      }
    );

    // Observe all heading elements
    this._observeHeadings();
  }

  private _observeHeadings() {
    if (!this._observer) return;

    // Disconnect from previous elements
    this._observer.disconnect();

    // Observe heading elements by ID
    for (const heading of this._headings) {
      const element = document.getElementById(heading.id);
      if (element) {
        this._observer.observe(element);
      }
    }
  }

  private _cleanupIntersectionObserver() {
    if (this._observer) {
      this._observer.disconnect();
      this._observer = null;
    }
  }

  // ========================================
  // Fuzzy Search
  // ========================================

  /**
   * Fuzzy search scoring algorithm.
   * Higher scores = better matches.
   *
   * Scoring:
   * - Exact substring: 1000
   * - Word-start match: 500
   * - Character-by-character fuzzy: sum of position bonuses
   */
  private _fuzzyScore(text: string, query: string): number {
    if (!query) return 0;

    const lowerText = text.toLowerCase();
    const lowerQuery = query.toLowerCase();

    // Exact substring match (highest priority)
    if (lowerText.includes(lowerQuery)) {
      // Bonus for exact match at start
      if (lowerText.startsWith(lowerQuery)) {
        return 1500;
      }
      // Bonus for word-start match
      const wordStart = new RegExp(`\\b${this._escapeRegex(lowerQuery)}`);
      if (wordStart.test(lowerText)) {
        return 1200;
      }
      return 1000;
    }

    // Character-by-character fuzzy matching
    let score = 0;
    let textIndex = 0;
    let consecutiveBonus = 0;

    for (const char of lowerQuery) {
      const foundIndex = lowerText.indexOf(char, textIndex);
      if (foundIndex === -1) {
        return 0; // Character not found, no match
      }

      // Bonus for consecutive characters
      if (foundIndex === textIndex) {
        consecutiveBonus += 10;
      } else {
        consecutiveBonus = 0;
      }

      // Base score + position bonus (earlier = better) + consecutive bonus
      score += 10 + Math.max(0, 50 - foundIndex) + consecutiveBonus;

      // Bonus for word boundary match
      if (foundIndex === 0 || /\W/.test(lowerText[foundIndex - 1])) {
        score += 25;
      }

      textIndex = foundIndex + 1;
    }

    return score;
  }

  private _escapeRegex(str: string): string {
    return str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }

  // ========================================
  // Get Filtered Items
  // ========================================

  private _getFilteredItems(): NavItem[] {
    let items: NavItem[] = [];

    switch (this._activeTab) {
      case 'links-out':
        items = this._getLinksOutItems();
        break;
      case 'links-in':
        items = this._getLinksInItems();
        break;
      case 'toc':
        items = this._getTocItems();
        break;
    }

    // Apply fuzzy filter
    if (this._searchQuery.trim()) {
      const scored = items
        .map(item => ({
          item,
          score: this._fuzzyScore(item.text, this._searchQuery),
        }))
        .filter(({ score }) => score > 0)
        .sort((a, b) => b.score - a.score);

      items = scored.map(({ item }) => item);
    } else {
      // Sort by visibility for TOC tab
      if (this._activeTab === 'toc') {
        items = this._sortByVisibility(items);
      }
    }

    return items;
  }

  private _getLinksOutItems(): NavItem[] {
    if (!this._links) return [];

    return this._links.outbound.map((link, index) => ({
      id: `out-${index}`,
      text: link.text || link.to,
      url: link.internal ? link.to + (link.anchor || '') : link.to,
      type: 'link-out' as const,
      anchor: link.anchor,
      isVisible: false,
      isInternal: link.internal,
    }));
  }

  private _getLinksInItems(): NavItem[] {
    if (!this._links) return [];

    return this._links.inbound.map((link, index) => ({
      id: `in-${index}`,
      // For backlinks, show the source page path (where the link comes FROM)
      // not the link text (which is often just the current page's title)
      text: this._formatPagePath(link.from),
      url: link.from,
      type: 'link-in' as const,
      anchor: link.anchor,
      isVisible: false,
    }));
  }

  /**
   * Formats a URL path for display (e.g., "/docs/guide/" -> "docs/guide")
   */
  private _formatPagePath(urlPath: string): string {
    // Remove leading/trailing slashes and return a clean path
    return urlPath.replace(/^\/|\/$/g, '') || 'Home';
  }

  private _getTocItems(): NavItem[] {
    return this._headings.map((heading, index) => {
      const element = document.getElementById(heading.id);
      const yPosition = element?.getBoundingClientRect().top ?? 0;

      return {
        id: `toc-${index}`,
        text: heading.text,
        url: `#${heading.id}`,
        type: 'heading' as const,
        level: heading.level,
        isVisible: this._visibleHeadingIds.has(heading.id),
        yPosition,
      };
    });
  }

  private _sortByVisibility(items: NavItem[]): NavItem[] {
    // Separate visible and non-visible items
    const visible = items.filter(item => item.isVisible);
    const notVisible = items.filter(item => !item.isVisible);

    // Sort visible by Y position (top to bottom)
    visible.sort((a, b) => (a.yPosition || 0) - (b.yPosition || 0));

    // Keep non-visible in original order
    return [...visible, ...notVisible];
  }

  // ========================================
  // Event Handlers
  // ========================================

  private _handleBackdropClick() {
    this.close();
  }

  private _handleModalClick(e: Event) {
    e.stopPropagation();
  }

  private _handleSearchInput(e: Event) {
    const target = e.target as HTMLInputElement;
    this._searchQuery = target.value;
    this._selectedIndex = 0;
  }

  private _handleKeydown(e: KeyboardEvent) {
    const items = this._getFilteredItems();

    // Tab to cycle tabs
    if (e.key === 'Tab') {
      e.preventDefault();
      this._cycleTab(e.shiftKey ? -1 : 1);
      return;
    }

    // Escape to close
    if (e.key === 'Escape') {
      e.preventDefault();
      this.close();
      return;
    }

    // Enter to navigate
    if (e.key === 'Enter') {
      e.preventDefault();
      if (items[this._selectedIndex]) {
        this._navigateToItem(items[this._selectedIndex]);
      }
      return;
    }

    // Ctrl+N / Ctrl+P for navigation
    if (e.ctrlKey) {
      if (e.key === 'n') {
        e.preventDefault();
        this._selectedIndex = Math.min(this._selectedIndex + 1, items.length - 1);
        this._scrollSelectedIntoView();
        return;
      }
      if (e.key === 'p') {
        e.preventDefault();
        this._selectedIndex = Math.max(this._selectedIndex - 1, 0);
        this._scrollSelectedIntoView();
        return;
      }
      // Ctrl+D / Ctrl+U for scrolling
      if (e.key === 'd' || e.key === 'u') {
        e.preventDefault();
        const container = this.shadowRoot?.querySelector('.results-list');
        if (container) {
          const amount = container.clientHeight / 2;
          container.scrollBy({
            top: e.key === 'd' ? amount : -amount,
            behavior: 'smooth',
          });
        }
        return;
      }
    }

    // Arrow keys for navigation
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      this._selectedIndex = Math.min(this._selectedIndex + 1, items.length - 1);
      this._scrollSelectedIntoView();
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      this._selectedIndex = Math.max(this._selectedIndex - 1, 0);
      this._scrollSelectedIntoView();
      return;
    }
  }

  private _cycleTab(direction: 1 | -1) {
    const tabs: NavTab[] = ['links-out', 'links-in', 'toc'];
    const currentIndex = tabs.indexOf(this._activeTab);
    const newIndex = (currentIndex + direction + tabs.length) % tabs.length;
    this._activeTab = tabs[newIndex];
    this._selectedIndex = 0;
  }

  private _setTab(tab: NavTab) {
    this._activeTab = tab;
    this._selectedIndex = 0;
    this._searchInput?.focus();
  }

  private _navigateToItem(item: NavItem) {
    this.close();

    if (item.type === 'heading') {
      // Scroll to heading
      const element = document.getElementById(item.url.replace('#', ''));
      if (element) {
        element.scrollIntoView({ behavior: 'smooth', block: 'start' });
      }
    } else if (item.type === 'link-in' || (item.type === 'link-out' && item.isInternal)) {
      // Internal navigation
      window.location.href = resolveUrl(item.url);
    } else {
      // External link
      window.open(item.url, '_blank', 'noopener');
    }
  }

  private _scrollSelectedIntoView() {
    this.updateComplete.then(() => {
      const selected = this.shadowRoot?.querySelector('.result-item.selected');
      selected?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    });
  }

  // ========================================
  // Render Methods
  // ========================================

  private _renderTabs(): TemplateResult {
    const linksOutCount = this._links?.outbound.length ?? 0;
    const linksInCount = this._links?.inbound.length ?? 0;
    const tocCount = this._headings.length;

    return html`
      <div class="tabs" role="tablist">
        <button
          role="tab"
          class="tab ${this._activeTab === 'links-out' ? 'active' : ''}"
          aria-selected=${this._activeTab === 'links-out'}
          @click=${() => this._setTab('links-out')}
        >
          Links Out
          ${linksOutCount > 0 ? html`<span class="tab-count">${linksOutCount}</span>` : nothing}
        </button>
        <button
          role="tab"
          class="tab ${this._activeTab === 'links-in' ? 'active' : ''}"
          aria-selected=${this._activeTab === 'links-in'}
          @click=${() => this._setTab('links-in')}
        >
          Links In
          ${linksInCount > 0 ? html`<span class="tab-count">${linksInCount}</span>` : nothing}
        </button>
        <button
          role="tab"
          class="tab ${this._activeTab === 'toc' ? 'active' : ''}"
          aria-selected=${this._activeTab === 'toc'}
          @click=${() => this._setTab('toc')}
        >
          ToC
          ${tocCount > 0 ? html`<span class="tab-count">${tocCount}</span>` : nothing}
        </button>
      </div>
    `;
  }

  private _renderSearchInput(): TemplateResult {
    const placeholder = this._activeTab === 'toc'
      ? 'Filter headings...'
      : this._activeTab === 'links-out'
        ? 'Filter links out...'
        : 'Filter backlinks...';

    return html`
      <div class="search-wrapper">
        <svg class="search-icon" xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="11" cy="11" r="8"></circle>
          <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
        </svg>
        <input
          id="fuzzy-search-input"
          type="text"
          placeholder=${placeholder}
          .value=${this._searchQuery}
          @input=${this._handleSearchInput}
          @keydown=${this._handleKeydown}
          autocomplete="off"
          spellcheck="false"
        />
      </div>
    `;
  }

  private _renderResults(): TemplateResult {
    if (this._linksLoading && this._activeTab !== 'toc') {
      return html`
        <div class="results-list">
          <div class="loading-state">Loading links...</div>
        </div>
      `;
    }

    if (this._linksError && this._activeTab !== 'toc') {
      return html`
        <div class="results-list">
          <div class="error-state">${this._linksError}</div>
        </div>
      `;
    }

    const items = this._getFilteredItems();

    if (items.length === 0) {
      const emptyMessage = this._searchQuery
        ? `No matches for "${this._searchQuery}"`
        : this._activeTab === 'toc'
          ? 'No headings in this document'
          : this._activeTab === 'links-out'
            ? 'No outbound links'
            : 'No backlinks to this page';

      return html`
        <div class="results-list">
          <div class="empty-state">${emptyMessage}</div>
        </div>
      `;
    }

    return html`
      <div class="results-list" role="listbox">
        ${items.map((item, index) => this._renderResultItem(item, index))}
      </div>
    `;
  }

  private _renderResultItem(item: NavItem, index: number): TemplateResult {
    const isSelected = index === this._selectedIndex;
    const isHeading = item.type === 'heading';
    const isExternal = item.type === 'link-out' && !item.isInternal;

    return html`
      <div
        class="result-item ${isSelected ? 'selected' : ''} ${item.isVisible ? 'visible' : ''}"
        role="option"
        aria-selected=${isSelected}
        @click=${() => this._navigateToItem(item)}
        @mouseenter=${() => { this._selectedIndex = index; }}
      >
        ${isHeading ? html`
          <span class="heading-level" style="margin-left: ${((item.level || 1) - 1) * 0.75}rem">
            H${item.level}
          </span>
        ` : html`
          <span class="link-icon">
            ${item.type === 'link-in' ? html`<span class="icon-arrow">&#8592;</span>` : html`<span class="icon-arrow">&#8594;</span>`}
          </span>
        `}
        <span class="item-text">${item.text}</span>
        ${isExternal ? html`<span class="external-badge" title="External link">&#8599;</span>` : nothing}
        ${item.isVisible ? html`<span class="visible-badge" title="Currently visible">&#9679;</span>` : nothing}
      </div>
    `;
  }

  private _renderFooter(): TemplateResult {
    return html`
      <div class="modal-footer">
        <span class="hint">
          <kbd>^n</kbd><kbd>^p</kbd> navigate
          <kbd>Tab</kbd> switch tabs
          <kbd>Enter</kbd> select
          <kbd>Esc</kbd> close
        </span>
      </div>
    `;
  }

  override render() {
    if (!this._isOpen) {
      return nothing;
    }

    return html`
      <div class="modal-backdrop" @click=${this._handleBackdropClick}>
        <div class="modal" @click=${this._handleModalClick}>
          <div class="modal-header">
            ${this._renderTabs()}
          </div>
          ${this._renderSearchInput()}
          ${this._renderResults()}
          ${this._renderFooter()}
        </div>
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
    .modal-backdrop {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.6);
      display: flex;
      align-items: flex-start;
      justify-content: center;
      padding-top: 10vh;
      z-index: 10000;
      animation: fadeIn 0.15s ease;
    }

    @keyframes fadeIn {
      from { opacity: 0; }
      to { opacity: 1; }
    }

    /* Modal */
    .modal {
      width: 100%;
      max-width: 600px;
      max-height: 70vh;
      margin: 0 1rem;
      background: var(--pico-background-color, #fff);
      border-radius: 12px;
      box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.25);
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

    /* Header with tabs */
    .modal-header {
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    /* Tabs */
    .tabs {
      display: flex;
      padding: 0;
    }

    .tab {
      flex: 1;
      padding: 0.75rem 1rem;
      background: transparent;
      border: none;
      border-bottom: 2px solid transparent;
      color: var(--pico-muted-color, #666);
      font-size: 0.9rem;
      font-weight: 500;
      cursor: pointer;
      transition: all 0.15s ease;
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 0.5rem;
    }

    .tab:hover {
      color: var(--pico-color, #333);
      background: var(--pico-secondary-background, #f5f5f5);
    }

    .tab.active {
      color: var(--pico-primary, #0d6efd);
      border-bottom-color: var(--pico-primary, #0d6efd);
    }

    .tab-count {
      font-size: 0.75rem;
      padding: 0.1rem 0.4rem;
      background: var(--pico-muted-border-color, #e0e0e0);
      border-radius: 10px;
      color: var(--pico-muted-color, #666);
    }

    .tab.active .tab-count {
      background: var(--pico-primary-focus, rgba(99, 102, 241, 0.15));
      color: var(--pico-primary, #0d6efd);
    }

    /* Search input */
    .search-wrapper {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      padding: 0.75rem 1rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
    }

    .search-icon {
      color: var(--pico-muted-color, #999);
      flex-shrink: 0;
    }

    #fuzzy-search-input {
      flex: 1;
      border: none;
      background: transparent;
      font-size: 1rem;
      color: var(--pico-color, #333);
      outline: none;
      min-width: 0;
    }

    #fuzzy-search-input::placeholder {
      color: var(--pico-muted-color, #999);
    }

    /* Results list */
    .results-list {
      flex: 1;
      overflow-y: auto;
      padding: 0.5rem;
      min-height: 200px;
      max-height: 400px;
    }

    /* Result item */
    .result-item {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      padding: 0.6rem 0.75rem;
      border-radius: 6px;
      cursor: pointer;
      transition: background 0.1s ease;
    }

    .result-item:hover,
    .result-item.selected {
      background: var(--pico-primary-focus, rgba(99, 102, 241, 0.15));
    }

    .result-item.visible {
      border-left: 3px solid var(--pico-primary, #0d6efd);
    }

    /* Heading level indicator */
    .heading-level {
      font-size: 0.7rem;
      font-weight: 600;
      padding: 0.15rem 0.35rem;
      background: var(--pico-secondary-background, #f5f5f5);
      color: var(--pico-muted-color, #666);
      border-radius: 4px;
      flex-shrink: 0;
    }

    /* Link icon */
    .link-icon {
      display: flex;
      align-items: center;
      justify-content: center;
      width: 1.5rem;
      height: 1.5rem;
      flex-shrink: 0;
    }

    .icon-arrow {
      font-size: 1rem;
      color: var(--pico-muted-color, #666);
    }

    /* Item text */
    .item-text {
      flex: 1;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      color: var(--pico-color, #333);
      font-size: 0.9rem;
    }

    /* Badges */
    .external-badge {
      font-size: 0.8rem;
      color: var(--pico-muted-color, #666);
      flex-shrink: 0;
    }

    .visible-badge {
      font-size: 0.6rem;
      color: var(--pico-primary, #0d6efd);
      flex-shrink: 0;
    }

    /* Empty/Loading/Error states */
    .empty-state,
    .loading-state,
    .error-state {
      padding: 2rem;
      text-align: center;
      color: var(--pico-muted-color, #666);
    }

    .error-state {
      color: var(--pico-del-color, #dc3545);
    }

    /* Footer */
    .modal-footer {
      padding: 0.5rem 0.75rem;
      border-top: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    .hint {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      font-size: 0.75rem;
      color: var(--pico-muted-color, #999);
      flex-wrap: wrap;
    }

    kbd {
      padding: 0.1rem 0.3rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 3px;
      background: var(--pico-secondary-background, #f5f5f5);
      color: var(--pico-color, #333);
      font-family: inherit;
      font-size: 0.7rem;
    }

    /* Responsive */
    @media (max-width: 480px) {
      .modal {
        margin: 0 0.5rem;
        max-height: 80vh;
      }

      .tab {
        padding: 0.6rem 0.5rem;
        font-size: 0.85rem;
      }

      .hint {
        justify-content: center;
      }
    }
  `;
}
