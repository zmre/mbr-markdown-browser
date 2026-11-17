/**
 * Highlight.js dynamic loader component.
 *
 * Scans the page for code blocks with language classes and dynamically loads
 * only the HLJS core, CSS, and required language modules. Languages not embedded
 * locally are fetched from CDN.
 *
 * Detection: <code class="language-*"> elements
 */
import { LitElement, nothing } from 'lit'
import { customElement } from 'lit/decorators.js'
import { waitForDom, loadScript, loadCss, getMbrAssetBase } from './dynamic-loader.ts'
import { scheduleRenderTask } from './export-mode.ts'

/** Window with HLJS global */
interface WindowWithHljs extends Window {
  hljs?: {
    highlightAll: () => void
  }
}

/** Languages embedded locally (matches embedded_hljs.rs) */
const LOCAL_LANGUAGES = new Set([
  'bash',
  'css',
  'dockerfile',
  'go',
  'java',
  'javascript',
  'json',
  'markdown',
  'nix',
  'python',
  'ruby',
  'rust',
  'scala',
  'sql',
  'typescript',
  'xml',
  'yaml',
])

/** HLJS version - must match embedded version in embedded_hljs.rs */
const HLJS_VERSION = '11.12.0'

/** CDN base URL for languages not embedded locally */
const CDN_BASE = `https://cdn.jsdelivr.net/gh/highlightjs/cdn-release@${HLJS_VERSION}/build`

/**
 * The initial highlight in flight, if any: set when the page's code blocks are
 * found, settled once hljs and its languages have loaded and the highlight
 * pass has run (or anything along the way failed).
 */
let initialRender: Promise<void> | null = null

/**
 * Resolves once the page's code has been highlighted, at once when there is
 * none or highlighting has not started. Never rejects. The hljs counterpart of
 * `mermaidSettled()` / `mathSettled()`, for the PDF export's readiness wait
 * (`export-ready.ts`).
 */
export function hljsSettled(): Promise<void> {
  return initialRender ?? Promise.resolve()
}

@customElement('mbr-hljs')
export class MbrHljsElement extends LitElement {
  private _initialized = false

  override connectedCallback() {
    super.connectedCallback()
    waitForDom().then(() => {
      // Claimed synchronously, in the same callback that finds the blocks, so
      // `hljsSettled()` is never asked between the two.
      if (this._initialized) return
      this._initialized = true

      // Find all code blocks with language classes
      const codeBlocks = document.querySelectorAll('code[class*="language-"]')
      if (codeBlocks.length === 0) return

      // Extract unique languages needed
      const languages = this._extractLanguages(codeBlocks)
      if (languages.size === 0) return

      initialRender = this._enhance(languages).catch((e) => console.warn('hljs failed to load:', e))
    })
  }

  private async _enhance(languages: Set<string>) {
    const assetBase = getMbrAssetBase()

    // Step 1: Load CSS and core HLJS in parallel (CSS doesn't depend on JS)
    await Promise.all([
      loadCss(`${assetBase}hljs.atom-one-dark.css`),
      loadScript(`${assetBase}hljs.js`),
    ])

    // Step 2: NOW load language files (window.hljs is defined)
    // Language files call hljs.registerLanguage() on load, so core must be loaded first
    const langLoads = [...languages].map((lang) =>
      LOCAL_LANGUAGES.has(lang)
        ? loadScript(`${assetBase}hljs.lang.${lang}.js`)
        : loadScript(`${CDN_BASE}/languages/${lang}.min.js`)
    )
    const results = await Promise.allSettled(langLoads)
    for (const [i, result] of results.entries()) {
      if (result.status === 'rejected') {
        const lang = [...languages][i]
        console.warn(`hljs: failed to load language "${lang}"`, result.reason)
      }
    }

    // Step 3: Highlight during idle time to avoid blocking the main thread
    // (at once in a PDF export, where idle time comes late).
    await new Promise<void>((resolve) => {
      scheduleRenderTask(() => {
        try {
          const hljs = (window as WindowWithHljs).hljs
          hljs?.highlightAll()
        } finally {
          resolve()
        }
      })
    })
  }

  /**
   * Extract unique language names from code block class attributes.
   * Handles both "language-X" and "lang-X" patterns.
   */
  private _extractLanguages(codeBlocks: NodeListOf<Element>): Set<string> {
    const langs = new Set<string>()
    codeBlocks.forEach((el) => {
      // Match language-X or lang-X patterns
      const match = el.className.match(/(?:language|lang)-(\w+)/)
      if (match) {
        langs.add(match[1])
      }
    })
    return langs
  }

  // This component renders nothing - it only loads resources
  override render() {
    return nothing
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-hljs': MbrHljsElement
  }
}
