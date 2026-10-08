import { LitElement, css, html, nothing } from 'lit';
import { customElement, query, state } from 'lit/decorators.js';
import type { MbrOverlay } from './overlay.js';
import {
  SEARCH_ROOT_SELECTOR,
  buildTextIndex,
  compileQuery,
  createMatchScan,
  highlightRangeForMatch,
  rangeForMatch,
  scrollRangeIntoView,
  type MatchOffsets,
  type MatchScan,
  type TextIndex,
} from './find-in-page.js';

/** Highlight registry name for every match except the active one. */
const HIGHLIGHT_ALL = 'mbr-find';

/** Highlight registry name for the match the reader is currently on. */
const HIGHLIGHT_ACTIVE = 'mbr-find-active';

/**
 * Trailing debounce on typing, scaled by query length. Short queries are the
 * expensive ones — a single letter matches a large fraction of a long page —
 * and they are also the ones most likely to be superseded by the next
 * keystroke, so they wait longer. Enter / find-next flush any pending scan, so
 * the wait never delays a reader who asks for a result.
 */
function inputDebounceMs(query: string): number {
  const length = query.trim().length;
  if (length <= 1) return 350;
  if (length === 2) return 200;
  return 120;
}

/**
 * Time budget for one slice of a match scan before yielding to the event loop.
 * Well under a 16 ms frame, so a keystroke arriving mid-scan is handled within
 * a slice and abandons the stale scan (see {@link MbrFindBarElement._generation}).
 * WebKit has no `isInputPending`, so the budget is fixed rather than adaptive.
 */
const SCAN_SLICE_MS = 6;

/**
 * Ranges added to the highlight registry per animation frame. Registration is
 * the expensive half of painting: measured in WebKit 26, each newly registered
 * range costs ~45 us on the next rendering update, so 100 per frame is ~4.5 ms
 * — and 2000 in one go stalled the page for ~95 ms.
 */
const PAINT_SLICE = 100;

/** Coalescing window for content mutations while the bar is open. */
const REINDEX_DEBOUNCE_MS = 250;

/**
 * Most matches kept navigable. Offsets are 8 bytes each, so this is cheap; the
 * cap only exists so a one-letter query on a huge document cannot grow without
 * bound. `total` stays exact past it (see {@link findMatchOffsets}).
 */
const MATCH_CAP = 10000;

/**
 * Most ranges painted at once; past this a sliding window around the active
 * match is painted instead. Painting is frame-paced (see {@link PAINT_SLICE}),
 * so this bounds the total registration work per query rather than any single
 * stall. 1000 is still dozens of screens of a one-letter query.
 */
const HIGHLIGHT_CAP = 1000;

const NO_OFFSETS = new Int32Array(0);

/** `id` of the `<style>` installed by {@link installSelectableStyle}. */
export const SELECTABLE_STYLE_ID = 'mbr-find-selectable';

/**
 * Makes all searchable text selectable while the bar is open.
 *
 * WebKit paints `::highlight()` through its selection-painting code, which
 * moves a range endpoint that sits in `user-select: none` text forward to the
 * next selectable position. So a match in unselectable text either paints
 * nothing or paints the wrong element. theme.css gives every `main dl > dt`
 * `user-select: none` (the FAQ toggles on click), and searching a flashcard
 * question washed the whole of the next selectable block, a heading several
 * collapsed answers further down, instead of the match. Live ranges and
 * `StaticRange`s behave the same, and the registered ranges are correct.
 *
 * Fixing that one rule in theme.css would not be enough: a repository's own
 * `.mbr/theme.css` can make anything unselectable. So the override belongs to
 * the find bar, lives exactly as long as the bar is open, and leaves the
 * page's `user-select: none` alone at every other time. `!important` outranks any
 * author rule that is not itself `!important`; `*` is needed because only
 * WebKit inherits `-webkit-user-select`.
 */
