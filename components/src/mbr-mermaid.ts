/**
 * Mermaid diagram dynamic loader component.
 *
 * Scans the page for mermaid diagram blocks and dynamically loads mermaid.js
 * only when diagrams are detected. Initializes with appropriate theme based
 * on user's color scheme preference.
 *
 * Detection: <pre class="mermaid">, <div class="mermaid">, or <code class="language-mermaid">
 */
import { LitElement, nothing } from 'lit'
import { customElement } from 'lit/decorators.js'
import { waitForDom, loadScript, getMbrAssetBase } from './dynamic-loader.ts'

/** Mermaid sanitizer levels, strictest last-resort first. */
type MermaidSecurityLevel = 'strict' | 'antiscript' | 'loose' | 'sandbox'

/** Mermaid initialization options type */
interface MermaidConfig {
  startOnLoad: boolean
  theme: string
  securityLevel: MermaidSecurityLevel
}

/** Options for mermaid.run() */
interface MermaidRunOptions {
  nodes: HTMLElement[]
}

/** Window with mermaid global */
interface WindowWithMermaid extends Window {
  mermaid?: {
    initialize: (config: MermaidConfig) => void
    run: (options: MermaidRunOptions) => Promise<void>
  }
}

/** Shared mermaid.initialize() options, themed by color-scheme preference. */
function mermaidInitOptions(prefersDark: boolean): MermaidConfig {
  return {
    startOnLoad: false,
    theme: prefersDark ? 'dark' : 'default',
    // Diagram source is whatever markdown the repo happens to contain, so pin
    // the sanitizer level rather than inheriting mermaid's implicit default
    // (currently 'strict', but that is a config default that could change in
    // a future major version).
    //
    // 'strict' runs every label through DOMPurify and disables `click`/
    // callback directives, while still allowing the `<br/>` line breaks the
    // diagrams in docs/ rely on. The one stricter level, 'sandbox', re-hosts
    // each diagram in a `data:` URL iframe: that HTML-escapes label markup,
    // drops the page theme/CSS and breaks in-diagram links, so it is not
    // usable here.
    securityLevel: 'strict',
  }
}

@customElement('mbr-mermaid')
export class MbrMermaidElement extends LitElement {
  private _initialized = false

  override connectedCallback() {
    super.connectedCallback()
    waitForDom().then(() => this._enhance())
  }

  private async _enhance() {
    // Prevent double initialization
    if (this._initialized) return
    this._initialized = true

    // Find mermaid diagram blocks
    const mermaidBlocks = document.querySelectorAll(
      'pre.mermaid, div.mermaid, code.language-mermaid'
    )
    if (mermaidBlocks.length === 0) return

    const assetBase = getMbrAssetBase()

    // Load mermaid.js (no CSS needed - it's self-contained)
    await loadScript(`${assetBase}mermaid.min.js`)

    // Initialize mermaid and manually trigger rendering
    // Using startOnLoad: false + explicit run() avoids race conditions
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches
    const mermaid = (window as WindowWithMermaid).mermaid

    // Snapshot each block's outerHTML *before* mermaid mutates it in place, so
    // a print re-render (see below) can rebuild an unprocessed element from
    // markup rather than needing mermaid's own re-run/`data-processed` guard
    // semantics, which are not something this component can rely on.
    const originalBlocks = Array.from(mermaidBlocks) as HTMLElement[]
    const snapshots = originalBlocks.map((el) => el.outerHTML)

    mermaid?.initialize(mermaidInitOptions(prefersDark))

    // Manually render the diagrams we found
    mermaid?.run({
      nodes: originalBlocks,
    })

    // Mermaid bakes its theme's colors into the rendered SVG once, so a
    // prefers-color-scheme: dark render never gets light on paper from CSS
    // alone -- unlike the rest of the page, which pico already keeps light on
    // paper via `only screen` gating (see theme.css). Re-render in the light
    // theme just before printing and restore dark after. Skipped entirely for
    // light-preference users: there is nothing to swap.
    if (prefersDark) {
      let currentElements = originalBlocks

      const rerender = (rerenderPrefersDark: boolean): void => {
        const freshElements = snapshots.map((html) => {
          const container = document.createElement('div')
          container.innerHTML = html
          return container.firstElementChild as HTMLElement
        })
        currentElements.forEach((el, i) => el.replaceWith(freshElements[i]))
        currentElements = freshElements

        mermaid?.initialize(mermaidInitOptions(rerenderPrefersDark))
        mermaid?.run({ nodes: freshElements })
      }

      window.addEventListener('beforeprint', () => rerender(false))
      window.addEventListener('afterprint', () => rerender(true))
    }
  }

  // This component renders nothing - it only loads resources
  override render() {
    return nothing
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-mermaid': MbrMermaidElement
  }
}
