import { LitElement, css, html, nothing, type TemplateResult } from 'lit'
import { customElement, state, query } from 'lit/decorators.js'
import { unsafeHTML } from 'lit/directives/unsafe-html.js'
import { getBasePath, resolveUrl, isNewTabModifier, openInNewTab, safeDecodePath, siteNav } from './shared.js'
import type { MbrOverlay } from './overlay.js'
import type { MbrMediaBrowserElement } from './mbr-media-browser.js'

// Dynamically import the media browser component when needed
const loadMediaBrowser = () => import('./mbr-media-browser.js')

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
 * POST body for the server search endpoint.
 *
 * Field names and values mirror the Rust `SearchQuery` struct (src/search.rs):
 * `filetype`, `folder` and `folder_scope` are `#[serde(default)]` and the struct
 * does NOT set `deny_unknown_fields`, so a misspelled key is silently ignored by
 * the server and degrades to its default instead of returning an error. This
 * type plus the exact-body assertions in mbr-search.test.ts are the only guards
 * against that drift.
 */
export interface SearchRequestBody {
  /** Query string (supports `key:value` facet syntax). */
  q: string;
  /** Maximum number of results to return. */
  limit: number;
  /** Rust `SearchScope`, serialized lowercase. */
  scope: SearchScope;
  /** Rust `FolderScope`, serialized lowercase. */
  folder_scope: FolderScope;
  /** Folder prefix to search within; only sent when `folder_scope` is `current`. */
  folder?: string;
  /** Only sent when non-markdown files should be included. */
  filetype?: 'all';
}

/**
 * Get MBR configuration from the global scope.
 */
function getMbrConfig(): MbrConfig {
  return (window as any).__MBR_CONFIG__ ?? {
    serverMode: false,
    searchEndpoint: '/.mbr/search'
  };
}

/** The folder holding `path`: everything up to and including its last `/` but one. */
function parentFolder(path: string): string {
  const trimmed = path.endsWith('/') ? path.slice(0, -1) : path;
  const lastSlash = trimmed.lastIndexOf('/');
  return lastSlash > 0 ? trimmed.substring(0, lastSlash + 1) : '/';
}

/**
 * The folder "Current folder only" scopes a search to, as a DECODED url-path
 * prefix ending in `/` — the form the server's folder filter compares against
 * `url_path` (src/search.rs `matches_folder_filter`, a plain `starts_with`).
 *
 * The URL alone cannot tell a note from a folder: every markdown page is served
 * at a trailing-slash URL, so `/people/john/` is John's *note*, and treating it
 * as a folder scoped the search to John and nothing else. `markdownSource` (the
 * page's repo-relative source path, set on every rendered markdown page) is
 * what disambiguates:
 *
 * - no `markdownSource` (section, home, tag pages) → the URL is the folder;
 * - an index note (`people/index.md` served at `/people/`) → its URL is its
 *   own folder, recognised by the source's directory equalling the URL;
 * - any other note → the parent of its URL.
 *
 * The pathname is percent-decoded first (`url_path` is stored decoded, so
 * `/My%20Notes/` would match nothing); a malformed escape keeps the raw text.
 */
export function searchFolderFor(pathname: string, markdownSource?: string | null): string {
  const decoded = safeDecodePath(pathname) || '/';
  if (!markdownSource) {
    return decoded.endsWith('/') ? decoded : parentFolder(decoded);
  }
  const page = decoded.endsWith('/') ? decoded : `${decoded}/`;
  const sourceDir = markdownSource.split('/').slice(0, -1).filter(Boolean).join('/');
  const sourceFolder = sourceDir ? `/${sourceDir}/` : '/';
  return page === sourceFolder ? page : parentFolder(page);
}

/** Case-insensitive, locale-independent ordering for folder and type lists. */
function compareFolded(a: string, b: string): number {
  const x = a.toLowerCase();
  const y = b.toLowerCase();
  return x < y ? -1 : x > y ? 1 : a < b ? -1 : a > b ? 1 : 0;
}

