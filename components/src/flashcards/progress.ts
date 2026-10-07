/**
 * Reading-view progress indicators for a flashcard note (reading chunk only).
 *
 * - Each reviewed question (`<dt>`) gets `mbr-fc-last-{rating}` — a left
 *   border in its latest rating's colour (`templates/theme.css`) — and a
 *   `title` saying when.
 * - Each heading with cards under it gets a small pie of its cards' latest
 *   ratings, unreviewed cards a grey slice of their own. The page's H1 shows
 *   the whole note, whether or not the cards sit under it structurally.
 *
 * Nothing is drawn on a note with no parseable history at all: a fresh deck
 * would only be grey pies.
 *
 * # The pie adds no text
 *
 * It is a `<span role="img">` whose name lives in `aria-label` / `title`
 * attributes around an `aria-hidden` `<svg>` of `<path>`s — no text node, and
 * no SVG `<title>` (whose content *is* a text node). So selecting a heading
 * copies only its words, `find-in-page.ts` indexes nothing new, and
 * `headingText` / anything reading `textContent` is unaffected — the same rule
 * as the heading permalink and the review markers.
 *
 * Idempotent: every run first removes what the previous one drew.
 */
import { RATING_LABELS, formatShortDate, type Rating } from './history.js'
import { cardSections, deckCards, historyEntriesOf } from './dom.js'

/** Prefix of the class a reviewed `<dt>` carries; the rating follows. */
export const LAST_CLASS_PREFIX = 'mbr-fc-last-'
/** Class of a heading's pie. */
export const PIE_CLASS = 'mbr-fc-pie'
/** Marks a `<dt>` this module decorated (value: the rating). */
const LAST_ATTR = 'data-mbr-fc-last'
/** Marks a `title` this module set, so an author's own is never removed. */
const TITLE_ATTR = 'data-mbr-fc-title'

/** Pie slices, in drawing order. */
export const PIE_BUCKETS = ['again', 'hard', 'good', 'easy', 'unreviewed'] as const
export type PieBucket = (typeof PIE_BUCKETS)[number]
export type PieCounts = Readonly<Record<PieBucket, number>>

/** Tooltip order: best first, the way a reader scans for "how am I doing". */
const LABEL_ORDER: readonly PieBucket[] = ['easy', 'good', 'hard', 'again', 'unreviewed']
const BUCKET_LABELS: Readonly<Record<PieBucket, string>> = { ...RATING_LABELS, unreviewed: 'Not reviewed' }

const SVG_NS = 'http://www.w3.org/2000/svg'

/** Count latest ratings into pie buckets. */
export function pieCounts(latest: readonly (Rating | null)[]): PieCounts {
  const counts: Record<PieBucket, number> = { again: 0, hard: 0, good: 0, easy: 0, unreviewed: 0 }
  for (const rating of latest) counts[rating ?? 'unreviewed']++
  return counts
}

/** `Easy 40% · Good 20% · … · Not reviewed 10% (10 cards)`; empty buckets omitted. */
export function pieLabel(counts: PieCounts): string {
  const total = PIE_BUCKETS.reduce((sum, b) => sum + counts[b], 0)
  const parts = LABEL_ORDER.filter((b) => counts[b] > 0).map(
    (b) => `${BUCKET_LABELS[b]} ${Math.round((counts[b] / total) * 100)}%`
  )
  return `${parts.join(' · ')} (${total} card${total === 1 ? '' : 's'})`
}

/** A point on the unit circle centred at (1, 1), `turn` 0 at 12 o'clock. */
function point(turn: number): string {
  const angle = turn * 2 * Math.PI - Math.PI / 2
  return `${(1 + Math.cos(angle)).toFixed(4)} ${(1 + Math.sin(angle)).toFixed(4)}`
}

