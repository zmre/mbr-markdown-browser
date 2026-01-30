import { LitElement, css, html, nothing, type TemplateResult } from 'lit';
import { customElement, property, state, query } from 'lit/decorators.js';
import { subscribeSiteNav } from './shared.js';
import {
  type OtherFileInfo,
  type MediaType,
  MEDIA_TYPE_PRIORITY,
  isMediaFile,
  getMediaType,
  getMediaTitle,
  getCoverImageUrl,
  getViewerUrl,
  getMediaTypeLabel,
  formatFileSize,
  formatDuration,
} from './types.js';

/**
 * Event detail for media selection.
 */
export interface MediaSelectEventDetail {
  file: OtherFileInfo;
  viewerUrl: string;
}

/**
 * Media browser component for browsing media files (videos, PDFs, audio, images).
 *
 * Displays a grid of media cards with cover images, filtering by type,
 * and text search. Designed to be opened in a popup from the search component.
 *
 * @element mbr-media-browser
 *
 * @fires mbr-media-select - Dispatched when user selects a media card (before navigation)
 * @fires mbr-media-close - Dispatched on Escape key press (only in inline mode)
 *
 * @csspart grid - The media card grid container
 * @csspart card - Individual media cards
 *
 * @cssprop [--mbr-media-grid-gap=1rem] - Gap between grid cards
 * @cssprop [--mbr-media-card-min=200px] - Minimum card width
 * @cssprop [--mbr-media-card-max=1fr] - Maximum card width
 */
@customElement('mbr-media-browser')
export class MbrMediaBrowserElement extends LitElement {
  // === Configuration ===

  /**
   * When true, the component is used inline (not in a popup).
   * In inline mode, Escape key dispatches mbr-media-close event
   * instead of propagating to close a parent popup.
   */
  @property({ type: Boolean })
  inline = false;

  // === Data State ===
  @state()
  private _allMediaFiles: OtherFileInfo[] = [];

  @state()
  private _isLoading = true;

  @state()
  private _error: string | null = null;

  // === Filter State ===
  @state()
  private _selectedType: MediaType | null = null;

  @state()
  private _availableTypes: MediaType[] = [];

  @state()
  private _textFilter = '';

  // === Sorting State ===
  @state()
  private _sortField: 'created' | 'modified' | 'alpha' = 'created';

  @state()
  private _sortDirection: 'asc' | 'desc' = 'desc';

  // === Pagination State ===
  @state()
  private _displayLimit = 200;

  // === Cover Image State ===
  @state()
  private _failedCovers = new Set<string>();

  @query('#text-filter')
  private _textFilterInput!: HTMLInputElement;

  private _unsubscribeSiteNav: (() => void) | null = null;

  // ========================================
  // Lifecycle
  // ========================================

  override connectedCallback() {
    super.connectedCallback();

    // Subscribe to site navigation data
    this._unsubscribeSiteNav = subscribeSiteNav((siteNavState) => {
      this._isLoading = siteNavState.isLoading;
      this._error = siteNavState.error;

      if (siteNavState.data?.other_files) {
        this._processMediaFiles(siteNavState.data.other_files);
      }
    });
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    if (this._unsubscribeSiteNav) {
      this._unsubscribeSiteNav();
    }
  }

  // ========================================
  // Public Methods
  // ========================================

  /**
   * Focus the text filter input. Called by parent after popup opens.
   */
  public focusTextFilter(): void {
    this.updateComplete.then(() => {
      this._textFilterInput?.focus();
    });
  }

  /**
   * Reset all filters and sorting to their default values.
   * Useful for clearing state when the component is reopened.
   */
  public reset(): void {
    this._textFilter = '';
    this._sortField = 'created';
    this._sortDirection = 'desc';
    this._displayLimit = 200;

    // Reset selected type to first available, or null if none
    this._selectedType = this._availableTypes.length > 0 ? this._availableTypes[0] : null;
  }

  // ========================================
  // Data Processing
  // ========================================

  /**
   * Process static files from site.json and filter to media files only.
   */
  private _processMediaFiles(otherFiles: OtherFileInfo[]): void {
    // Filter to only media files (video, pdf, audio, image)
    this._allMediaFiles = otherFiles.filter(isMediaFile);

    // Determine which media types are available
    const typeSet = new Set<MediaType>();
    for (const file of this._allMediaFiles) {
      const mediaType = getMediaType(file);
      if (mediaType) {
        typeSet.add(mediaType);
      }
    }

    // Sort available types by priority
    this._availableTypes = MEDIA_TYPE_PRIORITY.filter((t) => typeSet.has(t));

    // Auto-select first available type if nothing selected
    if (this._selectedType === null && this._availableTypes.length > 0) {
      this._selectedType = this._availableTypes[0];
    }
  }

