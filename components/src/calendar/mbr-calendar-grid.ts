/**
 * The large calendar grids (calendar chunk): `<mbr-calendar-month-grid>` (six
 * week rows, entries per day cell with a `+N` overflow) and
 * `<mbr-calendar-week-grid>` (seven tall columns, every entry).
 *
 * Written for two hosts: the calendar overlay (notes, design/calendar.md §5.2)
 * and the task panel's grids (tasks, §8.2), which create them by tag name
 * after `loadCalendarChunk()`. They know nothing about notes or tasks: a host
 * hands over `dayEntries(date, limit)` ({@link GridEntry} links, already
 * filtered; a host may stop at `limit`, and then says how many it has with
 * `dayCount(date)`), `dayMarks(date)` (a `dailyUrl` shades the cell and links
 * its number) and the focused day, and owns the keyboard (`month-grid.ts`'s
 * `overlayCommand` / `moveDay` are the shared key map). A grid only ever
 * draws what fits — a month cell its lines, a week column
 * {@link WEEK_COLUMN_CAP} — and a `+N` for the rest, so a day holding
 * thousands of notes costs a dozen elements. Events, all bubbling and
 * composed:
 *
 * - `mbr-calendar-focus` `{date}` — a day was clicked;
 * - `mbr-calendar-day` `{date, anchor}` — a month cell or a `+N` was
 *   clicked: the host opens its day popover at `anchor`.
 *
 * Accessibility (§9): `role="grid"`, rows, a roving tabindex on the day
 * cells, and each cell's `aria-label` spells the date and what it holds.
 */
import { LitElement, css, html, nothing, type CSSResultGroup, type TemplateResult } from 'lit'
import { property, state } from 'lit/decorators.js'
import { dayLabel, emitCalendarEvent, renderWeekNumber } from './mbr-calendar-month.js'
import { formatDate, monthGrid, monthOf, weekdayLabels, weekdayOf, type MonthKey, type Weekday } from './month-grid.js'
import { addDays, type DateKey } from './note-date.js'
import { cellLines, overflowSplit } from './overlay-model.js'
import type { DayMarks, GridEntry } from './types.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-calendar-month-grid': MbrCalendarMonthGridElement
    'mbr-calendar-week-grid': MbrCalendarWeekGridElement
  }
}

/** Fixed heights (px) the month cell is laid out in: its number row, one entry line. */
const HEAD_PX = 22
const LINE_PX = 19
/** The cell's own vertical border and padding, plus a pixel of rounding. */
const CELL_CHROME_PX = 5

const NONE: readonly GridEntry[] = Object.freeze([])

/** Most entries a week column draws before its `+N`. */
export const WEEK_COLUMN_CAP = 60

/** What both grids share: inputs, the entry line, events, focus. */
abstract class GridBase extends LitElement {
  /** The focused day (roving tabindex). */
  @property() focused: DateKey = ''
  /** The reader's local date. */
  @property() today: DateKey = ''
  /** BCP 47 locale for day and month names. */
  @property() locale = 'en-US'
  /** A day's entries, in display order; the grid draws at most `limit` of them. */
  @property({ attribute: false }) dayEntries: (date: DateKey, limit: number) => readonly GridEntry[] = () => NONE
  /**
   * How many entries a day has, for a host whose `dayEntries` stops at
   * `limit`; omitted = the length of what `dayEntries` returned.
   */
  @property({ attribute: false }) dayCount: ((date: DateKey) => number) | null = null
  /**
   * A day's marks: `dailyUrl` shades the cell and `overdue` reddens its
   * number (the task grids). A host whose entries are not notes passes
   * `notes` itself (`0`), so the spoken label does not call tasks "notes".
   */
  @property({ attribute: false }) dayMarks: (date: DateKey) => DayMarks = () => ({})

  /** The cell of `date`, if drawn (an anchor for the host's day popover). */
  public cellFor(date: DateKey): HTMLElement | null {
    return this.renderRoot.querySelector<HTMLElement>(`[data-date="${date}"]`)
  }

  /** Moves keyboard focus onto the focused day. */
  public async focusDay(): Promise<void> {
    await this.updateComplete
    this.cellFor(this.focused)?.focus()
  }

  protected emit(type: string, detail: Record<string, unknown>): void {
    emitCalendarEvent(this, type, detail)
  }

