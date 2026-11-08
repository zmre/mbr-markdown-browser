/**
 * The QuickLook day calendar's drawing: a header with the date, a mini month
 * with that day marked, and the day's agenda as a timeline. Static and
 * read-only (design/calendar.md §4.7): no links, no buttons, no fetches.
 *
 * Plain DOM rather than Lit: a preview has one panel and nothing to update
 * but the current-time line, and the bundle is inlined into every preview
 * that carries it. The timeline is the companion's own model
 * (`agenda-layout.ts`'s `timelineModel`) and the grid `month-grid.ts`'s, so
 * the preview and the companion lay a day out identically. Class names are
 * `mbr-ql-cal-*`, styled by `styles.ts`.
 */
import type { AgendaEntry } from '../calendar/agenda-parse.js'
import { blockStyle, nowLineTop, timelineModel } from '../calendar/agenda-layout.js'
import { formatDate, monthGrid, monthOf, resolveWeekStart, weekNumberLabel, weekdayLabels } from '../calendar/month-grid.js'
import type { DateKey } from '../calendar/note-date.js'
import type { DayCalendarPayload } from './model.js'

export interface RenderOptions {
  readonly payload: DayCalendarPayload
  readonly entries: readonly AgendaEntry[]
  /** The reader's date, `YYYY-MM-DD`. */
  readonly today: DateKey
  readonly locale: string
}

/** `tag` with a class and children; strings become text nodes. */
function el<K extends keyof HTMLElementTagNameMap>(
  doc: Document,
  tag: K,
  className: string,
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const node = doc.createElement(tag)
  if (className) node.className = className
  node.append(...children)
  return node
}

function renderHeader(doc: Document, { payload, today, locale }: RenderOptions): HTMLElement {
  const time = el(
    doc,
    'time',
    '',
    el(doc, 'span', 'mbr-ql-cal-weekday', formatDate(payload.date, locale, { weekday: 'long' })),
    el(doc, 'span', 'mbr-ql-cal-date', formatDate(payload.date, locale, { dateStyle: 'long' }))
  )
  time.dateTime = payload.date
  const header = el(doc, 'header', 'mbr-ql-cal-head', time)
  if (payload.date === today) header.append(el(doc, 'span', 'mbr-ql-cal-today-badge', 'Today'))
  return header
}

function renderMonth(doc: Document, { payload, today, locale }: RenderOptions): HTMLElement {
  const month = monthOf(payload.date)
  const weekStart = resolveWeekStart(payload.weekStart, locale)
  const title = formatDate(`${month}-01`, locale, { month: 'long', year: 'numeric' })
  const grid = el(doc, 'div', payload.weekNumbers ? 'mbr-ql-cal-grid with-weeks' : 'mbr-ql-cal-grid')
  grid.setAttribute('role', 'grid')
  grid.setAttribute('aria-label', title)

  const labels = el(doc, 'div', 'mbr-ql-cal-row labels')
  labels.setAttribute('role', 'row')
  if (payload.weekNumbers) labels.append(el(doc, 'span', 'mbr-ql-cal-wk'))
  for (const label of weekdayLabels(weekStart, locale)) {
    const dow = el(doc, 'span', 'mbr-ql-cal-dow', label)
    dow.setAttribute('role', 'columnheader')
    labels.append(dow)
  }
  grid.append(labels)

  for (const week of monthGrid(month, weekStart).weeks) {
    const row = el(doc, 'div', 'mbr-ql-cal-row')
    row.setAttribute('role', 'row')
    if (payload.weekNumbers) {
      const wk = el(doc, 'span', 'mbr-ql-cal-wk', weekNumberLabel(week.isoWeek))
      wk.setAttribute('role', 'rowheader')
      row.append(wk)
    }
    for (const day of week.days) {
      const classes = [
        'mbr-ql-cal-day',
        monthOf(day) === month ? '' : 'outside',
        day === today ? 'today' : '',
        day === payload.date ? 'selected' : '',
      ]
      const cell = el(doc, 'span', classes.filter(Boolean).join(' '), String(Number(day.slice(8))))
      cell.setAttribute('role', 'gridcell')
      cell.dataset.date = day
      if (day === payload.date) cell.setAttribute('aria-selected', 'true')
      if (day === today) cell.setAttribute('aria-current', 'date')
      row.append(cell)
    }
    grid.append(row)
  }
  return el(doc, 'div', 'mbr-ql-cal-month', el(doc, 'div', 'mbr-ql-cal-month-title', title), grid)
}

/** The timeline: hour rules, laid-out blocks, and a screen-reader list. */
function renderAgenda(doc: Document, { entries, locale }: RenderOptions): HTMLElement {
  const model = timelineModel(entries, locale)
  const timeline = el(doc, 'div', 'mbr-ql-cal-timeline')
  timeline.style.height = `${model.height}px`
  timeline.dataset.from = String(model.from)
  timeline.dataset.to = String(model.to)
  for (const h of model.hours) {
    const hour = el(doc, 'div', 'mbr-ql-cal-hour', el(doc, 'span', '', h.label))
    hour.style.top = `${h.top}px`
    hour.setAttribute('aria-hidden', 'true')
    timeline.append(hour)
  }
  for (const b of model.blocks) {
    const e = b.item
    const block = el(doc, 'div', ['mbr-ql-cal-block', b.open ? 'no-end' : '', e.task ? `task ${e.task}` : ''].filter(Boolean).join(' '))
    block.style.cssText = blockStyle(b)
    block.title = b.label
    block.setAttribute('aria-hidden', 'true')
    if (e.task) block.append(el(doc, 'span', 'mbr-ql-cal-check'))
    block.append(el(doc, 'span', 'mbr-ql-cal-span', b.time), ' ', el(doc, 'span', 'mbr-ql-cal-title', e.title))
    for (const detail of e.details) block.append(el(doc, 'span', 'mbr-ql-cal-detail', detail))
    timeline.append(block)
  }
  const list = el(doc, 'ol', 'mbr-ql-cal-sr-only', ...model.spoken.map((line) => el(doc, 'li', '', line)))
  return el(doc, 'section', 'mbr-ql-cal-agenda', el(doc, 'div', 'mbr-ql-cal-scroll', timeline), list)
}

/** The whole calendar, not yet in the document. */
export function renderDayCalendar(doc: Document, options: RenderOptions): HTMLElement {
  const aside = el(doc, 'aside', 'mbr-ql-cal', renderHeader(doc, options), renderMonth(doc, options), renderAgenda(doc, options))
  aside.setAttribute('aria-label', `Day calendar, ${formatDate(options.payload.date, options.locale, { dateStyle: 'full' })}`)
  return aside
}

/**
 * Draws (or moves, or removes) the current-time line: only when `minutes` is
 * given (the note's date is today) and inside the timeline's range.
 */
export function updateNowLine(calendar: HTMLElement, minutes: number | null): void {
  const timeline = calendar.querySelector<HTMLElement>('.mbr-ql-cal-timeline')
  if (!timeline) return
  const top = nowLineTop({ from: Number(timeline.dataset.from), to: Number(timeline.dataset.to) }, minutes)
  let line = timeline.querySelector<HTMLElement>('.mbr-ql-cal-now')
  if (top === null) {
    line?.remove()
    return
  }
  if (!line) {
    line = calendar.ownerDocument.createElement('div')
    line.className = 'mbr-ql-cal-now'
    line.setAttribute('aria-hidden', 'true')
    timeline.append(line)
  }
  line.style.top = `${top}px`
}
