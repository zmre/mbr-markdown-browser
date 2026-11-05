/**
 * Pure half of `<mbr-katex>`: which elements hold math, how each one's TeX is
 * obtained, and the render loop. Kept apart from the element so it can be
 * tested under happy-dom with stand-ins for KaTeX and the AsciiMath converter,
 * neither of which the tests load.
 *
 * The renderer (`html.rs`) emits three shapes:
 * - `<span class="math math-inline">` for `$…$`
 * - `<span class="math math-display">` for `$$…$$`
 * - `<div class="math math-display">` for a ```` ```latex ````/`katex`/`math`
 *   fence, plus `math-asciimath` for an ```` ```asciimath ```` fence, whose
 *   body is AsciiMath and has to be converted to TeX first
 */

/** Every element the renderer emits for math, rendered or not. */
export const MATH_SELECTOR = '.math-inline, .math-display'

/** The class an ```` ```asciimath ```` fence carries beside `math-display`. */
export const ASCIIMATH_CLASS = 'math-asciimath'

/**
 * Set on an element once it has been handled: `rendered` when KaTeX drew it,
 * `source` when it was left as its source text (an AsciiMath block without a
 * converter, or one the converter refused). Either way it is never touched
 * again — KaTeX replaces the element's children, so a second pass would read
 * KaTeX's own text back as TeX.
 */
export const MATH_STATE_ATTR = 'data-mbr-math'

/** The subset of KaTeX's API used here. */
export interface KatexLike {
  render: (tex: string, element: Element, options: { displayMode: boolean; throwOnError: boolean }) => void
}

/** An asciimath2tex parser instance: AsciiMath in, TeX out. */
export interface AsciiMathConverter {
  parse: (asciimath: string) => string
}

/** The global the vendored asciimath2tex UMD build defines. */
interface WindowWithAsciiMath {
  AsciiMathParser?: new () => AsciiMathConverter
}

/** Math elements in `root` that have not been handled yet, in document order. */
export function pendingMathElements(root: ParentNode = document): Element[] {
  return Array.from(root.querySelectorAll(MATH_SELECTOR)).filter((el) => !el.hasAttribute(MATH_STATE_ATTR))
}

/** True when any of `elements` is an AsciiMath block, i.e. the converter is needed. */
export function hasAsciiMath(elements: readonly Element[]): boolean {
  return elements.some((el) => el.classList.contains(ASCIIMATH_CLASS))
}

/**
 * A converter from the asciimath2tex global, or `null` (with a warning) when
 * the script ran but did not define it, or the constructor throws.
 */
export function createAsciiMathConverter(win: Window = window): AsciiMathConverter | null {
  const Parser = (win as unknown as WindowWithAsciiMath).AsciiMathParser
  if (typeof Parser !== 'function') {
    console.warn('asciimath2tex loaded but AsciiMathParser is missing; AsciiMath blocks stay as source')
    return null
  }
  try {
    return new Parser()
  } catch (e) {
    console.warn('asciimath2tex failed to initialize; AsciiMath blocks stay as source:', e)
    return null
  }
}

/**
 * The TeX to hand KaTeX for `element`, or `null` to leave it alone.
 *
 * TeX elements give their trimmed text. An AsciiMath block is converted as one
 * expression (AsciiMath has no line breaks: a newline is whitespace, as in
 * MathJax's AsciiMath input); `null` when there is no converter or it throws,
 * so the reader keeps the source rather than a blank or a KaTeX error.
 */
export function texFor(element: Element, converter: AsciiMathConverter | null): string | null {
  const source = (element.textContent || '').trim()
  if (!source) return null
  if (!element.classList.contains(ASCIIMATH_CLASS)) return source
  if (!converter) return null
  try {
    return converter.parse(source)
  } catch (e) {
    console.warn('AsciiMath conversion error:', e)
    return null
  }
}

/**
 * Renders every element in `elements` with KaTeX, marking each with
 * {@link MATH_STATE_ATTR} first so it is never rendered twice. Returns how many
 * KaTeX drew.
 */
export function renderMathElements(
  elements: readonly Element[],
  katex: KatexLike,
  converter: AsciiMathConverter | null
): number {
  let rendered = 0
  const unconverted = elements.filter((el) => !el.hasAttribute(MATH_STATE_ATTR) && el.classList.contains(ASCIIMATH_CLASS))
  if (!converter && unconverted.length > 0) {
    console.warn(`AsciiMath converter unavailable; ${unconverted.length} AsciiMath block(s) left as source`)
  }
  for (const el of elements) {
    if (el.hasAttribute(MATH_STATE_ATTR)) continue
    const tex = texFor(el, converter)
    if (tex === null) {
      el.setAttribute(MATH_STATE_ATTR, 'source')
      continue
    }
    el.setAttribute(MATH_STATE_ATTR, 'rendered')
    try {
      katex.render(tex, el, {
        displayMode: el.classList.contains('math-display'),
        throwOnError: false,
      })
      rendered++
    } catch (e) {
      console.warn('KaTeX render error:', e)
    }
  }
  return rendered
}
