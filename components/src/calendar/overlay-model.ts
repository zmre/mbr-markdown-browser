/**
 * The calendar overlay's model (design/calendar.md §5): bucketing notes by
 * day, the filter, "next month with matches", the jump-picker parser, the
 * month-cell overflow arithmetic, the type-colour hash and the key map.
 *
 * Pure, with type-only imports plus the other pure calendar modules: the
 * overlay element is a thin view over this, and the phase 6 task grids can
 * reuse the key map and the overflow arithmetic.
 *
 * **Performance (§5.4).** `bucketNotes` is the one O(notes) pass, run once
 * per (scope, plot-by): every entry's title, folder, type, tags and lowercase
 * haystack are computed there, and the days come out as a map plus a sorted
 * key array. After that a view touches at most 42 buckets, and a filter only
 * re-tests the entries of the buckets it draws (`nextMatch` walks the sorted
 * keys forward from the visible range, testing bucket by bucket).
 */
import type { MarkdownFile } from '../sorting.js'
import { noteTitle } from './day-notes.js'
import { addMonthsToDate, monthOf, periodStart, weekdayOf, type Weekday } from './month-grid.js'
import { addDays, noteDate, parseDateKey, type DateKey } from './note-date.js'
import { periodKindOf, type PeriodKind } from './periodic.js'
import type { PlotBy } from './types.js'

/** One note placed on one day. */
export interface OverlayEntry {
  readonly file: MarkdownFile
  readonly title: string
  /** The repository folder, `''` at the root. */
  readonly folder: string
  readonly date: DateKey
  /** Minutes after midnight when the plotted date carries a time, else `null`. */
  readonly minutes: number | null
  /** `type`, trimmed, as authored; `null` when the note has none. */
  readonly type: string | null
  /** Type-colour slot (`--mbr-cal-type-N`), `null` for an untyped note. */
  readonly color: number | null
  /**
   * The day note of `date` (plot-by note date only): drawn as the cell's
   * shading and number link rather than as a line. At most one per day.
   */
  readonly daily: boolean
  /** Lowercase `type`, for `type:x`. */
  readonly typeKey: string
  /** Lowercase tags, without a leading `#`. */
  readonly tags: readonly string[]
  /** Lowercase title, path, type and tags, for plain filter terms. */
  readonly hay: string
}

/** Notes by day, plus the days in order. */
export interface Buckets {
  /** Each day's entries: the day note first, then untimed by title, then timed by time. */
  readonly days: ReadonlyMap<DateKey, readonly OverlayEntry[]>
  /** Every key of `days`, ascending. */
  readonly keys: readonly DateKey[]
}

/** Plot-by choices in `p`'s cycle order. */
export const PLOT_BYS: readonly PlotBy[] = ['date', 'modified', 'created']

export const PLOT_BY_LABELS: Readonly<Record<PlotBy, string>> = {
  date: 'Note date',
  modified: 'Modified',
  created: 'Created',
}

/** The plot-by after `current` (`p`). */
export function nextPlotBy(current: PlotBy): PlotBy {
  return PLOT_BYS[(PLOT_BYS.indexOf(current) + 1) % PLOT_BYS.length]
}

export type CalendarView = 'month' | 'week' | 'list'

/** List on a phone (≤600px), where month cells are too small to read; Month otherwise. */
export function defaultView(width: number): CalendarView {
  return width <= 600 ? 'list' : 'month'
}

/** Number of `--mbr-cal-type-N` colour slots. */
export const TYPE_COLORS = 8

/**
 * The colour slot of a note `type`: FNV-1a (32-bit) over the UTF-8 bytes of
 * the trimmed, lowercased type, mod 8 — `chat.rs`'s speaker-colour idea.
 * Pinned by a test: changing it recolours everyone's calendar.
 */
export function typeColor(type: string): number {
  let hash = 0x811c9dc5
  for (const byte of new TextEncoder().encode(type.trim().toLowerCase())) {
    hash = Math.imul(hash ^ byte, 0x01000193) >>> 0
  }
  return hash % TYPE_COLORS
}

/** Options for {@link bucketNotes}. */
export interface BucketOptions {
  /** Frontmatter fields holding tags (the configured tag sources). */
  readonly tagFields: readonly string[]
  /** `[calendar] date_fields`: where a frontmatter date (and its time) came from. */
  readonly dateFields: readonly string[]
}

const FRONTMATTER_TIME = /^(\d{4}-\d{2}-\d{2})[T ](\d{2}):(\d{2})/

