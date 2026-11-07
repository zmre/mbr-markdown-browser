/**
 * `<mbr-calendar-overlay>` — the calendar overlay (calendar chunk;
 * design/calendar.md §5): a full-screen Month / Week / List view of a set of
 * notes, plotted by note date, modified or created, with a type-to-filter
 * field, a jump picker and its own keys.
 *
 * `<mbr-calendar>` (main bundle) renders it once first asked and keeps it, so
 * the view, plot-by and filter persist for the life of the page; it hands over
 * `scope` (`CalendarScope`: the notes, or all of them) and the services the
 * chunk may not import (`siteIndex`, `resolveUrl`, `navigate`, `config`,
 * `tagFields`). `tasksEndpoint` is accepted for phase 6 and never called: the
 * overlay reads `site.json` alone, so it works the same in static builds.
 *
 * The model is `overlay-model.ts` (bucketing once per scope and plot-by, the
 * filter, the key map); the grids are `mbr-calendar-grid.ts`. While open, one
 * capture-phase `window` keydown listener owns the keyboard (the flashcard
 * deck's pattern): it stops every key's propagation, so the page's `[`, `]`,
 * `p` and `?` never see them, and prevents the default only for keys it
 * handles, so typing in the filter and activating a focused control still work.
 */
import { LitElement, css, html, nothing, type PropertyValues, type TemplateResult } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { positionAnchored } from '../anchored-popover.js'
import { ICON_BUTTON_CSS, KBD_CSS } from '../controls.js'
import { ICON_SM, icon, iconChevronDown, iconChevronLeft, iconChevronRight, iconCircleHelp, iconX } from '../icons.js'
import type { MarkdownFile } from '../sorting.js'
import { DEFAULT_CALENDAR_CONFIG, type CalendarConfig } from './config.js'
import { formatDate, isoWeekOf, monthOf, resolveWeekStart } from './month-grid.js'
import { addDays, todayKey, type DateKey } from './note-date.js'
import {
  OVERLAY_SHORTCUTS,
  PLOT_BYS,
  PLOT_BY_LABELS,
  bucketNotes,
  daysBetween,
  defaultView,
  filesInFolder,
  matchesFilter,
  moveDay,
  nextMatch,
  nextPlotBy,
  prevMatch,
  overlayCommand,
  pageFolder,
  parseFilter,
  parseJump,
  viewRange,
  type Buckets,
  type CalendarView,
  type OverlayCommand,
  type OverlayEntry,
} from './overlay-model.js'
import { findPeriod, periodicIndexOf, type PeriodicIndex } from './periodic.js'
import { OVERLAY_CLOSE_EVENT, type CalendarScope, type DayMarks, type GridEntry, type PlotBy, type SiteIndex } from './types.js'
import './mbr-calendar-grid.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-calendar-overlay': MbrCalendarOverlayElement
  }
}

/** Filter debounce (§5.3, the media browser's). */
const FILTER_DEBOUNCE_MS = 100

/** One day as the views draw it, after the filter. */
interface DayView {
  readonly daily: OverlayEntry | null
  readonly entries: readonly OverlayEntry[]
}

const EMPTY_DAY: DayView = Object.freeze({ daily: null, entries: [] })

@customElement('mbr-calendar-overlay')
export class MbrCalendarOverlayElement extends LitElement {
  @property({ type: Boolean, reflect: true }) open = false
  /** What the overlay plots; a new object re-scopes it. */
  @property({ attribute: false }) scope: CalendarScope = { label: 'All notes' }
  @property({ attribute: false }) siteIndex: () => Promise<SiteIndex | null> = () => Promise.resolve(null)
  @property({ attribute: false }) resolveUrl: (path: string) => string = (path) => path
  @property({ attribute: false }) navigate: (url: string) => void = (url) => window.location.assign(url)
  @property({ attribute: false }) config: CalendarConfig = DEFAULT_CALENDAR_CONFIG
  /** Frontmatter fields holding tags (the configured tag sources). */
  @property({ attribute: false }) tagFields: readonly string[] = ['tags']
  /** Phase 6; never called here (static builds have none). */
  @property({ attribute: false }) tasksEndpoint: string | null = null
  @property() locale = typeof navigator === 'undefined' ? 'en-US' : navigator.language || 'en-US'
  @property({ attribute: false }) now: () => Date = () => new Date()

  @state() private _all: readonly MarkdownFile[] | null = null
  @state() private _label = 'All notes'
  /** The scope's own notes; `null` = all. */
  @state() private _scoped: readonly MarkdownFile[] | null = null
  /** The page's folder when opened with `c` from a page ('' = none). */
  @state() private _folder = ''
  @state() private _folderOnly = false
  @state() private _view: CalendarView | null = null
  @state() private _plotBy: PlotBy = 'date'
  @state() private _filterText = ''
  @state() private _terms: ReturnType<typeof parseFilter> = []
  @state() private _focus: DateKey = ''
  @state() private _today: DateKey = ''
  @state() private _listIndex = 0
  @state() private _popover: { date: DateKey; anchor: HTMLElement } | null = null
  @state() private _picker = false
  @state() private _pickerYear = 2026
  @state() private _jumpError = false
  @state() private _help = false

