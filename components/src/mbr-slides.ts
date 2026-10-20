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
import { LitElement, html, css, nothing, type PropertyValues } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { waitForDom, loadScript, loadCss, getMbrAssetBase, scheduleIdleTask } from './dynamic-loader.ts'
import { isPlayKey } from './mbr-keys.js'
import { FAB_CSS, PlayFabController } from './fab.ts'
import { ICON_BUTTON_CSS } from './controls.ts'
import { ICON_LG, ICON_SM, icon, iconElement, iconPlay, iconX } from './icons.ts'
import { mermaidSettled } from './mbr-mermaid.ts'

/**
 * How long Play waits for the page's diagrams before cloning the slides
 * anyway. Generous, because a phone can take seconds over a large diagram,
 * but bounded so a stuck render never leaves the button spinning.
 */
const DIAGRAM_WAIT_MS = 8000

/**
 * Dispatched on `document` with `detail: { playing: boolean }`: `true` once
 * the page has been turned into the presentation (the deck is what is on
 * screen), `false` when a start fails and on exit. The iOS app bridge
 * (`app-bridge.ts`) relays it so the native bar steps aside while a deck
 * plays — the slides' own counterpart to `mbr-editor.ts`'s
 * `EDITOR_STATE_EVENT`.
 */
export const SLIDES_STATE_EVENT = 'mbr-slides-state'

/** Tell the app bridge (and anyone else) whether a presentation is playing. */
function announceSlidesState(playing: boolean): void {
  document.dispatchEvent(new CustomEvent(SLIDES_STATE_EVENT, { detail: { playing } }))
}

