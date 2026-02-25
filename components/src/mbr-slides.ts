/**
 * Reveal.js slides component with Play button.
 *
 * When a page has the "slides" class (set by `style: slides` frontmatter),
 * this component displays a "Play Slides" button in the nav bar.
 * Clicking the button transforms the page into a Reveal.js presentation.
 *
 * Detection: <body class="slides">
 *
 * Transformation (on button click):
 * 1. Changes body class from "slides" to "slides-container"
 * 2. Loads Reveal.js CSS, JS, and notes plugin
 * 3. Removes <hr> elements (slide separators in markdown become DOM sections)
 * 4. Transforms triple blockquotes (>>>) into speaker notes (<aside class="notes">)
 * 5. Wraps sections in .reveal > .slides structure
 * 6. Initializes Reveal.js with notes plugin
 */
import { LitElement, html, css, nothing } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { waitForDom, loadScript, loadCss, getMbrAssetBase, scheduleIdleTask } from './dynamic-loader.ts'

interface RevealPlugin {
  id: string
}

interface RevealApi {
  initialize: (config: Record<string, unknown>) => Promise<RevealApi>
  on: RevealOn;
  off: RevealOff;
  getCurrentSlide(): Element;
  isReady?(): boolean;
}

interface WindowWithReveal extends Window {
  Reveal?: RevealApi
  RevealNotes?: RevealPlugin
}

export interface RevealReadyEvent {
  currentSlide: HTMLElement;
  indexh: number;
  indexv: number;
}

export interface RevealSlideChangedEvent {
  previousSlide: HTMLElement | null;
  currentSlide: HTMLElement;
  indexh: number;
  indexv: number;
}

export interface RevealResizeEvent {
  scale: number;
  oldScale: number;
  size: { width: number; height: number };
}

export interface RevealEventMap {
  ready: RevealReadyEvent;
  slidechanged: RevealSlideChangedEvent;
  slidetransitionend: RevealSlideChangedEvent;
  resize: RevealResizeEvent;

  // If you use more events (fragments, overview, autoslide, etc),
  // add them here as you need them.
  [eventName: string]: any;
}

// ---- Reveal.on / Reveal.off signatures ----

export type RevealOn = {
  <K extends keyof RevealEventMap>(
    type: K,
    listener: (event: RevealEventMap[K]) => void,
    options?: boolean | AddEventListenerOptions
  ): void;

  // Fallback for any custom/untyped events
  (type: string, listener: (event: any) => void, options?: boolean | AddEventListenerOptions): void;
};

export type RevealOff = RevealOn;

@customElement('mbr-slides')
export class MbrSlidesElement extends LitElement {
  @state()
  private _isSlideDocument = false

  @state()
  private _isPlaying = false

  @state()
  private _isLoading = false

  private _deck: RevealApi | null = null

  private _slidecontainer: HTMLElement | null = null

  static override styles = css`
    :host {
      display: contents;
    }

    .play-slides-btn {
      display: flex;
      align-items: center;
      gap: 0.4rem;
      padding: 0.35rem 0.7rem;
      background: var(--pico-primary-background, #1095c1);
      color: var(--pico-primary-inverse, #fff);
      border: none;
      border-radius: 4px;
      cursor: pointer;
      font-size: 0.85rem;
      font-weight: 500;
      transition: background 0.15s ease, transform 0.1s ease;
      white-space: nowrap;
    }

    .play-slides-btn:hover {
      background: var(--pico-primary-hover-background, #0d7a9c);
      transform: translateY(-1px);
    }

    .play-slides-btn:active {
      transform: translateY(0);
    }

    .play-slides-btn:disabled {
      opacity: 0.7;
      cursor: wait;
    }

    .play-icon {
      width: 0;
      height: 0;
      border-left: 8px solid currentColor;
      border-top: 5px solid transparent;
      border-bottom: 5px solid transparent;
      margin-right: 2px;
    }

    .loading-spinner {
      width: 14px;
      height: 14px;
      border: 2px solid currentColor;
      border-top-color: transparent;
      border-radius: 50%;
      animation: spin 0.8s linear infinite;
    }

    @keyframes spin {
      to { transform: rotate(360deg); }
    }
  `

  private _boundKeyHandler: ((e: KeyboardEvent) => void) | null = null

