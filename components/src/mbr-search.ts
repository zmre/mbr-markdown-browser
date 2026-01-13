import { LitElement, css, html, nothing } from 'lit'
import { customElement, state, query } from 'lit/decorators.js'
import { unsafeHTML } from 'lit/directives/unsafe-html.js'
import { getBasePath, resolveUrl } from './shared.js'

/**
 * MBR configuration injected by the server/build.
 */
interface MbrConfig {
  serverMode: boolean;
  searchEndpoint: string;
}

/**
 * Search result from the API (unified format for both server and Pagefind).
 */
interface SearchResult {
  url_path: string;
  title: string | null;
  description: string | null;
  tags: string | null;
  score: number;
  snippet: string | null;
  snippetHtml: string | null; // HTML snippet with <mark> highlights (from Pagefind)
  is_content_match: boolean;
  filetype: string;
}

/**
 * Search response from the server API.
 */
interface SearchResponse {
  query: string;
  total_matches: number;
  results: SearchResult[];
  duration_ms: number;
  error?: string;
}

/**
 * Pagefind types (minimal subset we need).
 */
interface PagefindResult {
  id: string;
  data: () => Promise<PagefindResultData>;
}

interface PagefindResultData {
  url: string;
  excerpt: string;
  meta: {
    title?: string;
    image?: string;
  };
  sub_results?: Array<{
    title: string;
    url: string;
    excerpt: string;
  }>;
}

interface PagefindSearchResponse {
  results: PagefindResult[];
}

interface Pagefind {
  init: () => Promise<void>;
  options: (opts: { baseUrl?: string;[key: string]: any }) => Promise<void>;
  search: (query: string) => Promise<PagefindSearchResponse>;
  debouncedSearch: (query: string, options?: { debounceTimeoutMs?: number }) => Promise<PagefindSearchResponse | null>;
}

/**
 * Search scope options.
 */
type SearchScope = 'all' | 'metadata' | 'content';

/**
 * Folder scope options.
 */
type FolderScope = 'current' | 'everywhere';

/**
 * Filetype options.
 */
type FiletypeFilter = 'markdown' | 'all';

/**
 * Get MBR configuration from the global scope.
 */
function getMbrConfig(): MbrConfig {
  return (window as any).__MBR_CONFIG__ ?? {
    serverMode: false,
    searchEndpoint: '/.mbr/search'
  };
}

/**
 * Get the current folder context from the URL path.
 * - Section pages (ending with /) search from current path
 * - Markdown pages (not ending with / after navigation) search from parent
 */
function getCurrentFolder(): string {
  const path = window.location.pathname;
  // If path ends with /, it's a section page - search from current folder
  if (path.endsWith('/')) {
    return path;
  }
  // Otherwise it's a markdown file rendered at a trailing-slash URL,
  // so search from parent directory
  const lastSlash = path.lastIndexOf('/');
  if (lastSlash > 0) {
    return path.substring(0, lastSlash + 1);
  }
  return '/';
}

/**
 * Search component for MBR.
 *
 * In server mode, queries the POST /.mbr/search endpoint.
 * In static mode, uses Pagefind for client-side search.
 */
@customElement('mbr-search')
export class MbrSearchElement extends LitElement {
  @state()
  private _query = '';

  @state()
  private _results: SearchResult[] = [];

  @state()
  private _totalMatches = 0;

  @state()
  private _durationMs = 0;

  @state()
  private _isLoading = false;

  @state()
  private _isOpen = false;

  @state()
  private _selectedIndex = -1;

  @state()
  private _scope: SearchScope = 'all';

  @state()
  private _folderScope: FolderScope = 'everywhere';

  @state()
  private _filetypeFilter: FiletypeFilter = 'markdown';

  @state()
  private _error: string | null = null;

  @query('#search-input')
  private _input!: HTMLInputElement;

  @state()
  private _isPagefindLoading = false;

  private _debounceTimeout: number | null = null;
  private _abortController: AbortController | null = null;
  private _pagefind: Pagefind | null = null;
  private _pagefindLoadPromise: Promise<Pagefind | null> | null = null;

