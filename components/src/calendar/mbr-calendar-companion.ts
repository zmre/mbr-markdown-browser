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
 * `reveal`, `config`, `tasksEndpoint`, `editEnabled`. The page's own agenda is
 * parsed from the live DOM (`root`); another day's from that day note's HTML,
 * fetched and parsed by the same parser (`day-agenda.ts`).
 *
 * Which day is shown first: the note's own day on a day note. On any other
 * note that has agenda entries, the agenda is that note's, titled "Times in
 * this note" with no date claim (§11 O5: a log note's single date says
 * nothing about when its times happened); picking a day or Today switches to
 * that day. Otherwise today.
 *
 * No editing (D13): blocks only navigate.
 */
import { LitElement, css, html, nothing, unsafeCSS, type PropertyValues, type TemplateResult } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { ICON_BUTTON_CSS } from '../controls.js'
import { ICON_SM, icon, iconX } from '../icons.js'
import type { MarkdownFile } from '../sorting.js'
import { DEFAULT_CALENDAR_CONFIG, type CalendarConfig } from './config.js'
import { agendaEntries, type AgendaEntry } from './agenda-parse.js'
import { layoutAgenda, visibleRange } from './agenda-layout.js'
import { agendaFragment } from './agenda-page.js'
import { loadDayAgenda, type FetchPageHtml } from './day-agenda.js'
import { bucketByDate, groupDayNotes, type DayBuckets } from './day-notes.js'
import { formatDate, monthGrid, monthOf, periodStart, resolveWeekStart, type MonthKey } from './month-grid.js'
import { todayKey, type DateKey } from './note-date.js'
import { buildPeriodicIndex, findPeriod, periodKindOf, type PeriodicIndex } from './periodic.js'
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

/** Pixels per minute on the timeline (40px an hour). */
const PX_PER_MINUTE = 40 / 60
/** Width of the hour-label gutter. */
const GUTTER = '2.6rem'
const MINUTE_MS = 60_000

/** The indexes derived from `site.json`, built once per page. */
interface SiteData {
  readonly index: PeriodicIndex
  readonly buckets: DayBuckets
}

/** Built once per `markdown_files` array: the companion may be re-created. */
const siteDataCache = new WeakMap<readonly MarkdownFile[], SiteData>()

function siteDataFor(files: readonly MarkdownFile[]): SiteData {
  let data = siteDataCache.get(files)
  if (!data) {
    data = { index: buildPeriodicIndex(files), buckets: bucketByDate(files) }
    siteDataCache.set(files, data)
  }
  return data
}

/** What the agenda section is showing. */
type DayAgenda =
  | { readonly kind: 'page'; readonly entries: readonly AgendaEntry[] }
  | { readonly kind: 'none' }
  | { readonly kind: 'loading'; readonly url: string }
  | { readonly kind: 'error'; readonly url: string }
  | { readonly kind: 'fetched'; readonly url: string; readonly entries: readonly AgendaEntry[] }

@customElement('mbr-calendar-companion')
export class MbrCalendarCompanionElement extends LitElement {
  /** Docked column or drawer. */
  @property({ reflect: true }) placement: CompanionPlacement = 'drawer'
  /** The note's rendered body (`main#wrapper`), or `null` off a note. */
  @property({ attribute: false }) root: ParentNode | null = null
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
  /** `POST /.mbr/tasks`, or `null` (tasks off, static build). Phase 6. */
  @property({ attribute: false }) tasksEndpoint: string | null = null
  /** Whether the page may write (unused until a later phase needs it). */
  @property({ attribute: false }) editEnabled = false
  /** Phase 6 hook: per-day task counts; `null` draws no rings. */
  @property({ attribute: false }) taskMarkers: TaskMarkerSource | null = null
  /** The page's site-absolute `url_path` (decoded), to recognise itself. */
  @property() currentPath = ''
  /** The page's canonical period key when it is a periodic note. */
  @property() pagePeriod: string | null = null
  /** BCP 47 locale for every label. */
  @property() locale = typeof navigator === 'undefined' ? 'en-US' : navigator.language || 'en-US'
  /** The clock; tests pass a fixed one. */
  @property({ attribute: false }) now: () => Date = () => new Date()

  @state() private _site: SiteData | null = null
  @state() private _month: MonthKey = ''
  /** The day shown; `null` = this note's own times ("Times in this note"). */
  @state() private _selected: DateKey | null = null
  @state() private _agenda: DayAgenda = { kind: 'none' }
  @state() private _today: DateKey = ''
  @state() private _minutes = 0
  @state() private _taskCounts: ReadonlyMap<DateKey, DayTaskCount> | null = null

  private _pageEntries: readonly AgendaEntry[] = []
  private _timer: number | null = null
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

