/**
 * Keeping a fixed overlay inside the *visual* viewport.
 *
 * iOS Safari does not resize the layout viewport for the on-screen keyboard:
 * `position: fixed`, `100%`, `vh` and `dvh` all keep measuring the screen
 * behind the keyboard, so the bottom of an overlay sits under it (and when
 * Safari scrolls to reveal the focused field, the top can slide off too).
 * `interactive-widget=resizes-content` in the viewport meta would fix it where
 * honoured, but WebKit ignores it and it would change every page. The visual
 * viewport is the one thing that tracks the keyboard everywhere, so an overlay
 * publishes its box as custom properties and its CSS sizes itself from them:
 *
 * - `--mbr-vvh`: the visible height, in px
 * - `--mbr-vv-top`: how far the visible area is scrolled into the layout one
 *
 * Both are unset when there is no `visualViewport`, so every rule using them
 * must carry a fallback (`var(--mbr-vvh, 100dvh)`). On a desktop the values
 * equal the window's, so nothing moves.
 */

/** The part of `VisualViewport` this reads; injectable for tests. */
export interface ViewportBox {
  readonly height: number
  readonly offsetTop: number
}

/** A `VisualViewport`-shaped event source; injectable for tests. */
export type ViewportSource = ViewportBox & Pick<EventTarget, 'addEventListener' | 'removeEventListener'>

export const VVH_PROPERTY = '--mbr-vvh'
export const VV_TOP_PROPERTY = '--mbr-vv-top'

/**
 * The custom-property values for `box`, or `null` when it cannot be used (a
 * non-finite or non-positive height, as a detached or mid-rotation viewport
 * can report). Whole pixels, so a sub-pixel scroll does not restyle.
 */
export function visualViewportVars(box: ViewportBox): Record<string, string> | null {
  const { height, offsetTop } = box
  if (!Number.isFinite(height) || height <= 0) return null
  const top = Number.isFinite(offsetTop) && offsetTop > 0 ? Math.round(offsetTop) : 0
  return {
    [VVH_PROPERTY]: `${Math.round(height)}px`,
    [VV_TOP_PROPERTY]: `${top}px`,
  }
}

/** Removes the properties `trackVisualViewport` set. */
export function clearVisualViewportVars(target: HTMLElement): void {
  target.style.removeProperty(VVH_PROPERTY)
  target.style.removeProperty(VV_TOP_PROPERTY)
}

/**
 * Keep `target`'s `--mbr-vvh` / `--mbr-vv-top` in step with `source` (the
 * window's `visualViewport` by default) until the returned function is called,
 * which also removes them. Without a source this sets nothing and the CSS
 * fallbacks apply. The properties inherit, so setting them on a custom
 * element's host reaches its shadow root.
 */
export function trackVisualViewport(
  target: HTMLElement,
  source: ViewportSource | null | undefined = typeof window === 'undefined' ? null : window.visualViewport,
): () => void {
  if (!source) return () => {}
  const apply = (): void => {
    const vars = visualViewportVars(source)
    if (!vars) return
    for (const [name, value] of Object.entries(vars)) target.style.setProperty(name, value)
  }
  apply()
  source.addEventListener('resize', apply)
  source.addEventListener('scroll', apply)
  return () => {
    source.removeEventListener('resize', apply)
    source.removeEventListener('scroll', apply)
    clearVisualViewportVars(target)
  }
}