  override connectedCallback() {
    super.connectedCallback();
    // Listen for keyboard shortcut (Ctrl+K or Cmd+K)
    document.addEventListener('keydown', this._handleGlobalKeydown);

    // Pre-check Pagefind availability in static mode
    const config = getMbrConfig();
    if (!config.serverMode) {
      this._loadPagefind();
    }
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener('keydown', this._handleGlobalKeydown);
    if (this._debounceTimeout) {
      clearTimeout(this._debounceTimeout);
    }
    if (this._abortController) {
      this._abortController.abort();
    }
  }

  /**
   * Lazily load and initialize Pagefind.
   */
  private async _loadPagefind(): Promise<Pagefind | null> {
    // Return cached instance if available
    if (this._pagefind) {
      return this._pagefind;
    }

    // Return existing promise if already loading
    if (this._pagefindLoadPromise) {
      return this._pagefindLoadPromise;
    }

    this._isPagefindLoading = true;

    this._pagefindLoadPromise = (async () => {
      try {
        // Load Pagefind from the .mbr assets location
        // Use URL() to resolve relative to page, not the component module
        const basePath = getBasePath();
        const pagefindUrl = new URL(basePath + '.mbr/pagefind/pagefind.js', window.location.href).href;
        const pagefind = await import(/* @vite-ignore */ pagefindUrl) as Pagefind;
        // Configure baseUrl and ranking to prioritize title/filename matches
        await pagefind.options({
          baseUrl: "/",
          ranking: {
            termFrequency: 0.5,    // Short docs (title matches) less penalized
            pageLength: 0.0,       // Neutralize page length effect
            termSaturation: 2.0    // High density helps (titles repeat term)
          }
        });
        await pagefind.init();
        this._pagefind = pagefind;
        return pagefind;
      } catch (err) {
        console.warn('Pagefind not available:', err);
        return null;
      } finally {
        this._isPagefindLoading = false;
      }
    })();

    return this._pagefindLoadPromise;
  }

