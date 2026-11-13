/**
 * `[` / `]` on a page that is not a periodic note: jump to the previous / next
 * heading of the document, landing it just below the sticky header.
 *
 * MAIN BUNDLE. The selection is pure ({@link adjacentHeadingIndex},
 * {@link jumpStillInFlight}) because happy-dom lays nothing out; the DOM half
 * ({@link jumpToAdjacentHeading}) only measures and scrolls.
 *
 * Only scrolls: the URL hash is left alone, like the table of contents
 * (`T`), which scrolls to a heading without touching history either. A hash
 * per keypress would fill Back with every heading passed.
 */

/** Headings in the rendered note (the same container the ToC and find bar use). */
export const HEADING_SELECTOR = ['h1', 'h2', 'h3', 'h4', 'h5', 'h6']
  .map((tag) => `main#wrapper ${tag}`)
  .join(', ')

/**
 * Slack (px) when deciding whether a heading is already "at" the landing
 * line: the scroll position is fractional on zoomed or high-DPI pages, and a
 * heading resting 1px off its target must count as the current one, or `]`
 * would jump to it again instead of past it.
 */
export const HEADING_TOLERANCE = 4

export type HeadingDirection = 'prev' | 'next'

/**
 * Index of the heading to jump to, or -1 when there is none that way.
 *
 * `tops` are the headings' viewport tops in document order; `landing` is the
 * line a jump puts a heading on (the sticky header's height, from
 * `scroll-padding-top`). `anchor` is the heading the previous jump is still
 * heading to (-1 for none): while a smooth scroll is in flight, or when the page
 * could not scroll far enough to bring that heading up to the line (the end of
 * the document), geometry alone would pick the same heading again, so a repeated
 * key steps on from the anchor instead.
 */
export function adjacentHeadingIndex(
  tops: readonly number[],
  landing: number,
  direction: HeadingDirection,
  anchor = -1,
  tolerance = HEADING_TOLERANCE,
): number {
  if (anchor >= 0 && anchor < tops.length) {
    const next = direction === 'next' ? anchor + 1 : anchor - 1
    return next >= 0 && next < tops.length ? next : -1
  }
  if (direction === 'next') {
    return tops.findIndex((top) => top > landing + tolerance)
  }
  for (let i = tops.length - 1; i >= 0; i--) {
    if (tops[i] < landing - tolerance) return i
  }
  return -1
}

/**
 * Whether the window is still where the last jump left it, or on its way
 * there: anywhere between the scroll position it started from and the one it
 * aimed at. Anything else means the reader scrolled since, and the next jump
 * must be measured afresh.
 */
export function jumpStillInFlight(
  scrollY: number,
  from: number,
  to: number,
  tolerance = HEADING_TOLERANCE,
): boolean {
  const low = Math.min(from, to) - tolerance
  const high = Math.max(from, to) + tolerance
  return scrollY >= low && scrollY <= high
}

/** The last jump: which heading it went to and the scroll it started / aimed at. */
interface LastJump {
  heading: Element
  from: number
  to: number
}

let lastJump: LastJump | null = null

/** Test hook: forget the previous jump. */
export function resetHeadingJump(): void {
  lastJump = null
}

/** A heading the reader can see: not in a closed `<details>` or otherwise unrendered. */
function isRendered(el: Element): boolean {
  const rect = el.getBoundingClientRect()
  return rect.width > 0 || rect.height > 0
}

/** The landing line: `scroll-padding-top` (theme.css sizes it to the sticky header). */
function landingLine(): number {
  const padding = parseFloat(getComputedStyle(document.documentElement).scrollPaddingTop)
  return Number.isFinite(padding) ? padding : 0
}

/**
 * Scroll to the previous / next heading of the note. Returns false (and does
 * nothing) when there is no heading that way, so the caller leaves the key alone.
 */
export function jumpToAdjacentHeading(direction: HeadingDirection, root: ParentNode = document): boolean {
  const headings = Array.from(root.querySelectorAll(HEADING_SELECTOR)).filter(isRendered)
  if (headings.length === 0) return false

  const tops = headings.map((h) => h.getBoundingClientRect().top)
  const landing = landingLine()
  const anchor =
    lastJump && jumpStillInFlight(window.scrollY, lastJump.from, lastJump.to)
      ? headings.indexOf(lastJump.heading)
      : -1

  const index = adjacentHeadingIndex(tops, landing, direction, anchor)
  if (index < 0) return false

  const target = headings[index]
  // Measured from the target's own top, not the in-flight scroll's goal, so a
  // jump issued mid-scroll still lands exactly.
  const maxScroll = Math.max(0, document.documentElement.scrollHeight - window.innerHeight)
  const from = window.scrollY
  const to = Math.min(maxScroll, Math.max(0, from + tops[index] - landing))
  lastJump = { heading: target, from, to }
  window.scrollTo({ top: to, behavior: 'smooth' })
  return true
}
