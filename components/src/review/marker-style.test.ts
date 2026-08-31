/**
 * Guards for the marker's CSS box, which broke three times in a row.
 *
 * The root cause was never the icon artwork. `.mbr-review-marker` is
 * `role="button"`, and Pico styles `[role=button]` as a full button — form
 * padding of roughly `0.75rem 1rem`, a border, and
 * `background-color: var(--pico-primary-background)`. With Pico's global
 * `box-sizing: border-box` on `*`, a fixed `width` on that box left a content
 * area of zero, and the `<svg>` inside — a shrinkable flex item — collapsed to
 * nothing. What rendered was Pico's blue button with no icon in it.
 *
 * These assertions are over stylesheet text, which is weak evidence in
 * general. They are here because the failure is invisible in every other
 * layer: happy-dom does no layout, so no runtime test in this suite can
 * observe a box collapsing.
 */
import { describe, expect, it } from 'vitest'

/** `templates/theme.css`, injected by `vitest.config.ts` — see the note there. */
declare const __MBR_THEME_CSS__: string
const THEME = __MBR_THEME_CSS__

/** The declarations of the base `.mbr-review-marker` rule. */
const MARKER = (() => {
  const at = THEME.indexOf('.mbr-review-marker {')
  return THEME.slice(at, THEME.indexOf('}', at))
})()

/** The declarations of the `.mbr-review-marker > svg` rule. */
const ICON = (() => {
  const at = THEME.indexOf('.mbr-review-marker > svg {')
  return THEME.slice(at, THEME.indexOf('}', at))
})()

describe('marker box', () => {
  it('read the stylesheet, so nothing below passes vacuously', () => {
    expect(THEME.length).toBeGreaterThan(10_000)
    expect(MARKER).toContain('.mbr-review-marker {')
    expect(ICON).toContain('.mbr-review-marker > svg {')
  })

  it('overrides the padding Pico puts on every [role=button]', () => {
    // Pico's is ~0.75rem 1rem. Left alone it is wider than the whole marker.
    expect(MARKER).toMatch(/padding:\s*\d/)
  })

  it('drops Pico’s button border and box-shadow', () => {
    expect(MARKER).toContain('border: none')
    expect(MARKER).toContain('box-shadow: none')
  })

  it('does not pin a width, so the box shrink-wraps its icon', () => {
    // A fixed width plus border-box plus Pico's padding is exactly what
    // collapsed the content area to zero. `width: auto` makes the box grow
    // from the icon and its padding instead, which no box-sizing can undo.
    expect(MARKER).toContain('width: auto')
    expect(MARKER).not.toMatch(/^\s*width:\s*\d*\.?\d+(em|rem|px)/m)
    expect(MARKER).not.toContain('aspect-ratio')
  })

  it('forbids the icon from being shrunk by flex', () => {
    // The icon is a flex item; without this it is the first thing squeezed
    // out when anything constrains the box, and it goes to zero silently.
    expect(ICON).toContain('flex: none')
  })

  it('paints the colour on the background with a white icon', () => {
    // A tinted stroke on the page background failed contrast at this size.
    expect(MARKER).toContain('background-color: var(--mbr-review-note-color')
    expect(MARKER).toMatch(/color:\s*#fff/)
  })

  it('gives every type a background colour, not a foreground one', () => {
    for (const type of ['issue', 'suggestion', 'praise', 'question', 'insight']) {
      const at = THEME.indexOf(`.mbr-review-marker[data-mbr-review-type="${type}"] {`)
      expect(at).toBeGreaterThan(-1)
      const rule = THEME.slice(at, THEME.indexOf('}', at))
      expect(rule).toContain(`background-color: var(--mbr-review-${type}-color`)
    }
  })
})
