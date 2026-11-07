/**
 * The QuickLook day calendar's drawing: a header with the date, a mini month
 * with that day marked, and the day's agenda as a timeline. Static and
 * read-only (design/calendar.md §4.7): no links, no buttons, no fetches.
 *
 * Plain DOM rather than Lit: a preview has one panel and nothing to update
 * but the current-time line, and the bundle is inlined into every preview
 * that carries it. The geometry and the grid come from Phase 2's pure modules
 * (`agenda-layout.ts`, `month-grid.ts`), so the preview and the companion lay
 * a day out identically. Class names are `mbr-ql-cal-*`, styled by
 * `styles.ts`.
 */
import { formatMinutes, type AgendaEntry } from '../calendar/agenda-parse.js'
import { layoutAgenda, visibleRange, type LaidOut } from '../calendar/agenda-layout.js'
import { formatDate, monthGrid, monthOf, resolveWeekStart, weekdayLabels } from '../calendar/month-grid.js'
import type { DateKey } from '../calendar/note-date.js'
import type { DayCalendarPayload } from './model.js'

/** Pixels per minute on the timeline (40px an hour), as in the companion. */
export const PX_PER_MINUTE = 40 / 60
/** Shortest drawn block, so a 5-minute entry still has a readable line. */
const MIN_BLOCK_PX = 18

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

/** A clock time in the reader's locale (`9:00 AM`, `09:00`). */
export function formatClock(minutes: number, locale: string): string {
  const d = new Date(Date.UTC(1970, 0, 1, Math.floor(minutes / 60), minutes % 60))
  try {
    return new Intl.DateTimeFormat(locale, { hour: 'numeric', minute: '2-digit', timeZone: 'UTC' }).format(d)
  } catch {
    return formatMinutes(minutes)
  }
}

/** An entry's time or time range, localized. */
export function formatSpan(entry: Pick<AgendaEntry, 'start' | 'end'>, locale: string): string {
  const start = formatClock(entry.start, locale)
  return entry.end === undefined ? start : `${start}–${formatClock(entry.end, locale)}`
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
      const wk = el(doc, 'span', 'mbr-ql-cal-wk', String(Number(week.isoWeek.slice(6))))
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

function renderBlock(doc: Document, b: LaidOut<AgendaEntry>, from: number, locale: string): HTMLElement {
  const e = b.item
  const y = (m: number): number => (m - from) * PX_PER_MINUTE
  const block = el(doc, 'div', ['mbr-ql-cal-block', b.open ? 'no-end' : '', e.task ? `task ${e.task}` : ''].filter(Boolean).join(' '))
  block.style.top = `${y(b.start)}px`
  block.style.height = `${Math.max(y(b.end) - y(b.start), MIN_BLOCK_PX)}px`
  block.style.left = `calc(var(--mbr-ql-cal-gutter) + (100% - var(--mbr-ql-cal-gutter)) * ${b.column} / ${b.columns})`
  block.style.width = `calc((100% - var(--mbr-ql-cal-gutter)) / ${b.columns} - 2px)`
  block.title = [formatSpan(e, locale), e.title, ...e.details].join(' · ')
  block.setAttribute('aria-hidden', 'true')
  if (e.task) block.append(el(doc, 'span', 'mbr-ql-cal-check'))
  block.append(el(doc, 'span', 'mbr-ql-cal-span', formatClock(e.start, locale)), ' ', el(doc, 'span', 'mbr-ql-cal-title', e.title))
  for (const detail of e.details) block.append(el(doc, 'span', 'mbr-ql-cal-detail', detail))
  return block
}

/** The timeline: hour rules, laid-out blocks, and a screen-reader list. */
function renderAgenda(doc: Document, { entries, locale }: RenderOptions): HTMLElement {
  const { from, to } = visibleRange(entries)
  const y = (m: number): number => (m - from) * PX_PER_MINUTE
  const timeline = el(doc, 'div', 'mbr-ql-cal-timeline')
  timeline.style.height = `${y(to)}px`
  timeline.dataset.from = String(from)
  timeline.dataset.to = String(to)
  for (let h = from; h <= to; h += 60) {
    const label = h < 1440 ? formatClock(h, locale).replace(/:00(?=\D|$)/, '') : ''
    const hour = el(doc, 'div', 'mbr-ql-cal-hour', el(doc, 'span', '', label))
    hour.style.top = `${y(h)}px`
    hour.setAttribute('aria-hidden', 'true')
    timeline.append(hour)
  }
  for (const b of layoutAgenda(entries)) timeline.append(renderBlock(doc, b, from, locale))

  const list = el(doc, 'ol', 'mbr-ql-cal-sr-only')
  for (const e of entries) {
    const details = e.details.length ? `: ${e.details.join('; ')}` : ''
    list.append(el(doc, 'li', '', `${formatSpan(e, locale)} ${e.title}${e.task ? ` (${e.task})` : ''}${details}`))
  }
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
  const from = Number(timeline.dataset.from)
  const to = Number(timeline.dataset.to)
  let line = timeline.querySelector<HTMLElement>('.mbr-ql-cal-now')
  if (minutes === null || minutes < from || minutes > to) {
    line?.remove()
    return
  }
  if (!line) {
    line = calendar.ownerDocument.createElement('div')
    line.className = 'mbr-ql-cal-now'
    line.setAttribute('aria-hidden', 'true')
    timeline.append(line)
  }
  line.style.top = `${(minutes - from) * PX_PER_MINUTE}px`
}