  /**
   * What a day cell draws when it has room for `lines` entries: the entries,
   * the `+N` after them, and the cell's spoken label.
   */
  protected cellContent(date: DateKey, lines: number): { entries: readonly GridEntry[]; more: number; marks: DayMarks; label: string } {
    const all = this.dayEntries(date, lines)
    const marks = this.dayMarks(date)
    const count = this.dayCount ? this.dayCount(date) : all.length
    const { shown, more } = overflowSplit(Math.min(all.length, count), lines, count + (marks.hidden ?? 0))
    return {
      entries: all.length > shown ? all.slice(0, shown) : all,
      more,
      marks,
      label: dayLabel(date, this.locale, { notes: count, ...marks }, this.today),
    }
  }

  /** A day's `+N`: a click opens the host's day popover on the cell. */
  protected renderMore(date: DateKey, more: number): TemplateResult | typeof nothing {
    if (!more) return nothing
    const open = (e: Event): void => {
      e.stopPropagation()
      const cell = (e.currentTarget as Element).closest<HTMLElement>('[data-date]')
      this.emit('mbr-calendar-focus', { date })
      if (cell) this.emit('mbr-calendar-day', { date, anchor: cell })
    }
    return html`<button type="button" class="more" tabindex="-1" aria-label=${`${more} more`} @click=${open}>+${more}</button>`
  }

  protected cellClass(date: DateKey, marks: DayMarks, extra = ''): string {
    return `cell${extra}${marks.dailyUrl ? ' daily' : ''}${marks.overdue ? ' overdue' : ''}${date === this.today ? ' today' : ''}${date === this.focused ? ' focus' : ''}`
  }

  protected renderNumber(marks: DayMarks, text: string): TemplateResult {
    return marks.dailyUrl
      ? html`<a class="num" href=${marks.dailyUrl} tabindex="-1" title="Open the daily note">${text}</a>`
      : html`<span class="num">${text}</span>`
  }

  protected renderEntry(e: GridEntry): TemplateResult {
    const dot =
      e.status !== undefined
        ? html`<i class=${`check ${e.status}`}></i>`
        : html`<i class="dot" style=${e.color === null || e.color === undefined ? '' : `background:var(--mbr-cal-type-${e.color})`}></i>`
    const body = html`${dot}${e.time ? html`<span class="time">${e.time}</span>` : nothing}<span class="t">${e.title}</span>`
    return html`<a class=${`entry ${e.status ?? ''}`} href=${e.href} tabindex="-1" title=${e.tooltip ?? e.title}>${body}</a>`
  }

