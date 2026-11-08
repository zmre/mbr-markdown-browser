/**
 * The browse widgets' "Calendar" section (design/calendar.md §5.6, D12): one
 * model and one template, shared by `<mbr-browse>` (the three-pane navigator)
 * and `<mbr-browse-single>` (the docked sidebar), which each draw only their
 * own section heading around it.
 *
 * MAIN BUNDLE: both widgets ship on every page. The model is pure; the rows
 * are plain links, and "Calendar…" opens the chunk's overlay through
 * `open-overlay.ts`, so nothing here loads the chunk.
 */
import { css, html, nothing, type TemplateResult } from 'lit'
import { ICON_SM, icon, iconCalendarDays } from '../icons.js'
import { formatDate } from './month-grid.js'
import type { DateKey } from './note-date.js'
import { periodKeysContaining } from './period.js'
import { adjacent, findPeriod, lowerBound, type PeriodKind, type PeriodicIndex } from './periodic.js'

/** One "This …" row. */
export interface CalendarRow {
  readonly kind: PeriodKind
  readonly label: string
  /** The note for the current period, else the latest one (site-absolute). */
  readonly url: string
  /** Set when the current period has no note: "none — latest: Oct 30". */
  readonly latest?: string
}

export interface CalendarSectionModel {
  readonly rows: readonly CalendarRow[]
  /** The last {@link RECENT_DAILY} day notes up to today, newest first. */
  readonly recent: readonly { readonly label: string; readonly url: string }[]
}

const ROW_LABELS: Readonly<Record<PeriodKind, string>> = {
  day: 'Today',
  week: 'This week',
  month: 'This month',
  quarter: 'This quarter',
  year: 'This year',
}

/** Recent daily notes listed. */
export const RECENT_DAILY = 3

/** The last model built per index: the widgets re-render often, and the inputs rarely change. */
const modelCache = new WeakMap<PeriodicIndex, { today: DateKey; locale: string; model: CalendarSectionModel | null }>()

/**
 * The section for `index` on `today`, or `null` when the repository has no
 * periodic notes (the section is then not drawn at all). A row is listed only
 * for a kind the repository has; when the current period has no note it links
 * the latest one before it (else the earliest), saying so. The current
 * periods are Rust's `Period::containing` of today (`periodKeysContaining`).
 * Memoized per (index, today, locale).
 */
export function calendarSectionModel(index: PeriodicIndex, today: DateKey, locale: string): CalendarSectionModel | null {
  const cached = modelCache.get(index)
  if (cached && cached.today === today && cached.locale === locale) return cached.model
  const model = buildModel(index, today, locale)
  modelCache.set(index, { today, locale, model })
  return model
}

function buildModel(index: PeriodicIndex, today: DateKey, locale: string): CalendarSectionModel | null {
  const keys = periodKeysContaining(today)
  if (!keys) return null
  const dayLabel = (date: DateKey, weekday: boolean): string =>
    formatDate(date, locale, weekday ? { weekday: 'short', month: 'short', day: 'numeric' } : { month: 'short', day: 'numeric' })
  const rows: CalendarRow[] = []
  for (const kind of Object.keys(ROW_LABELS) as PeriodKind[]) {
    const entries = index[kind]
    if (entries.length === 0) continue
    const exact = findPeriod(index, keys[kind])
    if (exact) {
      rows.push({ kind, label: ROW_LABELS[kind], url: exact.url })
      continue
    }
    const latest = adjacent(index, keys[kind], -1) ?? entries[0]
    rows.push({
      kind,
      label: ROW_LABELS[kind],
      url: latest.url,
      latest: kind === 'day' ? dayLabel(latest.period, false) : latest.period,
    })
  }
  if (rows.length === 0) return null
  // Day notes are sorted: the last RECENT_DAILY up to and including today.
  const at = lowerBound(index.day, today)
  const end = at + (index.day[at]?.period === today ? 1 : 0)
  const recent = index.day
    .slice(Math.max(0, end - RECENT_DAILY), end)
    .reverse()
    .map((e) => ({ label: dayLabel(e.period, true), url: e.url }))
  return { rows, recent }
}

