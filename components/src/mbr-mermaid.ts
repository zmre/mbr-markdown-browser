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

    mermaid?.initialize({
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
    })

    // Manually render the diagrams we found
    mermaid?.run({
      nodes: Array.from(mermaidBlocks) as HTMLElement[]
    })
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
