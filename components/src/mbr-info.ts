import { LitElement, html, css, nothing, type TemplateResult } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { getTagSources, resolveUrl, type TagSourceConfig } from './shared.js';

interface Heading {
  level: number;
  id: string;
  text: string;
}

interface Frontmatter {
  [key: string]: unknown;
}

interface OutboundLink {
  to: string;
  text: string;
  anchor?: string;
  internal: boolean;
}

interface InboundLink {
  from: string;
  text: string;
  anchor?: string;
}

interface PageLinks {
  inbound: InboundLink[];
  outbound: OutboundLink[];
}

interface PageNavLink {
  url: string;
  title: string;
}

interface ExtendedMeta {
  wordCount: number;
  readingTimeMinutes: number;
  filePath: string;
  modifiedTimestamp: number;
  prevPage?: PageNavLink;
  nextPage?: PageNavLink;
}

declare global {
  interface Window {
    frontmatter?: Frontmatter;
    headings?: Heading[];
    extendedMeta?: ExtendedMeta;
  }
  interface HTMLElementTagNameMap {
    'mbr-info': MbrInfoElement;
  }
}

/**
 * Info panel component - displays document metadata, table of contents, and links.
 * Self-contained with trigger button and slide-out panel.
 */
@customElement('mbr-info')
export class MbrInfoElement extends LitElement {
  @state()
  private _isOpen = false;

  @state()
  private _frontmatter: Frontmatter = {};

  @state()
  private _headings: Heading[] = [];

  @state()
  private _links: PageLinks | null = null;

  @state()
  private _linksLoading = false;

  @state()
  private _linksError: string | null = null;

  @state()
  private _extendedMeta: ExtendedMeta | null = null;

  // Keys to skip (internal/technical fields)
  private static skipKeys = new Set(['markdown_source', 'server_mode', 'gui_mode']);

  // Preferred display order for common keys
  private static preferredOrder = [
    'title', 'description', 'tags', 'keywords', 'date',
    'author', 'category', 'categories', 'draft', 'slug'
  ];

  override connectedCallback() {
    super.connectedCallback();
    document.addEventListener('keydown', this._handleKeydown);

    // Load frontmatter and headings from window
    this._frontmatter = window.frontmatter || {};
    this._headings = window.headings || [];
    this._extendedMeta = window.extendedMeta || null;
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener('keydown', this._handleKeydown);
  }

  private _handleKeydown = (e: KeyboardEvent) => {
    // Ctrl+g or Cmd+g to toggle
    if ((e.ctrlKey || e.metaKey) && e.key === 'g') {
      e.preventDefault();
      this._toggle();
    }
    // Escape to close
    if (e.key === 'Escape' && this._isOpen) {
      e.preventDefault();
      this._close();
    }
  };

  private _toggle() {
    if (this._isOpen) {
      this._close();
    } else {
      this._open();
    }
  }

  private _open() {
    this._isOpen = true;
    // Load links data when panel opens (if not already loaded)
    if (!this._links && !this._linksLoading) {
      this._loadLinks();
    }
  }

  private _close() {
    this._isOpen = false;
  }

  private async _loadLinks() {
    this._linksLoading = true;
    this._linksError = null;

    try {
      // Get current path and construct links.json URL
      const currentPath = window.location.pathname;
      // Ensure path ends with / for directory-style URLs
      const normalizedPath = currentPath.endsWith('/') ? currentPath : currentPath + '/';
      const linksUrl = normalizedPath + 'links.json';

      const response = await fetch(linksUrl);

      if (!response.ok) {
        if (response.status === 404) {
          // Link tracking may be disabled - not an error
          this._links = { inbound: [], outbound: [] };
          return;
        }
        throw new Error(`Failed to load links: ${response.status}`);
      }

      this._links = await response.json() as PageLinks;
    } catch (error) {
      console.warn('Failed to load links:', error);
      this._linksError = error instanceof Error ? error.message : 'Unknown error';
      this._links = { inbound: [], outbound: [] };
    } finally {
      this._linksLoading = false;
    }
  }