  static override styles: CSSResultGroup = css`
    :host {
      display: block;
      min-height: 0;
      font-size: 0.8rem;
      color: var(--pico-color);
    }
    .labels {
      display: grid;
      grid-template-columns: repeat(7, minmax(0, 1fr));
    }
    .dow {
      padding: 0 0.3rem 0.2rem;
      font-size: 0.72rem;
      color: var(--pico-muted-color);
    }
    .cell {
      box-sizing: border-box;
      min-width: 0;
      overflow: hidden;
      border: 1px solid var(--pico-muted-border-color, #e0e0e0);
      margin: 0 -1px -1px 0;
      padding: 1px 2px;
      cursor: pointer;
      outline: none;
    }
    .cell.outside {
      background: color-mix(in srgb, var(--pico-muted-border-color, #e0e0e0) 25%, transparent);
    }
    .cell.outside .num {
      opacity: 0.6;
    }
    .cell.daily {
      background: var(--mbr-cal-daily-bg, rgba(1, 114, 173, 0.12));
    }
    .cell:focus-visible,
    .cell.focus:focus {
      box-shadow: inset 0 0 0 2px var(--pico-primary, #0172ad);
    }
    .num {
      display: inline-block;
      min-width: 1.5em;
      height: 20px;
      line-height: 20px;
      padding: 0 0.25em;
      border-radius: 10px;
      text-align: center;
      font-variant-numeric: tabular-nums;
      color: var(--pico-color);
      text-decoration: none;
    }
    a.num {
      font-weight: 700;
    }
    .overdue .num {
      color: var(--mbr-cal-overdue, #c62828);
      font-weight: 700;
    }
    a.num:hover {
      color: var(--pico-primary);
      text-decoration: underline;
    }
    .today .num {
      background: var(--mbr-cal-today, var(--pico-primary));
      color: var(--pico-primary-inverse, #fff);
    }
    .entry {
      display: flex;
      align-items: center;
      gap: 0.3em;
      width: 100%;
      box-sizing: border-box;
      height: ${LINE_PX}px;
      margin: 0;
      padding: 0 0.2em;
      border: 0;
      border-radius: 4px;
      background: none;
      color: var(--pico-color);
      font: inherit;
      line-height: ${LINE_PX}px;
      text-align: left;
      text-decoration: none;
      cursor: pointer;
    }
    .entry:hover {
      background: var(--mbr-control-hover-bg, rgba(0, 0, 0, 0.06));
    }
    .t {
      min-width: 0;
      overflow: hidden;
      white-space: nowrap;
      text-overflow: ellipsis;
    }
    .time {
      flex: none;
      color: var(--pico-muted-color);
      font-variant-numeric: tabular-nums;
    }
    .dot {
      flex: none;
      width: 7px;
      height: 7px;
      border-radius: 50%;
      background: var(--pico-muted-color, #888);
    }
    .check {
      flex: none;
      width: 9px;
      height: 9px;
      border: 1.5px solid currentColor;
      border-radius: 2px;
    }
    .check.done {
      background: currentColor;
    }
    .entry.done .t,
    .entry.canceled .t {
      text-decoration: line-through;
      color: var(--pico-muted-color);
    }
    .more {
      height: ${LINE_PX}px;
      margin: 0;
      padding: 0 0.3em;
      border: 0;
      border-radius: 4px;
      background: none;
      color: var(--pico-primary);
      font: inherit;
      font-weight: 600;
      cursor: pointer;
    }
    .more:hover {
      background: var(--mbr-control-hover-bg, rgba(0, 0, 0, 0.06));
    }
  `
}

/**
 * The overlay's month view: six week rows, an optional ISO week-number column,
 * and as many entry lines per cell as its height allows (at least two,
 * measured once per resize), the last replaced by `+N` when they do not fit.
 */
export class MbrCalendarMonthGridElement extends GridBase {
  /** The month shown, `YYYY-MM`. */
  @property() month: MonthKey = ''
  /** First column, 0 = Sunday. */
  @property({ type: Number }) weekStart: Weekday = 1
  /** Show the ISO week-number column. */
  @property({ type: Boolean }) weekNumbers = false
  /** The week note for an ISO week key (`2026-W44`), as an href, or `null`. */
  @property({ attribute: false }) weekHref: (isoWeek: string) => string | null = () => null

  /** Entry lines per cell. */
  @state() private _lines = 2
  private _observer: ResizeObserver | null = null

  override connectedCallback(): void {
    super.connectedCallback()
    if (typeof ResizeObserver === 'function') {
      this._observer = new ResizeObserver((records) => {
        const height = records[0]?.contentRect.height ?? 0
        const lines = cellLines(height / 6 - CELL_CHROME_PX, HEAD_PX, LINE_PX)
        if (lines !== this._lines) this._lines = lines
      })
      void this.updateComplete.then(() => {
        const body = this.renderRoot.querySelector('.body')
        if (body) this._observer?.observe(body)
      })
    }
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    this._observer?.disconnect()
    this._observer = null
  }

  private _cell(date: DateKey): TemplateResult {
    const { entries, more, marks, label } = this.cellContent(date, this._lines)
    const onClick = (e: Event): void => {
      if ((e.target as Element).closest('a')) return
      this.emit('mbr-calendar-focus', { date })
      this.emit('mbr-calendar-day', { date, anchor: e.currentTarget as HTMLElement })
    }
    return html`
      <div
        role="gridcell"
        class=${this.cellClass(date, marks, monthOf(date) === this.month ? '' : ' outside')}
        data-date=${date}
        tabindex=${date === this.focused ? 0 : -1}
        aria-label=${label}
        @click=${onClick}
      >
        ${this.renderNumber(marks, String(Number(date.slice(8))))}${entries.map((e) => this.renderEntry(e))}${this.renderMore(date, more)}
      </div>
    `
  }

