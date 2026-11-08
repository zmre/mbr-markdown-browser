/**
 * `<mbr-calendar-companion>` — the daily-note companion (calendar chunk;
 * design/calendar.md §4).
 *
 * A mini month, the shown day's agenda as a timeline, and "Notes from this
 * day". Docked as the right column on day notes (the trigger, `<mbr-calendar>`,
 * is the column; theme.css reserves it), or a right drawer elsewhere.
 *
 * Everything stateful is injected by the trigger (this chunk must not import
 * `shared.ts`): `siteIndex`, `fetchPageHtml`, `resolveUrl`, `navigate`,
 * `reveal`, `config`, `taskMarkers`. The page's own agenda arrives parsed
 * (`pageEntries`, from the trigger's `enhancePage`); another day's is that day
 * note's HTML, fetched and parsed by the same parser (`day-agenda.ts`).
 *
 * Which day is shown first: the note's own day on a day note. On any other
 * note that has agenda entries, the agenda is that note's, titled "Times in
 * this note" with no date claim (§11 O5: a log note's single date says
 * nothing about when its times happened); picking a day or Today switches to
 * that day. Otherwise today.
 *
 * Cost per page: the periodic index is built once per `site.json`
 * (`periodicIndexOf`), and notes are bucketed by day only for the days on
 * screen (the month grid, plus the selected day), with each day's dot count
 * computed once per bucketing — not per cell per minute.
 *
 * No editing (D13): blocks only navigate.
 */
import { LitElement, css, html, nothing, unsafeCSS, type PropertyValues, type TemplateResult } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { ICON_BUTTON_CSS } from '../controls.js'
import { ICON_SM, icon, iconX } from '../icons.js'
import type { MarkdownFile } from '../sorting.js'
import { DEFAULT_CALENDAR_CONFIG, type CalendarConfig } from './config.js'
import type { AgendaEntry } from './agenda-parse.js'
import { blockStyle, nowLineTop, timelineModel, type TimelineItem } from './agenda-layout.js'
import { agendaFragment } from './agenda-page.js'
import { loadDayAgenda, type AgendaSummary, type FetchPageHtml } from './day-agenda.js'
import { bucketByDate, groupDayNotes, type DayBuckets } from './day-notes.js'
import { onMinute } from './minute-clock.js'
import { formatDate, gridMonthRange, monthOf, periodStart, resolveWeekStart, type MonthKey } from './month-grid.js'
import { todayKey, type DateKey } from './note-date.js'
import { findPeriod, periodKindOf, periodicIndexOf, type PeriodicIndex } from './periodic.js'
import {
  COMPANION_CLOSE_EVENT,
  type CompanionPlacement,
  type DayMarks,
  type DayTaskCount,
  type SiteIndex,
  type TaskMarkerSource,
} from './types.js'
import './mbr-calendar-month.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-calendar-companion': MbrCalendarCompanionElement
  }
}

/** Width of the hour-label gutter. */
const GUTTER = '2.6rem'

/** Notes bucketed for the days on screen, and each day's dot count. */
interface DayWindow {
  readonly from: DateKey
  readonly to: DateKey
  readonly buckets: DayBuckets
  /** Notes dated each day other than its own day note. */
  readonly counts: ReadonlyMap<DateKey, number>
}

/** What the agenda section is showing. */
type DayAgenda =
  | { readonly kind: 'page'; readonly entries: readonly AgendaEntry[] }
  | { readonly kind: 'none' }
  | { readonly kind: 'loading'; readonly url: string }
  | { readonly kind: 'error'; readonly url: string }
  | { readonly kind: 'fetched'; readonly url: string; readonly entries: readonly AgendaSummary[] }