  override connectedCallback(): void {
    super.connectedCallback()
    this._tick()
    this._timer = window.setInterval(() => this._tick(), MINUTE_MS)
    document.addEventListener('visibilitychange', this._onVisibility)
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    if (this._timer !== null) window.clearInterval(this._timer)
    this._timer = null
    document.removeEventListener('visibilitychange', this._onVisibility)
  }

  protected override willUpdate(changed: PropertyValues<this>): void {
    if (this._initialized) {
      if (changed.has('taskMarkers')) this._loadTaskCounts()
      return
    }
    this._initialized = true
    this._pageEntries = this.root ? agendaEntries(this.root) : []
    const pageDay = this._pageDay
    this._selected = pageDay ?? (this._pageEntries.length > 0 ? null : this._today)
    const anchor = pageDay ?? (this.pagePeriod ? periodStart(this.pagePeriod) : null) ?? this._today
    this._month = monthOf(anchor)
    this._loadAgenda()
    void this.siteIndex()
      .then((site) => {
        this._site = siteDataFor(site?.markdown_files ?? [])
        this._loadAgenda()
        this._loadTaskCounts()
      })
      .catch((err: unknown) => console.warn('mbr-calendar: could not load the site index', err))
  }

  private _onVisibility = (): void => {
    if (!document.hidden) this._tick()
  }

  /** Keeps today and the current-time line fresh (skipped while hidden). */
  private _tick(): void {
    if (typeof document !== 'undefined' && document.hidden && this._today) return
    const date = this.now()
    this._today = todayKey(date)
    this._minutes = date.getHours() * 60 + date.getMinutes()
  }

  /** The day note for `date`, if the repository has one. */
  private _dayNote(date: DateKey): string | null {
    return this._site ? (findPeriod(this._site.index, date)?.url ?? null) : null
  }

