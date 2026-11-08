/**
 * `<mbr-calendar>` — the calendar trigger (main bundle; design/calendar.md
 * §4.1).
 *
 * **Kept small: it ships on every page.** It owns `C` (Shift+c: toggle the
 * daily-note companion), `c` (the overlay), the floating calendar button, and
 * the docked column itself; everything else is in the lazy
 * `mbr-calendar.min.js` chunk (`calendar-chunk.ts`).
 *
 * - **Prefilter.** On a periodic note (the server-rendered
 *   `nav.mbr-calendar-nav`) the chunk is imported at idle; on any other note
 *   the cheap time-like-bullet test (`calendar/agenda-scope.ts`) itself runs
 *   at idle and imports the chunk only when it passes. The chunk paints the
 *   agenda time chips and, on a day note, fills the docked column; its
 *   `enhancePage` parses the page's agenda once, and those entries are what
 *   the companion draws. Anywhere else nothing loads until `C` or the button
 *   asks.
 * - **Docked.** `_footer.html` puts this element straight under `<body>`. On a
 *   day note under `companion = "auto"` the server adds `mbr-has-companion` to
 *   `<body>`, and theme.css (the single source of truth for the breakpoints)
 *   sets `--mbr-companion-docked: 1` on it where the column fits and turns
 *   this element into the fixed right column, so the space is there before
 *   any script runs. This element only reads that property back (on resize).
 *   `html.mbr-companion-hidden` (`calendar/companion-pref.ts`, applied
 *   pre-paint by `_head.html`) collapses it.
 * - **Drawer.** Elsewhere, `C` or the button opens the companion as a right
 *   drawer, which is when this element reports `isOpen` (`MbrOverlay`).
 * - **Overlay.** `c` (and `openOverlay`, which the browse widgets and the
 *   search modal reach through `calendar/open-overlay.ts`) opens the chunk's
 *   `<mbr-calendar-overlay>` (design/calendar.md §5). It is created once first
 *   asked, appended to `<body>` (never inside this element, whose docked
 *   column is a stacking context), and then kept, hidden, so its view,
 *   plot-by and filter last for the life of the page; while open it is this
 *   element's `isOpen`.
 * - **The app.** `open(kind)` plus the `overlayAvailable` /
 *   `companionAvailable` / `hasDayNotes` gates are the whole surface
 *   `app-bridge.ts` drives (`calendar` / `companion` / `today`).
 */
import { LitElement, css, html, nothing, type TemplateResult } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { getCalendarConfig, getCanonicalPath, getRepoId, getTagSources, isTasksEnabled, resolveUrl, siteNav } from './shared.js'
import { isInputTarget, isModalOpen, isPresentationPlaying } from './mbr-keys.js'
import { scheduleIdleTask, waitForDom } from './dynamic-loader.js'
import { FAB_CSS, PlayFabController } from './fab.js'
import { icon, iconCalendarDays } from './icons.ts'
import { loadCalendarChunk, type CalendarChunk } from './calendar-chunk.js'
import { isCompanionHidden, setCompanionHidden } from './calendar/companion-pref.js'
import { mayHaveAgenda } from './calendar/agenda-scope.js'
import type { AgendaEntry } from './calendar/agenda-parse.js'
import { knownHasDayNotes, openTodayDailyNote } from './calendar/periodic-nav.js'
import { revealElement } from './mbr-task-doc.js'
import type { CalendarConfig } from './calendar/config.js'
import type { CalendarScope } from './calendar/types.js'
import type { MbrCalendarOverlayElement } from './calendar/mbr-calendar-overlay.js'
import { createTaskMarkers, type TaskMarkers } from './calendar/task-markers.js'
import { FILE_CHANGED_EVENT } from './task-toggle.js'
import type { MbrOverlay } from './overlay.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-calendar': MbrCalendarElement
  }
}

/** What `open()` opens: the companion (the default), the overlay, or today's day note (`d`). */
export type CalendarOpen = 'companion' | 'overlay' | 'today'

/** The task endpoint (server/GUI only; `isTasksEnabled()` says whether it answers). */
const TASKS_ENDPOINT = '/.mbr/tasks'