@customElement('mbr-calendar-companion')
export class MbrCalendarCompanionElement extends LitElement {
  /** Docked column or drawer. */
  @property({ reflect: true }) placement: CompanionPlacement = 'drawer'
  /** This page's agenda entries (the trigger's `enhancePage`); empty off a note. */
  @property({ attribute: false }) pageEntries: readonly AgendaEntry[] = []
  /** `site.json` (the trigger hands over its shared fetch). */
  @property({ attribute: false }) siteIndex: () => Promise<SiteIndex | null> = () => Promise.resolve(null)
  /** Fetches a page's HTML, for another day's agenda. */
  @property({ attribute: false }) fetchPageHtml: FetchPageHtml = (url) => fetch(url).then((r) => r.text())
  /** A site-absolute URL for this page's mode (static builds are relative). */
  @property({ attribute: false }) resolveUrl: (path: string) => string = (path) => path
  /** Goes to a URL. */
  @property({ attribute: false }) navigate: (url: string) => void = (url) => window.location.assign(url)
  /** Scrolls an element of this page into view and flashes it. */
  @property({ attribute: false }) reveal: (el: HTMLElement) => void = (el) => el.scrollIntoView()
  /** The `[calendar]` table. */
  @property({ attribute: false }) config: CalendarConfig = DEFAULT_CALENDAR_CONFIG
  /** Per-day open-task counts; `null` (tasks off, static build) draws no rings. */
  @property({ attribute: false }) taskMarkers: TaskMarkerSource | null = null
  /** The page's site-absolute `url_path` (decoded), to recognise itself. */
  @property() currentPath = ''
  /** The page's canonical period key when it is a periodic note. */
  @property() pagePeriod: string | null = null
  /** BCP 47 locale for every label. */
  @property() locale = typeof navigator === 'undefined' ? 'en-US' : navigator.language || 'en-US'
  /** The clock; tests pass a fixed one. */
  @property({ attribute: false }) now: () => Date = () => new Date()

  @state() private _files: readonly MarkdownFile[] | null = null
  @state() private _month: MonthKey = ''
  /** The day shown; `null` = this note's own times ("Times in this note"). */
  @state() private _selected: DateKey | null = null
  @state() private _agenda: DayAgenda = { kind: 'none' }
  @state() private _today: DateKey = ''
  @state() private _minutes = 0
  @state() private _taskCounts: ReadonlyMap<DateKey, DayTaskCount> | null = null

  private _window: DayWindow | null = null
  private _windowFiles: readonly MarkdownFile[] | null = null
  private _offMinute: (() => void) | null = null
  private _initialized = false

  /** Moves keyboard focus onto the mini month. */
  public focusGrid(): void {
    void this.updateComplete.then(() => this.renderRoot.querySelector('mbr-calendar-month')?.focusGrid())
  }

  /** Shows `date` (`null`: this note's own times). */
  public showDay(date: DateKey | null): void {
    this._selected = date
    if (date) this._month = monthOf(date)
    this._loadAgenda()
  }

  /** The page's day when it is a day note. */
  private get _pageDay(): DateKey | null {
    return this.pagePeriod && periodKindOf(this.pagePeriod) === 'day' ? this.pagePeriod : null
  }

  private get _index(): PeriodicIndex | null {
    return this._files ? periodicIndexOf(this._files) : null
  }

  override connectedCallback(): void {
    super.connectedCallback()
    this._tick()
    this._offMinute = onMinute(() => this._tick())
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    this._offMinute?.()
    this._offMinute = null
  }

  protected override willUpdate(changed: PropertyValues<this>): void {
    if (this._initialized) {
      if (changed.has('taskMarkers')) this._loadTaskCounts()
      return
    }
    this._initialized = true
    const pageDay = this._pageDay
    this._selected = pageDay ?? (this.pageEntries.length > 0 ? null : this._today)
    const anchor = pageDay ?? (this.pagePeriod ? periodStart(this.pagePeriod) : null) ?? this._today
    this._month = monthOf(anchor)
    this._loadAgenda()
    void this.siteIndex()
      .then((site) => {
        this._files = site?.markdown_files ?? []
        this._loadAgenda()
        this._loadTaskCounts()
      })
      .catch((err: unknown) => console.warn('mbr-calendar: could not load the site index', err))
  }

  /** Keeps today and the current-time line fresh (the bundle's one minute tick). */
  private _tick(): void {
    const date = this.now()
    this._today = todayKey(date)
    this._minutes = date.getHours() * 60 + date.getMinutes()
  }

  /** The day note for `date`, if the repository has one. */
  private _dayNote(date: DateKey): string | null {
    const index = this._index
    return index ? (findPeriod(index, date)?.url ?? null) : null
  }

