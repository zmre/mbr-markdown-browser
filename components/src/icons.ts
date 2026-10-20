/**
 * mbr's icons: one set (Lucide, ISC — notices in
 * `crates/mbr-core/templates/icons/LICENSE`), one wrapper, three ways to draw.
 *
 * - {@link icon} — a Lit template, for `render()` methods.
 * - {@link iconElement} — (from `icon-dom.ts`) a fresh `SVGSVGElement`, for imperative DOM code
 *   (the editor chrome, review markers). Lit also accepts it in a child
 *   expression, but a new node per render makes Lit replace it every time, so
 *   render methods use {@link icon}.
 * - {@link iconMarkup} — (from `icon-dom.ts`) the same `<svg>` as a string, for `innerHTML`.
 *
 * The shapes are `icon-shapes.ts`, generated from
 * `crates/mbr-core/templates/icons/*.svg` — the same files the Tera `icon()`
 * function reads, so a template and a component cannot draw different
 * artwork for one name.
 *
 * # Why inline SVG
 *
 * `review/` tried CSS `mask-image` icons three times; each failed silently in
 * the browser (a solid block or nothing, no error anywhere). An inline `<svg>`
 * has no URI to encode and inherits its colour from `currentColor`. It also
 * adds nothing to `textContent`, and `svg` is in `find-in-page.ts`'s
 * `BLOCKED_SELECTOR`, so an icon is never selected, copied or searched.
 *
 * # Stroke weight
 *
 * Never an attribute: the inline `style` reads `--mbr-icon-stroke` (theme.css,
 * default 1.75), so one token sets the weight everywhere — custom properties
 * inherit into shadow roots, where theme.css rules cannot reach.
 *
 * Pure: no state beyond a parse cache, safe for lazy chunks to import.
 */
import { html, type TemplateResult } from 'lit'
import { unsafeSVG } from 'lit/directives/unsafe-svg.js'
import { ICON_MD, STROKE_STYLE, iconClass, type IconOptions } from './icon-dom.ts'

export * from './icon-dom.ts'

/** Lit template for `shape` (an `icon…` export), decorative (`aria-hidden`). */
export function icon(shape: string, options: IconOptions = {}): TemplateResult {
  const size = options.size ?? ICON_MD
  return html`<svg class=${iconClass(options)} viewBox="0 0 24 24" width=${size} height=${size} fill=${options.filled ? 'currentColor' : 'none'} stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" style=${STROKE_STYLE} aria-hidden="true" focusable="false">${unsafeSVG(shape)}</svg>`
}