/** The time of day a frontmatter date field gives `date`, if one does. */
function frontmatterMinutes(file: MarkdownFile, date: DateKey, fields: readonly string[]): number | null {
  for (const field of fields) {
    const value = file.frontmatter?.[field]
    const m = typeof value === 'string' ? FRONTMATTER_TIME.exec(value) : null
    if (m && m[1] === date) return Number(m[2]) * 60 + Number(m[3])
  }
  return null
}

/** The reader's local date and time of a unix timestamp in seconds. */
function localStamp(seconds: number | undefined): { date: DateKey; minutes: number } | null {
  if (typeof seconds !== 'number' || !Number.isFinite(seconds)) return null
  const d = new Date(seconds * 1000)
  if (Number.isNaN(d.getTime())) return null
  const pad = (n: number): string => String(n).padStart(2, '0')
  return {
    date: `${String(d.getFullYear()).padStart(4, '0')}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`,
    minutes: d.getHours() * 60 + d.getMinutes(),
  }
}

/** Where a note lands for `plotBy`, or `null` when it is not plotted. */
function placement(
  file: MarkdownFile,
  plotBy: PlotBy,
  dateFields: readonly string[]
): { date: DateKey; minutes: number | null } | null {
  if (plotBy !== 'date') return localStamp(plotBy === 'modified' ? file.modified : file.created)
  // A week, month, quarter or year note is reached through the week-number
  // and header links: its date is only its period's first day.
  const period = file.calendar?.period
  if (period && periodKindOf(period) !== 'day') return null
  if (!file.calendar) return localStamp(file.created)
  const date = noteDate(file)
  if (!date) return null
  return { date, minutes: file.calendar.source === 'frontmatter' ? frontmatterMinutes(file, date, dateFields) : null }
}