  override render(): TemplateResult | typeof nothing {
    if (!this.month) return nothing
    const grid = monthGrid(this.month, this.weekStart)
    const wk = this.weekNumbers ? ' with-weeks' : ''
    return html`
      <div role="grid" class=${`grid${wk}`} aria-label=${formatDate(`${this.month}-01`, this.locale, { month: 'long', year: 'numeric' })}>
        <div role="row" class="labels">
          ${wk ? html`<span></span>` : nothing}
          ${weekdayLabels(this.weekStart, this.locale, 'short').map((l) => html`<span role="columnheader" class="dow">${l}</span>`)}
        </div>
        <div class="body">
          ${grid.weeks.map(
            (w) => html`
              <div role="row" class="week">
                ${wk ? renderWeekNumber(w.isoWeek, this.weekHref(w.isoWeek)) : nothing}${w.days.map((d) => this._cell(d))}
              </div>
            `
          )}
        </div>
      </div>
    `
  }

  static override styles = [
    GridBase.styles,
    css`
      :host {
        height: 100%;
      }
      .grid {
        display: flex;
        flex-direction: column;
        height: 100%;
      }
      .body {
        flex: 1;
        min-height: 0;
        display: grid;
        grid-template-rows: repeat(6, minmax(${HEAD_PX + 2 * LINE_PX + 4}px, 1fr));
      }
      .week {
        display: grid;
        grid-template-columns: repeat(7, minmax(0, 1fr));
        min-height: 0;
      }
      .with-weeks .labels,
      .with-weeks .week {
        grid-template-columns: 1.8rem repeat(7, minmax(0, 1fr));
      }
      .wk {
        padding-top: 3px;
        text-align: center;
        font-size: 0.7rem;
        color: var(--pico-muted-color);
        opacity: 0.6;
        text-decoration: none;
      }
      .wk.has-note {
        font-weight: 700;
        opacity: 1;
        color: var(--pico-color);
      }
      .wk.has-note:hover {
        color: var(--pico-primary);
      }
    `,
  ]
}

/** The overlay's week view: seven tall columns, each day's full list in order. */
export class MbrCalendarWeekGridElement extends GridBase {
  /** The first day of the week shown. */
  @property() start: DateKey = ''

  override render(): TemplateResult | typeof nothing {
    if (!this.start) return nothing
    const days = Array.from({ length: 7 }, (_, i) => addDays(this.start, i))
    const labels = weekdayLabels(weekdayOf(this.start), this.locale, 'short')
    const column = (date: DateKey, i: number): TemplateResult => {
      const { entries, more, marks, label } = this.cellContent(date, WEEK_COLUMN_CAP)
      const onClick = (e: Event): void => {
        if (!(e.target as Element).closest('a')) this.emit('mbr-calendar-focus', { date })
      }
      return html`
        <div
          role="gridcell"
          class=${this.cellClass(date, marks)}
          data-date=${date}
          tabindex=${date === this.focused ? 0 : -1}
          aria-label=${label}
          @click=${onClick}
        >
          <div class="dh"><span class="dow">${labels[i]}</span>${this.renderNumber(marks, String(Number(date.slice(8))))}</div>
          <div class="list">${entries.map((e) => this.renderEntry(e))}${this.renderMore(date, more)}</div>
        </div>
      `
    }
    return html`
      <div role="grid" class="grid" aria-label=${`Week of ${formatDate(this.start, this.locale, { dateStyle: 'long' })}`}>
        <div role="row" class="cols">${days.map(column)}</div>
      </div>
    `
  }

  static override styles = [
    GridBase.styles,
    css`
      :host {
        height: 100%;
      }
      .grid,
      .cols {
        height: 100%;
      }
      .cols {
        display: grid;
        grid-template-columns: repeat(7, minmax(0, 1fr));
      }
      .cell {
        display: flex;
        flex-direction: column;
      }
      .dh {
        display: flex;
        align-items: baseline;
        gap: 0.3rem;
        padding: 0.2rem 0.2rem 0.3rem;
      }
      .dh .dow {
        padding: 0;
      }
      .list {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
      }
      @media (max-width: 600px) {
        .cols {
          grid-template-columns: 1fr;
          grid-auto-rows: minmax(4.5rem, auto);
          height: auto;
        }
        .grid {
          height: auto;
        }
      }
    `,
  ]
}

customElements.define('mbr-calendar-month-grid', MbrCalendarMonthGridElement)
customElements.define('mbr-calendar-week-grid', MbrCalendarWeekGridElement)