  override connectedCallback() {
    super.connectedCallback()
    waitForDom().then(() => {
      this._isSlideDocument = document.body.classList.contains('slides')

      // Auto-start presentation for speaker notes windows and multiplex receivers
      // - data-speaker-layout: Reveal.js speaker notes popup
      // - ?receiver: Reveal.js multiplex receiver window
      const isSpeakerLayout = document.body.hasAttribute('data-speaker-layout')
      const isReceiver = window.location.search.includes('receiver')
      if (this._isSlideDocument && (isSpeakerLayout || isReceiver)) {
        this._startPresentation()
      }

      // Add keyboard shortcut for slides documents
      if (this._isSlideDocument) {
        this._boundKeyHandler = this._handleKeyPress.bind(this)
        document.addEventListener('keydown', this._boundKeyHandler)
      }
    }).catch(err => console.error('[mbr-slides] Error:', err))
  }

  override disconnectedCallback() {
    super.disconnectedCallback()
    if (this._boundKeyHandler) {
      document.removeEventListener('keydown', this._boundKeyHandler)
      this._boundKeyHandler = null
    }
  }

  private _handleKeyPress(e: KeyboardEvent) {
    // Don't trigger if user is typing in an input or textarea
    const target = e.target as HTMLElement
    if (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable) {
      return
    }

    // Press 'p' to start presentation
    if (e.key === 'p' || e.key === 'P') {
      if (!this._isPlaying && !this._isLoading) {
        e.preventDefault()
        this._startPresentation()
      }
    }
  }

  private _updateScrolling() {
    const slide = this._deck?.getCurrentSlide();
    if (slide) {
      this._slidecontainer?.classList.remove('scrollable-slide')

      // If content is taller than the slide, enable scrolling
      // This is a backup since text should shrink, but in some cases that doesn't work right
      if (slide.scrollHeight > (this._slidecontainer?.clientHeight ?? 0)) {
        this._slidecontainer?.classList.add('scrollable-slide')
      }
    }
  }

  private async _startPresentation() {
    if (this._isPlaying || this._isLoading) return

    this._isLoading = true

    try {
      // Transform body class to avoid conflict with Reveal.js .slides wrapper
      document.body.classList.remove('slides')
      document.body.classList.add('slides-container')

      const assetBase = getMbrAssetBase()

      // Load Reveal.js CSS, JS, and notes plugin
      await Promise.all([
        loadCss(`${assetBase}reveal.css`),
        loadCss(`${assetBase}reveal-theme-blank.css`),
        loadCss(`${assetBase}reveal-slides.css`),
        loadScript(`${assetBase}reveal.js`),
        loadScript(`${assetBase}reveal-notes.js`),
      ])

      const win = window as WindowWithReveal

      // Poll for Reveal.js to be available (script execution may be async)
      let attempts = 0
      while (!win.Reveal && attempts < 50) {
        await new Promise(resolve => setTimeout(resolve, 10))
        attempts++
      }

      if (!win.Reveal) {
        console.error('[mbr-slides] Reveal.js failed to load')
        this._isLoading = false
        return
      }

      this._transformDom()
      this._isPlaying = true

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
          this._deck = win.Reveal!
          this._deck.on('ready', (_e) => this._updateScrolling());
          this._deck.on('slidetransitionend', (_e) => this._updateScrolling());
          this._deck.on('slidechanged', (_e) => this._updateScrolling());
          this._deck.on('resize', () => this._updateScrolling());
          this._updateScrolling();
        } catch (initError) {
          console.error('[mbr-slides] Reveal.js initialization failed:', initError)
        }
      })
    } catch (loadError) {
      console.error('[mbr-slides] Failed to load assets:', loadError)
      // Restore original class on error
      document.body.classList.remove('slides-container')
      document.body.classList.add('slides')
    } finally {
      this._isLoading = false
    }
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

    // Hide nav/breadcrumbs/footer/sidebar in slides mode
    document.querySelector('nav')?.setAttribute('style', 'display: none')
    document.querySelector('.breadcrumbs')?.setAttribute('style', 'display: none')
    document.querySelector('footer')?.setAttribute('style', 'display: none')
    document.querySelector('mbr-browse-single')?.setAttribute('style', 'display: none')

    // Remove sidebar layout class so it doesn't reserve space
    document.body.classList.remove('mbr-has-sidebar')

    // Dispatch event so browser components can close any open panels
    window.dispatchEvent(new CustomEvent('mbr-slides-start'))

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
    this._slidecontainer = slidesDiv

    document.body.classList.add('reveal-viewport')
  }

  override render() {
    // Only show button if this is a slides document and not already playing
    if (!this._isSlideDocument || this._isPlaying) {
      return nothing
    }

    return html`
      <button
        class="play-slides-btn"
        @click=${this._startPresentation}
        ?disabled=${this._isLoading}
        title="Start presentation mode (P)"
      >
        ${this._isLoading
        ? html`<span class="loading-spinner"></span>`
        : html`<span class="play-icon"></span>`
      }
        <span>${this._isLoading ? 'Loading...' : 'Play Slides (P)'}</span>
      </button>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-slides': MbrSlidesElement
  }
}
