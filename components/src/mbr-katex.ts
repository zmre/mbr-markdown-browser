/**
 * KaTeX math rendering dynamic loader component.
 *
 * Scans the page for math elements and dynamically loads KaTeX CSS and JS
 * only when math content is detected. Renders both inline and display math.
 *
 * Detection: .math-inline or .math-display elements
 */
import { LitElement, nothing } from 'lit'
import { customElement } from 'lit/decorators.js'
import { waitForDom, loadScript, loadCss, getMbrAssetBase, scheduleIdleTask } from './dynamic-loader.ts'

/** KaTeX render options type */
interface KatexRenderOptions {
  displayMode: boolean
  throwOnError: boolean
}

/** Window with KaTeX global */
interface WindowWithKatex extends Window {
  katex?: {
    render: (tex: string, element: Element, options: KatexRenderOptions) => void
  }
}

@customElement('mbr-katex')
export class MbrKatexElement extends LitElement {
  private _initialized = false

  override connectedCallback() {
    super.connectedCallback()
    waitForDom().then(() => this._enhance())
  }

  private async _enhance() {
    // Prevent double initialization
    if (this._initialized) return
    this._initialized = true

    // Find math elements
    const mathElements = document.querySelectorAll('.math-inline, .math-display')
    if (mathElements.length === 0) return

    const assetBase = getMbrAssetBase()

    // Load CSS and JS in parallel from embedded assets
    await Promise.all([
      loadCss(`${assetBase}katex.min.css`),
      loadScript(`${assetBase}katex.min.js`),
    ])

    const katex = (window as WindowWithKatex).katex
    if (!katex) {
      console.warn('KaTeX failed to load')
      return
    }

    // Render all math elements in idle time to avoid blocking main thread
    scheduleIdleTask(() => {
      mathElements.forEach((el) => {
        const tex = (el.textContent || '').trim()
        if (!tex) return

        const isDisplay = el.classList.contains('math-display')
        try {
          katex.render(tex, el, {
            displayMode: isDisplay,
            throwOnError: false,
          })
        } catch (e) {
          console.warn('KaTeX render error:', e)
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