function tagsOf(file: MarkdownFile, fields: readonly string[]): string[] {
  const out: string[] = []
  for (const field of fields) {
    const value = file.frontmatter?.[field]
    if (value === null || value === undefined) continue
    const parts: unknown[] = Array.isArray(value) ? value : String(value).split(',')
    for (const part of parts) {
      const tag = String(part).trim().replace(/^#/, '').toLowerCase()
      if (tag && !out.includes(tag)) out.push(tag)
    }
  }
  return out
}

const collator = new Intl.Collator(undefined, { sensitivity: 'base', numeric: true })

type MutableEntry = { -readonly [K in keyof OverlayEntry]: OverlayEntry[K] } & { candidate: boolean }

/** Day note first (lowest `url_path`), then untimed by title, then timed by time, then title. */
function compareEntries(a: MutableEntry, b: MutableEntry): number {
  if (a.candidate !== b.candidate) return a.candidate ? -1 : 1
  if (a.candidate) return a.file.url_path < b.file.url_path ? -1 : 1
  if ((a.minutes === null) !== (b.minutes === null)) return a.minutes === null ? -1 : 1
  return (a.minutes ?? 0) - (b.minutes ?? 0) || collator.compare(a.title, b.title)
}

/**
 * Buckets `files` by the date `plotBy` names, in one pass (§5.4).
 *
 * - **Note date:** `noteDate` (filename, frontmatter, else the reader's local
 *   date of birthtime). Week, month, quarter and year notes are left out;
 *   a day note is flagged `daily` on its own day. A frontmatter date written
 *   with a time (`2026-10-31T14:00`) sorts by it, as does a birthtime.
 * - **Modified / Created:** the reader's local date and time of the file's
 *   own timestamps; every note is a plain entry there, periodic ones too.
 */
export function bucketNotes(files: readonly MarkdownFile[], plotBy: PlotBy, options: BucketOptions): Buckets {
  const days = new Map<DateKey, MutableEntry[]>()
  const colors = new Map<string, number>()
  for (const file of files) {
    const at = placement(file, plotBy, options.dateFields)
    if (!at) continue
    const rawType = file.frontmatter?.type
    const type = typeof rawType === 'string' && rawType.trim() ? rawType.trim() : null
    const typeKey = type?.toLowerCase() ?? ''
    let color: number | null = null
    if (type) {
      color = colors.get(typeKey) ?? typeColor(typeKey)
      colors.set(typeKey, color)
    }
    const title = noteTitle(file)
    const slash = file.raw_path.lastIndexOf('/')
    const tags = tagsOf(file, options.tagFields)
    const entry: MutableEntry = {
      file,
      title,
      folder: slash < 0 ? '' : file.raw_path.slice(0, slash),
      date: at.date,
      minutes: at.minutes,
      type,
      color,
      daily: false,
      candidate: plotBy === 'date' && file.calendar?.period === at.date,
      typeKey,
      tags,
      hay: `${title}\n${file.raw_path}\n${typeKey}\n${tags.join('\n')}`.toLowerCase(),
    }
    const bucket = days.get(at.date)
    if (bucket) bucket.push(entry)
    else days.set(at.date, [entry])
  }
  for (const bucket of days.values()) {
    bucket.sort(compareEntries)
    if (bucket[0].candidate) bucket[0].daily = true
  }
  return { days, keys: [...days.keys()].sort() }
}

/** One filter token. */
export interface FilterTerm {
  readonly kind: 'text' | 'tag' | 'type'
  readonly value: string
}

/**
 * The filter field's tokens (§5.3): whitespace-separated and AND'd; `#tag`
 * matches tags by prefix, `type:x` the type exactly, anything else is a
 * substring of the title, path, type or a tag. Case-insensitive.
 */
export function parseFilter(text: string): FilterTerm[] {
  return text
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .map((token): FilterTerm => {
      if (token.length > 1 && token.startsWith('#')) return { kind: 'tag', value: token.slice(1) }
      if (token.length > 5 && token.startsWith('type:')) return { kind: 'type', value: token.slice(5) }
      return { kind: 'text', value: token }
    })
}

/** Whether `entry` passes every term. */
export function matchesFilter(entry: OverlayEntry, terms: readonly FilterTerm[]): boolean {
  return terms.every((t) =>
    t.kind === 'tag'
      ? entry.tags.some((tag) => tag.startsWith(t.value))
      : t.kind === 'type'
        ? entry.typeKey === t.value
        : entry.hay.includes(t.value)
  )
}

/** Index of the first key `> date` (binary search). */
function firstAfter(keys: readonly DateKey[], date: DateKey): number {
  let lo = 0
  let hi = keys.length
  while (lo < hi) {
    const mid = (lo + hi) >>> 1
    if (keys[mid] <= date) lo = mid + 1
    else hi = mid
  }
  return lo
}

/**
 * The first day after `after` holding an entry that passes `terms` ("Next
 * month with matches ›"), or `null`. Walks the sorted keys, testing one bucket
 * at a time, so it stops at the first match.
 */
export function nextMatch(buckets: Buckets, after: DateKey, terms: readonly FilterTerm[]): DateKey | null {
  for (let i = firstAfter(buckets.keys, after); i < buckets.keys.length; i++) {
    const key = buckets.keys[i]
    if (buckets.days.get(key)!.some((e) => matchesFilter(e, terms))) return key
  }
  return null
}

/**
 * The last day before `before` holding an entry that passes `terms` ("‹
 * Previous month with matches"), or `null`. The mirror of {@link nextMatch}.
 */
export function prevMatch(buckets: Buckets, before: DateKey, terms: readonly FilterTerm[]): DateKey | null {
  let lo = 0
  let hi = buckets.keys.length
  while (lo < hi) {
    const mid = (lo + hi) >>> 1
    if (buckets.keys[mid] < before) lo = mid + 1
    else hi = mid
  }
  for (let i = lo - 1; i >= 0; i--) {
    const key = buckets.keys[i]
    if (buckets.days.get(key)!.some((e) => matchesFilter(e, terms))) return key
  }
  return null
}

/** The first and last day a view covers for `focus`. */
export function viewRange(view: CalendarView, focus: DateKey, weekStart: Weekday): { from: DateKey; to: DateKey } {
  if (view === 'week') {
    const from = addDays(focus, -((weekdayOf(focus) - weekStart + 7) % 7))
    return { from, to: addDays(from, 6) }
  }
  const from = `${monthOf(focus)}-01`
  return { from, to: addDays(addMonthsToDate(from, 1), -1) }
}

/** Every day from `from` to `to`, inclusive. */
export function daysBetween(from: DateKey, to: DateKey): DateKey[] {
  const out: DateKey[] = []
  for (let d = from; d <= to; d = addDays(d, 1)) out.push(d)
  return out
}

/**
 * The jump picker's input: any canonical period key (`2026-10-31`,
 * `2026-W44`, `2026-10`, `2026-Q4`, `2026`; `w`/`q` either case), as the day
 * to focus (its first day) and the kind it named. `null` for anything else.
 */
export function parseJump(input: string): { date: DateKey; kind: PeriodKind } | null {
  const key = input.trim().toUpperCase()
  const date = periodStart(key)
  const kind = periodKindOf(key)
  return date && kind && parseDateKey(date) ? { date, kind } : null
}

/** Entry lines that fit a month cell: at least two (§5.2). */
export function cellLines(cellHeight: number, headHeight: number, lineHeight: number): number {
  const lines = Math.floor((cellHeight - headHeight) / lineHeight)
  return Number.isFinite(lines) ? Math.max(2, lines) : 2
}

/**
 * How many of `count` entries a cell of `lines` lines draws, and the `+N`
 * that replaces its last line when they do not all fit.
 */
export function overflowSplit(count: number, lines: number): { shown: number; more: number } {
  const room = Math.max(2, lines)
  return count <= room ? { shown: count, more: 0 } : { shown: room - 1, more: count - (room - 1) }
}

/** What a key does inside the overlay (§5.5). */
export type OverlayCommand =
  | 'left'
  | 'right'
  | 'up'
  | 'down'
  | 'prev'
  | 'next'
  | 'today'
  | 'month'
  | 'week'
  | 'list'
  | 'jump'
  | 'plot'
  | 'filter'
  | 'open'
  | 'escape'
  | 'help'

const KEY_COMMANDS: ReadonlyMap<string, OverlayCommand> = new Map(
  Object.entries({
  ArrowLeft: 'left',
  h: 'left',
  ArrowRight: 'right',
  l: 'right',
  ArrowUp: 'up',
  k: 'up',
  ArrowDown: 'down',
  j: 'down',
  '[': 'prev',
  PageUp: 'prev',
  ']': 'next',
  PageDown: 'next',
  t: 'today',
  m: 'month',
  w: 'week',
  a: 'list',
  g: 'jump',
  p: 'plot',
  '/': 'filter',
  Enter: 'open',
  Escape: 'escape',
  '?': 'help',
  }) as [string, OverlayCommand][]
)

/** The overlay command for an unmodified key, or `null`. */
export function overlayCommand(key: string): OverlayCommand | null {
  return KEY_COMMANDS.get(key) ?? null
}

/**
 * Where a movement command takes the focused day in Month and Week (List
 * moves between entries instead). `[`/`]` page by a month, or a week in Week.
 */
export function moveDay(command: OverlayCommand, view: CalendarView, focus: DateKey): DateKey | null {
  switch (command) {
    case 'left':
      return addDays(focus, -1)
    case 'right':
      return addDays(focus, 1)
    case 'up':
      return addDays(focus, -7)
    case 'down':
      return addDays(focus, 7)
    case 'prev':
      return view === 'week' ? addDays(focus, -7) : addMonthsToDate(focus, -1)
    case 'next':
      return view === 'week' ? addDays(focus, 7) : addMonthsToDate(focus, 1)
    default:
      return null
  }
}

/** The `?` help list. */
export const OVERLAY_SHORTCUTS: readonly { readonly keys: string; readonly description: string }[] = [
  { keys: '← → ↑ ↓ / h l k j', description: 'Move between days (entries in List)' },
  { keys: '[ ] / PageUp PageDown', description: 'Previous / next month (week in Week)' },
  { keys: 't', description: 'Today' },
  { keys: 'm / w / a', description: 'Month / Week / List view' },
  { keys: 'g', description: 'Jump to a month or date' },
  { keys: 'p', description: 'Plot by note date, modified, created' },
  { keys: '/', description: 'Filter' },
  { keys: 'Enter', description: "Open the day's note, or list the day" },
  { keys: 'Esc', description: 'Close' },
]

/**
 * The folder of the page at `pagePath` (decoded, site-absolute): a note's
 * own folder, else the listing's path. `''` at the root.
 */
export function pageFolder(files: readonly MarkdownFile[], pagePath: string): string {
  const note = files.find((f) => f.url_path === pagePath)
  if (note) {
    const slash = note.raw_path.lastIndexOf('/')
    return slash < 0 ? '' : note.raw_path.slice(0, slash)
  }
  return pagePath.replace(/^\/+|\/+$/g, '')
}

/** The notes in `folder` and its subfolders. */
export function filesInFolder(files: readonly MarkdownFile[], folder: string): MarkdownFile[] {
  const prefix = `${folder}/`
  return files.filter((f) => f.raw_path.startsWith(prefix))
}