  // ========================================
  // Filtering
  // ========================================

  /**
   * Check if a file matches the selected type filter.
   */
  private _matchesType(file: OtherFileInfo): boolean {
    if (this._selectedType === null) {
      return true;
    }
    return getMediaType(file) === this._selectedType;
  }

  /**
   * Check if a file matches the text filter.
   */
  private _matchesTextFilter(file: OtherFileInfo): boolean {
    if (!this._textFilter.trim()) {
      return true;
    }

    const searchText = this._textFilter.toLowerCase();
    const title = getMediaTitle(file).toLowerCase();
    const path = file.url_path.toLowerCase();

    return title.includes(searchText) || path.includes(searchText);
  }

  /**
   * Compare two files for sorting.
   */
  private _compareFiles(a: OtherFileInfo, b: OtherFileInfo): number {
    const direction = this._sortDirection === 'asc' ? 1 : -1;

    if (this._sortField === 'alpha') {
      // Alphabetical sort by title
      const titleA = getMediaTitle(a).toLowerCase();
      const titleB = getMediaTitle(b).toLowerCase();
      return direction * titleA.localeCompare(titleB);
    }

    // Date-based sorting (created or modified)
    const dateA =
      this._sortField === 'created' ? a.metadata.created : a.metadata.modified;
    const dateB =
      this._sortField === 'created' ? b.metadata.created : b.metadata.modified;

    // Files missing dates sort to the end
    if (dateA === undefined && dateB === undefined) {
      return 0;
    }
    if (dateA === undefined) {
      return 1; // a goes to end
    }
    if (dateB === undefined) {
      return -1; // b goes to end
    }

    return direction * (dateA - dateB);
  }

  /**
   * Get filtered media files based on current filters.
   */
  private _getFilteredFiles(): OtherFileInfo[] {
    const filtered = this._allMediaFiles.filter(
      (file) => this._matchesType(file) && this._matchesTextFilter(file)
    );

    // Apply sorting
    return filtered.sort((a, b) => this._compareFiles(a, b));
  }

  /**
   * Get displayed files (filtered, sorted, and paginated).
   */
  private _getDisplayedFiles(): OtherFileInfo[] {
    const filtered = this._getFilteredFiles();
    return filtered.slice(0, this._displayLimit);
  }

  /**
   * Get count of files for a specific media type.
   */
  private _getTypeCount(type: MediaType): number {
    return this._allMediaFiles.filter((file) => getMediaType(file) === type).length;
  }

  // ========================================
  // Event Handlers
  // ========================================

  private _handleTypeSelect(type: MediaType): void {
    this._selectedType = type;
    this._displayLimit = 200; // Reset pagination on type change
  }

  private _handleTextFilterInput(e: Event): void {
    const target = e.target as HTMLInputElement;
    this._textFilter = target.value;
    this._displayLimit = 200; // Reset pagination on text filter change
  }

  private _handleSortChange(e: Event): void {
    const target = e.target as HTMLSelectElement;
    const value = target.value;

    // Parse the combined sort value (e.g., "created-desc", "alpha-asc")
    const [field, direction] = value.split('-') as [
      'created' | 'modified' | 'alpha',
      'asc' | 'desc',
    ];
    this._sortField = field;
    this._sortDirection = direction;
    this._displayLimit = 200; // Reset pagination on sort change
  }

  private _handleLoadMore(): void {
    this._displayLimit += 200;
  }

  private _handleCardClick(file: OtherFileInfo): void {
    const viewerUrl = getViewerUrl(file);

    // Dispatch selection event before navigation
    const event = new CustomEvent<MediaSelectEventDetail>('mbr-media-select', {
      detail: { file, viewerUrl },
      bubbles: true,
      composed: true,
    });
    this.dispatchEvent(event);

    // Navigate to the viewer
    window.location.href = viewerUrl;
  }

