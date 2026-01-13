import { LitElement, html, css, nothing, type TemplateResult } from 'lit';
import { customElement, state } from 'lit/decorators.js';

interface Heading {
  level: number;
  id: string;
  text: string;
}

interface Frontmatter {
  [key: string]: unknown;
}

declare global {
  interface Window {
    frontmatter?: Frontmatter;
    headings?: Heading[];
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

  // Keys to skip (internal/technical fields)
  private static skipKeys = new Set(['markdown_source', 'server_mode']);

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
    this._isOpen = !this._isOpen;
  }

  private _open() {
    this._isOpen = true;
  }

  private _close() {
    this._isOpen = false;
  }

  private _getOrderedKeys(): string[] {
    const allKeys = Object.keys(this._frontmatter)
      .filter(k => !MbrInfoElement.skipKeys.has(k));

    return [
      ...MbrInfoElement.preferredOrder.filter(k => allKeys.includes(k)),
      ...allKeys.filter(k => !MbrInfoElement.preferredOrder.includes(k))
    ];
  }

  private _formatKey(key: string): string {
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
      return html`
                  <tr>
                    <th scope="row">${this._formatKey(key)}</th>
                    <td>${this._formatValue(value)}</td>
                  </tr>
                `;
    })}
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

  private _renderLinksOutSection(): TemplateResult {
    return html`
      <details class="info-section">
        <summary><strong>Links Out</strong></summary>
        <div class="info-content">
          <p><em>Links from this document (coming soon)</em></p>
        </div>
      </details>
    `;
  }

  private _renderLinksInSection(): TemplateResult {
    return html`
      <details class="info-section">
        <summary><strong>Links In</strong></summary>
        <div class="info-content">
          <p><em>Links to this document (coming soon)</em></p>
        </div>
      </details>
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
          ${this._renderTocSection()}
          ${this._renderLinksOutSection()}
          ${this._renderLinksInSection()}
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
