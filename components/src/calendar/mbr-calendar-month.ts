/**
 * `<mbr-calendar-month>` — a compact month grid (calendar chunk).
 *
 * The companion's mini month (design/calendar.md §4.2), written to be reused:
 * it knows nothing about notes or tasks. A host hands it the month, the
 * selected and today's dates, the week start and a `dayMarks(date)` lookup,
 * and listens for:
 *
 * - `mbr-calendar-select` `{date}` — a day was clicked or Space was pressed;
 * - `mbr-calendar-activate` `{date}` — double-click or Enter (open the day);
 * - `mbr-calendar-month` `{month}` — ‹ ›, PageUp/PageDown, or focus moving
 *   into a neighbouring month;
 * - `mbr-calendar-today` — `t`.
 *
 * All events bubble and are composed. Grid math is `month-grid.ts`'s, so the
 * overlay's month view and the task grids can share it without this element.
 *
 * Accessibility: `role="grid"`, one `row` per week, a roving tabindex on the
 * day cells, and an `aria-label` on each cell spelling out the full date and
 * what the day holds.
 */
import { LitElement, css, html, nothing, type PropertyValues, type TemplateResult } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { ICON_BUTTON_CSS } from '../controls.js'
import { ICON_SM, icon, iconChevronLeft, iconChevronRight } from '../icons.js'
import type { DateKey } from './note-date.js'
import type { DayMarks } from './types.js'
import {
  addMonths,
  formatDate,
  monthGrid,
  monthOf,
  moveFocus,
  weekdayLabels,
  type MonthKey,
  type Weekday,
} from './month-grid.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-calendar-month': MbrCalendarMonthElement
  }
}

const NO_MARKS: DayMarks = Object.freeze({})

/** Spoken description of a day cell. */
export function dayLabel(date: DateKey, locale: string, marks: DayMarks, today: DateKey): string {
  const parts = [formatDate(date, locale, { weekday: 'long', month: 'long', day: 'numeric', year: 'numeric' })]
  if (date === today) parts.push('today')
  if (marks.dailyUrl) parts.push('daily note')
  if (marks.notes) parts.push(`${marks.notes} other ${marks.notes === 1 ? 'note' : 'notes'}`)
  if (marks.open) parts.push(`${marks.open} open ${marks.open === 1 ? 'task' : 'tasks'}`)
  return parts.join(', ')
}

@customElement('mbr-calendar-month')
export class MbrCalendarMonthElement extends LitElement {
  /** The month shown, `YYYY-MM`. */
  @property() month: MonthKey = ''
  /** The day the host is showing (solid accent), if any. */
  @property() selected: DateKey | null = null
  /** The reader's local date (accent ring). */
  @property() today: DateKey = ''
  /** First column, 0 = Sunday. */
  @property({ type: Number }) weekStart: Weekday = 1
  /** Show the ISO week-number column. */
  @property({ type: Boolean }) weekNumbers = false
  /** BCP 47 locale for month and weekday names. */
  @property() locale = 'en-US'
  /** What each day holds. */
  @property({ attribute: false }) dayMarks: (date: DateKey) => DayMarks = () => NO_MARKS
  /** The week note for an ISO week key (`2026-W44`), as an href, or `null`. */
  @property({ attribute: false }) weekHref: (isoWeek: string) => string | null = () => null
  /** The month note's href, or `null`. */
  @property({ attribute: false }) monthHref: string | null = null

  /** The day with the roving tabindex. */
  @state() private _focus: DateKey = ''
  private _refocus = false

  /** Moves keyboard focus onto the grid's focused day. */
  public focusGrid(): void {
    this._refocus = true
    this.requestUpdate()
  }

  protected override willUpdate(changed: PropertyValues<this>): void {
    // Focus follows a newly selected day, and otherwise stays inside the
    // month shown: the selected day, else today, else the 1st.
    const inMonth = (d: DateKey | null): d is DateKey => !!d && monthOf(d) === this.month
    if (changed.has('selected') && inMonth(this.selected)) {
      this._focus = this.selected
    } else if (!inMonth(this._focus)) {
      this._focus = [this.selected, this.today].find(inMonth) ?? `${this.month}-01`
    }
  }

