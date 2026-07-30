import { LitElement, css, html, nothing } from 'lit';
import { customElement, query, state } from 'lit/decorators.js';
import type { MbrOverlay } from './overlay.js';
import {
  SEARCH_ROOT_SELECTOR,
  buildTextIndex,
  compileQuery,
  findMatchOffsets,
  rangeForMatch,
  scrollRangeIntoView,
  type TextIndex,
} from './find-in-page.js';

/** Highlight registry name for every match except the active one. */
const HIGHLIGHT_ALL = 'mbr-find';

/** Highlight registry name for the match the reader is currently on. */
const HIGHLIGHT_ACTIVE = 'mbr-find-active';

/** Trailing debounce on typing. Enter / find-next flush any pending scan. */
const INPUT_DEBOUNCE_MS = 120;

/** Coalescing window for content mutations while the bar is open. */
const REINDEX_DEBOUNCE_MS = 250;

/**
 * Most matches kept navigable. Offsets are 8 bytes each, so this is cheap; the
 * cap only exists so a one-letter query on a huge document cannot grow without
 * bound. `total` stays exact past it (see {@link findMatchOffsets}).
 */
const MATCH_CAP = 10000;

/**
 * Most `Range`s materialized at once. Live ranges are not free — the engine
 * revalidates every one on every DOM mutation — so past this a sliding window
 * around the active match is painted instead.
 */
const HIGHLIGHT_CAP = 2000;

const NO_OFFSETS = new Int32Array(0);

/**
 * The document-scoped Custom Highlight API, or `null` where it is missing
 * (older WebKitGTK is the realistic gap; WKWebView needs Safari 17.2+ and
 * WebView2 needs Chromium 105+). Resolved per call so a test can stub it.
 */
function highlightApi(): { registry: HighlightRegistry; create: (ranges: Range[]) => Highlight } | null {
  const scope = globalThis as { CSS?: { highlights?: HighlightRegistry }; Highlight?: typeof Highlight };
  const registry = scope.CSS?.highlights;
  const ctor = scope.Highlight;
  if (!registry || typeof ctor !== 'function') return null;
  return {
    registry,
    create: (ranges) => {
      const highlight = new ctor();
      for (const range of ranges) highlight.add(range);
      return highlight;
    },
  };
}

/** Half-open `[from, to)` slice of matches to materialize as `Range`s. */
function highlightWindow(count: number, active: number): [number, number] {
  if (count <= HIGHLIGHT_CAP) return [0, count];
  const half = HIGHLIGHT_CAP >> 1;
  const from = Math.max(0, Math.min(Math.max(active, 0) - half, count - HIGHLIGHT_CAP));
  return [from, from + HIGHLIGHT_CAP];
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-find-bar': MbrFindBarElement;
  }
}

/**
 * Find-in-page bar for GUI mode.
 *
 * wry wraps a bare WKWebView / WebView2 / WebKitGTK with no browser chrome, so
 * `mbr -g` has no find bar and nothing claims Cmd+F. This supplies one.
 * `templates/_footer.html` emits the element only under `{% if gui_mode %}`,
 * so server and static modes keep the real browser's native find untouched.
 *
 * The element binds NO global key handler: the native Edit menu built in
 * `src/browser.rs` is the single entry point, and `mbr-keys` only stops
 * competing for `Ctrl+F`. Those menu items call `open()`, `findNext()` and
 * `findPrevious()` through `evaluate_script` — from Rust string literals, so a
 * TypeScript rename cannot fail at compile time. `mbr-find-bar.test.ts` asserts
 * all four public methods exist by name; that test is the only thing standing
 * between a rename and a silently dead menu item.
 *
 * Highlight painting is styled from `templates/theme.css`, not from
 * `static styles` below: `CSS.highlights` is a DOCUMENT-scoped registry and the
 * ranges live in the light DOM under `main#wrapper`, so a `::highlight()` rule
 * inside this shadow root would match nothing at all.
 */
@customElement('mbr-find-bar')
export class MbrFindBarElement extends LitElement implements MbrOverlay {
  // ========================================
  // State
  // ========================================

  @state()
  private _isOpen = false;

  @state()
  private _query = '';

  @state()
  private _caseSensitive = false;

