/**
 * The day agenda's grammar (design/calendar.md §4.4, decision D6): a top-level
 * list item whose own text starts with a time or a time range is an agenda
 * entry; its first-level child items are details.
 *
 * Pure: no module state and no I/O. The text half (`parseAgendaText`) is plain
 * string work; the DOM half (`agendaEntries`) reads a rendered note, either
 * the live page or one fetched and parsed with `DOMParser`, and never changes
 * it. Shared by the companion's agenda and the inline time chips, so the two
 * cannot disagree about what is an entry.
 *
 * ```
 * entry    := ["at" | "from" WS] time [WS? sep WS? time] term title
 * term     := WS | WS? (":" | "-" | "–" | "—") WS
 * time     := H[H] ":" MM [WS? ampm] | H[H] WS? ampm | "noon" | "midnight" | H[H] "." MM
 * sep      := "-" | "–" | "—" | WS "to" WS
 * ampm     := "am" | "pm" | "a.m." | "p.m."     (case-insensitive throughout)
 * ```
 *
 * The spec's terminator is "whitespace, `:` or a dash", and it also says
 * `10:30am-ish` is not an entry; both hold only if a `:` or dash terminator is
 * itself followed by whitespace, which is the rule implemented.
 */
import type { TaskStatus } from '../tasks/types.js'
import { topLevelListItems } from './agenda-scope.js'

/** The times an agenda line names, and where they sit in its text. */
export interface AgendaTime {
  /** Minutes after midnight. */
  readonly start: number
  /** Minutes after midnight, `> start`; absent when there is no (usable) end. */
  readonly end?: number
  /** The rest of the line, trimmed; never empty. */
  readonly title: string
  /** `[timeFrom, timeTo)`: the time or range in the text, for the inline chip. */
  readonly timeFrom: number
  readonly timeTo: number
}

/** One agenda entry in a rendered note. */
export interface AgendaEntry extends AgendaTime {
  /** The text of each first-level child item, in order. */
  readonly details: readonly string[]
  /** The checkbox state when the item is a task. */
  readonly task?: TaskStatus
  /** The item itself. */
  readonly li: HTMLLIElement
  /** 1-based position among the note's entries: `#mbr-agenda-<ordinal>`. */
  readonly ordinal: number
  /** The time range as DOM positions, when it lies in text nodes. */
  readonly timeRange: DomRange | null
}

/** A range of text in the DOM (a `StaticRange`'s init). */
export interface DomRange {
  readonly startContainer: Text
  readonly startOffset: number
  readonly endContainer: Text
  readonly endOffset: number
}

type Half = 'am' | 'pm'

interface RawTime {
  readonly hour: number
  readonly minute: number
  readonly half: Half | null
  readonly word: 'noon' | 'midnight' | null
  readonly from: number
  readonly to: number
}

const AMPM = String.raw`(a\.m\.|p\.m\.|am|pm)(?![a-z])`
/** Groups: 1 H, 2 MM, 3 ampm | 4 H, 5 ampm | 6 word. */
const TIME = new RegExp(
  String.raw`(?:(\d{1,2})[:.](\d{2})(?!\d)(?:\s*${AMPM})?|(\d{1,2})\s*${AMPM}|(noon|midnight)(?![a-z]))`,
  'iy'
)
const LEAD = /\s*(?:(?:at|from)\s+)?/iy
const SEP = /\s*[-–—]\s*|\s+to\s+/iy
const TERM = /\s*[:\-–—]\s+|\s+/y

/** `re` matched at exactly `pos`, or `null`. */
function matchAt(re: RegExp, text: string, pos: number): RegExpExecArray | null {
  re.lastIndex = pos
  return re.exec(text)
}

function readTime(text: string, pos: number): RawTime | null {
  const m = matchAt(TIME, text, pos)
  if (!m) return null
  const halfOf = (s: string | undefined): Half | null => (s ? (s[0].toLowerCase() === 'a' ? 'am' : 'pm') : null)
  const to = pos + m[0].length
  if (m[6]) {
    return { hour: 0, minute: 0, half: null, word: m[6].toLowerCase() as 'noon' | 'midnight', from: pos, to }
  }
  if (m[4]) return { hour: Number(m[4]), minute: 0, half: halfOf(m[5]), word: null, from: pos, to }
  return { hour: Number(m[1]), minute: Number(m[2]), half: halfOf(m[3]), word: null, from: pos, to }
}

