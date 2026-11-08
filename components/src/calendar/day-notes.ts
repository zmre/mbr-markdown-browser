/**
 * Notes by day: the `noteDate` buckets behind the mini month's dots and the
 * companion's "Notes from this day".
 *
 * Pure, type-only imports plus `note-date.ts`. Also the one reading of a
 * note's title, folder and type, shared with the overlay's buckets.
 */
import type { MarkdownFile } from '../sorting.js'
import { localDayBounds, noteDayOf, type DateKey } from './note-date.js'

/** Notes dated each day, in `site.json` order. */
export type DayBuckets = ReadonlyMap<DateKey, readonly MarkdownFile[]>

/**
 * Buckets every note by `noteDayOf` (week, month, quarter and year notes are
 * left out; day notes stay in, and callers exclude the one the cell or list
 * is about), keeping only days `from`..`to` when given: the companion buckets
 * just the months around the one on screen.
 *
 * `calendar.date` is compared as a string, and a birthtime against the
 * window's epoch bounds before it is turned into a date, so a note outside
 * the window costs a comparison, not a `Date`.
 */
export function bucketByDate(files: readonly MarkdownFile[], from: DateKey | null = null, to: DateKey | null = null): DayBuckets {
  const buckets = new Map<DateKey, MarkdownFile[]>()
  const bounds = from === null && to === null ? null : localDayBounds(from, to)
  for (const file of files) {
    const cal = file.calendar
    if (bounds) {
      if (cal) {
        if ((from !== null && cal.date < from) || (to !== null && cal.date > to)) continue
      } else if (typeof file.created !== 'number' || !(file.created >= bounds.from && file.created < bounds.to)) {
        continue
      }
    }
    const date = noteDayOf(file)
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

/** The repository folder of a `raw_path`, `''` at the root. */
export function folderOfRawPath(rawPath: string): string {
  const at = rawPath.lastIndexOf('/')
  return at < 0 ? '' : rawPath.slice(0, at)
}

/** A note's frontmatter `type`, trimmed as authored; `null` when it has none. */
export function noteTypeOf(file: MarkdownFile): string | null {
  const raw = file.frontmatter?.type
  return typeof raw === 'string' && raw.trim() ? raw.trim() : null
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
    const type = noteTypeOf(file)
    const row = { title: noteTitle(file), url: file.url_path, folder: folderOfRawPath(file.raw_path) }
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