  protected override updated(): void {
    if (!this._refocus) return
    this._refocus = false
    this.renderRoot.querySelector<HTMLElement>(`[data-date="${this._focus}"]`)?.focus()
  }

  private _emit(type: string, detail: Record<string, unknown> = {}): void {
    this.dispatchEvent(new CustomEvent(type, { detail, bubbles: true, composed: true }))
  }

  private _goMonth(delta: number): void {
    this._emit('mbr-calendar-month', { month: addMonths(this.month, delta) })
  }

  private _onKeydown = (e: KeyboardEvent): void => {
    if (e.ctrlKey || e.metaKey || e.altKey) return
    const handled = (): void => {
      e.preventDefault()
      e.stopPropagation()
    }
    if (e.key === 'Enter') {
      handled()
      this._emit('mbr-calendar-activate', { date: this._focus })
      return
    }
    if (e.key === ' ') {
      handled()
      this._emit('mbr-calendar-select', { date: this._focus })
      return
    }
    if (e.key === 't') {
      handled()
      this._emit('mbr-calendar-today')
      this._refocus = true
      return
    }
    const next = moveFocus(e.key, this._focus)
    if (!next) return
    handled()
    this._focus = next
    this._refocus = true
    if (monthOf(next) !== this.month) this._emit('mbr-calendar-month', { month: monthOf(next) })
  }

  private _renderCell(date: DateKey, inMonth: boolean): TemplateResult {
    const marks = this.dayMarks(date) ?? NO_MARKS
    const dots = Math.min(marks.notes ?? 0, 3)
    const classes = [
      'day',
      inMonth ? '' : 'outside',
      marks.dailyUrl ? 'daily' : '',
      date === this.today ? 'today' : '',
      date === this.selected ? 'selected' : '',
    ]
      .filter(Boolean)
      .join(' ')
    // prettier-ignore
    return html`<div role="gridcell" class=${classes} data-date=${date} tabindex=${date === this._focus ? 0 : -1} aria-selected=${date === this.selected ? 'true' : 'false'} aria-label=${dayLabel(date, this.locale, marks, this.today)} @click=${() => { this._focus = date; this._emit('mbr-calendar-select', { date }) }} @dblclick=${() => this._emit('mbr-calendar-activate', { date })}><span class="num" aria-hidden="true">${Number(date.slice(8))}</span>${marks.open ? html`<span class=${marks.overdue ? 'ring overdue' : 'ring'} aria-hidden="true"></span>` : nothing}<span class="dots" aria-hidden="true">${Array.from({ length: dots }, () => html`<i></i>`)}</span></div>`
  }

  private _renderWeekNumber(isoWeek: string): TemplateResult {
    const href = this.weekHref(isoWeek)
    const label = String(Number(isoWeek.slice(6)))
    return href
      ? html`<a role="rowheader" class="wk has-note" href=${href} title=${`Week ${label}`}>${label}</a>`
      : html`<span role="rowheader" class="wk" title=${`Week ${label}`}>${label}</span>`
  }

