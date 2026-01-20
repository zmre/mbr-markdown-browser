/**
 * Reveal.js slides dynamic loader component.
 *
 * Transforms markdown pages into Reveal.js presentations when the body has
 * the "slides" class (set by `style: slides` frontmatter).
 *
 * Detection: <body class="slides">
 *
 * Transformation:
 * 1. Changes body class from "slides" to "slides-container" (avoids conflict with Reveal.js .slides wrapper)
 * 2. Loads Reveal.js CSS, JS, and notes plugin
 * 3. Removes <hr> elements (slide separators in markdown become DOM sections)
 * 4. Transforms marginalia (>>>) into speaker notes (<aside class="notes">)
 * 5. Wraps sections in .reveal > .slides structure
 * 6. Initializes Reveal.js with notes plugin
 */
import { LitElement, nothing } from 'lit'
import { customElement } from 'lit/decorators.js'
import { waitForDom, loadScript, loadCss, getMbrAssetBase, scheduleIdleTask } from './dynamic-loader.ts'

interface RevealPlugin {
  id: string
}

interface WindowWithReveal extends Window {
  Reveal?: {
    initialize: (config: Record<string, unknown>) => Promise<void>
  }
  RevealNotes?: RevealPlugin
}

@customElement('mbr-slides')
export class MbrSlidesElement extends LitElement {
  private _initialized = false

  override connectedCallback() {
    super.connectedCallback()
    waitForDom().then(() => this._enhance()).catch(err => console.error('[mbr-slides] Error in enhance:', err))
  }

  private async _enhance() {
    if (this._initialized) return
    if (!document.body.classList.contains('slides')) return

    this._initialized = true

    // Transform body class to avoid conflict with Reveal.js .slides wrapper
    document.body.classList.remove('slides')
    document.body.classList.add('slides-container')

    const assetBase = getMbrAssetBase()

    // Load Reveal.js CSS, JS, and notes plugin
    try {
      await Promise.all([
        loadCss(`${assetBase}reveal.css`),
        loadCss(`${assetBase}reveal-theme-black.css`),
        loadCss(`${assetBase}reveal-slides.css`),
        loadScript(`${assetBase}reveal.js`),
        loadScript(`${assetBase}reveal-notes.js`),
      ])
    } catch (loadError) {
      console.error('[mbr-slides] Failed to load assets:', loadError)
      return
    }

    const win = window as WindowWithReveal

    // Poll for Reveal.js to be available (script execution may be async)
    let attempts = 0
    while (!win.Reveal && attempts < 50) {
      await new Promise(resolve => setTimeout(resolve, 10))
      attempts++
    }

    if (!win.Reveal) {
      console.error('[mbr-slides] Reveal.js failed to load')
      return
    }

    this._transformDom()

    // Build plugins array (only include loaded plugins)
    const plugins: RevealPlugin[] = []
    if (win.RevealNotes) plugins.push(win.RevealNotes)

    scheduleIdleTask(async () => {
      try {
        await win.Reveal!.initialize({
          hash: true,
          history: false,
          controls: true,
          progress: true,
          center: true,
          transition: 'slide',
          plugins,
        })
      } catch (initError) {
        console.error('[mbr-slides] Reveal.js initialization failed:', initError)
      }
    })
  }

  private _transformDom() {
    const main = document.querySelector('main#wrapper')
    if (!main) {
      console.warn('[mbr-slides] main#wrapper not found')
      return
    }

    // Remove <hr> elements (slide separators become section boundaries)
    main.querySelectorAll('hr').forEach(hr => hr.remove())

    // Transform triple blockquotes (>>>) into speaker notes
    // mbr renders >>> as nested blockquotes: <blockquote><blockquote><blockquote>
    // Reveal.js expects <aside class="notes">
    main.querySelectorAll('blockquote > blockquote > blockquote').forEach(innermost => {
      const middle = innermost.parentElement
      const outer = middle?.parentElement
      if (!outer || outer.tagName !== 'BLOCKQUOTE') return

      // Create speaker notes aside with the innermost content
      const notes = document.createElement('aside')
      notes.className = 'notes'

      // Move all children from innermost blockquote to the notes aside
      while (innermost.firstChild) {
        notes.appendChild(innermost.firstChild)
      }

      // Replace the outermost blockquote with the notes aside
      outer.parentElement?.replaceChild(notes, outer)
    })

    // Hide nav/breadcrumbs/footer in slides mode
    document.querySelector('nav')?.setAttribute('style', 'display: none')
    document.querySelector('.breadcrumbs')?.setAttribute('style', 'display: none')
    document.querySelector('footer')?.setAttribute('style', 'display: none')

    // Create Reveal.js wrapper structure
    const revealDiv = document.createElement('div')
    revealDiv.className = 'reveal'
    const slidesDiv = document.createElement('div')
    slidesDiv.className = 'slides'

    // Move sections into slides container
    main.querySelectorAll('section').forEach(section => {
      slidesDiv.appendChild(section.cloneNode(true))
    })

    revealDiv.appendChild(slidesDiv)
    main.classList.remove('container')
    main.innerHTML = ''
    main.appendChild(revealDiv)
    document.body.classList.add('reveal-viewport')
  }

  override render() {
    return nothing
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-slides': MbrSlidesElement
  }
}