  private _buckets: Buckets | null = null
  private _bucketsKey: readonly [readonly MarkdownFile[], boolean, PlotBy] | null = null
  private _dayCache = new Map<DateKey, DayView>()
  private _debounce: number | null = null
  private _focusAfterRender: 'grid' | 'popover' | 'list' | null = null

  override connectedCallback(): void {
    super.connectedCallback()
    if (this.open) window.addEventListener('keydown', this._onKeydown, true)
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    window.removeEventListener('keydown', this._onKeydown, true)
    if (this._debounce !== null) window.clearTimeout(this._debounce)
  }

  /** Re-scopes to all notes (the scope chip's ✕). */
  public widen(): void {
    this._scoped = null
    this._label = 'All notes'
    this._folderOnly = false
  }

  protected override willUpdate(changed: PropertyValues<this>): void {
    if (changed.has('open')) {
      if (this.open) window.addEventListener('keydown', this._onKeydown, true)
      else window.removeEventListener('keydown', this._onKeydown, true)
    }
    if ((changed.has('open') || changed.has('scope')) && this.open) this._applyScope()
  }

  protected override updated(): void {
    const target = this._focusAfterRender
    if (!target) return
    this._focusAfterRender = null
    if (target === 'popover') {
      const card = this.renderRoot.querySelector<HTMLElement>('.popover')
      if (card && this._popover) {
        positionAnchored(this._popover.anchor, card)
        card.querySelector<HTMLElement>('a')?.focus()
      }
    } else if (target === 'list' || this._view === 'list') {
      this.renderRoot.querySelector<HTMLElement>(`.row[data-i="${this._listIndex}"]`)?.focus()
    } else {
      void this._grid()?.focusDay()
    }
  }

  /** Opening (or a new scope while open): label, plot-by, starting day, data. */
  private _applyScope(): void {
    const scope = this.scope
    this._label = scope.label
    this._scoped = scope.files ?? null
    this._folderOnly = false
    this._folder = ''
    if (scope.plotBy) this._plotBy = scope.plotBy
    this._view ??= defaultView(window.innerWidth)
    this._today = todayKey(this.now())
    this._focus = this._today
    this._listIndex = 0
    this._popover = null
    this._picker = false
    this._help = false
    this._focusAfterRender = 'grid'
    const ready = (files: readonly MarkdownFile[]): void => {
      this._all = files
      if (scope.pagePath) this._folder = pageFolder(files, scope.pagePath)
      this._startNearMatches()
      this._focusAfterRender = this._view === 'list' ? 'list' : 'grid'
    }
    if (this._all) ready(this._all)
    else
      void this.siteIndex()
        .then((site) => ready(site?.markdown_files ?? []))
        .catch((err: unknown) => console.warn('mbr-calendar: could not load the site index', err))
  }

  /** A scope with nothing this month opens on the nearest month that has something. */
  private _startNearMatches(): void {
    const buckets = this._bucketsNow()
    if (!buckets || !this._scoped) return
    const { from, to } = viewRange(this._view ?? 'month', this._focus, this._weekStart())
    if (this._countRange(from, to).shown > 0) return
    const near = prevMatch(buckets, from, this._terms) ?? nextMatch(buckets, to, this._terms)
    if (near) this._focus = near
  }

  private _grid(): HTMLElementTagNameMap['mbr-calendar-month-grid'] | HTMLElementTagNameMap['mbr-calendar-week-grid'] | null {
    return this.renderRoot.querySelector('mbr-calendar-month-grid') ?? this.renderRoot.querySelector('mbr-calendar-week-grid')
  }

  private _weekStart() {
    return resolveWeekStart(this.config.weekStart, this.locale)
  }

  private get _index(): PeriodicIndex | null {
    return this._all ? periodicIndexOf(this._all) : null
  }

  /** The files in scope now (after "This folder"). */
  private _scopeFiles(): readonly MarkdownFile[] | null {
    const files = this._scoped ?? this._all
    if (!files) return null
    return this._folderOnly && this._folder ? filesInFolder(files, this._folder) : files
  }

  /** The buckets for the current scope and plot-by, built once per pair (§5.4). */
  private _bucketsNow(): Buckets | null {
    const base = this._scoped ?? this._all
    if (!base) return null
    const k = this._bucketsKey
    if (!this._buckets || !k || k[0] !== base || k[1] !== this._folderOnly || k[2] !== this._plotBy) {
      this._buckets = bucketNotes(this._scopeFiles()!, this._plotBy, {
        tagFields: this.tagFields,
        dateFields: this.config.dateFields,
      })
      this._bucketsKey = [base, this._folderOnly, this._plotBy]
      this._dayCache.clear()
    }
    return this._buckets
  }

