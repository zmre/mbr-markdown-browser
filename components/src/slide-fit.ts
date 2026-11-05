/**
 * Fitting one Reveal.js slide into its fixed 960x700 box (main bundle, used by
 * `<mbr-slides>`).
 *
 * Reveal scales the whole slide box to the window but lays out each slide's
 * content at its natural size. A slide taller than the box is pinned to the
 * top (`center: true` computes `top = max((700 - scrollHeight) / 2, 0)`) and
 * its bottom is cut off, which is what a heading, some bullets and an image at
 * its natural size turn into. So, for the slide on screen:
 *
 * 1. Images first: a slide that overflows and holds images caps them to the
 *    height the rest of its content leaves (never below `MIN_IMAGE_SHARE` of
 *    the slide), which is what an author placing a picture under a few bullets
 *    means.
 * 2. Then text: if it still overflows, the slide's font size is binary
 *    searched down (headings, lists and code are all sized relative to it) to
 *    the largest that fits, but not below `MIN_FONT_SCALE`.
 *
 * Nothing is measured on slides that already fit, so an ordinary deck pays one
 * `scrollHeight` read per slide change. The search is split from the measuring
 * (`largestFittingScale` takes the measurement as a function) so it can be
 * tested without a layout engine, as `flashcards/fit.ts` does.
 */

/** Smallest font scale text is shrunk to before the slide scrolls instead. */
export const MIN_FONT_SCALE = 0.5

/** Images are never squeezed below this share of the slide's height. */
export const MIN_IMAGE_SHARE = 0.3

/** Binary search steps: 1/2^6 of the 0.5..1 range is under 1%. */
const SEARCH_STEPS = 6

/** Marks an image whose `max-height` the fit set, so a re-fit can undo it. */
const FIT_ATTR = 'data-mbr-fit'

/**
 * The largest scale in `[min, 1]` whose `measure(scale)` is at most `limit`,
 * to within `1 / 2^steps` of the range. `measure` applies the scale and
 * returns the resulting height; the returned scale is the one left applied.
 * Returns `min` (applied) when even that overflows: the caller decides what
 * happens to a slide that cannot fit.
 */
export function largestFittingScale(
  measure: (scale: number) => number,
  limit: number,
  min = MIN_FONT_SCALE,
  steps = SEARCH_STEPS
): number {
  if (measure(1) <= limit) return 1
  if (measure(min) > limit) return min
  let fits = min
  let overflows = 1
  for (let i = 0; i < steps; i++) {
    const mid = (fits + overflows) / 2
    if (measure(mid) <= limit) fits = mid
    else overflows = mid
  }
  measure(fits)
  return fits
}

/** Clear what a previous fit set, so the slide is measured as authored. */
export function resetSlideFit(slide: HTMLElement): void {
  slide.style.removeProperty('font-size')
  slide.querySelectorAll<HTMLElement>(`[${FIT_ATTR}]`).forEach((img) => {
    img.style.removeProperty('max-height')
    img.removeAttribute(FIT_ATTR)
  })
}

/**
 * Caps the slide's images to the height its other content leaves. Images
 * sharing a parent block sit side by side, so each such row counts once.
 */
function capImages(slide: HTMLElement, height: number): void {
  const images = Array.from(slide.querySelectorAll<HTMLElement>('img, video'))
  if (images.length === 0) return
  const rows = new Map<Element | null, number>()
  for (const image of images) {
    const row = image.parentElement
    rows.set(row, Math.max(rows.get(row) ?? 0, image.offsetHeight))
  }
  let imageHeight = 0
  rows.forEach((rowHeight) => {
    imageHeight += rowHeight
  })
  const otherContent = slide.scrollHeight - imageHeight
  const room = Math.max(height - otherContent, height * MIN_IMAGE_SHARE)
  const cap = Math.floor(room / rows.size)
  for (const image of images) {
    if (image.offsetHeight <= cap) continue
    image.style.maxHeight = `${cap}px`
    image.setAttribute(FIT_ATTR, '')
  }
}

/**
 * Fits `slide` (the one on screen; a hidden slide measures as empty) into
 * `height` pixels of slide space. Returns whether it fits afterwards.
 */
export function fitSlide(slide: HTMLElement, height: number): boolean {
  resetSlideFit(slide)
  if (slide.scrollHeight <= height) return true

  capImages(slide, height)
  if (slide.scrollHeight <= height) return true

  const scale = largestFittingScale((s) => {
    if (s === 1) slide.style.removeProperty('font-size')
    else slide.style.fontSize = `${Math.round(s * 1000) / 10}%`
    return slide.scrollHeight
  }, height)
  return scale > MIN_FONT_SCALE || slide.scrollHeight <= height
}
