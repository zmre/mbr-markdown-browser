/**
 * KaTeX math rendering dynamic loader component.
 *
 * Scans the page for math elements and dynamically loads KaTeX CSS and JS
 * only when math content is detected. Renders inline math, `$$` display math
 * and fenced math blocks (```` ```latex ````/`katex`/`math`/`asciimath`).
 *
 * Detection: .math-inline or .math-display elements. The AsciiMath converter
 * (asciimath2tex) is fetched only when a `.math-asciimath` block exists, in
 * parallel with KaTeX; if it fails to load, those blocks keep their source.
 * The selection and conversion logic lives in `math-render.ts`.
 */
import { LitElement, nothing } from 'lit'
import { customElement } from 'lit/decorators.js'
import { waitForDom, loadScript, loadCss, getMbrAssetBase, scheduleIdleTask } from './dynamic-loader.ts'
import {
  type AsciiMathConverter,
  type KatexLike,
  createAsciiMathConverter,
  hasAsciiMath,
  pendingMathElements,
  renderMathElements,
} from './math-render.ts'

/** Window with KaTeX global */
interface WindowWithKatex extends Window {
  katex?: KatexLike
}

/**
 * Loads the vendored asciimath2tex script and returns a converter, or `null`
 * (with a warning) when it cannot be had. Never rejects: a missing converter
 * must not keep KaTeX from rendering the rest of the page.
 */
async function loadAsciiMathConverter(assetBase: string): Promise<AsciiMathConverter | null> {
  try {
    await loadScript(`${assetBase}asciimath2tex.min.js`)
  } catch (e) {
    console.warn('asciimath2tex failed to load; AsciiMath blocks stay as source:', e)
    return null
  }
  return createAsciiMathConverter()
}

/**
 * The initial render in flight, if any: set when the page's math is found,
 * settled once KaTeX (and the AsciiMath converter, if needed) has loaded and
 * the idle-time render pass has run — or anything along the way failed.
 */
let initialRender: Promise<void> | null = null

/**
 * Resolves once the page's math has been rendered, at once when there is none
 * or rendering has not started. Never rejects. The math counterpart of
 * `mermaidSettled()`, with the same audience: anything that copies the page's
 * markup (`<mbr-slides>` deep-clones each slide, the flashcard deck clones
 * cards) must wait, or a clone taken while `katex.min.js` is still loading
 * carries raw TeX while the originals are rendered off-screen.
 */
export function mathSettled(): Promise<void> {
  return initialRender ?? Promise.resolve()
}

@customElement('mbr-katex')
export class MbrKatexElement extends LitElement {
  private _initialized = false

  override connectedCallback() {
    super.connectedCallback()
    waitForDom().then(() => {
      // Claimed synchronously, in the same callback that finds the math, so
      // `mathSettled()` is never asked between the two.
      if (this._initialized) return
      this._initialized = true
      const mathElements = pendingMathElements()
      if (mathElements.length === 0) return
      initialRender = this._enhance(hasAsciiMath(mathElements)).catch((err) => {
        console.warn('[mbr-katex] Error:', err)
      })
    })
  }

  private async _enhance(needsAsciiMath: boolean): Promise<void> {
    const assetBase = getMbrAssetBase()

    // Load KaTeX's CSS and JS, and the AsciiMath converter only if a page
    // needs it, all in parallel from embedded assets.
    const [, converter] = await Promise.all([
      Promise.all([loadCss(`${assetBase}katex.min.css`), loadScript(`${assetBase}katex.min.js`)]),
      needsAsciiMath ? loadAsciiMathConverter(assetBase) : Promise.resolve(null),
    ])

    // Cast via `unknown`: adding @milkdown/crepe brings katex's own global
    // type declaration into scope, which no longer overlaps with this local
    // `WindowWithKatex` shape, so a direct cast is rejected.
    const katex = (window as unknown as WindowWithKatex).katex
    if (!katex) {
      console.warn('KaTeX failed to load')
      return
    }

    // Render in idle time to avoid blocking the main thread. The elements are
    // queried *now*, not the list found at startup: if something cloned the
    // page while KaTeX was loading (slides after their bounded wait ran out),
    // the clones are what is on screen, and they are still unhandled.
    await new Promise<void>((resolve) => {
      scheduleIdleTask(() => {
        try {
          renderMathElements(pendingMathElements(), katex, converter)
        } finally {
          resolve()
        }
      })
    })
  }

  // This component renders nothing - it only loads resources
  override render() {
    return nothing
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-katex': MbrKatexElement
  }
}