  /** One day after the filter, memoized until the filter or buckets change. */
  private _day(date: DateKey): DayView {
    const buckets = this._bucketsNow()
    const all = buckets?.days.get(date)
    if (!all) return EMPTY_DAY
    let view = this._dayCache.get(date)
    if (!view) {
      const kept = this._terms.length ? all.filter((e) => matchesFilter(e, this._terms)) : all
      const daily = kept[0]?.daily ? kept[0] : null
      view = { daily, entries: daily ? kept.slice(1) : kept }
      this._dayCache.set(date, view)
    }
    return view
  }

  private _countRange(from: DateKey, to: DateKey): { shown: number; total: number } {
    let shown = 0
    let total = 0
    const buckets = this._bucketsNow()
    if (!buckets) return { shown, total }
    for (const date of daysBetween(from, to)) {
      const all = buckets.days.get(date)
      if (!all) continue
      total += all.length
      const day = this._day(date)
      shown += day.entries.length + (day.daily ? 1 : 0)
    }
    return { shown, total }
  }

  private _time(minutes: number): string {
    const d = new Date(Date.UTC(1970, 0, 1, Math.floor(minutes / 60), minutes % 60))
    try {
      return new Intl.DateTimeFormat(this.locale, { hour: 'numeric', minute: '2-digit', timeZone: 'UTC' }).format(d)
    } catch {
      return `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, '0')}`
    }
  }

  private _gridEntry(e: OverlayEntry, withTime: boolean): GridEntry {
    return {
      key: e.file.url_path,
      title: e.title,
      href: this.resolveUrl(e.file.url_path),
      color: e.color,
      ...(withTime && e.minutes !== null ? { time: this._time(e.minutes) } : {}),
      tooltip: [e.title, e.type, e.folder].filter(Boolean).join(' · '),
    }
  }

  private _monthEntries = (date: DateKey): readonly GridEntry[] => this._day(date).entries.map((e) => this._gridEntry(e, false))
  private _weekEntries = (date: DateKey): readonly GridEntry[] => this._day(date).entries.map((e) => this._gridEntry(e, true))
  private _dayMarks = (date: DateKey): DayMarks => {
    const daily = this._day(date).daily
    return daily ? { dailyUrl: this.resolveUrl(daily.file.url_path) } : {}
  }

  private _periodUrl(key: string): string | null {
    const index = this._index
    const url = index ? findPeriod(index, key)?.url : undefined
    return url ? this.resolveUrl(url) : null
  }

  private _weekHref = (isoWeek: string): string | null => this._periodUrl(isoWeek)

  // ---- actions ----

  private _close(): void {
    this._popover = null
    this._picker = false
    this._help = false
    this.dispatchEvent(new CustomEvent(OVERLAY_CLOSE_EVENT, { bubbles: true, composed: true }))
  }

  private _setFocus(date: DateKey): void {
    this._focus = date
    this._listIndex = 0
    this._popover = null
    this._focusAfterRender = this._view === 'list' ? 'list' : 'grid'
  }

  private _setView(view: CalendarView): void {
    this._view = view
    this._listIndex = 0
    this._popover = null
    this._focusAfterRender = view === 'list' ? 'list' : 'grid'
  }

  private _setPlotBy(plotBy: PlotBy): void {
    this._plotBy = plotBy
    this._listIndex = 0
    this._popover = null
  }

  private _setFilter(text: string): void {
    this._filterText = text
    if (this._debounce !== null) window.clearTimeout(this._debounce)
    this._debounce = window.setTimeout(() => {
      this._debounce = null
      this._terms = parseFilter(text)
      this._dayCache.clear()
      this._listIndex = 0
    }, FILTER_DEBOUNCE_MS)
  }

  private _toggleFolder(): void {
    this._folderOnly = !this._folderOnly
    this._listIndex = 0
  }

  /** Enter on a day: its only note opens; several list in the popover. */
  private _openDay(date: DateKey, anchor?: HTMLElement | null): void {
    const day = this._day(date)
    const count = day.entries.length + (day.daily ? 1 : 0)
    if (count === 0) return
    if (count === 1) {
      this.navigate(this.resolveUrl((day.daily ?? day.entries[0]).file.url_path))
      return
    }
    const cell = anchor ?? this._grid()?.cellFor(date)
    if (!cell) return
    this._popover = { date, anchor: cell }
    this._focusAfterRender = 'popover'
  }