/** The pie for `counts`: a `<span role="img">` around an `aria-hidden` svg. */
export function pieElement(counts: PieCounts): HTMLElement {
  const total = PIE_BUCKETS.reduce((sum, b) => sum + counts[b], 0)
  const label = pieLabel(counts)
  const wrap = document.createElement('span')
  wrap.className = PIE_CLASS
  wrap.setAttribute('role', 'img')
  wrap.setAttribute('aria-label', label)
  wrap.title = label

  const svg = document.createElementNS(SVG_NS, 'svg')
  svg.setAttribute('viewBox', '0 0 2 2')
  svg.setAttribute('aria-hidden', 'true')
  svg.setAttribute('focusable', 'false')
  let start = 0
  for (const bucket of PIE_BUCKETS) {
    const count = counts[bucket]
    if (count === 0) continue
    const share = count / total
    let shape: SVGElement
    if (share >= 1) {
      shape = document.createElementNS(SVG_NS, 'circle')
      shape.setAttribute('cx', '1')
      shape.setAttribute('cy', '1')
      shape.setAttribute('r', '1')
    } else {
      shape = document.createElementNS(SVG_NS, 'path')
      const large = share > 0.5 ? 1 : 0
      shape.setAttribute('d', `M1 1L${point(start)}A1 1 0 ${large} 1 ${point(start + share)}Z`)
    }
    shape.setAttribute('class', `${PIE_CLASS}-${bucket}`)
    svg.append(shape)
    start += share
  }
  wrap.append(svg)
  return wrap
}

/** Remove every indicator a previous {@link decorateProgress} drew under `root`. */
export function clearProgress(root: ParentNode): void {
  root.querySelectorAll(`.${PIE_CLASS}`).forEach((el) => el.remove())
  for (const dt of Array.from(root.querySelectorAll<HTMLElement>(`[${LAST_ATTR}]`))) {
    dt.classList.remove(LAST_CLASS_PREFIX + dt.getAttribute(LAST_ATTR))
    dt.removeAttribute(LAST_ATTR)
    if (dt.hasAttribute(TITLE_ATTR)) {
      dt.removeAttribute('title')
      dt.removeAttribute(TITLE_ATTR)
    }
  }
}

/** Put `pie` at the end of `heading`'s text, before its permalink if it has one. */
function attachPie(heading: HTMLElement, pie: HTMLElement): void {
  const anchor = heading.querySelector(':scope > .mbr-heading-anchor')
  if (anchor) anchor.before(pie)
  else heading.append(pie)
}

/**
 * Draw (or redraw) the indicators for the deck under `root` — the page's
 * `main`. Re-run after reviews are written; the history `<dd>`s are the source.
 */
export function decorateProgress(root: ParentNode, now: Date = new Date()): void {
  clearProgress(root)
  const cards = deckCards(root)
  const histories = cards.map((card) => (card.history ? historyEntriesOf(card.history) : []))
  if (!histories.some((entries) => entries.length > 0)) return

  const latest = histories.map((entries) => entries[entries.length - 1]?.rating ?? null)
  cards.forEach((card, i) => {
    const last = histories[i][histories[i].length - 1]
    if (!last) return
    const dt = card.term
    dt.classList.add(LAST_CLASS_PREFIX + last.rating)
    dt.setAttribute(LAST_ATTR, last.rating)
    if (!dt.hasAttribute('title')) {
      dt.title = `Last review: ${RATING_LABELS[last.rating]}, ${formatShortDate(last.at, now)}`
      dt.setAttribute(TITLE_ATTR, '')
    }
  })

  // The note's title: the first H1 outside the cards (the body's own, or the
  // one the template generates when the body has none).
  const title = Array.from(root.querySelectorAll<HTMLElement>('h1')).find((h) => !h.closest('dl'))
  if (title) attachPie(title, pieElement(pieCounts(latest)))

  const { sections } = cardSections(
    root,
    cards.map((card) => card.term)
  )
  for (const section of sections) {
    if (section.heading === title) continue
    attachPie(section.heading, pieElement(pieCounts(section.cards.map((i) => latest[i]))))
  }
}