const SELECTABLE_CSS =
  `${SEARCH_ROOT_SELECTOR}, ${SEARCH_ROOT_SELECTOR} * ` +
  '{ -webkit-user-select: text !important; user-select: text !important; }';

/** Install the {@link SELECTABLE_CSS} override in `<head>`, once. */
function installSelectableStyle(): void {
  if (document.getElementById(SELECTABLE_STYLE_ID)) return;
  const style = document.createElement('style');
  style.id = SELECTABLE_STYLE_ID;
  style.textContent = SELECTABLE_CSS;
  // In <head>, outside main#wrapper, so the content observer never sees it.
  document.head.appendChild(style);
}

function removeSelectableStyle(): void {
  document.getElementById(SELECTABLE_STYLE_ID)?.remove();
}

/**
 * The document-scoped Custom Highlight API, or `null` where it is missing
 * (older WebKitGTK is the realistic gap; WKWebView needs Safari 17.2+ and
 * WebView2 needs Chromium 105+). Resolved per call so a test can stub it.
 */
function highlightApi(): { registry: HighlightRegistry; create: (ranges: AbstractRange[]) => Highlight } | null {
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

/** A scan in progress, resumed slice by slice from a zero-delay timer. */
interface ScanJob {
  scan: MatchScan;
  resetActive: boolean;
  generation: number;
}

/**
 * Highlight registration in progress: ranges are added outward from the
 * active match, so what is on screen paints first. `above` is the next index
 * at or after the active one, `below` the next one before it.
 */
interface PaintJob {
  highlight: Highlight;
  from: number;
  to: number;
  above: number;
  below: number;
  preferAbove: boolean;
  frame?: number;
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
 * inside this shadow root would match nothing at all. Page text must also be
 * selectable, or WebKit paints the highlight in the wrong place; see
 * {@link SELECTABLE_CSS}.
 *
 * Typing must never wait on searching. Four things keep it that way, each
 * measured against WebKit on a 900k-character page: the debounce is longer for
 * short (expensive) queries; the scan is time-sliced and abandoned the moment
 * the query changes; painted ranges are `StaticRange`s, which WebKit does not
 * revalidate on every keystroke the way it does live ones; and registration is
 * spread over animation frames, nearest-first. Stepping touches only the
 * one-range active highlight — it sits above the rest via `priority`, so the
 * "all" highlight can include it and never has to be rebuilt.
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

  /**
   * Whether `_total` / `_activeIndex` describe a finished scan. False while the
   * first result for a query is still debounced or scanning, so the label shows
   * nothing rather than a premature "No results".
   */
  @state()
  private _hasResult = false;

  @query('#find-input')
  private _input!: HTMLInputElement;

  /** Built lazily on open(), never at page load. */
  private _index: TextIndex | null = null;
  private _matchStarts: Int32Array = NO_OFFSETS;
  private _matchEnds: Int32Array = NO_OFFSETS;

  /**
   * Bumped on every query change and on close, and re-checked after every
   * yield of a sliced scan, so stale work is dropped rather than painted.
   */
  private _generation = 0;

  private _scan: ScanJob | null = null;
  private _scanTimer?: number;
  private _paintJob: PaintJob | null = null;
  /** Half-open slice of matches the "all" highlight covers (or is filling). */
  private _window: [number, number] = [0, 0];

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
    // Before anything is painted: highlights in unselectable text misplace.
    installSelectableStyle();
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
    this._generation++;
    this._cancelScan();
    this._clearSearchTimer();
    this._clearReindexTimer();
    this._disconnectObserver();
    this._clearHighlights();
    removeSelectableStyle();
    this._index = null;
    this._matchStarts = NO_OFFSETS;
    this._matchEnds = NO_OFFSETS;
    this._total = 0;
    this._hasResult = false;
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
    if (!this._isOpen) this.open();
    // Debounced or mid-scan, the result is needed now: stepping must never land
    // on a stale match set.
    this._flushPendingSearch();

    const count = this._matchStarts.length;
    if (count === 0) return;
    this._activeIndex = (this._activeIndex + direction + count) % count;
    const [from, to] = this._window;
    if (this._activeIndex >= from && this._activeIndex < to) {
      this._paintActive();
    } else {
      this._repaint();
    }
  }

  private _runSearch(resetActive: boolean): void {
    const generation = ++this._generation;
    this._cancelScan();
    const pattern = compileQuery(this._query, this._caseSensitive);
    if (!pattern || !this._index) {
      this._resetMatches();
      return;
    }

    this._scan = { scan: createMatchScan(this._index, pattern, MATCH_CAP), resetActive, generation };
    this._continueScan();
  }

  /** Run one slice of the current scan; reschedule itself until it is done. */
  private _continueScan(): void {
    const job = this._scan;
    if (!job) return;
    if (job.generation !== this._generation) {
      this._scan = null;
      return;
    }
    if (!job.scan.step(performance.now() + SCAN_SLICE_MS)) {
      // Yield so a keystroke can run; it bumps the generation and the next
      // slice drops this scan instead of finishing it.
      this._scanTimer = window.setTimeout(() => {
        this._scanTimer = undefined;
        this._continueScan();
      }, 0);
      return;
    }
    this._scan = null;
    this._applyMatches(job.scan.result(), job.resetActive);
  }

  /** Complete an in-flight scan synchronously. */
  private _finishScan(): void {
    const job = this._scan;
    if (!job) return;
    this._cancelScan();
    if (job.generation !== this._generation) return;
    job.scan.step(Number.POSITIVE_INFINITY);
    this._applyMatches(job.scan.result(), job.resetActive);
  }

  private _cancelScan(): void {
    this._scan = null;
    if (this._scanTimer === undefined) return;
    clearTimeout(this._scanTimer);
    this._scanTimer = undefined;
  }

  private _applyMatches({ starts, ends, total }: MatchOffsets, resetActive: boolean): void {
    this._matchStarts = starts;
    this._matchEnds = ends;
    this._total = total;
    this._hasResult = true;
    if (starts.length === 0) {
      this._activeIndex = -1;
    } else if (resetActive || this._activeIndex < 0) {
      this._activeIndex = 0;
    } else {
      this._activeIndex = Math.min(this._activeIndex, starts.length - 1);
    }
    this._repaint();
  }

  /**
   * Replace both highlights for a new match set (or a window move): register
   * an empty "all" highlight, paint the active match, fill the first slice
   * nearest it now and the rest one slice per animation frame.
   */
  private _repaint(): void {
    const index = this._index;
    const count = this._matchStarts.length;
    this._clearHighlights();
    if (!index || count === 0) return;

    const api = highlightApi();
    if (!api) {
      this._paintActive();
      return;
    }

    const [from, to] = highlightWindow(count, this._activeIndex);
    const active = Math.max(this._activeIndex, from);
    const highlight = api.create([]);
    // Registered before the active highlight, so the active one also wins on
    // registration order where `priority` is unsupported.
    api.registry.set(HIGHLIGHT_ALL, highlight);
    this._window = [from, to];
    this._paintActive();

    const job: PaintJob = { highlight, from, to, above: active, below: active - 1, preferAbove: true };
    if (!this._paintSlice(job)) {
      this._paintJob = job;
      this._schedulePaint(job);
    }
  }

  /** Add up to {@link PAINT_SLICE} ranges, nearest the active match first. Returns true when done. */
  private _paintSlice(job: PaintJob): boolean {
    const index = this._index;
    if (!index) return true;
    let budget = PAINT_SLICE;
    while (budget > 0 && (job.above < job.to || job.below >= job.from)) {
      const takeAbove = job.above < job.to && (job.preferAbove || job.below < job.from);
      const i = takeAbove ? job.above++ : job.below--;
      job.preferAbove = !takeAbove;
      const range = highlightRangeForMatch(index, this._matchStarts[i], this._matchEnds[i]);
      if (range) {
        job.highlight.add(range);
        budget--;
      }
    }
    return job.above >= job.to && job.below < job.from;
  }

  private _schedulePaint(job: PaintJob): void {
    job.frame = requestAnimationFrame(() => {
      job.frame = undefined;
      if (this._paintJob !== job) return;
      if (this._paintSlice(job)) {
        this._paintJob = null;
      } else {
        this._schedulePaint(job);
      }
    });
  }

  private _cancelPaint(): void {
    const job = this._paintJob;
    this._paintJob = null;
    if (job?.frame !== undefined) cancelAnimationFrame(job.frame);
  }

  /**
   * Paint and scroll to the active match only. With the Highlight API this is
   * a one-range highlight drawn above the "all" one (which also contains it),
   * so stepping never re-registers the rest.
   */
  private _paintActive(): void {
    const index = this._index;
    const i = this._activeIndex;
    if (!index || i < 0) return;
    const active = rangeForMatch(index, this._matchStarts[i], this._matchEnds[i]);

    const api = highlightApi();
    if (api) {
      if (!active) {
        api.registry.delete(HIGHLIGHT_ACTIVE);
        return;
      }
      const highlight = api.create([highlightRangeForMatch(index, this._matchStarts[i], this._matchEnds[i]) ?? active]);
      highlight.priority = 1;
      api.registry.set(HIGHLIGHT_ACTIVE, highlight);
      scrollRangeIntoView(active, this._barHeight());
      return;
    }

    if (active) {
      // No CSS.highlights, so a real Selection is the only paint there is. It
      // must stay the fallback, never an extra: WebKit blurs the focused
      // element when the document selection moves outside it, and applies that
      // blur AFTER addRange() returns (during its next selection update), so
      // a refocus here is a no-op that the blur then overrides — the input lost
      // focus after every typed character. And a focused text field owns the
      // frame selection in WebKit, so the two could never coexist anyway. The
      // refocus below only helps engines that blur synchronously.
      const hadFocus = this.shadowRoot?.activeElement === this._input;
      this._selectRange(active);
      scrollRangeIntoView(active, this._barHeight());
      if (hadFocus) this._input?.focus();
    }
  }

  private _resetMatches(): void {
    this._matchStarts = NO_OFFSETS;
    this._matchEnds = NO_OFFSETS;
    this._total = 0;
    this._hasResult = false;
    this._activeIndex = -1;
    this._clearHighlights();
  }

  private _clearHighlights(): void {
    this._cancelPaint();
    this._window = [0, 0];
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

  /** Run a debounced or in-flight scan to completion now. */
  private _flushPendingSearch(): void {
    if (this._searchTimer !== undefined) {
      this._clearSearchTimer();
      this._runSearch(true);
    }
    this._finishScan();
  }

  // ========================================
  // Event Handlers
  // ========================================

  private _handleInput(e: Event): void {
    this._query = (e.target as HTMLInputElement).value;
    // Abandon whatever the previous query still had in flight — scan slices
    // and paint frames alike — so the next keystroke never waits on them.
    this._generation++;
    this._cancelScan();
    this._cancelPaint();
    this._clearSearchTimer();
    if (!this._query.trim()) {
      // Clearing the box has to un-paint immediately; a debounce here reads as lag.
      this._resetMatches();
      return;
    }
    this._searchTimer = window.setTimeout(() => {
      this._searchTimer = undefined;
      this._runSearch(true);
    }, inputDebounceMs(this._query));
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

  /**
   * "N of M", or a no-results notice, or nothing until something is typed and
   * scanned. While a newer query is pending this still describes the previous
   * result — which is also what is painted.
   */
  private _statusLabel(): string {
    if (!this._query.trim() || !this._hasResult) return '';
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
