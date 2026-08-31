/**
 * The six note-type icons, as real `<svg>` elements.
 *
 * # Why not CSS
 *
 * These began as `mask-image` data URIs in `templates/theme.css`, next to the
 * GitHub-alert icons that use that technique successfully. Three attempts
 * failed in the real browser — a `mask` shorthand that parsed inconsistently,
 * then a `%23` that truncated the URI, then an encoding matched byte-for-byte
 * against a working icon — each failing **silently**, as either a solid block
 * of the fill colour or an entirely transparent one. Nothing logs, nothing
 * throws, and no assertion over CSS text can tell a mask that renders from one
 * that does not.
 *
 * An inline `<svg>` has none of those failure modes: no URI to encode, no mask
 * to support, and `stroke="currentColor"` inherits the colour straight from
 * CSS. It is also what every other icon in this codebase already does —
 * `mbr-tasks.ts`, `mbr-info.ts` and the rest all inline feather-style SVG.
 *
 * # The marker stays textless
 *
 * An `<svg>` contributes nothing to `textContent`, so a marker still cannot be
 * selected, copied, or pulled into a quote — and `svg` is in `find-in-page.ts`'s
 * `BLOCKED_SELECTOR`, so it never reaches the search index either. That was the
 * whole reason the glyph was generated content before.
 *
 * Framework-free on purpose: the main bundle's marker layer builds DOM
 * imperatively while the panel renders with Lit, and Lit accepts a `Node`
 * directly in a child expression. One implementation serves both.
 */

import type { NoteType } from './types.ts'

const SVG_NS = 'http://www.w3.org/2000/svg'

/**
 * Inner markup for each icon, from Feather (MIT) at a 24x24 viewBox.
 *
 * Shapes only — the wrapper, its stroke settings and its size are applied in
 * {@link createIconSvg}, so all six are guaranteed to match.
 */
export const ICON_SHAPES: Readonly<Record<NoteType, string>> = {
  issue:
    "<path d='M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z'/><line x1='12' y1='9' x2='12' y2='13'/><line x1='12' y1='17' x2='12.01' y2='17'/>",
  suggestion:
    "<path d='M12 20h9'/><path d='M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z'/>",
  note:
    "<path d='M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z'/><polyline points='14 2 14 8 20 8'/><line x1='16' y1='13' x2='8' y2='13'/><line x1='16' y1='17' x2='8' y2='17'/>",
  praise:
    "<polygon points='12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2'/>",
  question:
    "<circle cx='12' cy='12' r='10'/><path d='M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3'/><line x1='12' y1='17' x2='12.01' y2='17'/>",
  insight: "<polygon points='13 2 3 14 12 14 11 22 21 10 12 10 13 2'/>",
}

/** One parsed template per type, cloned per use. */
const cache = new Map<NoteType, SVGSVGElement>()

/**
 * Build the `<svg>` for `type`, or `null` for an unknown one.
 *
 * The returned element is a fresh clone, so callers may insert it anywhere.
 * `stroke="currentColor"` means the colour comes from the `color` property of
 * whatever it is placed in — theme tokens keep working, and dark mode with them.
 */
export function createIconSvg(type: NoteType): SVGSVGElement | null {
  const shapes = ICON_SHAPES[type]
  if (shapes === undefined) return null

  const cached = cache.get(type)
  if (cached) return cached.cloneNode(true) as SVGSVGElement

  const svg = document.createElementNS(SVG_NS, 'svg')
  svg.setAttribute('viewBox', '0 0 24 24')
  svg.setAttribute('fill', 'none')
  svg.setAttribute('stroke', 'currentColor')
  svg.setAttribute('stroke-width', '2')
  svg.setAttribute('stroke-linecap', 'round')
  svg.setAttribute('stroke-linejoin', 'round')
  // Decorative: the marker itself carries the accessible name.
  svg.setAttribute('aria-hidden', 'true')
  svg.setAttribute('focusable', 'false')

  // The shapes are compile-time constants from this module, never user text.
  // Parsed rather than assigned as innerHTML so a malformed shape fails here,
  // in one place, instead of producing a silently empty icon.
  const parsed = new DOMParser().parseFromString(
    `<svg xmlns="${SVG_NS}">${shapes}</svg>`,
    'image/svg+xml'
  )
  if (parsed.querySelector('parsererror') !== null) return null
  for (const shape of Array.from(parsed.documentElement.children)) {
    svg.appendChild(shape.cloneNode(true))
  }

  cache.set(type, svg)
  return svg.cloneNode(true) as SVGSVGElement
}
