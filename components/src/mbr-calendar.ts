/**
 * `<mbr-calendar>` — the calendar trigger (main bundle; design/calendar.md
 * §4.1).
 *
 * **Kept small: it ships on every page.** It owns `C` (Shift+c: toggle the
 * daily-note companion), the floating calendar button, and the docked column
 * itself; everything else is in the lazy `mbr-calendar.min.js` chunk
 * (`calendar-chunk.ts`).
 *
 * - **Prefilter.** On a periodic note (the server-rendered
 *   `nav.mbr-calendar-nav`), or a note with a time-like top-level bullet
 *   (`calendar/agenda-scope.ts`), the chunk is imported at idle: it paints the
 *   agenda time chips and, on a day note, fills the docked column. Anywhere
 *   else nothing loads until `C` or the button asks.
 * - **Docked.** `_footer.html` puts this element straight under `<body>`. On a
 *   day note under `companion = "auto"` the server adds `mbr-has-companion` to
 *   `<body>` and theme.css turns this element into the fixed right column at
 *   ≥1280px (≥1440px with the docked sidebar), so the space is there before
 *   any script runs. `html.mbr-companion-hidden` (`calendar/companion-pref.ts`,
 *   applied pre-paint by `_head.html`) collapses it.
 * - **Drawer.** Elsewhere, `C` or the button opens the companion as a right
 *   drawer, which is when this element reports `isOpen` (`MbrOverlay`).
 *
 * - **Overlay.** `c` (and `openOverlay`, which the browse widgets and the
 *   search modal reach through `calendar/open-overlay.ts`) opens the chunk's
 *   `<mbr-calendar-overlay>` (design/calendar.md §5). It is rendered here once
 *   first asked and then kept, hidden, so its view, plot-by and filter last
 *   for the life of the page; while open it is this element's `isOpen`.
 */
import { LitElement, css, html, nothing, type TemplateResult } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { getCalendarConfig, getCanonicalPath, getTagSources, isEditEnabled, isTasksEnabled, resolveUrl, siteNav } from './shared.js'
import { isInputTarget, isModalOpen, isPresentationPlaying } from './mbr-keys.js'
import { scheduleIdleTask, waitForDom } from './dynamic-loader.js'
import { FAB_CSS, PlayFabController } from './fab.js'
import { icon, iconCalendarDays } from './icons.ts'
import { loadCalendarChunk, type CalendarChunk } from './calendar-chunk.js'
import { isCompanionHidden, setCompanionHidden } from './calendar/companion-pref.js'
import { mayHaveAgenda } from './calendar/agenda-scope.js'
import { revealElement } from './mbr-task-doc.js'
import type { CalendarConfig } from './calendar/config.js'
import type { CalendarScope, TaskMarkerSource } from './calendar/types.js'
import { createTaskMarkers, type TaskMarkers } from './calendar/task-markers.js'
import { onTaskWritten } from './task-toggle.js'
import type { MbrOverlay } from './overlay.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-calendar': MbrCalendarElement
  }
}

/** The task endpoint (server/GUI only; `isTasksEnabled()` says whether it answers). */
const TASKS_ENDPOINT = '/.mbr/tasks'

/** The companion docks from this width (theme.css, "Daily-note Companion")… */
export const COMPANION_DOCK_QUERY = 'screen and (min-width: 1280px)'
/** …or from this one when the left sidebar is docked too. */
export const COMPANION_DOCK_SIDEBAR_QUERY = 'screen and (min-width: 1440px)'

/**
 * Whether the docked column applies, ignoring the reader's dismissal: the
 * server asked for it (`mbr-has-companion`) and the window is wide enough,
 * counting the sidebar the server emitted. Mirrors theme.css's selectors.
 */
export function companionDockable(body: HTMLElement, wide: boolean, wider: boolean): boolean {
  return (
    body.classList.contains('mbr-has-companion') &&
    (wider || (wide && !body.querySelector(':scope > mbr-browse-single')))
  )
}

/** The page's period from the server-rendered bar, if it is a periodic note. */
function pagePeriod(): string | null {
  return document.querySelector<HTMLElement>('nav.mbr-calendar-nav[data-mbr-period]')?.dataset.mbrPeriod ?? null
}

function pageRoot(): HTMLElement | null {
  return document.querySelector<HTMLElement>('main#wrapper') ?? document.querySelector('main')
}

/** HTML of another page, for another day's agenda. */
function fetchPageHtml(url: string): Promise<string> {
  return fetch(url, { credentials: 'same-origin' }).then((r) => {
    if (!r.ok) throw new Error(`HTTP ${r.status}`)
    return r.text()
  })
}