  /**
   * The notes of the days on screen (the grid, and the selected day), bucketed
   * once per window: a month change re-buckets, a minute tick does not.
   */
  private _days(): DayWindow | null {
    const files = this._files
    if (!files || !this._month) return null
    const grid = gridMonthRange(this._month, this._weekStart())
    const sel = this._selected
    const from = sel && sel < grid.from ? sel : grid.from
    const to = sel && sel > grid.to ? sel : grid.to
    const w = this._window
    if (w && w.from === from && w.to === to && this._windowFiles === files) return w
    const buckets = bucketByDate(files, from, to)
    const counts = new Map<DateKey, number>()
    for (const [date, notes] of buckets) {
      const daily = this._dayNote(date)
      const n = daily ? notes.filter((f) => f.url_path !== daily).length : notes.length
      if (n) counts.set(date, n)
    }
    this._windowFiles = files
    this._window = { from, to, buckets, counts }
    return this._window
  }

  private _loadAgenda(): void {
    const date = this._selected
    if (date === null || date === this._pageDay) {
      this._agenda = { kind: 'page', entries: this.pageEntries }
      return
    }
    if (!this._files) {
      this._agenda = { kind: 'loading', url: '' }
      return
    }
    const url = this._dayNote(date)
    if (!url) {
      this._agenda = { kind: 'none' }
      return
    }
    if (url === this.currentPath) {
      this._agenda = { kind: 'page', entries: this.pageEntries }
      return
    }
    this._agenda = { kind: 'loading', url }
    loadDayAgenda(this.resolveUrl(url), this.fetchPageHtml).then(
      (entries) => {
        if (this._selected === date) this._agenda = { kind: 'fetched', url, entries }
      },
      (err: unknown) => {
        console.warn('mbr-calendar: could not load', url, err)
        if (this._selected === date) this._agenda = { kind: 'error', url }
      }
    )
  }

  /** Task rings for the visible grid, at idle, when a source exists. */
  private _loadTaskCounts(): void {
    const source = this.taskMarkers
    if (!source || !this._month) return
    const { from, to } = gridMonthRange(this._month, this._weekStart())
    const month = this._month
    const run = (): void => {
      void source(from, to).then(
        (counts) => {
          if (this._month === month) this._taskCounts = counts
        },
        (err: unknown) => console.warn('mbr-calendar: task markers failed', err)
      )
    }
    if (typeof window.requestIdleCallback === 'function') window.requestIdleCallback(run, { timeout: 2000 })
    else setTimeout(run, 0)
  }

  private _weekStart() {
    return resolveWeekStart(this.config.weekStart, this.locale)
  }

  private _weekHref = (isoWeek: string): string | null => {
    const url = this._dayNote(isoWeek)
    return url ? this.resolveUrl(url) : null
  }

  private _onSelect = (e: CustomEvent<{ date: DateKey }>): void => {
    this.showDay(e.detail.date)
  }

  private _onActivate = (e: CustomEvent<{ date: DateKey }>): void => {
    const url = this._dayNote(e.detail.date)
    if (url) this.navigate(this.resolveUrl(url))
    else this.showDay(e.detail.date)
  }

  private _onMonth = (e: CustomEvent<{ month: MonthKey }>): void => {
    this._month = e.detail.month
    this._loadTaskCounts()
  }

  private _goToday = (): void => {
    this._tick()
    this.showDay(this._today)
  }

  private _close = (): void => {
    this.dispatchEvent(new CustomEvent(COMPANION_CLOSE_EVENT, { bubbles: true, composed: true }))
  }

  private _onKeydown = (e: KeyboardEvent): void => {
    if (e.key !== 'Escape') return
    e.preventDefault()
    e.stopPropagation()
    if (this.placement === 'drawer') {
      this._close()
    } else {
      // Docked: leave the companion, back to the page. The focused element
      // may sit one shadow root further down, in the mini month.
      let active = this.shadowRoot?.activeElement ?? null
      while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement
      if (active instanceof HTMLElement) active.blur()
    }
  }

  /** A block was clicked: this page's item is revealed, another day's opened. */
  private _openEntry(entry: AgendaSummary & { readonly li?: HTMLLIElement }): void {
    const agenda = this._agenda
    if (agenda.kind === 'fetched') {
      this.navigate(this.resolveUrl(agenda.url) + agendaFragment(entry.ordinal))
    } else if (entry.li?.isConnected) {
      this.reveal(entry.li)
    }
  }