  override render(): TemplateResult | typeof nothing {
    if (!this.month) return nothing
    const grid = monthGrid(this.month, this.weekStart)
    const title = formatDate(`${this.month}-01`, this.locale, { month: 'long', year: 'numeric' })
    const labels = weekdayLabels(this.weekStart, this.locale)
    // prettier-ignore
    return html`
      <div class="head">
        ${this.monthHref ? html`<a class="title" href=${this.monthHref} title=${`${title} (open the month note)`}>${title}</a>` : html`<span class="title" title=${title}>${title}</span>`}
        <button type="button" class="mbr-icon-btn sm" aria-label="Previous month" title="Previous month (PageUp)" @click=${() => this._goMonth(-1)}>${icon(iconChevronLeft, { size: ICON_SM })}</button>
        <button type="button" class="mbr-icon-btn sm" aria-label="Next month" title="Next month (PageDown)" @click=${() => this._goMonth(1)}>${icon(iconChevronRight, { size: ICON_SM })}</button>
        <slot name="actions"></slot>
      </div>
      <div role="grid" class=${this.weekNumbers ? 'grid with-weeks' : 'grid'} aria-label=${title} @keydown=${this._onKeydown}>
        <div role="row" class="row labels">${this.weekNumbers ? html`<span class="wk" aria-hidden="true"></span>` : nothing}${labels.map((l) => html`<span role="columnheader" class="dow">${l}</span>`)}</div>
        ${grid.weeks.map((week) => html`<div role="row" class="row">${this.weekNumbers ? this._renderWeekNumber(week.isoWeek) : nothing}${week.days.map((d) => this._renderCell(d, monthOf(d) === this.month))}</div>`)}
      </div>
    `
  }

  static override styles = [
    ICON_BUTTON_CSS,
    css`
      :host {
        display: block;
        font-size: 0.85rem;
      }
      .head {
        display: flex;
        align-items: center;
        gap: 0.1rem;
        margin-bottom: 0.35rem;
      }
      .title {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
        padding-left: 0.15rem;
        font-weight: 600;
        font-size: 0.85rem;
        color: var(--pico-color);
        text-decoration: none;
      }
      .head .mbr-icon-btn.sm {
        width: 24px;
      }
      a.title:hover,
      a.title:focus-visible {
        color: var(--pico-primary);
        text-decoration: underline;
      }
      .row {
        display: grid;
        grid-template-columns: repeat(7, minmax(0, 1fr));
      }
      .with-weeks .row {
        grid-template-columns: 1.6rem repeat(7, minmax(0, 1fr));
      }
      .dow {
        text-align: center;
        font-size: 0.7rem;
        color: var(--pico-muted-color);
        padding-bottom: 0.15rem;
      }
      .wk {
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 0.68rem;
        color: var(--pico-muted-color);
        opacity: 0.6;
        text-decoration: none;
      }
      .wk.has-note {
        font-weight: 700;
        opacity: 1;
        color: var(--pico-color);
      }
      .wk.has-note:hover,
      .wk.has-note:focus-visible {
        color: var(--pico-primary);
      }
      .day {
        position: relative;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        aspect-ratio: 1;
        min-height: 2rem;
        border-radius: 6px;
        cursor: pointer;
        color: var(--pico-color);
        user-select: none;
      }
      .day:hover {
        background: var(--mbr-control-hover-bg, rgba(0, 0, 0, 0.05));
      }
      .day:focus-visible {
        outline: none;
        box-shadow: var(--mbr-focus-ring, 0 0 0 2px var(--pico-primary));
      }
      .day.outside {
        color: var(--pico-muted-color);
        opacity: 0.55;
      }
      .day.daily {
        background: var(--mbr-cal-daily-bg, rgba(1, 114, 173, 0.12));
      }
      .day.today {
        box-shadow: inset 0 0 0 2px var(--mbr-cal-today, var(--pico-primary));
      }
      .day.selected {
        background: var(--mbr-cal-selected, var(--pico-primary));
        color: var(--pico-primary-inverse, #fff);
      }
      .num {
        line-height: 1;
        font-variant-numeric: tabular-nums;
      }
      .dots {
        display: flex;
        gap: 2px;
        height: 4px;
        margin-top: 2px;
      }
      .dots i {
        width: 4px;
        height: 4px;
        border-radius: 50%;
        background: currentColor;
        opacity: 0.7;
      }
      .ring {
        position: absolute;
        top: 3px;
        right: 3px;
        width: 6px;
        height: 6px;
        border-radius: 50%;
        border: 1.5px solid var(--mbr-cal-open, var(--pico-primary));
      }
      .ring.overdue {
        border-color: var(--mbr-cal-overdue, var(--pico-del-color, #c62828));
      }
    `,
  ]
}