  /** Index into the match arrays, or -1 when there is nothing to step to. */
  @state()
  private _activeIndex = -1;

  /** Exact match count, which can exceed the number of navigable matches. */
  @state()
  private _total = 0;

  @query('#find-input')
  private _input!: HTMLInputElement;

  /** Built lazily on open(), never at page load. */
  private _index: TextIndex | null = null;
  private _matchStarts: Int32Array = NO_OFFSETS;
  private _matchEnds: Int32Array = NO_OFFSETS;

  /**
   * Bumped on every query change and re-checked before painting, so a scan can
   * later be time-sliced across rAF without repainting a stale result set.
   */
  private _generation = 0;

  private _searchTimer?: number;
  private _reindexTimer?: number;
  private _observer: MutationObserver | null = null;
  private _ownsSelection = false;

  // ========================================
  // Lifecycle
  // ========================================

  override disconnectedCallback() {
    super.disconnectedCallback();
    this.close();
  }

  // ========================================
  // Public Methods (called from the native Edit menu via evaluate_script)
  // ========================================

  /**
   * Show the bar, index the page and re-run the current query.
   *
   * Deliberately NOT a toggle. The open script polls for this element because
   * the bundle is deferred, and a menu accelerator can fire more than once for
   * one keystroke; a toggle would leave the bar shut. `open(); open(); open()`
   * leaves it open, refocused and with its text selected — which is what a
   * native find bar does anyway. Nothing here may close the bar.
   */
  public open(): void {
    const wasOpen = this._isOpen;
    this._isOpen = true;
    // Indexing is lazy: never at page load, and not again while the bar is
    // already open (the mutation observer keeps it fresh from there).
    if (!this._index) this._rebuildIndex();
    this._observeContent();
    void this.updateComplete.then(() => {
      this._input?.focus();
      this._input?.select();
    });
    // Re-scanning a query that is already settled would scroll the reader back
    // to match 1, so a repeat fire only refocuses.
    if (!wasOpen && this._query.trim()) {
      this._runSearch(true);
    }
  }

  /**
   * Hide the bar and drop everything it was holding: both highlight
   * registries, the text index (and with it every reference into the page's
   * text nodes), the mutation observer and any pending timers.
   *
   * The query itself survives, so a later Find Next resumes where the reader
   * left off — again matching a native find bar.
   */
  public close(): void {
    this._isOpen = false;
    this._clearSearchTimer();
    this._clearReindexTimer();
    this._disconnectObserver();
    this._clearHighlights();
    this._index = null;
    this._matchStarts = NO_OFFSETS;
    this._matchEnds = NO_OFFSETS;
    this._total = 0;
    this._activeIndex = -1;
  }

  /** Move to the next match, wrapping past the last one. */
  public findNext(): void {
    this._step(1);
  }

  /** Move to the previous match, wrapping past the first one. */
  public findPrevious(): void {
    this._step(-1);
  }

  public get isOpen(): boolean {
    return this._isOpen;
  }

  // ========================================
  // Search
  // ========================================

  private _step(direction: 1 | -1): void {
    // Find Next with the bar shut is a normal way to resume a search, so this
    // opens rather than no-ops. open() restores the index and re-runs the
    // retained query, which is what makes the step below meaningful.
    if (!this._isOpen) {
      this.open();
    } else {
      this._flushPendingSearch();
    }

    const count = this._matchStarts.length;
    if (count === 0) return;
    this._activeIndex = (this._activeIndex + direction + count) % count;
    this._paint();
  }

  private _runSearch(resetActive: boolean): void {
    const generation = ++this._generation;
    const pattern = compileQuery(this._query, this._caseSensitive);
    if (!pattern || !this._index) {
      this._resetMatches();
      return;
    }

    const { starts, ends, total } = findMatchOffsets(this._index, pattern, MATCH_CAP);
    if (generation !== this._generation) return;

    this._matchStarts = starts;
    this._matchEnds = ends;
    this._total = total;
    if (starts.length === 0) {
      this._activeIndex = -1;
    } else if (resetActive || this._activeIndex < 0) {
      this._activeIndex = 0;
    } else {
      this._activeIndex = Math.min(this._activeIndex, starts.length - 1);
    }
    this._paint();
  }