  private _handleGlobalKeydown = (e: KeyboardEvent) => {
    // Ctrl+K or Cmd+K to open search
    if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
      e.preventDefault();
      this._openSearch();
    }
    // Escape to close
    if (e.key === 'Escape' && this._isOpen) {
      e.preventDefault();
      this._closeSearch();
    }
  };

  private _openSearch() {
    this._isOpen = true;
    this.updateComplete.then(() => {
      this._input?.focus();
    });
  }

  private _closeSearch() {
    this._isOpen = false;
    this._query = '';
    this._results = [];
    this._selectedIndex = -1;
    this._error = null;
  }

  private _handleInput(e: Event) {
    const target = e.target as HTMLInputElement;
    this._query = target.value;
    this._selectedIndex = -1;

    // Debounce search
    if (this._debounceTimeout) {
      clearTimeout(this._debounceTimeout);
    }

    if (this._query.length >= 2) {
      this._debounceTimeout = window.setTimeout(() => {
        this._performSearch();
      }, 150);
    } else {
      this._results = [];
      this._totalMatches = 0;
    }
  }

  private _handleKeydown(e: KeyboardEvent) {
    // Handle Ctrl key combinations for scrolling and navigation
    if (e.ctrlKey) {
      const resultsContainer = this.shadowRoot?.querySelector('.results-container');

      switch (e.key.toLowerCase()) {
        case 'n': // Ctrl+n - next result (readline-style)
          e.preventDefault();
          this._selectedIndex = Math.min(this._selectedIndex + 1, this._results.length - 1);
          this._scrollSelectedIntoView();
          return;
        case 'p': // Ctrl+p - previous result (readline-style)
          e.preventDefault();
          this._selectedIndex = Math.max(this._selectedIndex - 1, -1);
          this._scrollSelectedIntoView();
          return;
        case 'd': // Ctrl+d - half page down
          if (resultsContainer) {
            e.preventDefault();
            resultsContainer.scrollBy({ top: resultsContainer.clientHeight / 2, behavior: 'smooth' });
          }
          return;
        case 'u': // Ctrl+u - half page up
          if (resultsContainer) {
            e.preventDefault();
            resultsContainer.scrollBy({ top: -resultsContainer.clientHeight / 2, behavior: 'smooth' });
          }
          return;
        case 'f': // Ctrl+f - full page down
          if (resultsContainer) {
            e.preventDefault();
            resultsContainer.scrollBy({ top: resultsContainer.clientHeight - 50, behavior: 'smooth' });
          }
          return;
        case 'b': // Ctrl+b - full page up
          if (resultsContainer) {
            e.preventDefault();
            resultsContainer.scrollBy({ top: -(resultsContainer.clientHeight - 50), behavior: 'smooth' });
          }
          return;
      }
    }

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        this._selectedIndex = Math.min(this._selectedIndex + 1, this._results.length - 1);
        this._scrollSelectedIntoView();
        break;
      case 'ArrowUp':
        e.preventDefault();
        this._selectedIndex = Math.max(this._selectedIndex - 1, -1);
        this._scrollSelectedIntoView();
        break;
      case 'Enter':
        e.preventDefault();
        if (this._selectedIndex >= 0 && this._results[this._selectedIndex]) {
          this._navigateToResult(this._results[this._selectedIndex]);
        }
        break;
      case 'Escape':
        e.preventDefault();
        this._closeSearch();
        break;
    }
  }

  /**
   * Scroll the selected result into view if needed.
   */
  private _scrollSelectedIntoView() {
    this.updateComplete.then(() => {
      const resultsContainer = this.shadowRoot?.querySelector('.results-container');
      const selectedEl = this.shadowRoot?.querySelector('.result.selected');
      if (resultsContainer && selectedEl) {
        selectedEl.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
      }
    });
  }

  private _handleScopeChange(e: Event) {
    const target = e.target as HTMLSelectElement;
    this._scope = target.value as SearchScope;
    if (this._query.length >= 2) {
      this._performSearch();
    }
  }

  private _handleFolderScopeChange(e: Event) {
    const target = e.target as HTMLInputElement;
    this._folderScope = target.checked ? 'current' : 'everywhere';
    if (this._query.length >= 2) {
      this._performSearch();
    }
  }

  private _handleFiletypeChange(e: Event) {
    const target = e.target as HTMLInputElement;
    this._filetypeFilter = target.checked ? 'all' : 'markdown';
    if (this._query.length >= 2) {
      this._performSearch();
    }
  }

  private async _performSearch() {
    const config = getMbrConfig();

    if (config.serverMode) {
      await this._performServerSearch();
    } else {
      await this._performPagefindSearch();
    }
  }

  /**
   * Perform search using the server API.
   */
  private async _performServerSearch() {
    const config = getMbrConfig();

    // Cancel any in-flight request
    if (this._abortController) {
      this._abortController.abort();
    }
    this._abortController = new AbortController();

    this._isLoading = true;
    this._error = null;

    try {
      // Build search request with folder context
      const searchBody: Record<string, any> = {
        q: this._query,
        limit: 20,
        scope: this._scope,
        folder_scope: this._folderScope,
      };

      // Add folder path when searching current folder
      if (this._folderScope === 'current') {
        searchBody.folder = getCurrentFolder();
      }

      // Add filetype filter
      if (this._filetypeFilter === 'all') {
        searchBody.filetype = 'all';
      }

      const response = await fetch(config.searchEndpoint, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(searchBody),
        signal: this._abortController.signal,
      });

      const data: SearchResponse = await response.json();

      if (!response.ok || data.error) {
        throw new Error(data.error || `Search failed: ${response.status}`);
      }
      this._results = data.results.map((r: SearchResult) => ({ ...r, snippetHtml: null }));
      this._totalMatches = data.total_matches;
      this._durationMs = data.duration_ms;
    } catch (err) {
      if (err instanceof Error && err.name === 'AbortError') {
        // Ignore abort errors
        return;
      }
      console.error('Search error:', err);
      this._error = err instanceof Error ? err.message : 'Search failed';
      this._results = [];
    } finally {
      this._isLoading = false;
    }
  }

  /**
   * Perform search using Pagefind (static mode).
   */
  private async _performPagefindSearch() {
    const startTime = performance.now();

    this._isLoading = true;
    this._error = null;

    try {
      const pagefind = await this._loadPagefind();
      if (!pagefind) {
        this._error = 'Search index not available. Run "npx pagefind --site <build_dir> --output-subdir .mbr/pagefind" after building.';
        this._results = [];
        return;
      }

      // Perform the search
      const searchResponse = await pagefind.search(this._query);
      this._totalMatches = searchResponse.results.length;

      // Load data for the first 20 results
      const resultPromises = searchResponse.results.slice(0, 20).map(r => r.data());
      const resultData = await Promise.all(resultPromises);

      // Map Pagefind results to our format
      this._results = resultData.map((data, index): SearchResult => ({
        url_path: data.url,
        title: data.meta?.title || null,
        description: null,
        tags: null,
        score: searchResponse.results.length - index, // Higher rank = higher score
        snippet: null,
        snippetHtml: data.excerpt || null, // Pagefind provides HTML with <mark> tags
        is_content_match: true, // Pagefind searches content
        filetype: 'markdown', // Pagefind indexes HTML from markdown
      }));

      this._durationMs = Math.round(performance.now() - startTime);
    } catch (err) {
      console.error('Pagefind search error:', err);
      this._error = err instanceof Error ? err.message : 'Search failed';
      this._results = [];
    } finally {
      this._isLoading = false;
    }
  }

  private _navigateToResult(result: SearchResult) {
    // Use resolveUrl to handle relative paths in static mode
    window.location.href = resolveUrl(result.url_path);
  }

  private _renderResult(result: SearchResult, index: number) {
    const isSelected = index === this._selectedIndex;
    const title = result.title || result.url_path;

    return html`
      <div
        class="result ${isSelected ? 'selected' : ''} ${result.is_content_match ? 'content-match' : 'metadata-match'}"
        @click=${() => this._navigateToResult(result)}
        @mouseenter=${() => this._selectedIndex = index}
      >
        <div class="result-header">
          <span class="result-title">${title}</span>
          <span class="result-type">${result.filetype}</span>
        </div>
        <div class="result-path">${result.url_path}</div>
        ${result.snippetHtml ? html`
          <div class="result-snippet">${unsafeHTML(result.snippetHtml)}</div>
        ` : result.snippet ? html`
          <div class="result-snippet">${result.snippet}</div>
        ` : nothing}
        ${result.tags ? html`
          <div class="result-tags">
            ${result.tags.split(',').map(tag => html`
              <span class="tag">${tag.trim()}</span>
            `)}
          </div>
        ` : nothing}
      </div>
    `;
  }

  private _renderTrigger() {
    const isMac = navigator.platform.toUpperCase().indexOf('MAC') >= 0;
    const shortcut = isMac ? '⌘K' : 'Ctrl+K';

    return html`
      <button class="search-trigger" @click=${this._openSearch} title="Search (${shortcut})">
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="11" cy="11" r="8"></circle>
          <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
        </svg>
        <span class="search-text">Search</span>
        <kbd class="shortcut">${shortcut}</kbd>
      </button>
    `;
  }

  private _renderModal() {
    if (!this._isOpen) return nothing;

    const config = getMbrConfig();
    // Hide scope selector in static mode (Pagefind searches everything)
    const showScopeSelector = config.serverMode;

    return html`
      <div class="modal-backdrop" @click=${this._closeSearch}>
        <div class="modal" @click=${(e: Event) => e.stopPropagation()}>
          <div class="search-header">
            <div class="search-input-wrapper">
              <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="search-icon">
                <circle cx="11" cy="11" r="8"></circle>
                <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
              </svg>
              <input
                id="search-input"
                type="search"
                placeholder="Search files..."
                .value=${this._query}
                @input=${this._handleInput}
                @keydown=${this._handleKeydown}
                autocomplete="off"
                spellcheck="false"
              />
              ${this._isLoading ? html`<span class="loading-indicator" aria-busy="true">Searching...</span>` : nothing}
            </div>
            ${showScopeSelector ? html`
              <select class="scope-select" @change=${this._handleScopeChange}>
                <option value="all" ?selected=${this._scope === 'all'}>All</option>
                <option value="metadata" ?selected=${this._scope === 'metadata'}>Titles & Tags</option>
                <option value="content" ?selected=${this._scope === 'content'}>Content</option>
              </select>
            ` : nothing}
          </div>

          ${showScopeSelector ? html`
            <div class="search-options">
              <label class="option-toggle">
                <input
                  type="checkbox"
                  ?checked=${this._folderScope === 'current'}
                  @change=${this._handleFolderScopeChange}
                />
                <span>Current folder only</span>
              </label>
              <label class="option-toggle">
                <input
                  type="checkbox"
                  ?checked=${this._filetypeFilter === 'all'}
                  @change=${this._handleFiletypeChange}
                />
                <span>Include PDFs & text files</span>
              </label>
            </div>
          ` : nothing}

          <div class="results-container">
            ${this._error ? html`
              <div class="error">${this._error}</div>
            ` : nothing}

            ${this._results.length > 0 ? html`
              <div class="results-meta">
                ${this._totalMatches} result${this._totalMatches !== 1 ? 's' : ''} in ${this._durationMs}ms
              </div>
              <div class="results-list">
                ${this._results.map((r, i) => this._renderResult(r, i))}
              </div>
            ` : this._query.length >= 2 && !this._isLoading && !this._error ? html`
              <div class="no-results">No results found for "${this._query}"</div>
            ` : nothing}

            ${this._query.length < 2 && !this._error ? html`
              <div class="hint">
                ${this._isPagefindLoading ? html`
                  <p aria-busy="true">Loading search index...</p>
                ` : html`
                  <p>Type at least 2 characters to search</p>
                  ${showScopeSelector ? html`
                    <p class="hint-facets">Tip: Use <code>field:value</code> for faceted search (e.g., <code>tags:rust</code> or <code>category:guide</code>)</p>
                  ` : nothing}
                `}
              </div>
            ` : nothing}
          </div>

          <div class="search-footer">
            <span class="footer-hint">
              <kbd>^n</kbd><kbd>^p</kbd> navigate
              <kbd>↵</kbd> select
              <kbd>esc</kbd> close
              <kbd>^d</kbd><kbd>^u</kbd> scroll
            </span>
          </div>
        </div>
      </div>
    `;
  }

  override render() {
    return html`
      ${this._renderTrigger()}
      ${this._renderModal()}
    `;
  }

  static override styles = css`
    :host {
      display: inline-block;
    }

    /* Trigger button */
    .search-trigger {
      display: inline-flex;
      align-items: center;
      gap: 0.5rem;
      padding: 0.4rem 0.75rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 6px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-muted-color, #666);
      cursor: pointer;
      font-size: 0.875rem;
      transition: all 0.15s ease;
    }

    .search-trigger:hover {
      border-color: var(--pico-primary, #0d6efd);
      color: var(--pico-color, #333);
    }

    .search-text {
      display: none;
    }

    @media (min-width: 640px) {
      .search-text {
        display: inline;
      }
    }

    .shortcut {
      display: none;
      padding: 0.15rem 0.35rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      background: var(--pico-secondary-background, #f5f5f5);
      color: var(--pico-primary-inverse, #eee);
      font-size: 0.75rem;
      font-family: inherit;
    }

    @media (min-width: 640px) {
      .shortcut {
        display: inline;
      }
    }

    /* Modal backdrop */
    .modal-backdrop {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.5);
      display: flex;
      align-items: flex-start;
      justify-content: center;
      padding-top: 10vh;
      z-index: 1000;
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
    }

    /* Search header */
    .search-header {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      padding: 0.75rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
    }

    .search-input-wrapper {
      flex: 1;
      display: flex;
      align-items: center;
      gap: 0.5rem;
    }

    .search-icon {
      color: var(--pico-muted-color, #999);
      flex-shrink: 0;
    }

    #search-input {
      flex: 1;
      border: none;
      background: transparent;
      font-size: 1rem;
      color: var(--pico-color, #333);
      outline: none;
      min-width: 0;
    }

    #search-input::placeholder {
      color: var(--pico-muted-color, #999);
    }

    .loading-indicator {
      color: var(--pico-muted-color, #999);
      animation: pulse 1s infinite;
    }

    @keyframes pulse {
      0%, 100% { opacity: 1; }
      50% { opacity: 0.5; }
    }

    .scope-select {
      padding: 0.25rem 0.5rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      background: var(--pico-background-color, #fff);
      font-size: 0.75rem;
      color: var(--pico-color, #333);
      cursor: pointer;
    }

    /* Search options toggles */
    .search-options {
      display: flex;
      align-items: center;
      gap: 1rem;
      padding: 0.5rem 0.75rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      font-size: 0.8rem;
    }

    .option-toggle {
      display: flex;
      align-items: center;
      gap: 0.35rem;
      cursor: pointer;
      color: var(--pico-muted-color, #666);
      user-select: none;
    }

    .option-toggle input[type="checkbox"] {
      margin: 0;
      width: 14px;
      height: 14px;
      cursor: pointer;
    }

    .option-toggle:hover {
      color: var(--pico-color, #333);
    }

    /* Results container */
    .results-container {
      flex: 1;
      overflow-y: auto;
      padding: 0.5rem;
    }

    .results-meta {
      padding: 0.25rem 0.5rem;
      font-size: 0.75rem;
      color: var(--pico-muted-color, #999);
    }

    .results-list {
      display: flex;
      flex-direction: column;
      gap: 0.25rem;
    }

    /* Result item */
    .result {
      padding: 0.75rem;
      border-radius: 8px;
      cursor: pointer;
      transition: background 0.1s ease;
    }

    .result:hover,
    .result.selected {
      background: var(--pico-secondary-background, #f5f5f5);
    }

    .result-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 0.5rem;
    }

    .result-title {
      font-weight: 500;
      color: var(--pico-color, #333);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .result-type {
      flex-shrink: 0;
      padding: 0.1rem 0.4rem;
      border-radius: 4px;
      background: var(--pico-primary-background, #e3f2fd);
      color: var(--pico-primary, #0d6efd);
      font-size: 0.7rem;
      text-transform: uppercase;
    }

    .result-path {
      font-size: 0.8rem;
      color: var(--pico-muted-color, #666);
      margin-top: 0.25rem;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .result-snippet {
      font-size: 0.85rem;
      color: var(--pico-muted-color, #666);
      margin-top: 0.35rem;
      line-height: 1.4;
      display: -webkit-box;
      -webkit-line-clamp: 2;
      -webkit-box-orient: vertical;
      overflow: hidden;
    }

    /* Pagefind highlight styling */
    .result-snippet mark {
      background: var(--pico-mark-background-color, #ff0);
      color: var(--pico-mark-color, inherit);
      padding: 0 0.1em;
      border-radius: 2px;
    }

    .result-tags {
      display: flex;
      flex-wrap: wrap;
      gap: 0.25rem;
      margin-top: 0.35rem;
    }

    .tag {
      padding: 0.1rem 0.4rem;
      border-radius: 4px;
      background: var(--pico-secondary-background, #f0f0f0);
      color: var(--pico-muted-color, #666);
      font-size: 0.7rem;
    }

    .content-match .result-snippet {
      border-left: 2px solid var(--pico-primary, #0d6efd);
      padding-left: 0.5rem;
    }

    /* Empty states */
    .no-results,
    .hint,
    .error {
      padding: 1.5rem;
      text-align: center;
      color: var(--pico-muted-color, #666);
    }

    .hint p {
      margin: 0 0 0.5rem 0;
    }

    .hint p:last-child {
      margin-bottom: 0;
    }

    .hint-facets {
      font-size: 0.8rem;
      opacity: 0.8;
    }

    .hint-facets code {
      padding: 0.1rem 0.3rem;
      border-radius: 3px;
      background: var(--pico-secondary-background, #f5f5f5);
      font-size: 0.75rem;
    }

    .error {
      color: var(--pico-del-color, #dc3545);
    }

    /* Footer */
    .search-footer {
      padding: 0.5rem 0.75rem;
      border-top: 1px solid var(--pico-muted-border-color, #eee);
      font-size: 0.75rem;
      color: var(--pico-muted-color, #999);
    }

    .footer-hint {
      display: flex;
      align-items: center;
      gap: 0.5rem;
    }

    .footer-hint kbd {
      padding: 0.1rem 0.3rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 3px;
      background: var(--pico-secondary-background, #f5f5f5);
      color: var(--pico-primary-inverse, #eee);
      font-family: inherit;
      font-size: 0.7rem;
    }
  `;
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-search': MbrSearchElement
  }
}