@customElement('mbr-calendar')
export class MbrCalendarElement extends LitElement implements MbrOverlay {
  /** Periodic note, or a note with agenda-like bullets. */
  @state() private _relevant = false
  @state() private _dockable = false
  @state() private _hidden = isCompanionHidden()
  @state() private _drawerOpen = false
  @state() private _chunk: CalendarChunk | null = null
  /** The overlay's scope once it has been opened (kept while hidden). */
  @state() private _overlayScope: CalendarScope | null = null
  @state() private _overlayOpen = false
  /** The companion's task markers; `null` without a task endpoint (static builds). */
  @state() private _taskMarkers: TaskMarkerSource | null = null
  private _markers: TaskMarkers | null = null
  private _offTaskWritten: (() => void) | null = null
  private _returnFocus: HTMLElement | null = null
  private _config: CalendarConfig = getCalendarConfig()
  private _queries: MediaQueryList[] = []
  private _dispose: (() => void) | null = null

  constructor() {
    super()
    // Registers itself with this host; only its stacking (the review button
    // moves above the calendar button) is wanted, so it is not kept.
    new PlayFabController(this, () => this._showsFab(), 'calendar')
  }

  override connectedCallback(): void {
    super.connectedCallback()
    this._config = getCalendarConfig()
    if (!this._config.enabled) return
    document.addEventListener('keydown', this._handleKeydown)
    if (isTasksEnabled() && !this._markers) {
      const markers = createTaskMarkers(TASKS_ENDPOINT)
      this._markers = markers
      this._taskMarkers = markers.source
      // A write makes the counts stale; a new source makes the companion ask again.
      this._offTaskWritten = onTaskWritten(() => (this._taskMarkers = markers.invalidate()))
    }
    void waitForDom().then(() => this._init())
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    document.removeEventListener('keydown', this._handleKeydown)
    this._queries.forEach((q) => q.removeEventListener?.('change', this._onMedia))
    this._queries = []
    this._dispose?.()
    this._dispose = null
    this._offTaskWritten?.()
    this._offTaskWritten = null
    this._markers = null
  }

  /** True while the companion drawer or the overlay is open (it owns the keyboard). */
  public get isOpen(): boolean {
    return this._drawerOpen || this._overlayOpen
  }

  /** Whether `openOverlay` can do anything here (`[calendar] enabled`). */
  public get overlayAvailable(): boolean {
    return this._config.enabled
  }

  /**
   * Opens the calendar overlay over `scope` (default: all notes). Returns
   * false when the calendar is off; resolves the chunk in the background.
   */
  public openOverlay(scope: CalendarScope = { label: 'All notes' }): boolean {
    if (!this._config.enabled) return false
    this._drawerOpen = false
    if (!this._overlayOpen) {
      const active = document.activeElement
      this._returnFocus = active instanceof HTMLElement ? active : null
    }
    this._overlayScope = scope
    this._overlayOpen = true
    void this._ensureChunk().then((chunk) => {
      if (!chunk) this._overlayOpen = false
    })
    return true
  }

  public closeOverlay(): void {
    if (!this._overlayOpen) return
    this._overlayOpen = false
    this._returnFocus?.focus?.()
    this._returnFocus = null
  }

  /** True while the companion is the docked column. */
  public get isDocked(): boolean {
    return this._dockable && !this._hidden && this._config.companion === 'auto'
  }

  public open(): void {
    if (this._config.companion === 'off' || this._drawerOpen) return
    if (this._dockable) {
      this._setHidden(false)
    } else {
      this._drawerOpen = true
    }
    void this._ensureChunk().then((chunk) => {
      if (!chunk) this._drawerOpen = false
      else this._focusCompanion()
    })
  }

  public close(): void {
    this._drawerOpen = false
    this.closeOverlay()
  }

  /**
   * `C` and the button. Docked: dismissed → shown and focused; shown with focus
   * elsewhere → focused; focused → dismissed (remembered). Otherwise the
   * drawer opens or closes.
   */
  public toggleCompanion(): void {
    if (this._config.companion === 'off') return
    if (this._dockable && this._config.companion === 'auto') {
      if (this._hidden) this.open()
      else if (this.matches(':focus-within')) this._setHidden(true)
      else this._focusCompanion()
    } else if (this._drawerOpen) {
      this.close()
    } else {
      this.open()
    }
  }

  private _init(): void {
    const root = pageRoot()
    this._relevant = pagePeriod() !== null || (root !== null && mayHaveAgenda(root))
    if (typeof window.matchMedia === 'function') {
      this._queries = [COMPANION_DOCK_QUERY, COMPANION_DOCK_SIDEBAR_QUERY].map((q) => window.matchMedia(q))
      this._queries.forEach((q) => q.addEventListener?.('change', this._onMedia))
    }
    this._onMedia()
    if (this._relevant && (this._config.companion !== 'off' || this._config.agendaChips)) {
      scheduleIdleTask(() => void this._ensureChunk())
    }
  }