/**
 * Every folder that holds at least one note, derived from site.json `url_path`s
 * (no endpoint needed). A note's own URL is not a folder — `/people/john/` is
 * John's page — so only *proper* ancestors count; a note that also has notes
 * beneath it contributes its URL through those children. The root is omitted:
 * scoping to `/` is the same as searching everywhere.
 */
export function foldersFromUrlPaths(paths: readonly string[]): string[] {
  const folders = new Set<string>();
  for (const path of paths) {
    if (typeof path !== 'string') continue;
    const segments = path.split('/').filter(Boolean);
    let prefix = '/';
    for (const segment of segments.slice(0, -1)) {
      prefix += `${segment}/`;
      folders.add(prefix);
    }
  }
  return [...folders].sort(compareFolded);
}

/** One distinct frontmatter `type`, with how many notes carry it. */
export interface NoteTypeCount {
  type: string;
  count: number;
}

/**
 * Distinct frontmatter `type` values across site.json, case-insensitively
 * merged (the server's facet match is case-insensitive, so `Person` and
 * `person` select the same notes) under the first spelling seen.
 */
export function noteTypesFromSite(
  files: ReadonlyArray<{ frontmatter?: Record<string, unknown> | null }> | null | undefined
): NoteTypeCount[] {
  const byKey = new Map<string, NoteTypeCount>();
  for (const file of files ?? []) {
    const raw = file?.frontmatter?.['type'];
    if (typeof raw !== 'string') continue;
    const type = raw.trim();
    if (!type) continue;
    const key = type.toLowerCase();
    const entry = byKey.get(key);
    if (entry) entry.count += 1;
    else byKey.set(key, { type, count: 1 });
  }
  return [...byKey.values()].sort((a, b) => compareFolded(a.type, b.type));
}

/**
 * The `type:` facet token for a note type. A value with whitespace is quoted —
 * `parse_query` (src/search.rs `query_tokens`) keeps `key:"two words"` whole.
 * A literal `"` cannot be expressed inside that quoting, so it is dropped; the
 * facet is a substring match, so the remaining text still selects the type.
 */