  private _onGridDay = (e: CustomEvent<{ date: DateKey; anchor: HTMLElement }>): void => {
    this._focus = e.detail.date
    const day = this._day(e.detail.date)
    if (day.entries.length + (day.daily ? 1 : 0) > 0) {
      this._popover = { date: e.detail.date, anchor: e.detail.anchor }
      this._focusAfterRender = 'popover'
    }
  }

  private _onGridFocus = (e: CustomEvent<{ date: DateKey }>): void => {
    this._focus = e.detail.date
  }

  private _togglePicker(): void {
    this._picker = !this._picker
    this._pickerYear = Number(this._focus.slice(0, 4))
    this._jumpError = false
  }

  private _jump(date: DateKey): void {
    this._picker = false
    this._setFocus(date)
  }

  private _onJumpSubmit = (e: Event): void => {
    e.preventDefault()
    const input = (e.currentTarget as HTMLFormElement).querySelector('input')
    const parsed = parseJump(input?.value ?? '')
    if (!parsed) {
      this._jumpError = true
      return
    }
    if (parsed.kind === 'week' && this._view === 'month') this._view = 'week'
    this._jump(parsed.date)
  }

  /** The rows of the List view, flat. */
  private _listRows(): { date: DateKey; entry: OverlayEntry }[] {
    const { from, to } = viewRange('list', this._focus, this._weekStart())
    const rows: { date: DateKey; entry: OverlayEntry }[] = []
    for (const date of daysBetween(from, to)) {
      const day = this._day(date)
      if (day.daily) rows.push({ date, entry: day.daily })
      for (const entry of day.entries) rows.push({ date, entry })
    }
    return rows
  }

  private _run(command: OverlayCommand): boolean {
    const view = this._view ?? 'month'
    switch (command) {
      case 'escape':
        if (this._help) this._help = false
        else if (this._popover) {
          this._popover = null
          this._focusAfterRender = 'grid'
        } else if (this._picker) this._picker = false
        else this._close()
        return true
      case 'help':
        this._help = !this._help
        return true
      case 'today':
        this._today = todayKey(this.now())
        this._setFocus(this._today)
        return true
      case 'month':
      case 'week':
      case 'list':
        this._setView(command)
        return true
      case 'jump':
        this._togglePicker()
        return true
      case 'plot':
        this._setPlotBy(nextPlotBy(this._plotBy))
        return true
      case 'filter':
        this.renderRoot.querySelector<HTMLInputElement>('input.filter')?.focus()
        return true
      case 'open': {
        if (view !== 'list') {
          this._openDay(this._focus)
          return true
        }
        const row = this._listRows()[this._listIndex]
        if (row) this.navigate(this.resolveUrl(row.entry.file.url_path))
        return true
      }
      default:
        break
    }
    if (view === 'list' && command !== 'prev' && command !== 'next') {
      const rows = this._listRows()
      const delta = command === 'left' || command === 'up' ? -1 : 1
      this._listIndex = Math.max(0, Math.min(rows.length - 1, this._listIndex + delta))
      this._focusAfterRender = 'list'
      return true
    }
    const next = moveDay(command, view, this._focus)
    if (!next) return false
    this._focus = next
    this._listIndex = 0
    this._popover = null
    this._focusAfterRender = view === 'list' ? 'list' : 'grid'
    return true
  }

  /** The popover's links, for ↑/↓ inside it. */
  private _movePopover(delta: number): void {
    const links = [...this.renderRoot.querySelectorAll<HTMLAnchorElement>('.popover a')]
    const at = links.indexOf(this.shadowRoot?.activeElement as HTMLAnchorElement)
    links[Math.max(0, Math.min(links.length - 1, at + delta))]?.focus()
  }

  private _onKeydown = (e: KeyboardEvent): void => {
    if (!this.open) return
    // Every key stops here, so the page's own shortcuts never see one; only
    // handled keys lose their default action.
    e.stopPropagation()
    if (e.ctrlKey || e.metaKey || e.altKey) return
    const target = e.composedPath()[0] as HTMLElement | undefined
    const tag = target?.tagName
    const handled = (): void => e.preventDefault()
    if (tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA') {
      const toGrid = e.key === 'Escape' || (target!.classList.contains('filter') && (e.key === 'Enter' || e.key === 'ArrowDown'))
      if (!toGrid) return
      if (e.key === 'Escape' && this._picker) this._picker = false
      handled()
      target!.blur()
      this._focusAfterRender = this._view === 'list' ? 'list' : 'grid'
      this.requestUpdate()
      return
    }
    if ((e.key === 'Enter' || e.key === ' ') && (tag === 'BUTTON' || tag === 'A')) return
    if (this._picker && e.key !== 'Escape') return
    if (this._popover && e.key !== 'Escape') {
      const command = overlayCommand(e.key)
      if (command === 'up' || command === 'down') {
        handled()
        this._movePopover(command === 'up' ? -1 : 1)
      }
      return
    }
    const command = overlayCommand(e.key)
    if (command && this._run(command)) handled()
  }

