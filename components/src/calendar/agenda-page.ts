/**
 * What the calendar chunk does to the page on screen (design/calendar.md
 * §4.3–4.5), without changing its text:
 *
 * - every agenda `<li>` gets `id="mbr-agenda-<n>"`, so the companion on
 *   another page can link to an entry, and a `#mbr-agenda-<n>` fragment is
 *   revealed once the ids exist (the browser's own jump ran before they did);
 * - the leading time of each entry is painted as a chip with the CSS Custom
 *   Highlight API (`::highlight(mbr-agenda-time)` in theme.css), from
 *   `StaticRange`s — never by wrapping text, which would split the text nodes
 *   review markers, find-in-page and cached ranges hold on to;
 * - on **today's** day note, the entry in progress gets `mbr-agenda-now` and
 *   the next one `mbr-agenda-next` (a class on an existing element is safe),
 *   re-evaluated each minute while the page is visible (`minute-clock.ts`,
 *   the bundle's one tick), and only while the day is today.
 *
 * Without `CSS.highlights` there is simply no chip. The QuickLook preview
 * runs this too, with its current-time line on {@link EnhancePageOptions.onTick}.
 */
import { agendaEntries, type AgendaEntry } from './agenda-parse.js'
import { drawnEnd } from './agenda-layout.js'
import { onMinute } from './minute-clock.js'
import { todayKey, type DateKey } from './note-date.js'

/** `CSS.highlights` registry name; styled in theme.css. */
export const AGENDA_TIME_HIGHLIGHT = 'mbr-agenda-time'
/** Classes on today's in-progress and next entries; styled in theme.css. */
export const AGENDA_NOW_CLASS = 'mbr-agenda-now'
export const AGENDA_NEXT_CLASS = 'mbr-agenda-next'
/** Agenda `<li>` ids are this prefix plus the entry's ordinal. */
export const AGENDA_ID_PREFIX = 'mbr-agenda-'

/** The fragment that names entry `ordinal`. */
export function agendaFragment(ordinal: number): string {
  return `#${AGENDA_ID_PREFIX}${ordinal}`
}

/** The element id a `#mbr-agenda-<n>` fragment names, else `null`. */
export function agendaIdFromHash(hash: string): string | null {
  const m = /^#?(mbr-agenda-\d+)$/.exec(hash)
  return m ? m[1] : null
}

/**
 * Gives each entry's `<li>` its `mbr-agenda-<ordinal>` id. An item that already
 * has an id (raw HTML) keeps it.
 */
export function assignAgendaIds(entries: readonly AgendaEntry[]): void {
  for (const entry of entries) {
    if (!entry.li.id) entry.li.id = `${AGENDA_ID_PREFIX}${entry.ordinal}`
  }
}

/**
 * The entry in progress at `minutes` (the latest-starting one, if several)
 * and the first one starting after it.
 */
export function nowAndNext<T extends { start: number; end?: number }>(
  entries: readonly T[],
  minutes: number
): { now: T | null; next: T | null } {
  let now: T | null = null
  let next: T | null = null
  for (const entry of entries) {
    if (entry.start <= minutes && minutes < drawnEnd(entry) && (!now || entry.start >= now.start)) now = entry
    if (entry.start > minutes && (!next || entry.start < next.start)) next = entry
  }
  return { now, next }
}

/** True where the CSS Custom Highlight API exists. */
export function highlightsSupported(): boolean {
  return typeof CSS !== 'undefined' && 'highlights' in CSS && typeof Highlight === 'function'
}

/** Paints the time chips; returns whether anything was registered. */
export function paintTimeChips(entries: readonly AgendaEntry[]): boolean {
  if (!highlightsSupported()) return false
  const ranges = entries.flatMap((entry) => (entry.timeRange ? [new StaticRange(entry.timeRange)] : []))
  if (ranges.length === 0) {
    CSS.highlights.delete(AGENDA_TIME_HIGHLIGHT)
    return false
  }
  CSS.highlights.set(AGENDA_TIME_HIGHLIGHT, new Highlight(...ranges))
  return true
}

/** Sets `mbr-agenda-now` / `mbr-agenda-next` for `minutes`, clearing the rest. */
export function markNowAndNext(entries: readonly AgendaEntry[], minutes: number | null): void {
  const { now, next } = minutes === null ? { now: null, next: null } : nowAndNext(entries, minutes)
  for (const entry of entries) {
    entry.li.classList.toggle(AGENDA_NOW_CLASS, entry === now)
    entry.li.classList.toggle(AGENDA_NEXT_CLASS, entry === next)
  }
}

export interface EnhancePageOptions {
  /** The note's rendered body (`main#wrapper`). */
  readonly root: ParentNode
  /** Its agenda entries, when the caller has parsed them already. */
  readonly entries?: AgendaEntry[]
  /** `[calendar] agenda_chips`. */
  readonly agendaChips: boolean
  /** The page's own day when it is a day note, else `null`. */
  readonly pageDay: DateKey | null
  /** Scrolls an element clear of the sticky header and flashes it. */
  readonly reveal?: (el: HTMLElement) => void
  /**
   * Told the minute (minutes after midnight) on each tick while `pageDay` is
   * today, then `null` once it no longer is (QuickLook's current-time line).
   */
  readonly onTick?: (minutes: number | null) => void
  /** The clock; tests pass a fixed one. */
  readonly now?: () => Date
}

/**
 * Enhances the page once. Returns the entries found and a disposer that
 * removes every listener, tick, class and highlight it added (ids stay).
 */
export function enhancePage(options: EnhancePageOptions): { entries: AgendaEntry[]; dispose: () => void } {
  const clock = options.now ?? (() => new Date())
  const entries = options.entries ?? agendaEntries(options.root)
  assignAgendaIds(entries)

  const revealHash = (): void => {
    const id = agendaIdFromHash(window.location.hash)
    const el = id ? document.getElementById(id) : null
    if (el) (options.reveal ?? ((e: HTMLElement) => e.scrollIntoView()))(el)
  }
  revealHash()
  window.addEventListener('hashchange', revealHash)

  const chips = options.agendaChips && entries.length > 0 && paintTimeChips(entries)

  // Now / next (and `onTick`) only on today's own day note, and only while it
  // is today: the tick starts only then and stops at midnight.
  let stop: (() => void) | null = null
  const tick = (): void => {
    const date = clock()
    const live = options.pageDay === todayKey(date)
    const minutes = live ? date.getHours() * 60 + date.getMinutes() : null
    if (options.agendaChips) markNowAndNext(entries, minutes)
    options.onTick?.(minutes)
    if (!live) {
      stop?.()
      stop = null
    }
  }
  const wanted = entries.length > 0 && (options.agendaChips || options.onTick !== undefined)
  if (wanted && options.pageDay !== null && options.pageDay === todayKey(clock())) {
    tick()
    stop = onMinute(tick)
  }

  return {
    entries,
    dispose: () => {
      window.removeEventListener('hashchange', revealHash)
      stop?.()
      stop = null
      markNowAndNext(entries, null)
      if (chips) CSS.highlights.delete(AGENDA_TIME_HIGHLIGHT)
    },
  }
}
