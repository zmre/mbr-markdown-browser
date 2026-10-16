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
  layout: string
  look: string
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
    // Mermaid 12 changed two defaults that re-lay out and recolour every
    // existing diagram: the global layout became ELK (from dagre) and most
    // diagram types default to the `neo` look (from `classic`). Pin the 11.x
    // values so upgrading mbr does not silently redraw a repository's diagrams,
    // and because dagre is the cheaper layout on the render path. These sit at
    // the initialize() layer, so a diagram can still opt in per block with
    // `config: { layout: elk, look: neo }` front matter.
    layout: 'dagre',
    look: 'classic',
  }
}

/**
 * The initial render in flight, if any: set when a page's diagrams are found,
 * settled once mermaid has loaded and rendered them (or failed to).
 */
let initialRender: Promise<void> | null = null

/**
 * Resolves once the page's diagrams have been rendered, at once when there
 * are none or rendering has not started. Never rejects.
 *
 * For anything that copies the page's markup: `<mbr-slides>` deep-clones each
 * slide, and a clone taken while `mermaid.min.js` is still loading (a quick
 * Play on a phone) carries the raw diagram source, while the originals are
 * then rendered off-DOM where nobody sees them.
 */
export function mermaidSettled(): Promise<void> {
  return initialRender ?? Promise.resolve()
}

/** Distinguishes one element's diagrams from another's in `data-mbr-mermaid`. */
let nextRunId = 0

@customElement('mbr-mermaid')
export class MbrMermaidElement extends LitElement {
  private _initialized = false

  override connectedCallback() {
    super.connectedCallback()
    waitForDom().then(() => {
      // Claimed synchronously, in the same callback that finds the blocks,
      // so `mermaidSettled()` is never asked between the two.
      if (this._initialized) return
      this._initialized = true
      const blocks = Array.from(
        document.querySelectorAll('pre.mermaid, div.mermaid, code.language-mermaid')
      ) as HTMLElement[]
      if (blocks.length === 0) return
      initialRender = this._enhance(blocks).catch((err) => {
        console.error('[mbr-mermaid] Error:', err)
      })
    })
  }

  private async _enhance(originalBlocks: HTMLElement[]) {
    const assetBase = getMbrAssetBase()

    // Load mermaid.js (no CSS needed - it's self-contained)
    await loadScript(`${assetBase}mermaid.min.js`)

    // Initialize mermaid and manually trigger rendering
    // Using startOnLoad: false + explicit run() avoids race conditions
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches
    const mermaid = (window as WindowWithMermaid).mermaid

    // Tag each block, so the print re-render below finds the diagrams that are
    // on the page *then*: `<mbr-slides>` replaces them with clones, which
    // carry the attribute (mermaid keeps a block's attributes).
    const runId = String(nextRunId++)
    originalBlocks.forEach((el) => el.setAttribute('data-mbr-mermaid', runId))

    // Snapshot each block's outerHTML *before* mermaid mutates it in place, so
    // a print re-render (see below) can rebuild an unprocessed element from
    // markup rather than needing mermaid's own re-run/`data-processed` guard
    // semantics, which are not something this component can rely on.
    const snapshots = originalBlocks.map((el) => el.outerHTML)

    mermaid?.initialize(mermaidInitOptions(prefersDark))

    // Mermaid bakes its theme's colors into the rendered SVG once, so a
    // prefers-color-scheme: dark render never gets light on paper from CSS
    // alone -- unlike the rest of the page, which pico already keeps light on
    // paper via `only screen` gating (see theme.css). Re-render in the light
    // theme just before printing and restore dark after. Skipped entirely for
    // light-preference users: there is nothing to swap.
    if (prefersDark) {
      const rerender = (rerenderPrefersDark: boolean): void => {
        const current = Array.from(
          document.querySelectorAll(`[data-mbr-mermaid="${runId}"]`)
        ) as HTMLElement[]
        const freshElements = current.map((el, i) => {
          const container = document.createElement('div')
          container.innerHTML = snapshots[i] ?? el.outerHTML
          const fresh = container.firstElementChild as HTMLElement
          el.replaceWith(fresh)
          return fresh
        })
        if (freshElements.length === 0) return

        mermaid?.initialize(mermaidInitOptions(rerenderPrefersDark))
        mermaid?.run({ nodes: freshElements })
      }

      window.addEventListener('beforeprint', () => rerender(false))
      window.addEventListener('afterprint', () => rerender(true))
    }

    // Manually render the diagrams we found
    await mermaid?.run({
      nodes: originalBlocks,
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