  // ---- render ----

  private _title(view: CalendarView): string {
    if (view !== 'week') return formatDate(`${monthOf(this._focus)}-01`, this.locale, { month: 'long', year: 'numeric' })
    const { from, to } = viewRange('week', this._focus, this._weekStart())
    return `${formatDate(from, this.locale, { month: 'short', day: 'numeric' })} – ${formatDate(to, this.locale, { month: 'short', day: 'numeric', year: 'numeric' })}`
  }

  /** Week / month / quarter / year notes of the period on screen that exist. */
  private _periodLinks(view: CalendarView): TemplateResult[] {
    const { from } = viewRange(view, this._focus, this._weekStart())
    // A week belongs to its Thursday's month, quarter and year (ISO).
    const anchor = view === 'week' ? addDays(from, (4 - this._weekStart() + 7) % 7) : from
    const year = anchor.slice(0, 4)
    const quarter = `${year}-Q${Math.floor((Number(anchor.slice(5, 7)) - 1) / 3) + 1}`
    const week = isoWeekOf(anchor).key
    const keys: [string, string][] = [
      ...(view === 'week' ? ([[week, `Week ${Number(week.slice(6))}`]] as [string, string][]) : []),
      [monthOf(anchor), formatDate(`${monthOf(anchor)}-01`, this.locale, { month: 'short' })],
      [quarter, `Q${quarter.slice(6)} ${year}`],
      [year, year],
    ]
    return keys.flatMap(([key, label]) => {
      const url = this._periodUrl(key)
      return url ? [html`<a href=${url} title=${`Open the ${label} note`}>${label}</a>`] : []
    })
  }

  private _renderPicker(): TemplateResult {
    const months = Array.from({ length: 12 }, (_, i) => `${this._pickerYear}-${String(i + 1).padStart(2, '0')}`)
    // prettier-ignore
    return html`<div class="picker" role="dialog" aria-label="Jump to a date"><div class="yr"><button type="button" class="mbr-icon-btn sm" aria-label="Previous year" @click=${() => this._pickerYear--}>${icon(iconChevronLeft, { size: ICON_SM })}</button><strong>${this._pickerYear}</strong><button type="button" class="mbr-icon-btn sm" aria-label="Next year" @click=${() => this._pickerYear++}>${icon(iconChevronRight, { size: ICON_SM })}</button></div><div class="months">${months.map((m) => html`<button type="button" class=${m === monthOf(this._focus) ? 'cur' : ''} @click=${() => this._jump(`${m}-01`)}>${formatDate(`${m}-01`, this.locale, { month: 'short' })}</button>`)}</div><form @submit=${this._onJumpSubmit}><input type="text" placeholder="2026-10 or 2026-W44" aria-label="Date, month or week" aria-invalid=${this._jumpError ? 'true' : 'false'} @input=${() => (this._jumpError = false)} /><button type="submit">Go</button></form>${this._jumpError ? html`<p class="err" role="alert">Try 2026-10, 2026-10-31 or 2026-W44.</p>` : nothing}</div>`
  }

  private _renderPopover(): TemplateResult | typeof nothing {
    const pop = this._popover
    if (!pop) return nothing
    const day = this._day(pop.date)
    const rows = [...(day.daily ? [day.daily] : []), ...day.entries]
    // prettier-ignore
    return html`<div class="popover" role="dialog" aria-label=${formatDate(pop.date, this.locale, { dateStyle: 'full' })}><div class="ph"><strong>${formatDate(pop.date, this.locale, { weekday: 'long', month: 'long', day: 'numeric' })}</strong><button type="button" class="mbr-icon-btn sm" aria-label="Close" @click=${() => { this._popover = null; this._focusAfterRender = 'grid' }}>${icon(iconX, { size: ICON_SM })}</button></div><ul>${rows.map((e) => html`<li>${this._renderRowBody(e, e.daily)}</li>`)}</ul></div>`
  }

  private _renderRowBody(e: OverlayEntry, daily: boolean, attrs?: { i: number; focus: boolean }): TemplateResult {
    const dot = e.color === null ? '' : `background:var(--mbr-cal-type-${e.color})`
    // prettier-ignore
    return html`<a class=${attrs ? 'row' : 'prow'} href=${this.resolveUrl(e.file.url_path)} data-i=${attrs?.i ?? nothing} tabindex=${attrs ? (attrs.focus ? 0 : -1) : nothing}><i class="dot" style=${dot}></i>${e.minutes !== null ? html`<span class="time">${this._time(e.minutes)}</span>` : nothing}<span class="t">${daily ? 'Daily note' : e.title}</span>${e.folder ? html`<span class="folder">${e.folder}</span>` : nothing}</a>`
  }