/**
 * Minutes after midnight for `t`, read with `half` (its own by default), or
 * `null` when out of range: hours 0–23, or 1–12 with am/pm; minutes 0–59.
 */
function minutesOf(t: RawTime, half: Half | null = t.half): number | null {
  if (t.word === 'noon') return 720
  if (t.word === 'midnight') return 0
  if (t.minute > 59) return null
  if (half) {
    if (t.hour < 1 || t.hour > 12) return null
    return ((t.hour % 12) + (half === 'pm' ? 12 : 0)) * 60 + t.minute
  }
  return t.hour <= 23 ? t.hour * 60 + t.minute : null
}

/**
 * The end of a range, or `undefined`. An end without am/pm inherits the
 * start's half when that puts it after the start; otherwise it is read as
 * 24-hour. An end at or before the start means "no end". `midnight` as an end
 * is the end of the day.
 */
function endMinutes(end: RawTime, start: number, startHalf: Half | null): number | undefined {
  const after = (v: number | null): number | undefined => (v !== null && v > start ? v : undefined)
  if (end.word === 'midnight') return start < 1440 ? 1440 : undefined
  if (end.word || end.half) return after(minutesOf(end))
  if (startHalf) {
    const inherited = after(minutesOf(end, startHalf))
    if (inherited !== undefined) return inherited
  }
  return after(minutesOf(end, null))
}

/**
 * The title after `pos`, or `null` when no terminator follows or nothing
 * worded does (`09:00 -` is a dangling separator, not a title "-").
 */
function titleAfter(text: string, pos: number): string | null {
  const term = matchAt(TERM, text, pos)
  if (!term) return null
  const title = text.slice(pos + term[0].length).trim()
  return /[\p{L}\p{N}]/u.test(title) ? title : null
}

/** Parses the start of one line of text; `null` when it is not an entry. */
export function parseAgendaText(text: string): AgendaTime | null {
  const lead = matchAt(LEAD, text, 0)
  const first = readTime(text, lead ? lead[0].length : 0)
  if (!first) return null

  // A range when a second time follows a separator; then the title must
  // follow the range (`09:00 - 10:00` alone is not an entry titled "10:00").
  const sep = matchAt(SEP, text, first.to)
  const last = sep ? readTime(text, first.to + sep[0].length) : null
  const title = titleAfter(text, (last ?? first).to)
  if (title === null) return null

  let start = minutesOf(first)
  if (start === null) return null
  let end: number | undefined
  if (last) {
    // "9:30-10:30pm": a start without am/pm takes the end's when that keeps
    // it before the end.
    const lastValue = minutesOf(last)
    if (!first.half && !first.word && last.half && lastValue !== null) {
      const inherited = minutesOf(first, last.half)
      if (inherited !== null && inherited < lastValue) start = inherited
    }
    end = endMinutes(last, start, first.half)
  }
  return {
    start,
    ...(end === undefined ? {} : { end }),
    title,
    timeFrom: first.from,
    timeTo: (last ?? first).to,
  }
}

// ---------------------------------------------------------------------------
// The rendered note
// ---------------------------------------------------------------------------

/** Block elements that end an item's leading line. */
const BLOCK_TAGS = new Set(['UL', 'OL', 'P', 'DIV', 'PRE', 'BLOCKQUOTE', 'TABLE', 'DL', 'FIGURE', 'DETAILS', 'SECTION', 'HR'])

interface Segment {
  readonly node: Text
  /** Offset of the node's first included character in the leading text. */
  readonly at: number
  /** Characters of the node included. */
  readonly length: number
}

interface LeadingText {
  readonly text: string
  readonly segments: readonly Segment[]
}

/**
 * An element the leading line skips: the task checkbox, and the chips the
 * renderer appends to a task's text (`mbr-task-pri`, `-tag`, `-due`, …, but
 * not the text span itself, `mbr-task-text`).
 */