  private _paint(): void {
    const index = this._index;
    const count = this._matchStarts.length;
    if (!index || count === 0) {
      this._clearHighlights();
      return;
    }

    const [from, to] = highlightWindow(count, this._activeIndex);
    const others: Range[] = [];
    let active: Range | null = null;
    for (let i = from; i < to; i++) {
      const range = rangeForMatch(index, this._matchStarts[i], this._matchEnds[i]);
      if (!range) continue;
      if (i === this._activeIndex) {
        active = range;
      } else {
        others.push(range);
      }
    }

    // Two registries, set once per settled query rather than once per match.
    const api = highlightApi();
    if (api) {
      api.registry.set(HIGHLIGHT_ALL, api.create(others));
      if (active) {
        api.registry.set(HIGHLIGHT_ACTIVE, api.create([active]));
      } else {
        api.registry.delete(HIGHLIGHT_ACTIVE);
      }
    }

    if (active) {
      // A real Selection as well: free, matches native behaviour, and it is the
      // whole fallback when ::highlight() is unsupported or a custom template
      // dropped theme.css.
      this._selectRange(active);
      scrollRangeIntoView(active, this._barHeight());
    }
  }

  private _resetMatches(): void {
    this._matchStarts = NO_OFFSETS;
    this._matchEnds = NO_OFFSETS;
    this._total = 0;
    this._activeIndex = -1;
    this._clearHighlights();
  }

  private _clearHighlights(): void {
    const api = highlightApi();
    if (api) {
      api.registry.delete(HIGHLIGHT_ALL);
      api.registry.delete(HIGHLIGHT_ACTIVE);
    }
    if (this._ownsSelection) {
      this._ownsSelection = false;
      window.getSelection()?.removeAllRanges();
    }
  }

  private _selectRange(range: Range): void {
    const selection = window.getSelection();
    if (!selection) return;
    selection.removeAllRanges();
    selection.addRange(range);
    this._ownsSelection = true;
  }

  /** Height of the bar, so the first match does not scroll under it. */
  private _barHeight(): number {
    return this.shadowRoot?.querySelector('.find-bar')?.getBoundingClientRect().height ?? 0;
  }

  // ========================================
  // Indexing
  // ========================================

  private _rebuildIndex(): void {
    const root = document.querySelector(SEARCH_ROOT_SELECTOR);
    this._index = root ? buildTextIndex(root) : null;
  }

  /**
   * Watch the page for content changes, but only while the bar is open — which
   * is approximately never, so the cost when closed is zero. Painting mutates
   * no DOM, so this cannot feed itself; what it catches is hljs, KaTeX or
   * Mermaid finishing after the bar opened.
   */
  private _observeContent(): void {
    if (this._observer || typeof MutationObserver === 'undefined') return;
    const root = document.querySelector(SEARCH_ROOT_SELECTOR);
    if (!root) return;
    this._observer = new MutationObserver(() => this._scheduleReindex());
    this._observer.observe(root, { childList: true, subtree: true, characterData: true });
  }

  private _scheduleReindex(): void {
    if (this._reindexTimer !== undefined) return;
    this._reindexTimer = window.setTimeout(() => {
      this._reindexTimer = undefined;
      if (!this._isOpen) return;
      this._rebuildIndex();
      // Keep the reader's place across a rebuild rather than jumping to match 1.
      this._runSearch(false);
    }, REINDEX_DEBOUNCE_MS);
  }

  private _disconnectObserver(): void {
    this._observer?.disconnect();
    this._observer = null;
  }

  private _clearSearchTimer(): void {
    if (this._searchTimer === undefined) return;
    clearTimeout(this._searchTimer);
    this._searchTimer = undefined;
  }

  private _clearReindexTimer(): void {
    if (this._reindexTimer === undefined) return;
    clearTimeout(this._reindexTimer);
    this._reindexTimer = undefined;
  }

  /** Run a debounced scan now, so stepping never lands on a stale match set. */
  private _flushPendingSearch(): void {
    if (this._searchTimer === undefined) return;
    this._clearSearchTimer();
    this._runSearch(true);
  }

  // ========================================
  // Event Handlers
  // ========================================

