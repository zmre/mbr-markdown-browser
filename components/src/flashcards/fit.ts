/**
 * Fit-to-screen for a card face: the largest font size at which the face's
 * content fits its box, down to a readable minimum below which the face
 * scrolls instead.
 *
 * The search is split from the measuring so it can be tested: happy-dom lays
 * nothing out (every size reads as 0), so `fitFontSize` takes the "does it
 * fit?" question as a function.
 */

/** Smallest size text is ever set at; below this the face scrolls. */
export const MIN_FONT_PX = 16
/** Largest size; a one-word card at 200px reads as a poster, not a card. */
export const MAX_FONT_PX = 72

/**
 * Largest integer size in `[min, max]` for which `fits(size)` holds, assuming
 * `fits` is monotone (bigger text never fits where smaller text did not).
 * Returns `min` when nothing fits. About seven layouts for the default range.
 */
export function fitFontSize(
  fits: (size: number) => boolean,
  min: number = MIN_FONT_PX,
  max: number = MAX_FONT_PX
): number {
  let lo = min
  let hi = max
  if (fits(hi)) return hi
  while (hi - lo > 1) {
    const mid = Math.floor((lo + hi) / 2)
    if (fits(mid)) lo = mid
    else hi = mid
  }
  return lo
}

/**
 * Size `content` to fill `face`.
 *
 * "Fits" means no overflow in either direction inside the face's padding box.
 * Width matters as much as height: a long unbreakable token (a URL, a code
 * span) overflows sideways long before a short card runs out of height.
 */
export function fitFace(face: HTMLElement, content: HTMLElement): number {
  const style = getComputedStyle(face)
  const availableWidth =
    face.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight)
  const availableHeight =
    face.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom)
  if (!(availableWidth > 0 && availableHeight > 0)) return MIN_FONT_PX

  // Code blocks and tables scroll themselves (Pico gives them
  // `overflow: auto`), so their overflow never reaches `content.scrollWidth`;
  // without asking them directly a long code line is fitted as if it wrapped.
  const scrollers = Array.from(content.querySelectorAll<HTMLElement>('pre, table'))
  const size = fitFontSize((px) => {
    content.style.fontSize = `${px}px`
    return (
      content.scrollWidth <= availableWidth + 1 &&
      content.offsetHeight <= availableHeight + 1 &&
      scrollers.every((el) => el.scrollWidth <= el.clientWidth + 1)
    )
  })
  content.style.fontSize = `${size}px`
  return size
}