  private _getOrderedKeys(): string[] {
    const allKeys = Object.keys(this._frontmatter)
      .filter(k => !MbrInfoElement.skipKeys.has(k));

    return [
      ...MbrInfoElement.preferredOrder.filter(k => allKeys.includes(k)),
      ...allKeys.filter(k => !MbrInfoElement.preferredOrder.includes(k))
    ];
  }

  private _formatKey(key: string, tagSource?: TagSourceConfig): string {
    // Use configured label if this is a tag field
    if (tagSource) {
      return tagSource.label;
    }
    // Default: title-case the field name
    return key.charAt(0).toUpperCase() + key.slice(1).replace(/_/g, ' ');
  }

  private _formatValue(value: unknown): string {
    if (Array.isArray(value)) {
      return value.join(', ');
    } else if (typeof value === 'object' && value !== null) {
      return JSON.stringify(value);
    } else {
      return String(value);
    }
  }

  // ========================================
  // Tag Linking Helpers
  // ========================================

  /**
   * Normalize a tag value for URL construction.
   * Converts to lowercase and replaces spaces with underscores.
   */
  private _normalizeTagValue(value: string): string {
    return value.toLowerCase().replace(/\s+/g, '_');
  }

  /**
   * Find a matching TagSource config for a frontmatter field.
   * Matches field names case-insensitively.
   */
  private _getTagSourceForField(field: string): TagSourceConfig | undefined {
    const lowerField = field.toLowerCase();
    return getTagSources().find(ts => ts.field.toLowerCase() === lowerField);
  }

  /**
   * Build a URL for a tag value using the tag source's URL pattern.
   * E.g., for urlSource="tags" and value="Rust", returns "/tags/rust/"
   */
  private _buildTagUrl(tagSource: TagSourceConfig, value: string): string {
    const normalized = this._normalizeTagValue(value);
    return resolveUrl(`/${tagSource.urlSource}/${normalized}/`);
  }

  /**
   * Render a tag value as a clickable link.
   */
  private _renderTagLink(tagSource: TagSourceConfig, value: string): TemplateResult {
    const url = this._buildTagUrl(tagSource, value);
    return html`<a href="${url}" class="tag-link" @click=${() => this._close()}>${value}</a>`;
  }

  /**
   * Render a metadata value, using links for tag fields.
   */
  private _renderValue(key: string, value: unknown): TemplateResult | string {
    const tagSource = this._getTagSourceForField(key);

    // If not a tag field, use simple formatting
    if (!tagSource) {
      return this._formatValue(value);
    }

    // Handle arrays of tags
    if (Array.isArray(value)) {
      const links = value.map((v, i) => {
        const str = String(v);
        const link = this._renderTagLink(tagSource, str);
        return i < value.length - 1 ? html`${link}, ` : link;
      });
      return html`${links}`;
    }

    // Handle single tag value
    return this._renderTagLink(tagSource, String(value));
  }

  // ========================================
  // Extended Metadata Helpers
  // ========================================