/**
 * Whether theme.css docks the companion here, ignoring the reader's
 * dismissal: its width rules set `--mbr-companion-docked: 1` on a
 * `mbr-has-companion` body wide enough for the column (counting the docked
 * sidebar), so the breakpoints live in the stylesheet alone.
 */
export function companionDockable(body: HTMLElement = document.body): boolean {
  return getComputedStyle(body).getPropertyValue('--mbr-companion-docked').trim() === '1'
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

const navigate = (url: string): void => window.location.assign(url)
const siteIndex = () => siteNav

@customElement('mbr-calendar')
export class MbrCalendarElement extends LitElement implements MbrOverlay {
  /** Periodic note, or a note with agenda-like bullets. */
  @state() private _relevant = false
  @state() private _dockable = false
  @state() private _drawerOpen = false
  @state() private _chunk: CalendarChunk | null = null
  /** The overlay's scope once it has been opened (kept while hidden). */
  @state() private _overlayScope: CalendarScope | null = null
  @state() private _overlayOpen = false
  /** The companion's task markers; `null` without a task endpoint (static builds). */
  private _markers: TaskMarkers | null = null
  /** The page's agenda, parsed once by the chunk's `enhancePage`. */
  private _pageEntries: readonly AgendaEntry[] = []
  private _returnFocus: HTMLElement | null = null
  /**
   * The overlay, a `<body>` child rather than part of this element's tree:
   * docked, this element is a `position: fixed; z-index: 100` column, a
   * stacking context that would keep the overlay under the sticky header and
   * the floating buttons. Created the first time it opens, kept (hidden) for
   * the page, removed with this element.
   */
  private _overlayEl: MbrCalendarOverlayElement | null = null
  private _config: CalendarConfig = getCalendarConfig()
  private _dispose: (() => void) | null = null
  private _resizeFrame = 0

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
      this._markers = createTaskMarkers(TASKS_ENDPOINT)
      // A changed file makes the counts stale; a new source makes the companion ask again.
      document.addEventListener(FILE_CHANGED_EVENT, this._onFileChanged)
    }
    void waitForDom().then(() => this._init())
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    document.removeEventListener('keydown', this._handleKeydown)
    document.removeEventListener(FILE_CHANGED_EVENT, this._onFileChanged)
    window.removeEventListener('resize', this._onResize)
    cancelAnimationFrame(this._resizeFrame)
    this._dispose?.()
    this._dispose = null
    this._markers = null
    this._overlayEl?.remove()
    this._overlayEl = null
  }

  /** True while the companion drawer or the overlay is open (it owns the keyboard). */
  public get isOpen(): boolean {
    return this._drawerOpen || this._overlayOpen
  }

  /** Whether `open('overlay')` / `openOverlay` can do anything here (`[calendar] enabled`). */
  public get overlayAvailable(): boolean {
    return this._config.enabled
  }

  /** Whether `open('companion')` shows something new: not while docked, nor with `companion = "off"`. */
  public get companionAvailable(): boolean {
    return this._config.enabled && this._config.companion !== 'off' && !this.isDocked
  }

  /** Whether `open('today')` has a day note to go to: `null`-safe, false until the site index is known. */
  public get hasDayNotes(): boolean {
    return this._config.enabled && knownHasDayNotes() === true
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

  /** `c`: the overlay over all notes, offering this page's folder. */
  public openOverlayHere(): boolean {
    return this.openOverlay({ label: 'All notes', pagePath: getCanonicalPath() })
  }

  public closeOverlay(): void {
    if (!this._overlayOpen) return
    this._overlayOpen = false
    this._returnFocus?.focus?.()
    this._returnFocus = null
  }

  /** True while the companion is the docked column. */
  public get isDocked(): boolean {
    return this._dockable && !isCompanionHidden() && this._config.companion === 'auto'
  }

  /** Opens the companion (default), the overlay (`c`), or today's day note (`d`). */
  public open(kind: CalendarOpen = 'companion'): void {
    if (kind === 'overlay') {
      this.openOverlayHere()
      return
    }
    if (kind === 'today') {
      void openTodayDailyNote()
      return
    }
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
      if (isCompanionHidden()) this.open()
      else if (this.matches(':focus-within')) this._setHidden(true)
      else this._focusCompanion()
    } else if (this._drawerOpen) {
      this.close()
    } else {
      this.open()
    }
  }

  private _init(): void {
    window.addEventListener('resize', this._onResize)
    this._readDocking()
    if (this._config.companion === 'off' && !this._config.agendaChips) return
    if (pagePeriod() !== null) {
      this._relevant = true
      scheduleIdleTask(() => void this._ensureChunk())
      return
    }
    // The bullet test walks the note's lists: at idle, never in the way of first paint.
    scheduleIdleTask(() => {
      const root = pageRoot()
      if (!root || !mayHaveAgenda(root)) return
      this._relevant = true
      void this._ensureChunk()
    })
  }

  /** theme.css decides the docking; this reads it back, at most once a frame. */
  private _onResize = (): void => {
    cancelAnimationFrame(this._resizeFrame)
    this._resizeFrame = requestAnimationFrame(() => this._readDocking())
  }

  private _readDocking(): void {
    this._dockable = companionDockable()
    if (this._dockable) this._drawerOpen = false
  }

  private _onFileChanged = (): void => {
    this._markers?.invalidate()
    this.requestUpdate()
  }

  private _setHidden(hidden: boolean): void {
    setCompanionHidden(hidden, getRepoId())
    this.requestUpdate()
  }

  /** Loads the chunk once and enhances the page with it. */
  private async _ensureChunk(): Promise<CalendarChunk | null> {
    const chunk = await loadCalendarChunk()
    if (chunk && !this._chunk) {
      this._chunk = chunk
      const root = pageRoot()
      const period = pagePeriod()
      if (root && !this._dispose) {
        const page = chunk.enhancePage({
          root,
          agendaChips: this._config.agendaChips,
          pageDay: period && /^\d{4}-\d{2}-\d{2}$/.test(period) ? period : null,
          reveal: revealElement,
        })
        this._pageEntries = page.entries
        this._dispose = page.dispose
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
      this.openOverlayHere()
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

  protected override updated(): void {
    this._syncOverlay()
  }

  /** Creates the overlay once it is first asked for (and the chunk defines it), then keeps it in step. */
  private _syncOverlay(): void {
    const scope = this._overlayScope
    if (!scope || !this._chunk) return
    let overlay = this._overlayEl
    if (!overlay) {
      overlay = document.createElement('mbr-calendar-overlay') as MbrCalendarOverlayElement
      overlay.siteIndex = siteIndex
      overlay.resolveUrl = resolveUrl
      overlay.navigate = navigate
      overlay.config = this._config
      const tagFields = getTagSources().map((t) => t.field)
      overlay.tagFields = tagFields.length ? tagFields : ['tags']
      overlay.addEventListener('mbr-calendar-overlay-close', () => this.closeOverlay())
      this._overlayEl = overlay
    }
    overlay.scope = scope
    overlay.open = this._overlayOpen
    if (!overlay.isConnected) document.body.append(overlay)
  }

  override render(): TemplateResult {
    const chunk = this._chunk
    const placement = this.isDocked ? 'docked' : this._drawerOpen ? 'drawer' : null
    const companion =
      placement && chunk
        ? html`
            <mbr-calendar-companion
              .placement=${placement}
              .pageEntries=${this._pageEntries}
              .siteIndex=${siteIndex}
              .fetchPageHtml=${fetchPageHtml}
              .resolveUrl=${resolveUrl}
              .navigate=${navigate}
              .reveal=${revealElement}
              .config=${this._config}
              .taskMarkers=${this._markers?.source ?? null}
              .currentPath=${getCanonicalPath()}
              .pagePeriod=${pagePeriod()}
              @mbr-calendar-close=${this._onCompanionClose}
            ></mbr-calendar-companion>
          `
        : nothing
    const fab = this._showsFab()
      ? html`
          <button type="button" class="mbr-fab calendar-fab" @click=${() => this.toggleCompanion()} aria-label="Calendar" title="Calendar (Shift+C)">
            ${icon(iconCalendarDays)}
          </button>
        `
      : nothing
    return html`${companion}${fab}`
  }

  static override styles = [
    FAB_CSS,
    css`
      :host {
        display: contents;
      }
      .calendar-fab {
        right: calc(1rem + 4px + var(--mbr-inset-right, 0px) + env(safe-area-inset-right, 0px));
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