  /** Page entries (live `<li>`s) or a fetched day's {@link AgendaSummary}s, as `agenda-layout.ts`'s timeline. */
  private _renderTimeline(entries: readonly (AgendaSummary & { readonly li?: HTMLLIElement })[], showNow: boolean): TemplateResult {
    if (entries.length === 0) return html`<p class="muted">No times in this note.</p>`
    const tl = timelineModel<TimelineItem & AgendaSummary & { readonly li?: HTMLLIElement }>(entries, this.locale)
    const now = showNow ? nowLineTop(tl, this._minutes) : null
    const hours = tl.hours.map(
      (h) => html`<div class="hour" style=${`top:${h.top}px`} aria-hidden="true"><span>${h.label}</span></div>`
    )
    const blocks = tl.blocks.map((b) => {
      const e = b.item
      const cls = `block${b.open ? ' no-end' : ''}${e.task ? ` task ${e.task}` : ''}`
      const check = e.task ? html`<span class="check" aria-hidden="true"></span>` : nothing
      const details = e.details.map((d) => html`<span class="detail">${d}</span>`)
      return html`<button type="button" class=${cls} style=${blockStyle(b)} title=${b.label} aria-label=${b.label} @click=${() => this._openEntry(e)}>
        ${check}<span class="span">${b.time}</span> <span class="title">${e.title}</span>${details}
      </button>`
    })
    return html`
      <div class="scroll">
        <div class="timeline" style=${`height:${tl.height}px`}>
          ${hours} ${blocks} ${now !== null ? html`<div class="now" style=${`top:${now}px`} aria-hidden="true"></div>` : nothing}
        </div>
      </div>
      <ol class="sr-only">
        ${tl.spoken.map((line) => html`<li>${line}</li>`)}
      </ol>
    `
  }

  private _renderAgenda(): TemplateResult {
    const date = this._selected
    const agenda = this._agenda
    const dailyUrl = date ? this._dayNote(date) : null
    const due = date ? (this._taskCounts?.get(date)?.open ?? 0) : 0
    const heading =
      date === null
        ? 'Times in this note'
        : formatDate(date, this.locale, { weekday: 'short', month: 'short', day: 'numeric' })
    let body: TemplateResult
    switch (agenda.kind) {
      case 'page':
        body = this._renderTimeline(agenda.entries, date !== null && date === this._today)
        break
      case 'fetched':
        body = this._renderTimeline(agenda.entries, date === this._today)
        break
      case 'loading':
        body = html`<p class="muted" role="status">Loading…</p>`
        break
      case 'error':
        body = html`<p class="muted" role="status">Could not load this day's note.</p>`
        break
      case 'none':
        body = html`<p class="muted">No note for this day.</p>`
        break
    }
    const label = date === null ? heading : `Agenda for ${formatDate(date, this.locale, { dateStyle: 'full' })}`
    const open =
      dailyUrl && dailyUrl !== this.currentPath
        ? html`<a class="open-note" href=${this.resolveUrl(dailyUrl)}>Open note →</a>`
        : nothing
    return html`
      <section class="agenda" aria-label=${label}>
        <h3>${date === null ? heading : html`<time datetime=${date}>${heading}</time>`}${open}</h3>
        ${due ? html`<p class="due">${due} ${due === 1 ? 'task' : 'tasks'} due</p>` : nothing} ${body}
      </section>
    `
  }

  private _renderNotes(): TemplateResult | typeof nothing {
    const date = this._selected
    const days = this._days()
    if (date === null || !days) return nothing
    const groups = groupDayNotes(days.buckets.get(date) ?? [], this._dayNote(date))
    if (groups.length === 0) return nothing
    const titled = groups.length > 1 || groups[0].type !== null
    return html`
      <section class="notes" aria-label="Notes from this day">
        <h3>Notes from this day</h3>
        ${groups.map(
          (g) => html`
            ${titled ? html`<h4>${g.type ?? 'Other'}</h4>` : nothing}
            <ul>
              ${g.notes.map(
                (n) => html`<li>
                  <a href=${this.resolveUrl(n.url)}>${n.title}</a>${n.folder ? html`<span class="folder">${n.folder}</span>` : nothing}
                </li>`
              )}
            </ul>
          `
        )}
      </section>
    `
  }

