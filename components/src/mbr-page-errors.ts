import { LitElement, html, css, nothing, type TemplateResult } from 'lit';
import { customElement, state } from 'lit/decorators.js';

/**
 * The tagged variants emitted by `src/page_errors.rs`. Extending this union
 * here when adding new error types keeps the Rust and TS sides in lock-step.
 */
interface BrokenInternalLinkError {
  type: 'broken_internal_link';
  target: string;
  text: string;
  anchor?: string;
}

interface BrokenMediaReferenceError {
  type: 'broken_media_reference';
  src: string;
  kind: 'image' | 'video' | 'audio' | 'source';
}

interface UnresolvedWikilinkError {
  type: 'unresolved_wikilink';
  raw: string;
}

interface FrontmatterParseError {
  type: 'frontmatter_parse_error';
  message: string;
}

/**
 * A parent/child (or other inverse-pair) loop in the typed relationships. The
 * genealogy chart cannot lay out a cyclic hierarchy, so one of the closing
 * edges is discarded — see `breakHierarchicalCycles` in
 * `graph/relationship-graph.ts`.
 */
interface RelationshipCycleError {
  type: 'relationship_cycle';
  /** Note `url_path`s taking part in the loop, in traversal order. */
  members: string[];
  /** Canonical relationship type whose edges close the loop, e.g. "child". */
  rel_type: string;
}

/**
 * A relationship endpoint that matched several notes and was resolved to the
 * first one silently.
 */
interface AmbiguousRelationshipEndpointError {
  type: 'ambiguous_relationship_endpoint';
  raw: string;
  resolved_to: string;
  candidates: string[];
}

/** The same ambiguity, for a `[[wikilink]]` in the page body. */
interface AmbiguousWikilinkError {
  type: 'ambiguous_wikilink';
  raw: string;
  resolved_to: string;
  candidates: string[];
}

/**
 * Both ambiguity variants carry an identical payload, so one renderer serves
 * both and only the heading differs.
 */
type AmbiguousNameError =
  | AmbiguousRelationshipEndpointError
  | AmbiguousWikilinkError;

type PageErrorEntry =
  | BrokenInternalLinkError
  | BrokenMediaReferenceError
  | UnresolvedWikilinkError
  | FrontmatterParseError
  | RelationshipCycleError
  | AmbiguousNameError;

interface PageErrorsResponse {
  page_url: string;
  errors: PageErrorEntry[];
}

/**
 * Singular / plural wording per problem type, in the order the detail groups
 * are rendered below, so the summary reads in the same order as the panel.
 */
const ERROR_LABELS: ReadonlyArray<
  readonly [PageErrorEntry['type'], string, string]
> = [
  ['broken_internal_link', 'broken link', 'broken links'],
  [
    'broken_media_reference',
    'broken media reference',
    'broken media references',
  ],
  ['unresolved_wikilink', 'unresolved wikilink', 'unresolved wikilinks'],
  [
    'frontmatter_parse_error',
    'frontmatter parse error',
    'frontmatter parse errors',
  ],
  ['relationship_cycle', 'relationship cycle', 'relationship cycles'],
  [
    'ambiguous_relationship_endpoint',
    'ambiguous relationship name',
    'ambiguous relationship names',
  ],
  ['ambiguous_wikilink', 'ambiguous wikilink', 'ambiguous wikilinks'],
];

/**
 * One-line summary naming ONLY the problem types actually present.
 *
 * Enumerating every type — "0 broken links, 0 broken media references, 1
 * frontmatter parse error, 0 relationship cycles, …" — buries the one real
 * problem in noise, and gets worse with each type added. Zero-count types are
 * therefore omitted entirely.
 *
 * Returns '' for an empty list; the panel is not rendered in that case.
 */
export function summarizePageErrors(errors: PageErrorEntry[]): string {
  const phrases = ERROR_LABELS.flatMap(([type, singular, plural]) => {
    const count = errors.filter((e) => e.type === type).length;
    return count === 0 ? [] : [`${count} ${count === 1 ? singular : plural}`];
  });
  if (phrases.length === 0) return '';
  const last = phrases[phrases.length - 1];
  const listed =
    phrases.length === 1
      ? last
      : // Oxford comma from three items up; a bare "and" for exactly two.
        phrases.length === 2
        ? `${phrases[0]} and ${last}`
        : `${phrases.slice(0, -1).join(', ')}, and ${last}`;
  return `Detected ${listed}.`;
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-page-errors': MbrPageErrorsElement;
  }
}