export function typeFacetToken(type: string): string {
  const value = type.replace(/"/g, '').trim();
  return /\s/.test(value) ? `type:"${value}"` : `type:${value}`;
}

/** Matches an existing `type:` facet token, quoted or bare. */
const TYPE_TOKEN = /(^|\s)type:("[^"]*"?|\S*)/gi;

/**
 * `query` with any existing `type:` facet replaced by the one for `type`, so
 * choosing a second type swaps rather than stacking two facets (which would
 * have to both match and select nothing).
 */
export function withTypeFacet(query: string, type: string): string {
  const rest = query.replace(TYPE_TOKEN, ' ').replace(/\s+/g, ' ').trim();
  const token = typeFacetToken(type);
  return rest ? `${rest} ${token}` : token;
}

/**
 * Value prefix of the note-type `<option>`s in the scope select. Scope values
 * (`all`/`metadata`/`content`) never contain a colon, so the two cannot clash.
 */
const TYPE_OPTION_PREFIX = 'type:';

/** Most folders the picker lists at once; the filter narrows the rest. */
const MAX_LISTED_FOLDERS = 200;

/** The current page's search folder (see {@link searchFolderFor}). */
function getCurrentFolder(): string {
  const source = window.frontmatter?.['markdown_source'];
  return searchFolderFor(window.location.pathname, typeof source === 'string' ? source : null);
}

/**
 * Search component for MBR.
 *
 * In server mode, queries the POST /.mbr/search endpoint.
 * In static mode, uses Pagefind for client-side search.
 */
@customElement('mbr-search')
export class MbrSearchElement extends LitElement implements MbrOverlay {
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

  /**
   * A folder chosen in the folder picker, overriding the current page's folder
   * while `_folderScope` is `current`. `null` means "the folder I am in".
   */
  @state()
  private _folderOverride: string | null = null;

  @state()
  private _isFolderPickerOpen = false;

  @state()
  private _folderFilter = '';

  /** Highlighted row in the folder picker (index into the filtered list). */
  @state()
  private _folderPickerIndex = 0;

  /** Folders and note types derived from site.json, loaded on first open. */
  @state()
  private _folders: string[] = [];

  @state()
  private _noteTypes: NoteTypeCount[] = [];

  /** site.json payload `_folders`/`_noteTypes` were derived from. */
  private _facetSource: unknown = null;

  @state()
  private _error: string | null = null;

  @query('#search-input')
  private _input!: HTMLInputElement;

  @state()
  private _isPagefindLoading = false;

  @state()
  private _isMediaBrowserOpen = false;

  @query('mbr-media-browser')
  private _mediaBrowser!: MbrMediaBrowserElement;

  private _debounceTimeout: number | null = null;
  private _abortController: AbortController | null = null;
  private _pagefind: Pagefind | null = null;
  private _pagefindLoadPromise: Promise<Pagefind | null> | null = null;

  /**
   * Monotonic id for search runs. Bumped when a search starts and when the modal
   * closes, so a slow in-flight search can detect that it has been superseded and
   * skip its state writes. The 150 ms input debounce only prevents *starting* a
   * search per keystroke; a search slower than that can still overlap the next
   * one and land last, leaving results that do not match the visible query.
   */
  private _searchGeneration = 0;

  /**
   * Build a predicate that reports whether the search run identified by
   * `generation`/`query` has been superseded (newer search started, query
   * changed, or the modal closed). Callers must consult it before every state
   * write that follows an `await`.
   */
  private _makeStaleCheck(generation: number, query: string): () => boolean {
    return () => generation !== this._searchGeneration || query !== this._query;
  }

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

  // ========================================
  // Public Methods (the MbrOverlay contract, called from mbr-keys)
  // ========================================

  /** True while the search modal is showing. */
  public get isOpen(): boolean {
    return this._isOpen;
  }

  /** Open the search modal and focus its input. */
  public open(): void {
    this._openSearch();
  }

  /** Close the search modal, clearing the query and any in-flight search. */
  public close(): void {
    this._closeSearch();
  }

  /**
   * Open the media-browser popup (the `=` shortcut). Resolves once the lazily
   * imported `<mbr-media-browser>` chunk has loaded.
   */
  public openMediaBrowser(): Promise<void> {
    return this._openMediaBrowser();
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
    // Escape to close - media browser popup first, then search modal
    if (e.key === 'Escape') {
      if (this._isMediaBrowserOpen) {
        e.preventDefault();
        this._closeMediaBrowser();
      } else if (this._isFolderPickerOpen) {
        e.preventDefault();
        this._closeFolderPicker();
      } else if (this._isOpen) {
        e.preventDefault();
        this._closeSearch();
      }
    }
  };

  private _openSearch() {
    this._isOpen = true;
    if (getMbrConfig().serverMode) this._loadSiteFacets();
    this.updateComplete.then(() => {
      this._input?.focus();
    });
  }

  /**
   * Derive the folder list and note types from site.json. Runs when the modal
   * opens — never at page load — and only once per site.json payload; both are
   * O(files), which is fine on a user action but not on the critical path.
   */
  private _loadSiteFacets(): void {
    siteNav
      .then((data: { markdown_files?: Array<{ url_path?: string; frontmatter?: Record<string, unknown> }> }) => {
        if (!data || data === this._facetSource) return;
        this._facetSource = data;
        const files = Array.isArray(data.markdown_files) ? data.markdown_files : [];
        this._folders = foldersFromUrlPaths(
          files.map((f) => f?.url_path).filter((p): p is string => typeof p === 'string')
        );
        this._noteTypes = noteTypesFromSite(files);
      })
      .catch(() => {
        // site.json failed: no type options and an empty picker, search still works.
      });
  }

  private _closeSearch() {
    this._isOpen = false;
    this._isFolderPickerOpen = false;
    this._query = '';
    this._results = [];
    this._selectedIndex = -1;
    this._error = null;
    // Invalidate any in-flight search so its response cannot repopulate the
    // results we just cleared (visible again the next time the modal opens).
    this._searchGeneration++;
    this._isLoading = false;
    if (this._abortController) {
      this._abortController.abort();
      this._abortController = null;
    }
    if (this._debounceTimeout) {
      clearTimeout(this._debounceTimeout);
      this._debounceTimeout = null;
    }
  }

  private async _openMediaBrowser() {
    // Dynamically load the media browser component
    await loadMediaBrowser();
    this._isMediaBrowserOpen = true;
    // Focus the text filter after the component renders
    this.updateComplete.then(() => {
      this._mediaBrowser?.focusTextFilter();
    });
  }

  private _closeMediaBrowser() {
    this._isMediaBrowserOpen = false;
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
          const selectedLink = this.shadowRoot?.querySelector('a.result.selected') as HTMLAnchorElement | null;
          if (selectedLink) {
            if (isNewTabModifier(e)) {
              openInNewTab(selectedLink.href);
            } else {
              selectedLink.click();
            }
          }
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
    if (target.value.startsWith(TYPE_OPTION_PREFIX)) {
      this._applyNoteType(target.value.slice(TYPE_OPTION_PREFIX.length));
      // The type is now visible as a `type:` token in the query; the select
      // goes back to showing the scope, which is what it controls.
      target.value = this._scope;
      return;
    }
    this._scope = target.value as SearchScope;
    if (this._query.length >= 2) {
      this._performSearch();
    }
  }

  /**
   * Restrict the search to one note type by writing a `type:` facet into the
   * query. Content-only scope never consults facets on a facet-only query (the
   * server runs content search only when there are free-text terms), so it is
   * widened to All; the other scopes already apply facets.
   */
  private _applyNoteType(type: string): void {
    this._query = withTypeFacet(this._query, type);
    if (this._scope === 'content') this._scope = 'all';
    this._selectedIndex = -1;
    this.updateComplete.then(() => this._input?.focus());
    if (this._query.length >= 2) this._performSearch();
  }

  private _handleFolderScopeChange(e: Event) {
    const target = e.target as HTMLInputElement;
    this._folderScope = target.checked ? 'current' : 'everywhere';
    // Unchecking clears a picked folder: the next check means "here" again.
    if (!target.checked) this._folderOverride = null;
    if (this._query.length >= 2) {
      this._performSearch();
    }
  }

  /** Clicks inside the modal never reach the backdrop; outside the picker they close it. */
  private _handleModalClick = (e: Event): void => {
    e.stopPropagation();
    if (!this._isFolderPickerOpen) return;
    const inPicker = e
      .composedPath()
      .some((node) => node instanceof Element && node.classList.contains('folder-scope'));
    if (!inPicker) this._closeFolderPicker(false);
  };

  /** Folders matching the picker's filter, capped for rendering. */
  private _filteredFolders(): string[] {
    const needle = this._folderFilter.trim().toLowerCase();
    const matches = needle
      ? this._folders.filter((f) => f.toLowerCase().includes(needle))
      : this._folders;
    return matches.slice(0, MAX_LISTED_FOLDERS);
  }

  private _toggleFolderPicker(): void {
    if (this._isFolderPickerOpen) {
      this._closeFolderPicker();
      return;
    }
    this._loadSiteFacets();
    this._isFolderPickerOpen = true;
    this._folderFilter = '';
    this._folderPickerIndex = 0;
    this.updateComplete.then(() => {
      this.shadowRoot?.querySelector<HTMLInputElement>('.folder-filter')?.focus();
    });
  }

  private _closeFolderPicker(refocus = true): void {
    if (!this._isFolderPickerOpen) return;
    this._isFolderPickerOpen = false;
    if (refocus) {
      this.updateComplete.then(() => {
        this.shadowRoot?.querySelector<HTMLButtonElement>('.folder-picker-button')?.focus();
      });
    }
  }

  private _chooseFolder(folder: string): void {
    this._folderOverride = folder;
    this._folderScope = 'current';
    this._closeFolderPicker(false);
    this.updateComplete.then(() => this._input?.focus());
    if (this._query.length >= 2) this._performSearch();
  }

  private _handleFolderFilterInput(e: Event): void {
    this._folderFilter = (e.target as HTMLInputElement).value;
    this._folderPickerIndex = 0;
  }

  private _handleFolderFilterKeydown(e: KeyboardEvent): void {
    const folders = this._filteredFolders();
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        this._folderPickerIndex = Math.min(this._folderPickerIndex + 1, folders.length - 1);
        this._scrollFolderOptionIntoView();
        break;
      case 'ArrowUp':
        e.preventDefault();
        this._folderPickerIndex = Math.max(this._folderPickerIndex - 1, 0);
        this._scrollFolderOptionIntoView();
        break;
      case 'Enter': {
        e.preventDefault();
        const folder = folders[this._folderPickerIndex];
        if (folder) this._chooseFolder(folder);
        break;
      }
      case 'Escape':
        // Close only the picker: the search modal's own Escape handlers (this
        // element's document listener) must not see this keypress.
        e.preventDefault();
        e.stopPropagation();
        this._closeFolderPicker();
        break;
    }
  }

  private _scrollFolderOptionIntoView(): void {
    this.updateComplete.then(() => {
      this.shadowRoot
        ?.querySelector('.folder-option[aria-selected="true"]')
        ?.scrollIntoView?.({ block: 'nearest' });
    });
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
    const abortController = new AbortController();
    this._abortController = abortController;

    const query = this._query;
    const generation = ++this._searchGeneration;
    const isStale = this._makeStaleCheck(generation, query);

    this._isLoading = true;
    this._error = null;

    try {
      // Build search request with folder context
      const searchBody: SearchRequestBody = {
        q: query,
        limit: 20,
        scope: this._scope,
        folder_scope: this._folderScope,
      };

      // Add folder path when searching current folder
      if (this._folderScope === 'current') {
        searchBody.folder = this._folderOverride ?? getCurrentFolder();
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
        signal: abortController.signal,
      });

      const data: SearchResponse = await response.json();

      if (!response.ok || data.error) {
        throw new Error(data.error || `Search failed: ${response.status}`);
      }
      if (isStale()) return;
      this._results = data.results.map((r: SearchResult) => ({ ...r, snippetHtml: null }));
      this._totalMatches = data.total_matches;
      this._durationMs = data.duration_ms;
    } catch (err) {
      if (err instanceof Error && err.name === 'AbortError') {
        // Ignore abort errors
        return;
      }
      if (isStale()) return;
      console.error('Search error:', err);
      this._error = err instanceof Error ? err.message : 'Search failed';
      this._results = [];
    } finally {
      // Only the newest run owns the loading indicator; a superseded run
      // clearing it would hide "Searching..." while a search is still running.
      if (generation === this._searchGeneration) {
        this._isLoading = false;
      }
    }
  }

  /**
   * Perform search using Pagefind (static mode).
   */
  private async _performPagefindSearch() {
    const startTime = performance.now();

    const query = this._query;
    const generation = ++this._searchGeneration;
    const isStale = this._makeStaleCheck(generation, query);

    this._isLoading = true;
    this._error = null;

    try {
      const pagefind = await this._loadPagefind();
      if (isStale()) return;
      if (!pagefind) {
        this._error = 'Search index not available. Run "npx pagefind --site <build_dir> --output-subdir .mbr/pagefind" after building.';
        this._results = [];
        return;
      }

      // Perform the search against the captured query, not the live one
      const searchResponse = await pagefind.search(query);
      if (isStale()) return;

      // Load data for the first 20 results
      const resultPromises = searchResponse.results.slice(0, 20).map(r => r.data());
      const resultData = await Promise.all(resultPromises);
      // All state writes happen after the last await so a superseded run cannot
      // leave _totalMatches describing one query and _results another.
      if (isStale()) return;

      this._totalMatches = searchResponse.results.length;

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
      if (isStale()) return;
      console.error('Pagefind search error:', err);
      this._error = err instanceof Error ? err.message : 'Search failed';
      this._results = [];
    } finally {
      if (generation === this._searchGeneration) {
        this._isLoading = false;
      }
    }
  }

  private _renderResult(result: SearchResult, index: number) {
    const isSelected = index === this._selectedIndex;
    const title = result.title || result.url_path;

    return html`
      <a
        href=${resolveUrl(result.url_path)}
        class="result ${isSelected ? 'selected' : ''} ${result.is_content_match ? 'content-match' : 'metadata-match'}"
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
      </a>
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
        <div class="modal" @click=${this._handleModalClick}>
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
                ${this._noteTypes.length > 0 ? html`
                  <option disabled>── Note types ──</option>
                  ${this._noteTypes.map((t) => html`
                    <option value=${TYPE_OPTION_PREFIX + t.type}>${t.type} (${t.count})</option>
                  `)}
                ` : nothing}
              </select>
            ` : nothing}
          </div>

          ${showScopeSelector ? html`
            <div class="search-options">
              <span class="folder-scope">
                <label class="option-toggle">
                  <input
                    type="checkbox"
                    .checked=${this._folderScope === 'current'}
                    @change=${this._handleFolderScopeChange}
                  />
                  ${this._folderOverride && this._folderScope === 'current'
                    ? html`<span>Only in: <span class="folder-name" title=${this._folderOverride}>${this._folderOverride}</span></span>`
                    : html`<span>Current folder only</span>`}
                </label>
                <button
                  type="button"
                  class="folder-picker-button"
                  title="Choose a folder to search in"
                  aria-label="Choose a folder to search in"
                  aria-haspopup="listbox"
                  aria-expanded=${this._isFolderPickerOpen ? 'true' : 'false'}
                  @click=${this._toggleFolderPicker}
                >
                  <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path>
                  </svg>
                </button>
                ${this._renderFolderPicker()}
              </span>
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
            <button class="media-browser-button" @click=${this._openMediaBrowser} title="Browse media files (=)">
              <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polygon points="23 7 16 12 23 17 23 7"></polygon>
                <rect x="1" y="5" width="15" height="14" rx="2" ry="2"></rect>
              </svg>
              <span>Browse Media</span>
              <kbd>=</kbd>
            </button>
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

  /**
   * The folder picker: a filter field over a listbox of every note folder.
   * Focus stays in the filter field and the highlighted row is announced via
   * `aria-activedescendant`, the same arrow/Enter model as the results list.
   */
  private _renderFolderPicker(): TemplateResult | typeof nothing {
    if (!this._isFolderPickerOpen) return nothing;
    const folders = this._filteredFolders();
    const active = Math.min(this._folderPickerIndex, folders.length - 1);
    return html`
      <div class="folder-picker" @click=${(e: Event) => e.stopPropagation()}>
        <input
          class="folder-filter"
          type="text"
          placeholder="Filter folders…"
          aria-label="Filter folders"
          role="combobox"
          aria-expanded="true"
          aria-controls="folder-listbox"
          aria-activedescendant=${active >= 0 ? `folder-option-${active}` : ''}
          autocomplete="off"
          spellcheck="false"
          .value=${this._folderFilter}
          @input=${this._handleFolderFilterInput}
          @keydown=${this._handleFolderFilterKeydown}
        />
        <ul id="folder-listbox" class="folder-list" role="listbox" aria-label="Folders">
          ${folders.length === 0
            ? html`<li class="folder-empty" role="presentation">No folders match</li>`
            : folders.map((folder, i) => html`
                <li
                  id=${`folder-option-${i}`}
                  class="folder-option"
                  role="option"
                  aria-selected=${i === active ? 'true' : 'false'}
                  title=${folder}
                  @mousedown=${(e: Event) => e.preventDefault()}
                  @click=${() => this._chooseFolder(folder)}
                >${folder}</li>
              `)}
        </ul>
      </div>
    `;
  }

  private _renderMediaBrowserPopup(): TemplateResult | typeof nothing {
    if (!this._isMediaBrowserOpen) return nothing;

    return html`
      <div class="media-browser-backdrop" @click=${this._closeMediaBrowser}>
        <div class="media-browser-popup" @click=${(e: Event) => e.stopPropagation()}>
          <div class="media-browser-header">
            <h2>Browse Media</h2>
            <button
              class="media-browser-close"
              @click=${this._closeMediaBrowser}
              aria-label="Close media browser"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <line x1="18" y1="6" x2="6" y2="18"></line>
                <line x1="6" y1="6" x2="18" y2="18"></line>
              </svg>
            </button>
          </div>
          <div class="media-browser-content">
            <mbr-media-browser></mbr-media-browser>
          </div>
        </div>
      </div>
    `;
  }

  override render() {
    return html`
      ${this._renderTrigger()}
      ${this._renderModal()}
      ${this._renderMediaBrowserPopup()}
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

    /* "Current folder only" + the folder-picker button and its popover */
    .folder-scope {
      position: relative;
      display: inline-flex;
      align-items: center;
      gap: 0.25rem;
      min-width: 0;
    }

    .folder-name {
      display: inline-block;
      max-width: 16rem;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      vertical-align: bottom;
      font-family: var(--pico-font-family-monospace, monospace);
      color: var(--pico-color, #333);
    }

    .folder-picker-button {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      width: 1.5rem;
      height: 1.5rem;
      margin: 0;
      padding: 0;
      border: 1px solid transparent;
      border-radius: 4px;
      background: transparent;
      color: var(--pico-muted-color, #666);
      cursor: pointer;
    }

    .folder-picker-button:hover,
    .folder-picker-button[aria-expanded="true"] {
      color: var(--pico-color, #333);
      border-color: var(--pico-muted-border-color, #ccc);
    }

    .folder-picker-button:focus-visible {
      outline: 2px solid var(--pico-primary, #0172ad);
      outline-offset: 1px;
    }

    .folder-picker {
      position: absolute;
      top: calc(100% + 0.35rem);
      left: 0;
      z-index: 5;
      width: min(22rem, 80vw);
      padding: 0.4rem;
      border: 1px solid var(--pico-muted-border-color, #ddd);
      border-radius: 8px;
      background: var(--pico-background-color, #fff);
      box-shadow: 0 10px 30px -8px rgba(0, 0, 0, 0.3);
    }

    .folder-filter {
      width: 100%;
      box-sizing: border-box;
      margin: 0 0 0.35rem;
      padding: 0.3rem 0.5rem;
      font-size: 0.8rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
    }

    .folder-list {
      list-style: none;
      margin: 0;
      padding: 0;
      max-height: 14rem;
      overflow-y: auto;
    }

    .folder-option,
    .folder-empty {
      margin: 0;
      padding: 0.25rem 0.5rem;
      border-radius: 4px;
      font-family: var(--pico-font-family-monospace, monospace);
      font-size: 0.78rem;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      list-style: none;
    }

    .folder-option {
      cursor: pointer;
      color: var(--pico-color, #333);
    }

    .folder-option:hover {
      background: var(--pico-secondary-background, rgba(0, 0, 0, 0.05));
      background: color-mix(in srgb, var(--pico-primary, #0172ad) 8%, transparent);
    }

    .folder-option[aria-selected="true"] {
      background: color-mix(in srgb, var(--pico-primary, #0172ad) 18%, transparent);
    }

    .folder-empty {
      color: var(--pico-muted-color, #888);
      font-family: inherit;
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
      display: block;
      text-decoration: none;
      color: inherit;
      padding: 0.75rem;
      border-radius: 8px;
      cursor: pointer;
      transition: background 0.1s ease;
    }

    .result:hover,
    .result.selected {
      background: var(--pico-primary-focus, rgba(99, 102, 241, 0.15));
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
      background: var(--pico-primary, #0d6efd);
      color: var(--pico-primary-inverse, #fff);
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
      background: var(--pico-muted-border-color, #d1d5db);
      color: var(--pico-color, #333);
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
      background: var(--pico-code-background-color, #f5f5f5);
      color: var(--pico-code-color, #333);
      font-size: 0.75rem;
    }

    .error {
      color: var(--pico-del-color, #dc3545);
    }

    /* Footer */
    .search-footer {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0.5rem 0.75rem;
      border-top: 1px solid var(--pico-muted-border-color, #eee);
      font-size: 0.75rem;
      color: var(--pico-muted-color, #999);
    }

    .media-browser-button {
      display: inline-flex;
      align-items: center;
      gap: 0.35rem;
      padding: 0.35rem 0.6rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 5px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-muted-color, #666);
      cursor: pointer;
      font-size: 0.75rem;
      transition: all 0.15s ease;
    }

    .media-browser-button:hover {
      border-color: var(--pico-primary, #0d6efd);
      color: var(--pico-primary, #0d6efd);
      background: var(--pico-primary-focus, rgba(13, 110, 253, 0.1));
    }

    .media-browser-button svg {
      flex-shrink: 0;
    }

    .media-browser-button kbd {
      padding: 0.1rem 0.3rem;
      margin-left: 0.25rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 3px;
      background: var(--pico-secondary-background, #f5f5f5);
      font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
      font-size: 0.7rem;
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

    /* Media Browser Popup */
    .media-browser-backdrop {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.7);
      display: flex;
      align-items: center;
      justify-content: center;
      z-index: 1100;
      padding: 1rem;
      animation: fadeIn 0.2s ease;
    }

    @keyframes fadeIn {
      from { opacity: 0; }
      to { opacity: 1; }
    }

    .media-browser-popup {
      width: 90vw;
      max-width: 90vw;
      height: 90vh;
      max-height: 90vh;
      background: var(--pico-background-color, #fff);
      border-radius: 12px;
      box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.4);
      display: flex;
      flex-direction: column;
      overflow: hidden;
      animation: slideUp 0.25s ease;
    }

    @keyframes slideUp {
      from { opacity: 0; transform: translateY(20px); }
      to { opacity: 1; transform: translateY(0); }
    }

    .media-browser-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0.75rem 1rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    .media-browser-header h2 {
      margin: 0;
      font-size: 1.1rem;
      font-weight: 600;
      color: var(--pico-color, #333);
    }

    .media-browser-close {
      display: flex;
      align-items: center;
      justify-content: center;
      width: 32px;
      height: 32px;
      padding: 0;
      border: none;
      border-radius: 6px;
      background: transparent;
      color: var(--pico-muted-color, #666);
      cursor: pointer;
      transition: all 0.15s ease;
    }

    .media-browser-close:hover {
      background: var(--pico-secondary-background, #f5f5f5);
      color: var(--pico-color, #333);
    }

    .media-browser-content {
      flex: 1;
      overflow: hidden;
    }

    .media-browser-content mbr-media-browser {
      height: 100%;
    }

    /* Responsive adjustments for media browser */
    @media (max-width: 768px) {
      .media-browser-popup {
        max-width: 100%;
        height: 100vh;
        max-height: 100vh;
        border-radius: 0;
      }

      .media-browser-backdrop {
        padding: 0;
      }
    }
  `;
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-search': MbrSearchElement
  }
}