  private _loadAgenda(): void {
    const date = this._selected
    if (date === null || date === this._pageDay) {
      this._agenda = { kind: 'page', entries: this._pageEntries }
      return
    }
    if (!this._site) {
      this._agenda = { kind: 'loading', url: '' }
      return
    }
    const url = this._dayNote(date)
    if (!url) {
      this._agenda = { kind: 'none' }
      return
    }
    if (url === this.currentPath) {
      this._agenda = { kind: 'page', entries: this._pageEntries }
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

  /** Phase 6: task rings for the visible grid, at idle, when a source exists. */
  private _loadTaskCounts(): void {
    const source = this.taskMarkers
    if (!source || !this._month) return
    const weeks = monthGrid(this._month, this._weekStart()).weeks
    const from = weeks[0].days[0]
    const to = weeks[weeks.length - 1].days[6]
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

  private _dayMarks = (date: DateKey): DayMarks => {
    const site = this._site
    if (!site) return {}
    const daily = findPeriod(site.index, date)?.url
    const notes = (site.buckets.get(date) ?? []).filter((f) => f.url_path !== daily).length
    const tasks = this._taskCounts?.get(date)
    return {
      ...(daily ? { dailyUrl: daily } : {}),
      ...(notes ? { notes } : {}),
      ...(tasks?.open ? { open: tasks.open, overdue: tasks.overdue } : {}),
    }
  }

  private _weekHref = (isoWeek: string): string | null => {
    const url = this._site ? findPeriod(this._site.index, isoWeek)?.url : undefined
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
  private _openEntry(entry: AgendaEntry): void {
    const agenda = this._agenda
    if (agenda.kind === 'fetched') {
      this.navigate(this.resolveUrl(agenda.url) + agendaFragment(entry.ordinal))
    } else if (entry.li.isConnected) {
      this.reveal(entry.li)
    }
  }

  private _time(minutes: number): string {
    const d = new Date(Date.UTC(1970, 0, 1, Math.floor(minutes / 60), minutes % 60))
    try {
      return new Intl.DateTimeFormat(this.locale, { hour: 'numeric', minute: '2-digit', timeZone: 'UTC' }).format(d)
    } catch {
      return `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, '0')}`
    }
  }

  private _span(entry: AgendaEntry): string {
    return entry.end === undefined ? this._time(entry.start) : `${this._time(entry.start)}–${this._time(entry.end)}`
  }

  private _renderTimeline(entries: readonly AgendaEntry[], showNow: boolean): TemplateResult {
    if (entries.length === 0) return html`<p class="muted">No times in this note.</p>`
    const { from, to } = visibleRange(entries)
    const hours = Array.from({ length: (to - from) / 60 + 1 }, (_, i) => from + i * 60)
    const y = (m: number): number => (m - from) * PX_PER_MINUTE
    const nowVisible = showNow && this._minutes >= from && this._minutes <= to
    // prettier-ignore
    return html`
      <div class="scroll"><div class="timeline" style=${`height:${y(to)}px`}>
        ${hours.map((h) => html`<div class="hour" style=${`top:${y(h)}px`} aria-hidden="true"><span>${h < 1440 ? this._time(h).replace(/:00(?=\D|$)/, '') : ''}</span></div>`)}
        ${layoutAgenda(entries).map((b) => {
          const e = b.item
          const label = [this._span(e), e.title, ...e.details].join(' · ')
          const geometry = `top:${y(b.start)}px;height:${Math.max(y(b.end) - y(b.start), 18)}px;left:calc(${GUTTER} + (100% - ${GUTTER}) * ${b.column} / ${b.columns});width:calc((100% - ${GUTTER}) / ${b.columns} - 2px)`
          return html`<button type="button" class=${`block${b.open ? ' no-end' : ''}${e.task ? ` task ${e.task}` : ''}`} style=${geometry} title=${label} aria-label=${label} @click=${() => this._openEntry(e)}>${e.task ? html`<span class="check" aria-hidden="true"></span>` : nothing}<span class="span">${this._time(e.start)}</span> <span class="title">${e.title}</span>${e.details.map((d) => html`<span class="detail">${d}</span>`)}</button>`
        })}
        ${nowVisible ? html`<div class="now" style=${`top:${y(this._minutes)}px`} aria-hidden="true"></div>` : nothing}
      </div></div>
      <ol class="sr-only">${entries.map((e) => html`<li>${this._span(e)} ${e.title}${e.task ? ` (${e.task})` : ''}${e.details.length ? `: ${e.details.join('; ')}` : ''}</li>`)}</ol>
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
    // prettier-ignore
    return html`
      <section class="agenda" aria-label=${date === null ? heading : `Agenda for ${formatDate(date, this.locale, { dateStyle: 'full' })}`}>
        <h3>${date === null ? html`${heading}` : html`<time datetime=${date}>${heading}</time>`}${dailyUrl && dailyUrl !== this.currentPath ? html`<a class="open-note" href=${this.resolveUrl(dailyUrl)}>Open note →</a>` : nothing}</h3>
        ${due ? html`<p class="due">${due} ${due === 1 ? 'task' : 'tasks'} due</p>` : nothing}
        ${body}
      </section>
    `
  }

  private _renderNotes(): TemplateResult | typeof nothing {
    const date = this._selected
    if (date === null || !this._site) return nothing
    const groups = groupDayNotes(this._site.buckets.get(date) ?? [], this._dayNote(date))
    if (groups.length === 0) return nothing
    const titled = groups.length > 1 || groups[0].type !== null
    // prettier-ignore
    return html`
      <section class="notes" aria-label="Notes from this day">
        <h3>Notes from this day</h3>
        ${groups.map((g) => html`${titled ? html`<h4>${g.type ?? 'Other'}</h4>` : nothing}<ul>${g.notes.map((n) => html`<li><a href=${this.resolveUrl(n.url)}>${n.title}</a>${n.folder ? html`<span class="folder">${n.folder}</span>` : nothing}</li>`)}</ul>`)}
      </section>
    `
  }

  private _renderPanel(): TemplateResult {
    // `dayMarks` is a fresh closure each render: the mini month re-renders only
    // when a property changes identity, and the task counts arrive later.
    const weekNumbers = this.config.weekNumbers ?? (this._site ? this._site.index.week.length > 0 : false)
    const monthUrl = this._site && this._month ? findPeriod(this._site.index, this._month)?.url : undefined
    // prettier-ignore
    return html`
      <div class="panel" @keydown=${this._onKeydown}>
        <mbr-calendar-month
          .month=${this._month}
          .selected=${this._selected}
          .today=${this._today}
          .weekStart=${this._weekStart()}
          .weekNumbers=${weekNumbers}
          .locale=${this.locale}
          .dayMarks=${(d: DateKey) => this._dayMarks(d)}
          .weekHref=${this._weekHref}
          .monthHref=${monthUrl ? this.resolveUrl(monthUrl) : null}
          @mbr-calendar-select=${this._onSelect}
          @mbr-calendar-activate=${this._onActivate}
          @mbr-calendar-month=${this._onMonth}
          @mbr-calendar-today=${this._goToday}
        >
          <button slot="actions" type="button" class="mbr-icon-btn sm labeled today" @click=${this._goToday} title="Today (t)">Today</button>
          <button slot="actions" type="button" class="mbr-icon-btn sm" @click=${this._close} aria-label="Close the calendar" title="Close (C)">${icon(iconX, { size: ICON_SM })}</button>
        </mbr-calendar-month>
        ${this._renderAgenda()}
        ${this._renderNotes()}
      </div>
    `
  }

  override render(): TemplateResult {
    if (this.placement === 'docked') return this._renderPanel()
    // prettier-ignore
    return html`<div class="backdrop" @click=${this._close}></div><div class="drawer" role="dialog" aria-modal="true" aria-label="Calendar">${this._renderPanel()}</div>`
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
