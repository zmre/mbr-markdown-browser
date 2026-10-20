/**
 * The Lit-free half of `icons.ts`: icon sizes, the shared wrapper as markup
 * (`iconMarkup`) and as a fresh element (`iconElement`), and every shape.
 *
 * Imperative-DOM code (the editor chunk, breadcrumbs, review markers, the
 * family chart, the pickers) imports this module directly so a bundle that
 * does not otherwise use Lit does not pull it in for an icon. Render methods
 * use `icons.ts`'s `icon()`, which re-exports everything here. Pure: no state
 * beyond a parse cache.
 */
export * from './icon-shapes.ts'

/** Icon sizes in CSS px: `sm` 16, `md` 20, `lg` 24 (and 96 for placeholders). */
export const ICON_SM = 16
export const ICON_MD = 20
export const ICON_LG = 24

export interface IconOptions {
  /** Width and height in CSS px. Default {@link ICON_MD}. */
  readonly size?: number
  /** Extra classes after `mbr-icon` (e.g. `chevron` for a rotating disclosure). */
  readonly className?: string
  /** Fill the shape too (`play` on a FAB). Default outline only. */
  readonly filled?: boolean
}

/** The inline style every icon carries: the weight comes from one CSS token. */
export const STROKE_STYLE = 'stroke-width:var(--mbr-icon-stroke,1.75)'

/** `mbr-icon` plus any extra classes. */
export function iconClass(options: IconOptions): string {
  return options.className ? `mbr-icon ${options.className}` : 'mbr-icon'
}

/** The `<svg>` as markup. `shape` must be an `icon…` export, never user text. */
export function iconMarkup(shape: string, options: IconOptions = {}): string {
  const size = options.size ?? ICON_MD
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" class="${iconClass(options)}" viewBox="0 0 24 24" ` +
    `width="${size}" height="${size}" fill="${options.filled ? 'currentColor' : 'none'}" ` +
    `stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" ` +
    `style="${STROKE_STYLE}" aria-hidden="true" focusable="false">${shape}</svg>`
  )
}

/** One parsed element per distinct markup, cloned per use. */
const cache = new Map<string, SVGSVGElement>()

/**
 * A fresh `<svg>` element for `shape`. Each call returns a new clone, so the
 * caller may insert it anywhere. Parsed as XML, so a malformed shape throws
 * here rather than drawing an empty box.
 */
export function iconElement(shape: string, options: IconOptions = {}): SVGSVGElement {
  const markup = iconMarkup(shape, options)
  let svg = cache.get(markup)
  if (!svg) {
    const doc = new DOMParser().parseFromString(markup, 'image/svg+xml')
    if (doc.querySelector('parsererror') !== null) {
      throw new Error(`icon shape does not parse: ${shape}`)
    }
    svg = doc.documentElement as unknown as SVGSVGElement
    cache.set(markup, svg)
  }
  return svg.cloneNode(true) as SVGSVGElement
}
