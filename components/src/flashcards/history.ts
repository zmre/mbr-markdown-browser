/**
 * Review-history entries: the `YYYY-MM-DD HH:MM - Rating` lines under a
 * flashcard's `___Review History___` definition.
 *
 * Pure and stateless, so both bundles import it: the main bundle to collapse a
 * history into its one-line summary on the reading view, the lazy chunk to
 * replay it through FSRS.
 *
 * # Timestamps are the reviewer's local wall-clock time
 *
 * An entry carries no offset (so it reads well as plain text), and it is
 * replayed here as **the browser's** local time. The writer therefore stamps it
 * with that same clock: the deck sends its own local time as `at`
 * (`review-writer.ts`, formatted by {@link formatEntryTime}) and the server only
 * validates it and writes it verbatim. A server stamp would be wrong whenever
 * the server runs in another time zone — every entry hours away from where the
 * replay puts "now", which breaks FSRS's 1-minute/10-minute learning steps
 * outright, not just at the margins.
 *
 * What remains is the reviewer moving: entries written in one zone and replayed
 * in another shift together by the difference. That can move a day boundary
 * slightly, but the history stays consistent with itself.
 */

/** The four self-ratings, in button order. */
export type Rating = 'again' | 'hard' | 'good' | 'easy'

export const RATINGS: readonly Rating[] = ['again', 'hard', 'good', 'easy']

/** How a rating is spelled in an entry and on its button. */
export const RATING_LABELS: Readonly<Record<Rating, string>> = {
  again: 'Again',
  hard: 'Hard',
  good: 'Good',
  easy: 'Easy',
}

/** One parsed history entry. */
export interface HistoryEntry {
  /** Local time of the review, to the minute. */
  readonly at: Date
  readonly rating: Rating
}

/**
 * `2026-10-06 13:45 - Good`, tolerating a `T` separator, an en/em dash and
 * trailing text. `Fail` is accepted as a synonym for `Again` (other flashcard
 * tools' word for it); mbr itself only ever writes `Again`.
 */
const ENTRY_PATTERN =
  /^\s*(\d{4})-(\d{2})-(\d{2})[ T]+(\d{1,2}):(\d{2})\s*[-–—]\s*(again|fail|hard|good|easy)\b/i

/** Parses one entry's text, or `null` when it is not an entry. */
export function parseHistoryEntry(text: string): HistoryEntry | null {
  const match = ENTRY_PATTERN.exec(text)
  if (!match) return null
  const [, y, mo, d, h, mi, word] = match
  const at = new Date(Number(y), Number(mo) - 1, Number(d), Number(h), Number(mi))
  // `Date` silently rolls 2026-02-31 into March; an impossible date is not an
  // entry, it is a typo, and replaying it would invent a review.
  if (
    at.getFullYear() !== Number(y) ||
    at.getMonth() !== Number(mo) - 1 ||
    at.getDate() !== Number(d) ||
    at.getHours() !== Number(h) ||
    at.getMinutes() !== Number(mi)
  ) {
    return null
  }
  const lower = word.toLowerCase()
  const rating: Rating = lower === 'fail' ? 'again' : (lower as Rating)
  return { at, rating }
}

/** Parses many entry texts, dropping the unparseable, oldest first. */
export function parseHistory(texts: Iterable<string>): HistoryEntry[] {
  const entries: HistoryEntry[] = []
  for (const text of texts) {
    const entry = parseHistoryEntry(text)
    if (entry) entries.push(entry)
  }
  // Stable, so equal timestamps keep their written order.
  return entries.sort((a, b) => a.at.getTime() - b.at.getTime())
}

/** True when emphasised text names the history definition. */
export function isHistoryLabel(text: string): boolean {
  return text.trim().toLowerCase() === 'review history'
}

/** `2026-10-06 13:45` for a local time. */
export function formatEntryTime(at: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0')
  return (
    `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())} ` +
    `${pad(at.getHours())}:${pad(at.getMinutes())}`
  )
}

/** `Oct 9`, or `Oct 9, 2025` outside the current year, in the reader's locale. */
export function formatShortDate(at: Date, now: Date = new Date()): string {
  const options: Intl.DateTimeFormatOptions =
    at.getFullYear() === now.getFullYear()
      ? { month: 'short', day: 'numeric' }
      : { month: 'short', day: 'numeric', year: 'numeric' }
  return at.toLocaleDateString(undefined, options)
}

/**
 * The collapsed reading-view line: `Reviewed 4× · last: Easy, Oct 9`.
 *
 * With no parseable entries it says so plainly rather than "Reviewed 0×", since
 * the list may well hold lines a human wrote in some other shape.
 */
export function summarizeHistory(entries: readonly HistoryEntry[], now: Date = new Date()): string {
  const last = entries[entries.length - 1]
  if (!last) return 'Review history'
  return `Reviewed ${entries.length}× · last: ${RATING_LABELS[last.rating]}, ${formatShortDate(last.at, now)}`
}