  private _handleCardKeydown(e: KeyboardEvent, file: OtherFileInfo): void {
    // Enter or Space activates the card
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      this._handleCardClick(file);
    }
  }

  private _handleCoverError(file: OtherFileInfo): void {
    // Track failed cover images to show fallback
    this._failedCovers = new Set([...this._failedCovers, file.url_path]);
  }

  private _handleKeydown(e: KeyboardEvent): void {
    // Handle Escape key
    if (e.key === 'Escape') {
      if (this.inline) {
        // In inline mode, dispatch close event
        e.preventDefault();
        e.stopPropagation();
        this.dispatchEvent(
          new CustomEvent('mbr-media-close', {
            bubbles: true,
            composed: true,
          })
        );
      }
      // In popup mode (inline=false), let Escape propagate to close popup
      return;
    }
  }

  private _handleRetry(): void {
    // Re-trigger data load by forcing a state update
    this._error = null;
    this._isLoading = true;
    // The subscribeSiteNav should have already loaded, but we can't re-trigger it
    // So just clear the error state
  }

  // ========================================
  // Render Helpers
  // ========================================

  /**
   * Get CSS class for fallback gradient based on media type.
   */
  private _getFallbackGradient(type: MediaType): string {
    switch (type) {
      case 'video':
        return 'fallback-video';
      case 'pdf':
        return 'fallback-pdf';
      case 'audio':
        return 'fallback-audio';
      case 'image':
        return 'fallback-image';
    }
  }

  /**
   * Get icon SVG for media type.
   */
  private _getTypeIcon(type: MediaType): TemplateResult {
    switch (type) {
      case 'video':
        return html`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="23 7 16 12 23 17 23 7"></polygon><rect x="1" y="5" width="15" height="14" rx="2" ry="2"></rect></svg>`;
      case 'pdf':
        return html`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path><polyline points="14 2 14 8 20 8"></polyline><line x1="16" y1="13" x2="8" y2="13"></line><line x1="16" y1="17" x2="8" y2="17"></line><polyline points="10 9 9 9 8 9"></polyline></svg>`;
      case 'audio':
        return html`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle></svg>`;
      case 'image':
        return html`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect><circle cx="8.5" cy="8.5" r="1.5"></circle><polyline points="21 15 16 10 5 21"></polyline></svg>`;
    }
  }

  /**
   * Get additional metadata for display based on file type.
   */
  private _getMetadataDisplay(file: OtherFileInfo): string {
    const kind = file.metadata.kind;
    const parts: string[] = [];

    // Add duration for video/audio
    if (kind.type === 'video' && kind.duration) {
      parts.push(formatDuration(kind.duration));
    } else if (kind.type === 'audio' && kind.duration) {
      parts.push(formatDuration(kind.duration));
    }

    // Add page count for PDFs
    if (kind.type === 'pdf' && kind.num_pages) {
      parts.push(`${kind.num_pages} pages`);
    }

    // Add dimensions for images/videos
    if ((kind.type === 'image' || kind.type === 'video') && kind.width && kind.height) {
      parts.push(`${kind.width}x${kind.height}`);
    }

    // Add file size
    const size = formatFileSize(file.metadata.file_size_bytes);
    if (size) {
      parts.push(size);
    }

    return parts.join(' | ');
  }

  // ========================================
  // Render Methods
  // ========================================

  private _renderTypeFilter(): TemplateResult | typeof nothing {
    if (this._availableTypes.length <= 1) {
      return nothing;
    }

    return html`
      <div class="type-filter" role="tablist" aria-label="Filter by media type">
        ${this._availableTypes.map((type) => {
          const count = this._getTypeCount(type);
          const isSelected = this._selectedType === type;
          return html`
            <button
              role="tab"
              class="type-tab ${isSelected ? 'selected' : ''}"
              aria-selected="${isSelected}"
              aria-pressed="${isSelected}"
              @click=${() => this._handleTypeSelect(type)}
            >
              <span class="type-icon">${this._getTypeIcon(type)}</span>
              <span class="type-label">${getMediaTypeLabel(type)}</span>
              <span class="type-count">${count}</span>
            </button>
          `;
        })}
      </div>
    `;
  }

  private _renderTextFilter(): TemplateResult {
    return html`
      <div class="text-filter-wrapper">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          class="search-icon"
        >
          <circle cx="11" cy="11" r="8"></circle>
          <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
        </svg>
        <input
          id="text-filter"
          type="search"
          placeholder="Filter by title or path..."
          .value=${this._textFilter}
          @input=${this._handleTextFilterInput}
          @keydown=${this._handleKeydown}
          autocomplete="off"
          spellcheck="false"
        />
      </div>
    `;
  }

  private _renderSortDropdown(): TemplateResult {
    const currentValue = `${this._sortField}-${this._sortDirection}`;

    return html`
      <div class="sort-wrapper">
        <label for="sort-select" class="sort-label">Sort:</label>
        <select
          id="sort-select"
          class="sort-select"
          .value=${currentValue}
          @change=${this._handleSortChange}
        >
          <option value="created-desc">Newest First</option>
          <option value="modified-desc">Recently Modified</option>
          <option value="alpha-asc">A-Z</option>
          <option value="alpha-desc">Z-A</option>
        </select>
      </div>
    `;
  }

  private _renderCard(file: OtherFileInfo): TemplateResult {
    const mediaType = getMediaType(file);
    if (!mediaType) return html``;

    const title = getMediaTitle(file);
    const coverUrl = getCoverImageUrl(file);
    const hasCoverFailed = this._failedCovers.has(file.url_path);
    const showCover = coverUrl && !hasCoverFailed;
    const metadata = this._getMetadataDisplay(file);

    // Extract filename from path for display
    const pathParts = file.url_path.split('/');
    const filename = pathParts[pathParts.length - 1] || '';

    return html`
      <article
        class="media-card"
        part="card"
        data-type="${mediaType}"
        @click=${() => this._handleCardClick(file)}
        @keydown=${(e: KeyboardEvent) => this._handleCardKeydown(e, file)}
        tabindex="0"
        role="button"
        aria-label="Open ${title}"
      >
        <header class="card-header">
          <span class="card-type-icon">${this._getTypeIcon(mediaType)}</span>
          <span class="card-type-label">${mediaType.toUpperCase()}</span>
        </header>
        <div class="card-body ${showCover ? '' : this._getFallbackGradient(mediaType)}">
          ${showCover
            ? html`
                <img
                  src="${coverUrl}"
                  alt=""
                  loading="lazy"
                  @error=${() => this._handleCoverError(file)}
                />
              `
            : html` <span class="fallback-icon">${this._getTypeIcon(mediaType)}</span> `}
        </div>
        <footer class="card-footer">
          <strong class="media-title" title="${title}">${title}</strong>
          ${metadata ? html`<small class="media-meta">${metadata}</small>` : nothing}
          <small class="media-path" title="${file.url_path}">${filename}</small>
        </footer>
      </article>
    `;
  }

  private _renderGrid(): TemplateResult {
    const filteredFiles = this._getFilteredFiles();
    const displayedFiles = this._getDisplayedFiles();

    if (filteredFiles.length === 0) {
      return html`
        <div class="empty-state">
          ${this._textFilter
            ? html`<p>No media files match "${this._textFilter}"</p>`
            : html`<p>No media files found</p>`}
        </div>
      `;
    }

    const hasMore = displayedFiles.length < filteredFiles.length;

    return html`
      <div
        class="media-grid"
        part="grid"
        role="tabpanel"
        aria-label="Media files"
      >
        ${displayedFiles.map((file) => this._renderCard(file))}
      </div>
      ${hasMore
        ? html`
            <footer class="pagination-footer">
              <span class="pagination-info">
                Showing ${displayedFiles.length} of ${filteredFiles.length}
              </span>
              <button class="load-more-button" @click=${this._handleLoadMore}>
                Load More
              </button>
            </footer>
          `
        : nothing}
    `;
  }

  private _renderLoading(): TemplateResult {
    return html`
      <div class="loading-state" aria-busy="true">
        <p>Loading media files...</p>
      </div>
    `;
  }

  private _renderError(): TemplateResult {
    return html`
      <div class="error-state">
        <p class="error-message">Failed to load media files</p>
        <p class="error-detail">${this._error}</p>
        <button class="retry-button" @click=${this._handleRetry}>Retry</button>
      </div>
    `;
  }

  override render() {
    if (this._isLoading) {
      return this._renderLoading();
    }

    if (this._error) {
      return this._renderError();
    }

    if (this._allMediaFiles.length === 0) {
      return html`
        <div class="empty-state">
          <p>No media files found in this repository</p>
        </div>
      `;
    }

    const filteredCount = this._getFilteredFiles().length;
    const totalCount = this._selectedType
      ? this._getTypeCount(this._selectedType)
      : this._allMediaFiles.length;

    return html`
      <div class="media-browser">
        <div class="browser-header">
          ${this._renderTypeFilter()}
          <div class="header-controls">
            ${this._renderTextFilter()} ${this._renderSortDropdown()}
          </div>
        </div>
        <div class="results-info">
          ${this._textFilter
            ? html`Showing ${filteredCount} of ${totalCount} files`
            : html`${totalCount} files`}
        </div>
        <div class="browser-content">${this._renderGrid()}</div>
      </div>
    `;
  }

  // ========================================
  // Styles
  // ========================================

  static override styles = css`
    :host {
      display: block;
      height: 100%;

      /* CSS Custom Properties with defaults */
      --_grid-gap: var(--mbr-media-grid-gap, 1rem);
      --_card-min: var(--mbr-media-card-min, 200px);
      --_card-max: var(--mbr-media-card-max, 1fr);
    }

    /* Browser layout */
    .media-browser {
      display: flex;
      flex-direction: column;
      height: 100%;
      background: var(--pico-background-color, #fff);
    }

    .browser-header {
      display: flex;
      flex-direction: column;
      gap: 0.75rem;
      padding: 0.75rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    @media (min-width: 640px) {
      .browser-header {
        flex-direction: row;
        align-items: center;
        justify-content: space-between;
      }
    }

    .header-controls {
      display: flex;
      gap: 0.75rem;
      flex-wrap: wrap;
      align-items: center;
    }

    @media (max-width: 639px) {
      .header-controls {
        flex-direction: column;
        width: 100%;
      }

      .text-filter-wrapper {
        width: 100%;
      }

      .sort-wrapper {
        width: 100%;
      }
    }

    .results-info {
      padding: 0.25rem 0.75rem;
      font-size: 0.75rem;
      color: var(--pico-muted-color, #666);
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    .browser-content {
      flex: 1;
      overflow-y: auto;
      padding: 0.75rem;
    }

    /* Type filter tabs */
    .type-filter {
      display: flex;
      gap: 0.25rem;
      flex-wrap: wrap;
    }

    .type-tab {
      display: inline-flex;
      align-items: center;
      gap: 0.35rem;
      padding: 0.4rem 0.75rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 6px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-muted-color, #666);
      cursor: pointer;
      font-size: 0.8rem;
      transition: all 0.15s ease;
    }

    .type-tab:hover {
      border-color: var(--pico-primary, #0d6efd);
      color: var(--pico-color, #333);
    }

    .type-tab.selected {
      background: var(--pico-primary, #0d6efd);
      border-color: var(--pico-primary, #0d6efd);
      color: var(--pico-primary-inverse, #fff);
    }

    .type-icon {
      display: flex;
      align-items: center;
    }

    .type-icon svg {
      width: 16px;
      height: 16px;
    }

    .type-label {
      font-weight: 500;
    }

    .type-count {
      padding: 0.1rem 0.35rem;
      background: rgba(0, 0, 0, 0.1);
      border-radius: 10px;
      font-size: 0.7rem;
    }

    .type-tab.selected .type-count {
      background: rgba(255, 255, 255, 0.2);
    }

    /* Text filter */
    .text-filter-wrapper {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      padding: 0.4rem 0.75rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 6px;
      background: var(--pico-background-color, #fff);
      min-width: 200px;
    }

    .text-filter-wrapper:focus-within {
      border-color: var(--pico-primary, #0d6efd);
      box-shadow: 0 0 0 2px var(--pico-primary-focus, rgba(13, 110, 253, 0.25));
    }

    .search-icon {
      color: var(--pico-muted-color, #999);
      flex-shrink: 0;
    }

    #text-filter {
      flex: 1;
      border: none;
      background: transparent;
      font-size: 0.875rem;
      color: var(--pico-color, #333);
      outline: none;
      min-width: 0;
    }

    #text-filter::placeholder {
      color: var(--pico-muted-color, #999);
    }

    /* Sort dropdown */
    .sort-wrapper {
      display: flex;
      align-items: center;
      gap: 0.5rem;
    }

    .sort-label {
      font-size: 0.8rem;
      color: var(--pico-muted-color, #666);
      white-space: nowrap;
    }

    .sort-select {
      padding: 0.4rem 0.75rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 6px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
      font-size: 0.8rem;
      cursor: pointer;
      min-width: 150px;
    }

    .sort-select:focus {
      border-color: var(--pico-primary, #0d6efd);
      box-shadow: 0 0 0 2px var(--pico-primary-focus, rgba(13, 110, 253, 0.25));
      outline: none;
    }

    /* Media grid */
    .media-grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(var(--_card-min), var(--_card-max)));
      gap: var(--_grid-gap);
    }

    @media (min-width: 1200px) {
      .media-grid {
        grid-template-columns: repeat(
          auto-fill,
          minmax(max(var(--_card-min), 220px), var(--_card-max))
        );
      }
    }

    /* Media cards */
    .media-card {
      display: flex;
      flex-direction: column;
      border: 1px solid var(--pico-muted-border-color, #eee);
      border-radius: 8px;
      overflow: hidden;
      cursor: pointer;
      transition: border-color 0.15s ease, box-shadow 0.15s ease, transform 0.15s ease;
      background: var(--pico-card-background-color, #fff);
    }

    .media-card:hover,
    .media-card:focus {
      border-color: var(--pico-primary, #0d6efd);
      box-shadow: 0 4px 12px rgba(0, 0, 0, 0.1);
      transform: translateY(-2px);
      outline: none;
    }

    .media-card:focus-visible {
      box-shadow: 0 0 0 3px var(--pico-primary-focus, rgba(13, 110, 253, 0.25));
    }

    .card-header {
      display: flex;
      align-items: center;
      gap: 0.35rem;
      padding: 0.4rem 0.6rem;
      background: var(--pico-secondary-background, #f5f5f5);
      font-size: 0.65rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      color: var(--pico-muted-color, #666);
    }

    .card-type-icon {
      display: flex;
      align-items: center;
    }

    .card-type-icon svg {
      width: 12px;
      height: 12px;
    }

    .card-body {
      aspect-ratio: 16 / 10;
      display: flex;
      align-items: center;
      justify-content: center;
      overflow: hidden;
      position: relative;
    }

    .card-body img {
      width: 100%;
      height: 100%;
      object-fit: cover;
    }

    /* Fallback gradients for missing covers */
    .fallback-video {
      background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    }

    .fallback-pdf {
      background: linear-gradient(135deg, #f093fb 0%, #f5576c 100%);
    }

    .fallback-audio {
      background: linear-gradient(135deg, #4facfe 0%, #00f2fe 100%);
    }

    .fallback-image {
      background: linear-gradient(135deg, #43e97b 0%, #38f9d7 100%);
    }

    .fallback-icon {
      display: flex;
      align-items: center;
      justify-content: center;
      color: rgba(255, 255, 255, 0.8);
    }

    .fallback-icon svg {
      width: 48px;
      height: 48px;
    }

    .card-footer {
      padding: 0.6rem;
      display: flex;
      flex-direction: column;
      gap: 0.25rem;
    }

    .media-title {
      font-size: 0.85rem;
      font-weight: 600;
      line-height: 1.3;
      color: var(--pico-color, #333);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .media-meta {
      font-size: 0.7rem;
      color: var(--pico-primary, #0d6efd);
    }

    .media-path {
      font-size: 0.7rem;
      color: var(--pico-muted-color, #999);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    /* Pagination footer */
    .pagination-footer {
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 1rem;
      padding: 1.5rem;
      margin-top: 1rem;
      border-top: 1px solid var(--pico-muted-border-color, #eee);
    }

    .pagination-info {
      font-size: 0.875rem;
      color: var(--pico-muted-color, #666);
    }

    .load-more-button {
      padding: 0.5rem 1.25rem;
      border: 1px solid var(--pico-primary, #0d6efd);
      border-radius: 6px;
      background: var(--pico-primary, #0d6efd);
      color: var(--pico-primary-inverse, #fff);
      cursor: pointer;
      font-size: 0.875rem;
      font-weight: 500;
      transition: background-color 0.15s ease;
    }

    .load-more-button:hover {
      background: var(--pico-primary-hover, #0b5ed7);
    }

    .load-more-button:focus {
      box-shadow: 0 0 0 3px var(--pico-primary-focus, rgba(13, 110, 253, 0.25));
      outline: none;
    }

    /* States */
    .loading-state,
    .error-state,
    .empty-state {
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      padding: 3rem;
      text-align: center;
      color: var(--pico-muted-color, #666);
      min-height: 200px;
    }

    .error-state {
      color: var(--pico-del-color, #dc3545);
    }

    .error-message {
      font-weight: 500;
      margin-bottom: 0.5rem;
    }

    .error-detail {
      font-size: 0.875rem;
      margin-bottom: 1rem;
      color: var(--pico-muted-color, #666);
    }

    .retry-button {
      padding: 0.5rem 1rem;
      border: 1px solid var(--pico-primary, #0d6efd);
      border-radius: 6px;
      background: var(--pico-primary, #0d6efd);
      color: var(--pico-primary-inverse, #fff);
      cursor: pointer;
      font-size: 0.875rem;
    }

    .retry-button:hover {
      background: var(--pico-primary-hover, #0b5ed7);
    }
  `;
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-media-browser': MbrMediaBrowserElement;
  }
}
