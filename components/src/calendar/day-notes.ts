/**
 * Notes by day: the `noteDate` buckets behind the mini month's dots and the
 * companion's "Notes from this day".
 *
 * Pure, type-only imports plus `note-date.ts` / `periodic.ts`: built once per
 * page from `site.json` (O(notes)), then read per visible cell.
 */
import type { MarkdownFile } from '../sorting.js'
import { noteDate, type DateKey } from './note-date.js'
import { periodKindOf } from './periodic.js'

/** Notes dated each day, in `site.json` order. */
export type DayBuckets = ReadonlyMap<DateKey, readonly MarkdownFile[]>

/**
 * Buckets every note by `noteDate`. Week, month, quarter and year notes are
 * left out: their date is only their period's first day, and they are reached
 * through the week-number and month-title links instead. Day notes stay in;
 * callers exclude the one the cell or list is about.
 */
export function bucketByDate(files: readonly MarkdownFile[]): DayBuckets {
  const buckets = new Map<DateKey, MarkdownFile[]>()
  for (const file of files) {
    const period = file.calendar?.period
    if (period && periodKindOf(period) !== 'day') continue
    const date = noteDate(file)
    if (!date) continue
    const bucket = buckets.get(date)
    if (bucket) bucket.push(file)
    else buckets.set(date, [file])
  }
  return buckets
}

/** One row of "Notes from this day". */
export interface DayNote {
  readonly title: string
  readonly url: string
  /** The repository folder, `''` at the root. */
  readonly folder: string
}

/** A group of rows; `type` is `null` for notes without one. */
export interface DayNoteGroup {
  readonly type: string | null
  readonly notes: readonly DayNote[]
}

/** A note's display title: frontmatter `title`, else its file stem. */
export function noteTitle(file: MarkdownFile): string {
  const title = file.frontmatter?.title
  if (typeof title === 'string' && title.trim()) return title.trim()
  const name = file.raw_path.split('/').pop() ?? file.raw_path
  return name.replace(/\.[^.]+$/, '')
}

function folderOf(file: MarkdownFile): string {
  const at = file.raw_path.lastIndexOf('/')
  return at < 0 ? '' : file.raw_path.slice(0, at)
}

const collator = new Intl.Collator(undefined, { sensitivity: 'base', numeric: true })

/**
 * The notes of one day, minus `excludeUrl` (the day note itself), grouped by
 * `type`: `meeting` first, then other types alphabetically, untyped last.
 * When no note has a type the result is one untyped group. Titles sort within
 * a group.
 */
export function groupDayNotes(notes: readonly MarkdownFile[], excludeUrl: string | null): DayNoteGroup[] {
  const groups = new Map<string | null, DayNote[]>()
  for (const file of notes) {
    if (file.url_path === excludeUrl) continue
    const raw = file.frontmatter?.type
    const type = typeof raw === 'string' && raw.trim() ? raw.trim() : null
    const row = { title: noteTitle(file), url: file.url_path, folder: folderOf(file) }
    const group = groups.get(type)
    if (group) group.push(row)
    else groups.set(type, [row])
  }
  const rank = (type: string | null): number => (type === null ? 2 : type.toLowerCase() === 'meeting' ? 0 : 1)
  return Array.from(groups, ([type, rows]) => ({
    type,
    notes: rows.sort((a, b) => collator.compare(a.title, b.title)),
  })).sort((a, b) => rank(a.type) - rank(b.type) || collator.compare(a.type ?? '', b.type ?? ''))
}