  private _handleInput(e: Event): void {
    this._query = (e.target as HTMLInputElement).value;
    this._generation++;
    this._clearSearchTimer();
    if (!this._query.trim()) {
      // Clearing the box has to un-paint immediately; a debounce here reads as lag.
      this._resetMatches();
      return;
    }
    this._searchTimer = window.setTimeout(() => {
      this._searchTimer = undefined;
      this._runSearch(true);
    }, INPUT_DEBOUNCE_MS);
  }

  private _handleKeydown(e: KeyboardEvent): void {
    // Cmd+G / F3 arrive through the native Edit menu, not through here.
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      this.close();
      return;
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      if (e.shiftKey) {
        this.findPrevious();
      } else {
        this.findNext();
      }
    }
  }

  private _toggleCaseSensitive(): void {
    this._caseSensitive = !this._caseSensitive;
    this._clearSearchTimer();
    this._runSearch(true);
    this._input?.focus();
  }

  // ========================================
  // Render
  // ========================================

  /** "N of M", or a no-results notice, or nothing until something is typed. */
  private _statusLabel(): string {
    if (!this._query.trim()) return '';
    if (this._total === 0) return 'No results';
    return `${this._activeIndex + 1} of ${this._total}`;
  }

  override render() {
    if (!this._isOpen) return nothing;

    const status = this._statusLabel();
    const disabled = this._total === 0;

    return html`
      <div class="find-bar" role="search">
        <input
          id="find-input"
          type="text"
          placeholder="Find in page"
          aria-label="Find in page"
          .value=${this._query}
          @input=${this._handleInput}
          @keydown=${this._handleKeydown}
          autocomplete="off"
          spellcheck="false"
        />
        <span class="status" role="status" aria-live="polite">${status}</span>
        <button
          class="toggle ${this._caseSensitive ? 'active' : ''}"
          title="Match case"
          aria-label="Match case"
          aria-pressed=${this._caseSensitive}
          @click=${this._toggleCaseSensitive}
        >Aa</button>
        <button class="step" title="Previous match" aria-label="Previous match" ?disabled=${disabled} @click=${() => this.findPrevious()}>&#8593;</button>
        <button class="step" title="Next match" aria-label="Next match" ?disabled=${disabled} @click=${() => this.findNext()}>&#8595;</button>
        <button class="step" title="Close" aria-label="Close find bar" @click=${() => this.close()}>&#10005;</button>
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

    .find-bar {
      position: fixed;
      top: 0.75rem;
      right: 1rem;
      z-index: 10001;
      display: flex;
      align-items: center;
      gap: 0.35rem;
      padding: 0.4rem 0.5rem;
      background: var(--pico-background-color, #fff);
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 8px;
      box-shadow: 0 10px 25px -10px rgba(0, 0, 0, 0.35);
    }

    #find-input {
      width: 14rem;
      min-width: 0;
      border: none;
      background: transparent;
      font-size: 0.9rem;
      color: var(--pico-color, #333);
      outline: none;
    }

    #find-input::placeholder {
      color: var(--pico-muted-color, #999);
    }

    .status {
      flex-shrink: 0;
      min-width: 4.5rem;
      text-align: right;
      font-size: 0.75rem;
      color: var(--pico-muted-color, #666);
      font-variant-numeric: tabular-nums;
    }

    .toggle,
    .step {
      flex-shrink: 0;
      padding: 0.15rem 0.4rem;
      background: transparent;
      border: 1px solid transparent;
      border-radius: 4px;
      color: var(--pico-muted-color, #666);
      font-size: 0.8rem;
      font-family: inherit;
      line-height: 1.4;
      cursor: pointer;
    }

    .toggle:hover,
    .step:hover:not(:disabled) {
      background: var(--pico-secondary-background, #f5f5f5);
      color: var(--pico-color, #333);
    }

    .toggle.active {
      border-color: var(--pico-primary, #0d6efd);
      color: var(--pico-primary, #0d6efd);
    }

    .step:disabled {
      opacity: 0.4;
      cursor: default;
    }

    @media (max-width: 480px) {
      .find-bar {
        left: 0.5rem;
        right: 0.5rem;
      }

      #find-input {
        flex: 1;
        width: auto;
      }
    }
  `;
}