  private _renderList(): TemplateResult {
    const rows = this._listRows()
    let i = 0
    let last = ''
    const out: TemplateResult[] = []
    for (const row of rows) {
      if (row.date !== last) {
        last = row.date
        // prettier-ignore
        out.push(html`<h3 class=${row.date === this._today ? 'today' : ''}><time datetime=${row.date}>${formatDate(row.date, this.locale, { weekday: 'long', month: 'long', day: 'numeric' })}</time></h3>`)
      }
      out.push(this._renderRowBody(row.entry, row.entry.daily, { i, focus: i === this._listIndex }))
      i++
    }
    // prettier-ignore
    return html`<div class="list" @focusin=${(e: Event) => { const i = (e.target as HTMLElement).dataset?.i; if (i !== undefined) this._listIndex = Number(i) }}>${out}</div>`
  }

  private _renderView(view: CalendarView): TemplateResult {
    const weekNumbers = this.config.weekNumbers ?? (this._index ? this._index.week.length > 0 : false)
    if (view === 'list') return this._renderList()
    if (view === 'week') {
      // prettier-ignore
      return html`<mbr-calendar-week-grid .start=${viewRange('week', this._focus, this._weekStart()).from} .focused=${this._focus} .today=${this._today} .locale=${this.locale} .dayEntries=${this._weekEntries} .dayMarks=${this._dayMarks} @mbr-calendar-focus=${this._onGridFocus}></mbr-calendar-week-grid>`
    }
    // prettier-ignore
    return html`<mbr-calendar-month-grid .month=${monthOf(this._focus)} .focused=${this._focus} .today=${this._today} .weekStart=${this._weekStart()} .weekNumbers=${weekNumbers} .locale=${this.locale} .dayEntries=${this._monthEntries} .dayMarks=${this._dayMarks} .weekHref=${this._weekHref} @mbr-calendar-focus=${this._onGridFocus} @mbr-calendar-day=${this._onGridDay}></mbr-calendar-month-grid>`
  }

  override render(): TemplateResult | typeof nothing {
    if (!this.open) return nothing
    const view = this._view ?? 'month'
    const buckets = this._bucketsNow()
    const { from, to } = viewRange(view, this._focus, this._weekStart())
    const { shown, total } = this._countRange(from, to)
    const where = view === 'week' ? 'this week' : `in ${formatDate(from, this.locale, { month: 'long' })}`
    const count = `${this._terms.length ? `${shown} of ` : ''}${total} ${total === 1 ? 'note' : 'notes'} ${where}`
    const before = buckets && shown === 0 ? prevMatch(buckets, from, this._terms) : null
    const after = buckets && shown === 0 ? nextMatch(buckets, to, this._terms) : null
    const unit = view === 'week' ? 'week' : 'month'
    const step = (d: number) => () => this._run(d < 0 ? 'prev' : 'next')
    // prettier-ignore
    return html`<div class="backdrop" @click=${() => this._close()}></div><div class="dialog" role="dialog" aria-modal="true" aria-label="Calendar">
<div class="bar"><div class="nav"><button type="button" class="mbr-icon-btn" aria-label=${`Previous ${unit}`} title=${`Previous ${unit} ([)`} @click=${step(-1)}>${icon(iconChevronLeft)}</button><button type="button" class="mbr-icon-btn labeled" title="Today (t)" @click=${() => this._run('today')}>Today</button><button type="button" class="mbr-icon-btn" aria-label=${`Next ${unit}`} title=${`Next ${unit} (])`} @click=${step(1)}>${icon(iconChevronRight)}</button><button type="button" class="title mbr-icon-btn labeled" aria-expanded=${this._picker ? 'true' : 'false'} title="Jump to… (g)" @click=${() => this._togglePicker()}>${this._title(view)}${icon(iconChevronDown, { size: ICON_SM })}</button><span class="links">${this._periodLinks(view)}</span></div>
<div class="views" role="group" aria-label="View">${(['month', 'week', 'list'] as const).map((v) => html`<button type="button" aria-pressed=${v === view ? 'true' : 'false'} title=${`${v === 'list' ? 'List' : v === 'week' ? 'Week' : 'Month'} (${v === 'list' ? 'a' : v[0]})`} @click=${() => this._setView(v)}>${v === 'list' ? 'List' : v === 'week' ? 'Week' : 'Month'}</button>`)}</div><button type="button" class="mbr-icon-btn" aria-label="Keyboard shortcuts" title="Keyboard shortcuts (?)" @click=${() => (this._help = !this._help)}>${icon(iconCircleHelp)}</button><button type="button" class="mbr-icon-btn" aria-label="Close" title="Close (Esc)" @click=${() => this._close()}>${icon(iconX)}</button></div>
<div class="tools"><span class="chip">${this._label}${this._scoped ? html`<button type="button" class="mbr-icon-btn sm" aria-label="Show all notes" title="Show all notes" @click=${() => this.widen()}>${icon(iconX, { size: ICON_SM })}</button>` : nothing}</span>${this._folder && !this._scoped ? html`<button type="button" class="chip toggle" aria-pressed=${this._folderOnly ? 'true' : 'false'} @click=${() => this._toggleFolder()}>This folder: ${this._folder}</button>` : nothing}<input class="filter" type="search" placeholder="Filter (/)" aria-label="Filter notes" .value=${this._filterText} @input=${(e: Event) => this._setFilter((e.target as HTMLInputElement).value)} autocomplete="off" spellcheck="false" /><label class="plot">Plot by <select @change=${(e: Event) => this._setPlotBy((e.target as HTMLSelectElement).value as PlotBy)}>${PLOT_BYS.map((p) => html`<option value=${p} ?selected=${p === this._plotBy}>${PLOT_BY_LABELS[p]}</option>`)}</select></label><span class="count" role="status">${buckets ? count : 'Loading…'}</span></div>
${buckets && shown === 0 ? html`<div class="empty">${this._terms.length ? 'No matches' : 'No notes'} ${where}.${before ? html` <button type="button" class="linkish" @click=${() => this._setFocus(before)}>‹ Previous ${unit} with matches</button>` : nothing}${after ? html` <button type="button" class="linkish" @click=${() => this._setFocus(after)}>Next ${unit} with matches ›</button>` : nothing}</div>` : nothing}
<div class=${`view ${view}`}>${buckets ? this._renderView(view) : nothing}</div>
${this._picker ? this._renderPicker() : nothing}${this._renderPopover()}${this._help ? html`<div class="help" role="dialog" aria-label="Calendar keys"><table>${OVERLAY_SHORTCUTS.map((s) => html`<tr><td><kbd>${s.keys}</kbd></td><td>${s.description}</td></tr>`)}</table></div>` : nothing}</div>`
  }

