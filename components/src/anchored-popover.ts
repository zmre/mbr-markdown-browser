/**
 * Viewport-clamped positioning for a floating card anchored to an element.
 *
 * The "footnote-card pattern": `<mbr-footnote-preview>` grew it, the mini
 * graph's node hover card copied it verbatim, and the review-note markers are
 * the third consumer — which is the point at which a third copy stops being
 * cheaper than a shared function.
 *
 * Pure apart from two style writes, so it is unit-testable without a layout
 * engine: pass rects in, assert the numbers. That matters because happy-dom
 * reports every `getBoundingClientRect()` as zeros, so the elements that use
 * this can only be tested through {@link positionRects}.
 *
 * Safe for a lazy chunk to import: no module-level state, no fetches, no
 * caches. Bundling it into the graph chunk costs a couple of dozen lines.
 */

/** Gap (px) between the anchor and the card, and the viewport clamp margin. */
export const GAP_PX = 8

/** A rectangle, in the shape `getBoundingClientRect()` returns. */
export interface Rect {
  readonly left: number
  readonly top: number
  readonly right: number
  readonly bottom: number
  readonly width: number
  readonly height: number
}

/** Viewport-relative coordinates for a `position: fixed` card. */
export interface Placement {
  readonly left: number
  readonly top: number
}

/**
 * Where to put a card of size `card` anchored to `anchor`, inside a viewport
 * `vw` x `vh`.
 *
 * Horizontally centred on the anchor and clamped to the viewport. Vertically it
 * prefers to sit *above* the anchor and flips below only when that would
 * overflow the top edge — the reading order a reader expects from a footnote,
 * and the one that keeps the pointer's travel path short.
 *
 * The clamps are applied after the flip, so a card taller than the viewport
 * still lands at `GAP_PX` rather than off-screen. `Math.min` before `Math.max`
 * would put a too-large card at a negative offset.
 */
export function positionRects(
  anchor: Rect,
  card: Rect,
  vw: number,
  vh: number,
  gap: number = GAP_PX
): Placement {
  let left = anchor.left + anchor.width / 2 - card.width / 2
  left = Math.max(gap, Math.min(left, vw - card.width - gap))

  let top = anchor.top - card.height - gap
  if (top < gap) top = anchor.bottom + gap
  top = Math.max(gap, Math.min(top, vh - card.height - gap))

  return { left, top }
}

/**
 * Measure `card` against `anchor` and write the resulting `left`/`top`.
 *
 * The card is moved to a neutral origin *before* it is measured. Without that
 * step a card already positioned near the right or bottom edge is measured
 * against its own constrained box, so `max-width`/`max-height` have not yet
 * taken effect and the returned size is wrong — the card then walks a few
 * pixels every time it is shown. The caller must have made the card visible
 * (`display` not `none`) first, or every measurement is zero.
 */
export function positionAnchored(
  anchor: Element,
  card: HTMLElement,
  gap: number = GAP_PX
): void {
  positionAt(anchor.getBoundingClientRect(), card, gap)
}

/**
 * As {@link positionAnchored}, but anchored to a bare rect.
 *
 * A text selection has no element to point at — `Range.getBoundingClientRect()`
 * is all there is — so the review note form anchors to the rect the reader
 * highlighted. Everything below the rect is shared with the element form, which
 * is the point: the neutral-origin measurement is easy to leave out and its
 * absence only shows up as a card that creeps a few pixels each time.
 */
export function positionAt(anchorRect: Rect, card: HTMLElement, gap: number = GAP_PX): void {
  card.style.left = '0px'
  card.style.top = '0px'
  const cardRect = card.getBoundingClientRect()

  const { left, top } = positionRects(
    anchorRect,
    cardRect,
    window.innerWidth,
    window.innerHeight,
    gap
  )

  card.style.left = `${left}px`
  card.style.top = `${top}px`
}