/** Options for {@link renderCalendarSection}. */
export interface CalendarSectionOptions {
  readonly resolveUrl: (path: string) => string
  /** "Calendar…": opens the overlay over all notes; omitted = no row. */
  readonly onOpenCalendar?: () => void
}

/** The section's body: the period rows, recent daily notes and "Calendar…". */
export function renderCalendarSection(model: CalendarSectionModel, options: CalendarSectionOptions): TemplateResult {
  const row = (r: CalendarRow): TemplateResult => html`
    <li>
      <a href=${options.resolveUrl(r.url)} title=${r.latest ? `No note yet; the latest is ${r.latest}` : r.label}>${r.label}</a>
      ${r.latest ? html`<a class="mbr-cal-latest" href=${options.resolveUrl(r.url)}>(none — latest: ${r.latest})</a>` : nothing}
    </li>
  `
  const recent = model.recent.length
    ? html`
        <li class="mbr-cal-recent">
          <span class="mbr-cal-sub">Recent daily notes</span>
          <ul>
            ${model.recent.map((d) => html`<li><a href=${options.resolveUrl(d.url)}>${d.label}</a></li>`)}
          </ul>
        </li>
      `
    : nothing
  const open = options.onOpenCalendar
    ? html`
        <li>
          <button type="button" class="mbr-cal-open" title="Calendar (c)" @click=${options.onOpenCalendar}>
            ${icon(iconCalendarDays, { size: ICON_SM })}<span>Calendar…</span>
          </button>
        </li>
      `
    : nothing
  return html`<ul class="mbr-cal-rows">${model.rows.map(row)}${recent}${open}</ul>`
}

/**
 * Styles for {@link renderCalendarSection}, for each widget's shadow root.
 * The host lines the rows up with its own by setting, on the section's
 * container: `--mbr-cal-indent` (left inset of a row's text),
 * `--mbr-cal-sub-indent` (extra inset of a recent day), `--mbr-cal-row-y`
 * (a row's vertical padding), `--mbr-cal-color` / `--mbr-cal-hover-color`
 * (link colours). "Calendar…" pulls itself left by its own padding so its
 * icon starts where the row text does.
 */
export const CALENDAR_SECTION_CSS = css`
  .mbr-cal-rows,
  .mbr-cal-rows ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .mbr-cal-rows li {
    margin: 0;
    padding: var(--mbr-cal-row-y, 0.25rem) 0.75rem var(--mbr-cal-row-y, 0.25rem) var(--mbr-cal-indent, 1rem);
    list-style: none;
    font-size: 0.875rem;
  }
  .mbr-cal-rows li.mbr-cal-recent {
    padding-bottom: 0;
  }
  .mbr-cal-rows ul li {
    padding: var(--mbr-cal-row-y, 0.25rem) 0 var(--mbr-cal-row-y, 0.25rem) var(--mbr-cal-sub-indent, 0.75rem);
  }
  .mbr-cal-rows a {
    color: var(--mbr-cal-color, var(--pico-color));
    text-decoration: none;
  }
  .mbr-cal-rows a:hover {
    color: var(--mbr-cal-hover-color, var(--pico-primary));
    text-decoration: underline;
  }
  .mbr-cal-rows a.mbr-cal-latest,
  .mbr-cal-sub {
    font-size: 0.8rem;
    color: var(--pico-muted-color);
  }
  .mbr-cal-sub {
    display: block;
  }
  .mbr-cal-open {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0 0 0 -0.4rem;
    padding: 0.2rem 0.4rem;
    border: 0;
    border-radius: 6px;
    background: none;
    color: var(--pico-primary);
    font: inherit;
    cursor: pointer;
  }
  .mbr-cal-open:hover,
  .mbr-cal-open:focus-visible {
    background: var(--mbr-control-hover-bg, rgba(0, 0, 0, 0.06));
  }
`