  static override styles = [
    ICON_BUTTON_CSS,
    KBD_CSS,
    css`
      :host {
        display: contents;
        --mbr-kbd-size: 0.75rem;
      }
      .backdrop {
        position: fixed;
        inset: 0;
        z-index: 1000;
        background: rgba(0, 0, 0, 0.4);
      }
      .dialog {
        position: fixed;
        inset: 2vh 2vw;
        z-index: 1001;
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
        box-sizing: border-box;
        padding: 0.6rem 0.8rem 0.8rem;
        border-radius: 10px;
        background: var(--pico-background-color, #fff);
        color: var(--pico-color);
        box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
        font-size: 0.9rem;
      }
      .bar,
      .tools,
      .nav {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.3rem;
        min-width: 0;
      }
      .nav {
        flex: 1;
      }
      .title {
        font-weight: 700;
        font-size: 1.05rem;
        width: auto;
        padding-inline: 0.4rem;
        gap: 0.25rem;
      }
      .labeled {
        width: auto;
        padding-inline: 0.5rem;
      }
      .links {
        display: flex;
        gap: 0.6rem;
        font-size: 0.8rem;
      }
      a {
        color: var(--pico-primary);
        text-decoration: none;
      }
      a:hover {
        text-decoration: underline;
      }
      .views {
        display: inline-flex;
        border: 1px solid var(--pico-muted-border-color, #ccc);
        border-radius: 8px;
        overflow: hidden;
      }
      .views button,
      .months button,
      form button,
      .chip,
      .linkish {
        margin: 0;
        padding: 0.25rem 0.6rem;
        border: 0;
        background: none;
        color: var(--pico-color);
        font: inherit;
        font-size: 0.85rem;
        cursor: pointer;
      }
      .views button[aria-pressed='true'] {
        background: var(--pico-primary);
        color: var(--pico-primary-inverse, #fff);
      }
      .chip {
        display: inline-flex;
        align-items: center;
        gap: 0.2rem;
        padding: 0.1rem 0.5rem;
        border-radius: 999px;
        background: var(--mbr-cal-daily-bg, rgba(1, 114, 173, 0.12));
        cursor: default;
      }
      .chip .mbr-icon-btn.sm {
        width: 22px;
        height: 22px;
      }
      .chip.toggle {
        cursor: pointer;
        background: none;
        border: 1px solid var(--pico-muted-border-color, #ccc);
      }
      .chip.toggle[aria-pressed='true'] {
        background: var(--mbr-cal-daily-bg, rgba(1, 114, 173, 0.12));
        border-color: var(--pico-primary);
      }
      .filter {
        flex: 1 1 10rem;
        min-width: 6rem;
        max-width: 22rem;
        height: 2rem;
        margin: 0;
        padding: 0 0.5rem;
        font-size: 0.9rem;
      }
      .plot {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        margin: 0;
        font-size: 0.85rem;
        white-space: nowrap;
      }
      .plot select {
        width: auto;
        height: 2rem;
        margin: 0;
        padding: 0 2rem 0 0.5rem;
        font-size: 0.85rem;
      }
      .count {
        margin-left: auto;
        font-size: 0.8rem;
        color: var(--pico-muted-color);
      }
      .empty {
        font-size: 0.85rem;
        color: var(--pico-muted-color);
      }
      .linkish {
        padding: 0;
        color: var(--pico-primary);
      }
      .view {
        flex: 1;
        min-height: 0;
        overflow: auto;
      }
      .view.month,
      .view.week {
        overflow: hidden;
      }
      .list h3 {
        position: sticky;
        top: 0;
        z-index: 1;
        margin: 0.6rem 0 0.2rem;
        padding: 0.2rem 0;
        background: var(--pico-background-color, #fff);
        font-size: 0.85rem;
      }
      .list h3.today {
        color: var(--pico-primary);
      }
      .row,
      .prow {
        display: flex;
        align-items: center;
        gap: 0.45rem;
        padding: 0.3rem 0.4rem;
        border-radius: 6px;
        color: var(--pico-color);
        outline: none;
      }
      .row:hover,
      .row:focus,
      .prow:hover,
      .prow:focus {
        background: var(--mbr-control-hover-bg, rgba(0, 0, 0, 0.06));
        text-decoration: none;
      }
      .row:focus-visible,
      .prow:focus-visible {
        box-shadow: inset 0 0 0 2px var(--pico-primary);
      }
      .dot {
        flex: none;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--pico-muted-color, #888);
      }
      .t {
        min-width: 0;
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
      }
      .time,
      .folder {
        flex: none;
        font-size: 0.8rem;
        color: var(--pico-muted-color);
        font-variant-numeric: tabular-nums;
      }
      .folder {
        margin-left: auto;
        max-width: 40%;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .picker,
      .popover,
      .help {
        position: fixed;
        z-index: 1002;
        box-sizing: border-box;
        max-width: calc(100vw - 16px);
        max-height: 70vh;
        overflow: auto;
        padding: 0.6rem;
        border: 1px solid var(--pico-muted-border-color, #ccc);
        border-radius: 8px;
        background: var(--pico-background-color, #fff);
        box-shadow: 0 6px 24px rgba(0, 0, 0, 0.25);
      }
      .picker {
        top: calc(2vh + 3.2rem);
        left: calc(2vw + 4rem);
        width: 17rem;
      }
      .help {
        top: calc(2vh + 3.2rem);
        right: calc(2vw + 1rem);
        font-size: 0.85rem;
      }
      .help td {
        padding: 0.15rem 0.4rem;
        border: 0;
      }
      .popover {
        width: 20rem;
      }
      .popover ul {
        list-style: none;
        margin: 0;
        padding: 0;
      }
      .popover li {
        margin: 0;
        list-style: none;
      }
      .ph,
      .yr {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: 0.4rem;
      }
      .months {
        display: grid;
        grid-template-columns: repeat(4, 1fr);
        gap: 0.2rem;
        margin-bottom: 0.5rem;
      }
      .months button {
        border-radius: 6px;
        text-align: center;
      }
      .months button:hover {
        background: var(--mbr-control-hover-bg, rgba(0, 0, 0, 0.06));
      }
      .months button.cur {
        background: var(--pico-primary);
        color: var(--pico-primary-inverse, #fff);
      }
      form {
        display: flex;
        gap: 0.3rem;
        margin: 0;
      }
      form input {
        flex: 1;
        height: 2rem;
        margin: 0;
        padding: 0 0.4rem;
        font-size: 0.85rem;
      }
      form button {
        border: 1px solid var(--pico-muted-border-color, #ccc);
        border-radius: 6px;
      }
      .err {
        margin: 0.3rem 0 0;
        font-size: 0.8rem;
        color: var(--mbr-cal-overdue, #c62828);
      }
      @media (max-width: 600px) {
        .dialog {
          inset: 0;
          border-radius: 0;
          padding: calc(0.4rem + env(safe-area-inset-top, 0px)) calc(0.5rem + env(safe-area-inset-right, 0px))
            calc(0.5rem + env(safe-area-inset-bottom, 0px)) calc(0.5rem + env(safe-area-inset-left, 0px));
        }
        .nav {
          flex-basis: 100%;
        }
        .links,
        .plot {
          font-size: 0.8rem;
        }
        .filter {
          font-size: 16px;
          max-width: none;
        }
        .count {
          margin-left: 0;
          width: 100%;
        }
        .picker {
          left: 8px;
        }
        .view.week {
          overflow: auto;
        }
      }
    `,
  ]
}