/** Resolves when `promise` does or after `ms`, whichever is first. */
function withTimeout(promise: Promise<void>, ms: number): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms)
    void promise.then(() => {
      clearTimeout(timer)
      resolve()
    })
  })
}

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

  /** The floating play button's narrow/app gate (`fab.ts`). */
  private _fab = new PlayFabController(this, () => this._showsPlayFab())

  /**
   * The "Exit slides" button, while a presentation plays where the floating
   * form applies. Lives on `<body>`, not in this shadow root: `_transformDom`
   * hides the `<nav>` this element sits in.
   */
  private _exitButton: HTMLButtonElement | null = null

  static override styles = [ICON_BUTTON_CSS, css`
    :host {
      display: contents;
    }

    .play-slides-btn:disabled {
      opacity: 0.7;
      cursor: wait;
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
  `, FAB_CSS]

  private _boundKeyHandler: ((e: KeyboardEvent) => void) | null = null

  override connectedCallback() {
    super.connectedCallback()
    waitForDom().then(() => {
      // The element can be removed again while this promise is pending (it
      // always is, since waitForDom() resolves on a microtask). Registering the
      // document-level key handler from a detached element would leave a
      // listener that disconnectedCallback has already run past, so a removed
      // component would keep hijacking "p" and rewriting the page.
      if (!this.isConnected) return

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
    this._exitButton?.remove()
    this._exitButton = null
    if (this._boundKeyHandler) {
      document.removeEventListener('keydown', this._boundKeyHandler)
      this._boundKeyHandler = null
    }
  }

  private _handleKeyPress(e: KeyboardEvent) {
    // Press 'p' to start presentation. `isPlayKey` carries the shared guards
    // (typing, open overlays, modifiers) — the same key starts flashcards.
    if (isPlayKey(e) && !this._isPlaying && !this._isLoading) {
      e.preventDefault()
      this._startPresentation()
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

  /** Whether this page is a slide deck (`style: slides`). */
  public get isSlideDocument(): boolean {
    return document.body.classList.contains('slides')
  }

  /**
   * Starts the presentation, exactly as `p` and the play button do. Public for
   * the iOS app's menu (`app-bridge.ts`); a no-op on a page that is not a deck.
   */
  public open(): void {
    if (this.isSlideDocument && !this._isPlaying && !this._isLoading) {
      void this._startPresentation()
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
        // `_transformDom` deep-clones the slides, so diagrams must be drawn
        // first: a clone of a block mermaid has not reached yet stays raw
        // source, and mermaid then draws into the detached original.
        withTimeout(mermaidSettled(), DIAGRAM_WAIT_MS),
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
        // Restore the original body class, same as the catch below: without
        // this the page is left as .slides-container with no presentation to
        // fill it, which loses the .slides page styling for good.
        document.body.classList.remove('slides-container')
        document.body.classList.add('slides')
        announceSlidesState(false)
        return
      }

      this._transformDom()
      this._isPlaying = true
      // The deck has replaced the page: announce it now, before Reveal lays
      // out, so the app's bar is already going when Reveal first measures (it
      // re-fits on the resize that follows anyway).
      announceSlidesState(true)

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
            // Reveal 6 switches to its scroll view whenever the viewport is
            // narrower than `scrollActivationWidth` (default 435px), i.e. on
            // every phone held upright. That view re-parents each slide into
            // `.scroll-page` wrappers, moves the backgrounds and reflows the
            // slides, so a deck's own CSS (`.reveal .slides > section`,
            // `section.present`), images and diagrams stop rendering as they
            // do on the desktop -- and `body.slides-container`'s
            // `overflow: hidden` leaves nothing to scroll anyway. Reveal only
            // checks the option when it is a number, so `null` keeps the
            // ordinary scaled 960x700 slides at every width.
            scrollActivationWidth: null,
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
          // No deck to drive: give the app its bar back so the reader can
          // navigate away from the half-transformed page.
          announceSlidesState(false)
        }
      })
    } catch (loadError) {
      console.error('[mbr-slides] Failed to load assets:', loadError)
      // Restore original class on error
      document.body.classList.remove('slides-container')
      document.body.classList.add('slides')
      announceSlidesState(false)
    } finally {
      this._isLoading = false
    }
  }

  /**
   * True when `section` has a `<section>` ancestor below `root`. Walking up to
   * `root` rather than using `closest('section')` keeps the answer independent
   * of whatever wraps `main` in a custom template.
   */
  private _isNestedIn(section: Element, root: Element): boolean {
    let parent = section.parentElement
    while (parent && parent !== root) {
      if (parent.tagName === 'SECTION') return true
      parent = parent.parentElement
    }
    return false
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

    // Move sections into slides container. Only sections that are not already
    // inside another section become slides: a nested section already travels
    // with its ancestor's deep clone (Reveal.js reads it as a vertical slide),
    // so cloning it again would repeat its content as its own horizontal
    // slide. The renderer emits nesting whenever a `---` divider lands inside
    // a blockquote or list item.
    main.querySelectorAll('section').forEach(section => {
      if (this._isNestedIn(section, main)) return
      slidesDiv.appendChild(section.cloneNode(true))
    })

    revealDiv.appendChild(slidesDiv)
    main.classList.remove('container')
    main.innerHTML = ''
    main.appendChild(revealDiv)
    this._slidecontainer = slidesDiv

    document.body.classList.add('reveal-viewport')
  }

  /** A slide deck, not yet playing, where the floating form applies. */
  private _showsPlayFab(): boolean {
    return this._isSlideDocument && !this._isPlaying && this._fab.visible
  }

  override updated(changed: PropertyValues<this>) {
    super.updated(changed)
    this._syncExitButton()
  }

  /**
   * Reveal.js has no way out of a presentation on a touch screen (its `Esc`
   * is the overview), and the transform has replaced the page's content, so
   * where the floating play button applies, a playing deck gets a small exit
   * button in the bottom-left corner — clear of Reveal's own controls at the
   * bottom right, and above `.reveal` and the (hidden) header.
   */
  private _syncExitButton() {
    const wanted = this._isPlaying && this._fab.visible
    if (wanted && !this._exitButton) {
      const button = document.createElement('button')
      button.type = 'button'
      button.className = 'mbr-slides-exit'
      button.append(iconElement(iconX))
      button.setAttribute('aria-label', 'Exit slides')
      button.title = 'Exit slides'
      // Light DOM, so the styles travel inline; theme.css is not ours to rely on.
      button.style.cssText =
        'position:fixed;z-index:1000;left:calc(0.75rem + env(safe-area-inset-left,0px));' +
        'bottom:calc(0.75rem + env(safe-area-inset-bottom,0px));width:44px;height:44px;' +
        'display:flex;align-items:center;justify-content:center;' +
        'margin:0;padding:0;border:none;border-radius:50%;line-height:1;' +
        'background:rgba(0,0,0,0.45);color:#fff;cursor:pointer;opacity:0.7'
      button.addEventListener('click', () => this._exitPresentation())
      document.body.append(button)
      this._exitButton = button
    } else if (!wanted && this._exitButton) {
      this._exitButton.remove()
      this._exitButton = null
    }
  }

  /**
   * Leave the presentation by reloading the page: `_transformDom` replaced
   * `main`'s content with clones, so there is nothing to put back. Reveal's
   * `#/2` slide hash is dropped first, or the reload would land on it.
   */
  protected _exitPresentation() {
    // The reload's fresh page message resets the app anyway; say so first.
    announceSlidesState(false)
    history.replaceState(history.state, '', location.pathname + location.search)
    location.reload()
  }

  override render() {
    // Only show button if this is a slides document and not already playing
    if (!this._isSlideDocument || this._isPlaying) {
      return nothing
    }

    const glyph = this._isLoading
      ? html`<span class="loading-spinner"></span>`
      : icon(iconPlay, { size: ICON_SM })

    return html`
      <button
        class="play-slides-btn mbr-icon-btn labeled"
        @click=${this._startPresentation}
        ?disabled=${this._isLoading}
        aria-label="Play slides"
        title="Play slides (P)"
      >
        ${glyph}
        <span>${this._isLoading ? 'Loading…' : 'Play'}</span>
      </button>
      ${this._showsPlayFab()
        ? html`<button
            type="button"
            class="mbr-fab play-fab"
            @click=${this._startPresentation}
            ?disabled=${this._isLoading}
            aria-label="Play slides"
            title="Play slides (P)"
          >${this._isLoading
            ? html`<span class="loading-spinner"></span>`
            : icon(iconPlay, { size: ICON_LG, filled: true, className: 'play-glyph' })}</button>`
        : nothing}
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-slides': MbrSlidesElement
  }
}