  private _renderPanel(): TemplateResult {
    // `dayMarks` is a fresh closure each render: the mini month re-renders only
    // when a property changes identity, and the task counts arrive later. Each
    // cell is two map lookups: the counts were taken when the window was bucketed.
    const index = this._index
    const days = this._days()
    const weekNumbers = this.config.weekNumbers ?? (index ? index.week.length > 0 : false)
    const monthUrl = index && this._month ? findPeriod(index, this._month)?.url : undefined
    const taskCounts = this._taskCounts
    const dayMarks = (date: DateKey): DayMarks => {
      if (!index) return {}
      const daily = findPeriod(index, date)?.url
      const notes = days?.counts.get(date)
      const tasks = taskCounts?.get(date)
      return {
        ...(daily ? { dailyUrl: daily } : {}),
        ...(notes ? { notes } : {}),
        ...(tasks?.open ? { open: tasks.open, overdue: tasks.overdue } : {}),
      }
    }
    return html`
      <div class="panel" @keydown=${this._onKeydown}>
        <mbr-calendar-month
          .month=${this._month}
          .selected=${this._selected}
          .today=${this._today}
          .weekStart=${this._weekStart()}
          .weekNumbers=${weekNumbers}
          .locale=${this.locale}
          .dayMarks=${dayMarks}
          .weekHref=${this._weekHref}
          .monthHref=${monthUrl ? this.resolveUrl(monthUrl) : null}
          @mbr-calendar-select=${this._onSelect}
          @mbr-calendar-activate=${this._onActivate}
          @mbr-calendar-month=${this._onMonth}
          @mbr-calendar-today=${this._goToday}
        >
          <button slot="actions" type="button" class="mbr-icon-btn sm labeled today" @click=${this._goToday} title="Today (t)">Today</button>
          <button slot="actions" type="button" class="mbr-icon-btn sm" @click=${this._close} aria-label="Close the calendar" title="Close (C)">
            ${icon(iconX, { size: ICON_SM })}
          </button>
        </mbr-calendar-month>
        ${this._renderAgenda()} ${this._renderNotes()}
      </div>
    `
  }

  override render(): TemplateResult {
    if (this.placement === 'docked') return this._renderPanel()
    return html`
      <div class="backdrop" @click=${this._close}></div>
      <div class="drawer" role="dialog" aria-modal="true" aria-label="Calendar">${this._renderPanel()}</div>
    `
  }