/**
 * Per-page error indicator. Appears only in server / GUI mode, and only when
 * the current page has at least one detected problem (broken link, broken
 * media, unresolved wikilink).
 *
 * Static-site guarantee: the template wraps this element in
 * `{% if server_mode %}`, the endpoint is only registered in `src/server.rs`,
 * and this component bails out on `connectedCallback` if
 * `window.__MBR_CONFIG__.serverMode` is not truthy. Any one of those guards
 * is sufficient to keep the element inert on static output.
 */
@customElement('mbr-page-errors')
export class MbrPageErrorsElement extends LitElement {
  @state()
  private _isOpen = false;

  @state()
  private _errors: PageErrorEntry[] = [];

  @state()
  private _loaded = false;

  override connectedCallback() {
    super.connectedCallback();

    // Self-guard: belt-and-suspenders so a static site that ships a custom
    // template with this element never fires a fetch.
    const config = (window as Window).__MBR_CONFIG__;
    if (!config?.serverMode) {
      return;
    }

    document.addEventListener('keydown', this._handleKeydown);
    void this._loadErrors();
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener('keydown', this._handleKeydown);
  }

  private _handleKeydown = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && this._isOpen) {
      e.preventDefault();
      this._close();
    }
  };

  private async _loadErrors() {
    try {
      const currentPath = window.location.pathname;
      const normalizedPath = currentPath.endsWith('/')
        ? currentPath
        : currentPath + '/';
      const url = normalizedPath + 'errors.json';

      const response = await fetch(url);

      // 404 is expected whenever the page isn't a markdown page, or when
      // link tracking is disabled. Stay silent in that case.
      if (!response.ok) {
        return;
      }

      const data = (await response.json()) as PageErrorsResponse;
      this._errors = Array.isArray(data.errors) ? data.errors : [];
      this._loaded = true;
    } catch (err) {
      // Graceful degradation: swallow network / parse errors. The indicator
      // is informational; failing to load should not break the page.
      console.warn('[mbr-page-errors] failed to load errors.json:', err);
    }
  }

  private _open() {
    this._isOpen = true;
  }

  private _close() {
    this._isOpen = false;
  }

  private _toggle() {
    if (this._isOpen) this._close();
    else this._open();
  }

  private _renderLinkGroup(): TemplateResult | typeof nothing {
    const items = this._errors.filter(
      (e): e is BrokenInternalLinkError => e.type === 'broken_internal_link'
    );
    if (items.length === 0) return nothing;

    return html`
      <section class="error-group">
        <h3>Broken internal links (${items.length})</h3>
        <ul>
          ${items.map(
            (e) => html`
              <li>
                <code class="target">${e.target}${e.anchor ?? ''}</code>
                ${e.text ? html`<span class="text"> — ${e.text}</span>` : nothing}
              </li>
            `
          )}
        </ul>
      </section>
    `;
  }

  private _renderMediaGroup(): TemplateResult | typeof nothing {
    const items = this._errors.filter(
      (e): e is BrokenMediaReferenceError => e.type === 'broken_media_reference'
    );
    if (items.length === 0) return nothing;

    return html`
      <section class="error-group">
        <h3>Broken media references (${items.length})</h3>
        <ul>
          ${items.map(
            (e) => html`
              <li>
                <span class="kind">[${e.kind}]</span>
                <code class="target">${e.src}</code>
              </li>
            `
          )}
        </ul>
      </section>
    `;
  }

  private _renderWikilinkGroup(): TemplateResult | typeof nothing {
    const items = this._errors.filter(
      (e): e is UnresolvedWikilinkError => e.type === 'unresolved_wikilink'
    );
    if (items.length === 0) return nothing;

    return html`
      <section class="error-group">
        <h3>Unresolved wikilinks (${items.length})</h3>
        <ul>
          ${items.map(
            (e) => html`<li><code class="target">${e.raw}</code></li>`
          )}
        </ul>
      </section>
    `;
  }

  private _renderFrontmatterGroup(): TemplateResult | typeof nothing {
    const items = this._errors.filter(
      (e): e is FrontmatterParseError => e.type === 'frontmatter_parse_error'
    );
    if (items.length === 0) return nothing;

    return html`
      <section class="error-group">
        <h3>Frontmatter parse errors (${items.length})</h3>
        <ul>
          ${items.map(
            (e) => html`<li><code class="target">${e.message}</code></li>`
          )}
        </ul>
      </section>
    `;
  }

  private _renderRelationshipCycleGroup(): TemplateResult | typeof nothing {
    const items = this._errors.filter(
      (e): e is RelationshipCycleError => e.type === 'relationship_cycle'
    );
    if (items.length === 0) return nothing;

    return html`
      <section class="error-group">
        <h3>Relationship cycles (${items.length})</h3>
        <ul>
          ${items.map(
            (e) => html`
              <li>
                <span class="kind">[${e.rel_type}]</span>
                <code class="target">${e.members.join(' → ')}</code>
                <span class="text">
                  — each of these notes claims to be an ancestor of the next,
                  which makes the family chart unrenderable until one of the
                  links is corrected.
                </span>
              </li>
            `
          )}
        </ul>
      </section>
    `;
  }

  /** Shared renderer for the two identically-shaped ambiguity variants. */
  private _renderAmbiguousGroup(
    heading: string,
    items: AmbiguousNameError[]
  ): TemplateResult | typeof nothing {
    if (items.length === 0) return nothing;

    return html`
      <section class="error-group">
        <h3>${heading} (${items.length})</h3>
        <ul>
          ${items.map(
            (e) => html`
              <li>
                <code class="target">${e.raw}</code>
                <span class="text"> — resolved to </span>
                <code class="target">${e.resolved_to}</code>
                ${e.candidates.length > 0
                  ? html`<div class="candidates">
                      <span class="text">also matches:</span>
                      ${e.candidates.map(
                        (c) => html`<code class="target">${c}</code>`
                      )}
                    </div>`
                  : nothing}
              </li>
            `
          )}
        </ul>
      </section>
    `;
  }

  private _renderAmbiguousEndpointGroup(): TemplateResult | typeof nothing {
    return this._renderAmbiguousGroup(
      'Ambiguous relationship names',
      this._errors.filter(
        (e): e is AmbiguousRelationshipEndpointError =>
          e.type === 'ambiguous_relationship_endpoint'
      )
    );
  }

  private _renderAmbiguousWikilinkGroup(): TemplateResult | typeof nothing {
    return this._renderAmbiguousGroup(
      'Ambiguous wikilinks',
      this._errors.filter(
        (e): e is AmbiguousWikilinkError => e.type === 'ambiguous_wikilink'
      )
    );
  }

  private _renderTrigger(): TemplateResult {
    const count = this._errors.length;
    const label = `This page has ${count} problem${count === 1 ? '' : 's'}`;

    return html`
      <button
        class="errors-trigger"
        @click=${() => this._toggle()}
        aria-label=${label}
        title=${label}
      >
        <span class="errors-icon">&#9888;</span>
        <span class="errors-count">${count}</span>
      </button>
    `;
  }

  private _renderPanel(): TemplateResult {
    return html`
      <div class="errors-backdrop" @click=${() => this._close()}></div>
      <aside class="errors-panel" aria-label="Page problems">
        <div class="errors-panel-content">
          <button
            class="errors-panel-close"
            @click=${() => this._close()}
            aria-label="Close errors panel"
          >
            <span aria-hidden="true">&times;</span>
          </button>
          <h2>
            Page Problems
            <span class="total-count">(${this._errors.length})</span>
          </h2>
          <p class="summary">${summarizePageErrors(this._errors)}</p>
          ${this._renderLinkGroup()}
          ${this._renderMediaGroup()}
          ${this._renderWikilinkGroup()}
          ${this._renderFrontmatterGroup()}
          ${this._renderRelationshipCycleGroup()}
          ${this._renderAmbiguousEndpointGroup()}
          ${this._renderAmbiguousWikilinkGroup()}
        </div>
      </aside>
    `;
  }

  override render() {
    // Hidden unless we have loaded and found at least one problem.
    if (!this._loaded || this._errors.length === 0) {
      return nothing;
    }
    return html`
      ${this._renderTrigger()}
      ${this._isOpen ? this._renderPanel() : nothing}
    `;
  }

  static override styles = css`
    :host {
      display: contents;
    }

    .errors-trigger {
      cursor: pointer;
      width: auto;
      min-width: 2rem;
      height: 2rem;
      padding: 0 0.4rem;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 0.25rem;
      border-radius: 4px;
      border: 1px solid transparent;
      background: transparent;
      transition: background 0.15s ease, border-color 0.15s ease;
      color: var(--pico-del-color, #b84a2b);
    }

    .errors-trigger:hover {
      border-color: var(--pico-contrast-hover-border, rgba(0, 0, 0, 0.1));
      background: var(
        --pico-secondary-background,
        rgba(184, 74, 43, 0.08)
      );
    }

    .errors-icon {
      font-size: 1.05rem;
      line-height: 1;
    }

    .errors-count {
      font-size: 0.85rem;
      font-weight: 600;
      line-height: 1;
    }

    /* Panel styling mirrors mbr-info's slide-out drawer for consistency. */
    .errors-backdrop {
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
      from {
        opacity: 0;
      }
      to {
        opacity: 1;
      }
    }

    .errors-panel {
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
      from {
        transform: translateX(100%);
      }
      to {
        transform: translateX(0);
      }
    }

    .errors-panel-content {
      padding: 2rem;
      max-width: 100%;
    }

    .errors-panel-content h2 {
      margin-top: 0;
      margin-bottom: 1rem;
      color: var(--pico-color, #333);
    }

    .total-count {
      font-weight: 400;
      color: var(--pico-muted-color, #666);
      font-size: 0.85em;
    }

    .errors-panel-close {
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
      font-size: 1.6rem;
      line-height: 1;
      color: var(--pico-color, #333);
    }

    .errors-panel-close:hover {
      background-color: var(--pico-secondary-hover, #f0f0f0);
    }

    .summary {
      color: var(--pico-muted-color, #666);
      font-size: 0.95rem;
      margin-bottom: 1.5rem;
    }

    .error-group {
      margin-bottom: 1.5rem;
      border: 1px solid var(--pico-muted-border-color, #e0e0e0);
      border-radius: 4px;
      padding: 0.75rem 1rem;
    }

    .error-group h3 {
      font-size: 0.95rem;
      margin: 0 0 0.5rem 0;
      color: var(--pico-color, #333);
    }

    .error-group ul {
      list-style: none;
      padding-left: 0;
      margin: 0;
    }

    .error-group li {
      margin: 0.25rem 0;
      padding: 0.25rem 0;
      border-bottom: 1px solid var(--pico-muted-border-color, #f0f0f0);
      font-size: 0.9rem;
    }

    .error-group li:last-child {
      border-bottom: none;
    }

    .kind {
      display: inline-block;
      margin-right: 0.25rem;
      color: var(--pico-muted-color, #666);
      font-size: 0.8rem;
      text-transform: uppercase;
      letter-spacing: 0.05em;
    }

    .target {
      font-family: var(--pico-font-family-monospace, monospace);
      background-color: var(--pico-code-background-color, #f4f4f4);
      padding: 0.1em 0.3em;
      border-radius: 3px;
      word-break: break-all;
    }

    .text {
      color: var(--pico-muted-color, #666);
      font-style: italic;
    }

    .candidates {
      margin-top: 0.2rem;
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 0.25rem;
      font-size: 0.85em;
    }

    @media (min-width: 768px) {
      .errors-panel {
        max-width: 450px;
        width: 45vw;
        min-width: 320px;
      }
    }

    @media (min-width: 1200px) {
      .errors-panel {
        max-width: 550px;
        width: 35vw;
        min-width: 400px;
      }
    }

    @media (max-width: 767px) {
      .errors-panel {
        width: 100vw;
        max-width: 100vw;
      }

      .errors-panel-content {
        padding: 1.5rem 1rem;
      }
    }
  `;
}