  private _onMedia = (): void => {
    const [wide, wider] = this._queries.map((q) => q.matches)
    this._dockable = companionDockable(document.body, !!wide, !!wider)
    if (this._dockable) this._drawerOpen = false
  }

  private _setHidden(hidden: boolean): void {
    setCompanionHidden(hidden)
    this._hidden = hidden
  }

  /** Loads the chunk once and enhances the page with it. */
  private async _ensureChunk(): Promise<CalendarChunk | null> {
    const chunk = await loadCalendarChunk()
    if (chunk && !this._chunk) {
      this._chunk = chunk
      const root = pageRoot()
      const period = pagePeriod()
      if (root && !this._dispose) {
        this._dispose = chunk.enhancePage({
          root,
          agendaChips: this._config.agendaChips,
          pageDay: period && /^\d{4}-\d{2}-\d{2}$/.test(period) ? period : null,
          reveal: revealElement,
        }).dispose
      }
    }
    return chunk
  }

  private _focusCompanion(): void {
    void this.updateComplete.then(() => this.renderRoot.querySelector('mbr-calendar-companion')?.focusGrid?.())
  }

  private _onCompanionClose = (): void => {
    if (this.isDocked) this._setHidden(true)
    else this.close()
  }

  /** `c` opens the overlay over all notes; `C` (Shift+c) toggles the companion. */
  private _handleKeydown = (e: KeyboardEvent): void => {
    if (e.ctrlKey || e.metaKey || e.altKey || e.repeat) return
    if (e.key === 'c' && !e.shiftKey) {
      if (isInputTarget(e) || isPresentationPlaying() || isModalOpen()) return
      e.preventDefault()
      this.openOverlay({ label: 'All notes', pagePath: getCanonicalPath() })
      return
    }
    if (e.key !== 'C' || !e.shiftKey) return
    if (isInputTarget(e) || isPresentationPlaying() || this._config.companion === 'off') return
    if (isModalOpen() && !this._drawerOpen) return
    e.preventDefault()
    this.toggleCompanion()
  }

  /** The floating button: a page the calendar is about, not already showing it. */
  private _showsFab(): boolean {
    return this._relevant && this._config.companion !== 'off' && !this.isDocked && !this._drawerOpen
  }

  override render(): TemplateResult {
    const placement = this.isDocked ? 'docked' : this._drawerOpen ? 'drawer' : null
    const toggle = () => this.toggleCompanion()
    // One line on purpose: template whitespace is not minified, and this
    // ships on every page.
    // prettier-ignore
    const tagFields = getTagSources().map((t) => t.field)
    // prettier-ignore
    const overlay = this._overlayScope && this._chunk ? html`<mbr-calendar-overlay .open=${this._overlayOpen} .scope=${this._overlayScope} .siteIndex=${() => siteNav} .resolveUrl=${resolveUrl} .navigate=${(url: string) => window.location.assign(url)} .config=${this._config} .tagFields=${tagFields.length ? tagFields : ['tags']} .tasksEndpoint=${isTasksEnabled() ? TASKS_ENDPOINT : null} @mbr-calendar-overlay-close=${() => this.closeOverlay()}></mbr-calendar-overlay>` : nothing
    return html`${overlay}${placement && this._chunk ? html`<mbr-calendar-companion .placement=${placement} .root=${pageRoot()} .siteIndex=${() => siteNav} .fetchPageHtml=${fetchPageHtml} .resolveUrl=${resolveUrl} .navigate=${(url: string) => window.location.assign(url)} .reveal=${revealElement} .config=${this._config} .tasksEndpoint=${isTasksEnabled() ? TASKS_ENDPOINT : null} .taskMarkers=${this._taskMarkers} .editEnabled=${isEditEnabled()} .currentPath=${getCanonicalPath()} .pagePeriod=${pagePeriod()} @mbr-calendar-close=${this._onCompanionClose}></mbr-calendar-companion>` : nothing}${this._showsFab() ? html`<button type="button" class="mbr-fab calendar-fab" @click=${toggle} aria-label="Calendar" title="Calendar (Shift+C)">${icon(iconCalendarDays)}</button>` : nothing}`
  }

  static override styles = [
    FAB_CSS,
    css`
      :host {
        display: contents;
      }
      .calendar-fab {
        right: calc(1rem + 4px + env(safe-area-inset-right, 0px));
        bottom: calc(1rem + env(safe-area-inset-bottom, 0px) + var(--mbr-fab-play-offset, 0px));
        width: 48px;
        height: 48px;
        background: var(--pico-background-color, #fff);
        color: var(--pico-primary, #0172ad);
        border: 1px solid var(--pico-muted-border-color, #ccc);
      }
    `,
  ]
}