function skipped(el: Element): boolean {
  if (el.tagName === 'INPUT' || el.tagName === 'SCRIPT' || el.tagName === 'STYLE') return true
  if (el.classList.contains('mbr-task-text')) return false
  return Array.from(el.classList).some((c) => c.startsWith('mbr-task-'))
}

/**
 * The item's own first line of text: inline content up to the first line
 * break, `<br>` or block (a nested list, a second paragraph), skipping a
 * task's checkbox and chips. A loose item's first paragraph is entered.
 */
export function leadingText(li: Element): LeadingText {
  const segments: Segment[] = []
  let text = ''
  let done = false

  const walk = (parent: Node): void => {
    for (let node = parent.firstChild; node && !done; node = node.nextSibling) {
      if (node.nodeType === 3) {
        const data = (node as Text).data
        // A line break ends the line, but not before the line has begun (the
        // newline a renderer writes between `<li>` and a loose item's `<p>`).
        const blank = text.trim() === ''
        const nl = blank ? -1 : data.indexOf('\n')
        const part = nl >= 0 ? data.slice(0, nl) : data
        segments.push({ node: node as Text, at: text.length, length: part.length })
        text += part
        if (nl >= 0) done = true
      } else if (node.nodeType === 1) {
        const el = node as Element
        if (el.tagName === 'BR') {
          done = text.trim() !== ''
        } else if (BLOCK_TAGS.has(el.tagName)) {
          if (el.tagName === 'P' && text.trim() === '') {
            walk(el)
          }
          done = true
        } else if (!skipped(el)) {
          walk(el)
        }
      }
    }
  }
  walk(li)
  return { text, segments }
}

/** DOM positions of `[from, to)` in a leading text, or `null`. */
function domRange(lead: LeadingText, from: number, to: number): DomRange | null {
  const find = (offset: number, end: boolean): { node: Text; offset: number } | null => {
    for (const seg of lead.segments) {
      const inside = end ? offset > seg.at && offset <= seg.at + seg.length : offset >= seg.at && offset < seg.at + seg.length
      if (inside) return { node: seg.node, offset: offset - seg.at }
    }
    return null
  }
  const start = find(from, false)
  const end = find(to, true)
  return start && end
    ? { startContainer: start.node, startOffset: start.offset, endContainer: end.node, endOffset: end.offset }
    : null
}

/** The checkbox state of a task item (`data-mbr-task-status`), if it is one. */
function taskStatus(li: Element): TaskStatus | undefined {
  const box = Array.from(li.children)
    .flatMap((child) => (child.tagName === 'P' ? Array.from(child.children) : [child]))
    .find((el) => el.tagName === 'INPUT' && el.classList.contains('mbr-task-check')) as HTMLElement | undefined
  const status = box?.dataset.mbrTaskStatus
  return status === 'open' || status === 'done' || status === 'canceled' ? status : undefined
}

/** The leading text of each first-level child item. */
function detailsOf(li: Element): string[] {
  const details: string[] = []
  for (const list of Array.from(li.children)) {
    if (list.tagName !== 'UL' && list.tagName !== 'OL') continue
    for (const child of Array.from(list.children)) {
      if (child.tagName !== 'LI') continue
      const text = leadingText(child).text.replace(/\s+/g, ' ').trim()
      if (text) details.push(text)
    }
  }
  return details
}

/** Every agenda entry under `root` (a note's `main#wrapper`), in document order. */
export function agendaEntries(root: ParentNode): AgendaEntry[] {
  const entries: AgendaEntry[] = []
  for (const li of topLevelListItems(root)) {
    const lead = leadingText(li)
    const time = parseAgendaText(lead.text)
    if (!time) continue
    const task = taskStatus(li)
    entries.push({
      ...time,
      title: time.title.replace(/\s+/g, ' '),
      details: detailsOf(li),
      ...(task ? { task } : {}),
      li,
      ordinal: entries.length + 1,
      timeRange: domRange(lead, time.timeFrom, time.timeTo),
    })
  }
  return entries
}

/** `HH:MM` for minutes after midnight (24-hour; `24:00` for the day's end). */
export function formatMinutes(minutes: number): string {
  return `${String(Math.floor(minutes / 60)).padStart(2, '0')}:${String(minutes % 60).padStart(2, '0')}`
}