  static override styles = [
    ICON_BUTTON_CSS,
    css`
      :host {
        display: block;
        color: var(--pico-color);
        font-size: 0.9rem;
      }
      :host([placement='docked']) {
        height: 100%;
      }
      .panel {
        box-sizing: border-box;
        padding: 0.6rem 0.55rem 1.5rem;
      }
      :host([placement='docked']) .panel {
        height: 100%;
        overflow-y: auto;
        overscroll-behavior: contain;
      }
      .backdrop {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.35);
        z-index: 1000;
      }
      .drawer {
        position: fixed;
        top: 0;
        right: 0;
        height: 100vh;
        height: 100dvh;
        width: min(100%, 340px);
        box-sizing: border-box;
        padding-top: env(safe-area-inset-top, 0px);
        padding-bottom: env(safe-area-inset-bottom, 0px);
        padding-right: env(safe-area-inset-right, 0px);
        background: var(--pico-background-color, #fff);
        border-left: 1px solid var(--pico-muted-border-color, #e0e0e0);
        box-shadow: -4px 0 12px rgba(0, 0, 0, 0.2);
        overflow-y: auto;
        overscroll-behavior: contain;
        z-index: 1001;
      }
      .mbr-icon-btn.sm {
        width: 24px;
      }
      .mbr-icon-btn.today {
        width: auto;
        padding-inline: 0.35rem;
        font-size: 0.78rem;
      }
      h3 {
        display: flex;
        align-items: baseline;
        justify-content: space-between;
        gap: 0.5rem;
        margin: 1rem 0 0.5rem;
        font-size: 0.9rem;
        font-weight: 600;
      }
      h4 {
        margin: 0.6rem 0 0.2rem;
        font-size: 0.75rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        color: var(--pico-muted-color);
      }
      a {
        color: var(--pico-primary, #0172ad);
        text-decoration: none;
      }
      a:hover,
      a:focus-visible {
        text-decoration: underline;
      }
      .open-note {
        font-size: 0.8rem;
        font-weight: normal;
        white-space: nowrap;
      }
      .due {
        margin: -0.2rem 0 0.35rem;
        font-size: 0.78rem;
        color: var(--mbr-cal-open, var(--pico-primary));
      }
      .muted {
        margin: 0.25rem 0;
        color: var(--pico-muted-color);
        font-size: 0.85rem;
      }
      /* Scrolls by itself only when the day is taller than the room. */
      .scroll {
        max-height: 60vh;
        overflow-y: auto;
        padding-top: 0.6em;
      }
      .timeline {
        position: relative;
      }
      .hour {
        position: absolute;
        left: 0;
        right: 0;
        border-top: 1px solid var(--pico-muted-border-color, #e0e0e0);
      }
      .hour span {
        position: absolute;
        top: -0.55em;
        left: 0;
        width: calc(${unsafeCSS(GUTTER)} - 0.3rem);
        padding-right: 0.3rem;
        background: var(--pico-background-color, #fff);
        font-size: 0.68rem;
        line-height: 1.1;
        color: var(--pico-muted-color);
        font-variant-numeric: tabular-nums;
      }
      .block {
        position: absolute;
        left: calc(${unsafeCSS(GUTTER)} + (100% - ${unsafeCSS(GUTTER)}) * var(--col) / var(--cols));
        width: calc((100% - ${unsafeCSS(GUTTER)}) / var(--cols) - 2px);
        box-sizing: border-box;
        display: block;
        overflow: hidden;
        margin: 0;
        padding: 1px 4px;
        border: none;
        border-left: 3px solid var(--mbr-cal-today, var(--pico-primary));
        border-radius: 4px;
        background: var(--mbr-cal-daily-bg, rgba(1, 114, 173, 0.12));
        color: var(--pico-color);
        font: inherit;
        font-size: 0.75rem;
        line-height: 1.25;
        text-align: left;
        cursor: pointer;
      }
      .block:hover {
        filter: brightness(0.96);
      }
      .block:focus-visible {
        outline: none;
        box-shadow: var(--mbr-focus-ring, 0 0 0 2px var(--pico-primary));
        z-index: 2;
      }
      .block.no-end {
        border-bottom: 1px dashed var(--pico-muted-color);
      }
      .block .span {
        font-variant-numeric: tabular-nums;
        color: var(--pico-muted-color);
      }
      .block .title {
        font-weight: 600;
      }
      .block .detail {
        display: block;
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
        color: var(--pico-muted-color);
      }
      .block .check {
        display: inline-block;
        width: 0.65rem;
        height: 0.65rem;
        margin-right: 0.25rem;
        vertical-align: -1px;
        border: 1.5px solid currentColor;
        border-radius: 2px;
      }
      .block.done .check {
        background: currentColor;
      }
      .block.done .title {
        text-decoration: line-through;
      }
      .block.canceled {
        opacity: 0.55;
      }
      .block.canceled .title {
        text-decoration: line-through;
      }
      .now {
        position: absolute;
        left: ${unsafeCSS(GUTTER)};
        right: 0;
        height: 0;
        border-top: 2px solid var(--mbr-cal-overdue, #d32f2f);
        z-index: 3;
        pointer-events: none;
      }
      .now::before {
        content: '';
        position: absolute;
        left: -4px;
        top: -5px;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--mbr-cal-overdue, #d32f2f);
      }
      .notes ul {
        list-style: none;
        margin: 0;
        padding: 0;
      }
      .notes li {
        display: flex;
        align-items: baseline;
        gap: 0.4rem;
        padding: 0.15rem 0;
        margin: 0;
        min-width: 0;
      }
      .notes a {
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
        min-width: 0;
      }
      .folder {
        flex: none;
        max-width: 45%;
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
        font-size: 0.75rem;
        color: var(--pico-muted-color);
      }
      .sr-only {
        position: absolute;
        width: 1px;
        height: 1px;
        padding: 0;
        margin: -1px;
        overflow: hidden;
        clip: rect(0, 0, 0, 0);
        white-space: nowrap;
        border: 0;
      }
    `,
  ]
}