  /**
   * Format a UNIX timestamp to a readable date string.
   */
  private _formatModifiedDate(timestamp: number): string {
    if (!timestamp) return '';
    const date = new Date(timestamp * 1000);
    return date.toLocaleDateString(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
    });
  }

  /**
   * Format reading time in a human-friendly way.
   */
  private _formatReadingTime(minutes: number): string {
    if (minutes < 1) return '< 1 min read';
    if (minutes === 1) return '1 min read';
    return `${minutes} min read`;
  }

  // ========================================
  // Render Methods
  // ========================================

  private _renderTrigger(): TemplateResult {
    return html`
      <button
        class="info-trigger"
        @click=${() => this._open()}
        aria-label="Open info panel"
        title="Info (Ctrl+g)"
      >
        <span class="info-icon">ℹ</span>
      </button>
    `;
  }

  private _renderMetadataSection(): TemplateResult | typeof nothing {
    const keys = this._getOrderedKeys();

    if (keys.length === 0) {
      return nothing;
    }

    return html`
      <details class="info-section" open>
        <summary><strong>Metadata</strong></summary>
        <div class="info-content">
          <table class="metadata-table striped">
            <tbody>
              ${keys.map(key => {
      const value = this._frontmatter[key];
      if (value === null || value === undefined || value === '') {
        return nothing;
      }
      const tagSource = this._getTagSourceForField(key);
      return html`
                  <tr>
                    <th scope="row">${this._formatKey(key, tagSource)}</th>
                    <td>${this._renderValue(key, value)}</td>
                  </tr>
                `;
    })}
            </tbody>
          </table>
        </div>
      </details>
    `;
  }

  private _renderDocumentInfoSection(): TemplateResult | typeof nothing {
    const meta = this._extendedMeta;
    if (!meta || (!meta.wordCount && !meta.modifiedTimestamp && !meta.filePath)) {
      return nothing;
    }

    return html`
      <details class="info-section" open>
        <summary><strong>Document Info</strong></summary>
        <div class="info-content">
          <table class="metadata-table striped">
            <tbody>
              ${meta.readingTimeMinutes > 0 ? html`
                <tr>
                  <th scope="row">Reading Time</th>
                  <td>
                    <span class="reading-time" title="${meta.wordCount.toLocaleString()} words">
                      ${this._formatReadingTime(meta.readingTimeMinutes)}
                    </span>
                  </td>
                </tr>
              ` : nothing}
              ${meta.modifiedTimestamp > 0 ? html`
                <tr>
                  <th scope="row">Modified</th>
                  <td>${this._formatModifiedDate(meta.modifiedTimestamp)}</td>
                </tr>
              ` : nothing}
              ${meta.filePath ? html`
                <tr>
                  <th scope="row">File</th>
                  <td><code class="file-path">${meta.filePath}</code></td>
                </tr>
              ` : nothing}
            </tbody>
          </table>
        </div>
      </details>
    `;
  }

  private _renderTocSection(): TemplateResult | typeof nothing {
    if (this._headings.length === 0) {
      return nothing;
    }

    return html`
      <details class="info-section" open>
        <summary><strong>Table of Contents</strong></summary>
        <nav class="toc-nav">
          <ul class="toc-list">
            ${this._headings.map(heading => html`
              <li class="toc-item toc-level-${heading.level}">
                <a href="#${heading.id}" @click=${() => this._close()}>${heading.text}</a>
              </li>
            `)}
          </ul>
        </nav>
      </details>
    `;
  }

  private _renderLinksOutSection(): TemplateResult | typeof nothing {
    if (this._linksLoading) {
      return html`
        <details class="info-section">
          <summary><strong>Links Out</strong></summary>
          <div class="info-content">
            <p class="loading-text">Loading...</p>
          </div>
        </details>
      `;
    }

    if (this._linksError) {
      return nothing; // Hide section on error
    }

    const outbound = this._links?.outbound || [];
    const internalLinks = outbound.filter(l => l.internal);
    const externalLinks = outbound.filter(l => !l.internal);

    if (outbound.length === 0) {
      return nothing;
    }

    return html`
      <details class="info-section">
        <summary><strong>Links Out</strong> <span class="link-count">(${outbound.length})</span></summary>
        <div class="info-content">
          ${internalLinks.length > 0 ? html`
            <div class="link-group">
              <h4 class="link-group-title">Internal (${internalLinks.length})</h4>
              <ul class="links-list">
                ${internalLinks.map(link => html`
                  <li class="link-item">
                    <a href="${link.to}${link.anchor || ''}" class="link-url" @click=${() => this._close()}>
                      ${link.text || link.to}
                    </a>
                    ${link.anchor ? html`<span class="link-anchor">${link.anchor}</span>` : nothing}
                  </li>
                `)}
              </ul>
            </div>
          ` : nothing}
          ${externalLinks.length > 0 ? html`
            <div class="link-group">
              <h4 class="link-group-title">External (${externalLinks.length})</h4>
              <ul class="links-list">
                ${externalLinks.map(link => html`
                  <li class="link-item">
                    <a href="${link.to}" class="link-url external" target="_blank" rel="noopener">
                      ${link.text || link.to}
                      <span class="external-icon" aria-label="Opens in new tab">↗</span>
                    </a>
                  </li>
                `)}
              </ul>
            </div>
          ` : nothing}
        </div>
      </details>
    `;
  }

  private _renderLinksInSection(): TemplateResult | typeof nothing {
    if (this._linksLoading) {
      return html`
        <details class="info-section">
          <summary><strong>Links In</strong></summary>
          <div class="info-content">
            <p class="loading-text">Loading...</p>
          </div>
        </details>
      `;
    }

    if (this._linksError) {
      return nothing; // Hide section on error
    }

    const inbound = this._links?.inbound || [];

    if (inbound.length === 0) {
      return nothing;
    }

    return html`
      <details class="info-section">
        <summary><strong>Links In</strong> <span class="link-count">(${inbound.length})</span></summary>
        <div class="info-content">
          <ul class="links-list">
            ${inbound.map(link => html`
              <li class="link-item">
                <a href="${link.from}" class="link-url" @click=${() => this._close()}>
                  <span class="link-source">${link.from}</span>
                </a>
                ${link.text ? html`<span class="link-text">"${link.text}"</span>` : nothing}
                ${link.anchor ? html`<span class="link-anchor">${link.anchor}</span>` : nothing}
              </li>
            `)}
          </ul>
        </div>
      </details>
    `;
  }

  private _renderPageNavSection(): TemplateResult | typeof nothing {
    const meta = this._extendedMeta;
    if (!meta || (!meta.prevPage && !meta.nextPage)) {
      return nothing;
    }

    return html`
      <div class="page-nav">
        ${meta.prevPage
          ? html`
            <a href="${meta.prevPage.url}" class="nav-prev" @click=${() => this._close()}>
              <span class="nav-arrow">←</span>
              <span class="nav-label">Previous</span>
              <span class="nav-title">${meta.prevPage.title}</span>
            </a>
          `
          : html`<div class="nav-spacer"></div>`}
        ${meta.nextPage
          ? html`
            <a href="${meta.nextPage.url}" class="nav-next" @click=${() => this._close()}>
              <span class="nav-label">Next</span>
              <span class="nav-title">${meta.nextPage.title}</span>
              <span class="nav-arrow">→</span>
            </a>
          `
          : html`<div class="nav-spacer"></div>`}
      </div>
    `;
  }

  private _renderPanel(): TemplateResult {
    return html`
      <div class="info-panel-backdrop" @click=${() => this._close()}></div>
      <aside class="info-panel" aria-label="Document information">
        <div class="info-panel-content">
          <button class="info-panel-close" @click=${() => this._close()} aria-label="Close info panel">
            <span class="info-panel-close-icon">&times;</span>
          </button>

          <h2>Info</h2>

          ${this._renderMetadataSection()}
          ${this._renderDocumentInfoSection()}
          ${this._renderTocSection()}
          ${this._renderLinksOutSection()}
          ${this._renderLinksInSection()}
          ${this._renderPageNavSection()}
        </div>
      </aside>
    `;
  }

  override render() {
    return html`
      ${this._renderTrigger()}
      ${this._isOpen ? this._renderPanel() : nothing}
    `;
  }

  // ========================================
  // Styles
  // ========================================

  static override styles = css`
    :host {
      display: contents;
    }

    /* Trigger button */
    .info-trigger {
      cursor: pointer;
      width: 2rem;
      height: 2rem;
      padding: 0;
      display: flex;
      align-items: center;
      justify-content: center;
      border-radius: 4px;
      border: none;
      background: transparent;
      transition: background 0.15s ease;
    }

    .info-trigger:hover {
      border: 1px solid var(--pico-contrast-hover-border, rgba(0, 0, 0, 0.05));
    }

    .info-icon {
      font-size: 1.1rem;
      line-height: 1;
      font-style: normal;
      color: var(--pico-color, #333);
    }

    /* Backdrop overlay */
    .info-panel-backdrop {
      position: fixed;
      top: 0;
      left: 0;
      width: 100vw;
      height: 100vh;
      background-color: rgba(0, 0, 0, 0.5);
      z-index: 1000;
      animation: fadeIn 0.2s ease;
    }

    @keyframes fadeIn {
      from { opacity: 0; }
      to { opacity: 1; }
    }

    /* Info panel drawer */
    .info-panel {
      position: fixed;
      top: 0;
      right: 0;
      height: 100vh;
      width: 100%;
      max-width: 500px;
      background-color: var(--pico-background-color, #fff);
      border-left: 1px solid var(--pico-muted-border-color, #e0e0e0);
      box-shadow: -4px 0 12px rgba(0, 0, 0, 0.2);
      overflow-y: auto;
      z-index: 1001;
      animation: slideIn 0.25s ease;
    }

    @keyframes slideIn {
      from { transform: translateX(100%); }
      to { transform: translateX(0); }
    }

    /* Panel content container */
    .info-panel-content {
      padding: 2rem;
      max-width: 100%;
    }

    .info-panel-content h2 {
      margin-top: 0;
      margin-bottom: 1.5rem;
      color: var(--pico-color, #333);
    }

    /* Close button */
    .info-panel-close {
      position: absolute;
      top: 1rem;
      right: 1rem;
      width: 2rem;
      height: 2rem;
      display: flex;
      align-items: center;
      justify-content: center;
      cursor: pointer;
      border-radius: 4px;
      border: none;
      background: transparent;
      transition: background-color 0.2s ease;
    }

    .info-panel-close:hover {
      background-color: var(--pico-secondary-hover, #f0f0f0);
    }

    .info-panel-close-icon {
      font-size: 2rem;
      line-height: 1;
      color: var(--pico-color, #333);
    }

    /* Info section styling */
    .info-section {
      margin-bottom: 1.5rem;
      border: 1px solid var(--pico-muted-border-color, #e0e0e0);
      border-radius: 4px;
      padding: 0.5rem;
    }

    .info-section summary {
      cursor: pointer;
      padding: 0.5rem;
      user-select: none;
      color: var(--pico-primary, #1095c1);
      border-radius: 4px;
      transition: background-color 0.2s ease, color 0.2s ease;
    }

    .info-section summary:hover {
      background-color: var(--pico-primary-hover-background, #1095c1);
      color: var(--pico-primary-inverse, #fff);
    }

    .info-section summary:hover strong {
      color: var(--pico-primary-inverse, #fff);
    }

    .info-content {
      padding: 1rem;
    }

    .info-content p {
      margin: 0;
      color: var(--pico-muted-color, #666);
    }

    /* Metadata table styling */
    .metadata-table {
      width: 100%;
      margin: 0;
    }

    .metadata-table th {
      text-align: left;
      font-weight: 600;
      color: var(--pico-muted-color, #666);
      white-space: nowrap;
      width: 1%;  /* Shrink to fit content */
      padding-right: 1rem;
      border-bottom: 1px solid var(--pico-muted-border-color);
    }

    .metadata-table td {
      color: var(--pico-color, #333);
      border-bottom: 1px solid var(--pico-muted-border-color);
    }

    /* Tag link styling */
    .tag-link {
      color: var(--pico-primary, #1095c1);
      text-decoration: none;
      transition: color 0.2s ease;
    }

    .tag-link:hover {
      text-decoration: underline;
      color: var(--pico-primary-hover, #0d7a9e);
    }

    /* Extended metadata styling */
    .reading-time {
      cursor: help;
    }

    .file-path {
      font-size: 0.85em;
      background-color: var(--pico-code-background-color, #f4f4f4);
      padding: 0.1em 0.3em;
      border-radius: 3px;
      word-break: break-all;
    }

    /* Table of Contents styling */
    .toc-nav {
      padding: 1rem 0;
    }

    .toc-list {
      list-style: none;
      padding-left: 0;
      margin: 0;
    }

    .toc-item {
      margin: 0.25rem 0;
    }

    .toc-item a {
      color: var(--pico-color, #333);
      text-decoration: none;
      display: block;
      padding: 0.25rem 0.5rem;
      border-radius: 4px;
      transition: background-color 0.2s ease, color 0.2s ease;
    }

    .toc-item a:hover {
      background-color: var(--pico-secondary-hover-background, #d3d3d3);
      color: var(--pico-secondary-inverse, #fff);
      text-decoration: none;
    }

    /* Indentation for different heading levels */
    .toc-level-1 {
      padding-left: 0;
      font-weight: 600;
    }

    .toc-level-2 {
      padding-left: 1rem;
    }

    .toc-level-3 {
      padding-left: 2rem;
    }

    .toc-level-4 {
      padding-left: 3rem;
    }

    .toc-level-5 {
      padding-left: 4rem;
    }

    .toc-level-6 {
      padding-left: 5rem;
      font-size: 0.9em;
    }

    /* Links section styling */
    .link-count {
      font-size: 0.85em;
      font-weight: normal;
      color: var(--pico-muted-color, #666);
    }

    .link-group {
      margin-bottom: 1rem;
    }

    .link-group:last-child {
      margin-bottom: 0;
    }

    .link-group-title {
      font-size: 0.85rem;
      font-weight: 600;
      color: var(--pico-muted-color, #666);
      margin: 0 0 0.5rem 0;
      text-transform: uppercase;
      letter-spacing: 0.05em;
    }

    .links-list {
      list-style: none;
      padding-left: 0;
      margin: 0;
    }

    .link-item {
      margin: 0.25rem 0;
      padding: 0.25rem 0;
      border-bottom: 1px solid var(--pico-muted-border-color, #e0e0e0);
    }

    .link-item:last-child {
      border-bottom: none;
    }

    .link-url {
      color: var(--pico-primary, #1095c1);
      text-decoration: none;
      display: inline-block;
      word-break: break-word;
    }

    .link-url:hover {
      text-decoration: underline;
    }

    .link-url.external {
      color: var(--pico-secondary, #5755d9);
    }

    .external-icon {
      font-size: 0.8em;
      margin-left: 0.25rem;
      opacity: 0.7;
    }

    .link-source {
      font-family: var(--pico-font-family-monospace, monospace);
      font-size: 0.9em;
    }

    .link-text {
      display: block;
      font-size: 0.85em;
      color: var(--pico-muted-color, #666);
      font-style: italic;
      margin-top: 0.15rem;
    }

    .link-anchor {
      display: inline-block;
      font-size: 0.8em;
      color: var(--pico-muted-color, #666);
      margin-left: 0.5rem;
      font-family: var(--pico-font-family-monospace, monospace);
    }

    .loading-text {
      color: var(--pico-muted-color, #666);
      font-style: italic;
    }

    /* Page navigation styling */
    .page-nav {
      display: flex;
      justify-content: space-between;
      gap: 1rem;
      margin-top: 1.5rem;
      padding-top: 1.5rem;
      border-top: 1px solid var(--pico-muted-border-color, #e0e0e0);
    }

    .nav-prev,
    .nav-next {
      display: flex;
      flex-direction: column;
      padding: 0.75rem 1rem;
      border-radius: 4px;
      text-decoration: none;
      background-color: var(--pico-secondary-background, #f8f8f8);
      transition: background-color 0.2s ease;
      max-width: 45%;
    }

    .nav-prev {
      align-items: flex-start;
    }

    .nav-next {
      align-items: flex-end;
      text-align: right;
    }

    .nav-prev:hover,
    .nav-next:hover {
      background-color: var(--pico-primary-hover-background, #1095c1);
      color: var(--pico-primary-inverse, #fff);
    }

    .nav-prev:hover .nav-label,
    .nav-prev:hover .nav-title,
    .nav-prev:hover .nav-arrow,
    .nav-next:hover .nav-label,
    .nav-next:hover .nav-title,
    .nav-next:hover .nav-arrow {
      color: var(--pico-primary-inverse, #fff);
    }

    .nav-arrow {
      font-size: 1.2rem;
      color: var(--pico-primary, #1095c1);
    }

    .nav-label {
      font-size: 0.75rem;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      color: var(--pico-muted-color, #666);
    }

    .nav-title {
      font-size: 0.9rem;
      color: var(--pico-color, #333);
      font-weight: 500;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      max-width: 150px;
    }

    .nav-spacer {
      flex: 1;
    }

    /* Responsive design for tablets */
    @media (min-width: 768px) {
      .info-panel {
        max-width: 450px;
        width: 45vw;
        min-width: 320px;
      }
    }

    /* Responsive design for larger screens */
    @media (min-width: 1200px) {
      .info-panel {
        max-width: 550px;
        width: 35vw;
        min-width: 400px;
      }
    }

    /* Responsive design for extra-wide screens */
    @media (min-width: 1800px) {
      .info-panel {
        max-width: 650px;
        width: 30vw;
        min-width: 500px;
      }
    }

    /* Responsive design for mobile */
    @media (max-width: 767px) {
      .info-panel {
        width: 100vw;
        max-width: 100vw;
      }

      .info-panel-content {
        padding: 1.5rem 1rem;
      }
    }
  `;
}
