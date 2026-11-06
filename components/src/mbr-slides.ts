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
 * 5. Wraps sections in .reveal > .slides structure, loads every image eagerly
 *    and groups a `.kanban` section's lists into columns
 * 6. Initializes Reveal.js with notes plugin
 *
 * While the deck plays, the slide on screen is fitted into Reveal's box
 * (`slide-fit.ts`), `q` and a corner ✕ leave (by reload), and the URL carries
 * Reveal's `#/N` slide hash — `#/0` on the first slide too — so a page loaded
 * with one (a live reload, a manual reload) goes straight back to that slide.
 */
import { LitElement, html, css, nothing, type PropertyValues } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { waitForDom, loadScript, loadCss, getMbrAssetBase, scheduleIdleTask, withTimeout } from './dynamic-loader.ts'
import { isInputTarget, isModalOpen, isPlayKey } from './mbr-keys.js'
import { FAB_CSS, PlayFabController } from './fab.ts'
import { ICON_BUTTON_CSS } from './controls.ts'
import { ICON_LG, ICON_SM, icon, iconElement, iconPlay, iconX } from './icons.ts'
import { mermaidSettled } from './mbr-mermaid.ts'
import { mathSettled } from './mbr-katex.ts'
import { fitSlide } from './slide-fit.ts'
import { getSlideSettingSources } from './shared.ts'
import { applyDefaultBackground, deckSettings, revealConfig } from './slide-settings.ts'

/**
 * How long Play waits for the page's diagrams and math before cloning the slides
 * anyway. Generous, because a phone can take seconds over a large diagram,
 * but bounded so a stuck render never leaves the button spinning.
 */
const DIAGRAM_WAIT_MS = 8000

/** Reveal's slide height when the config does not say (its default). */
const REVEAL_SLIDE_HEIGHT = 700

/**
 * Reveal writes the slide hash at most once a second, deferring a write that
 * comes sooner; the first-slide hash is re-checked just after that window.
 */
const REVEAL_HASH_WRITE_WINDOW_MS = 1100

/** `keyCode` of `q`: Reveal's key bindings are keyed by it. */
const EXIT_KEY_CODE = 81

/**
 * True for the URL fragment Reveal.js writes for a slide with `hash: true`:
 * `#/2`, `#/2/1`, `#/2/1/0` (with a fragment index) or `#/intro` (a slide with
 * an id). Heading anchors (`#text-and-an-image`) never start with `/`, task and
 * marker anchors are `#mbr-…`, so a page loaded with one of these was a
 * presentation when it was (re)loaded.
 */
export function isSlideHash(hash: string): boolean {
  return /^#\/[^\s/]*(\/[^\s/]*){0,2}$/.test(hash) && hash.length > 2
}

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

interface RevealPlugin {
  id: string
}

interface RevealKeyBinding {
  keyCode: number
  key: string
  description: string
}

interface RevealApi {
  initialize: (config: Record<string, unknown>) => Promise<RevealApi>
  on: RevealOn;
  off: RevealOff;
  getCurrentSlide(): Element;
  isReady?(): boolean;
  layout?(): void;
  getConfig?(): { height?: number | string };
  addKeyBinding?(binding: RevealKeyBinding, callback: () => void): void;
  getIndices?(): { h: number; v?: number };
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
   * The "Exit slides" button, while a presentation plays. Lives on `<body>`,
   * not in this shadow root: `_transformDom` hides the `<nav>` this element
   * sits in. Styled by `reveal-slides.css`, which is loaded before it exists.
   */
  private _exitButton: HTMLButtonElement | null = null

  /** The deferred first-slide hash check (`_keepFirstSlideHash`). */
  private _hashTimer = 0

  /** Set once leaving has begun: nothing may write a slide hash after it. */
  private _exiting = false

  /** Set while a fit runs: its `layout()` must not start another. */
  private _fitting = false

  /**
   * The speaker-notes window and its multiplex receivers show this page too
   * (auto-started); they get no exit button and no `q`, which would reload
   * one pane of the speaker view out of the presentation.
   */
  private _isPresenterView = false

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
      // - a Reveal slide hash (`#/3`): the page was reloaded (live reload
      //   after an edit, or by hand) while the deck was playing, so it comes
      //   back playing, on the same slide. `_exitPresentation` strips the hash.
      const isSpeakerLayout = document.body.hasAttribute('data-speaker-layout')
      const isReceiver = window.location.search.includes('receiver')
      this._isPresenterView = isSpeakerLayout || isReceiver
      if (this._isSlideDocument && (this._isPresenterView || isSlideHash(window.location.hash))) {
        void this._startPresentation()
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
    clearTimeout(this._hashTimer)
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

  /**
   * Fit the slide on screen into Reveal's box (`slide-fit.ts`: cap images,
   * then shrink text), then have Reveal lay out again so it centres the
   * result: its `top` was computed from the unfitted height. A slide that
   * cannot fit even at the smallest text scrolls instead.
   */
  private _fitCurrentSlide() {
    const deck = this._deck
    const slide = deck?.getCurrentSlide()
    if (!deck || !(slide instanceof HTMLElement) || this._fitting) return
    this._fitting = true
    try {
      const height = Number(deck.getConfig?.().height) || REVEAL_SLIDE_HEIGHT
      const fits = fitSlide(slide, height)
      deck.layout?.()
      this._slidecontainer?.classList.toggle('scrollable-slide', !fits)
    } finally {
      this._fitting = false
    }
  }

  /**
   * Keep `#/0` in the URL on the first slide. Reveal writes no hash there at
   * all (only `#/1`, `#/2`… after it), so a reload on the title slide would
   * come back as the page rather than the presentation. Reveal's URL writer
   * is internal and its writes are debounced, so this runs after each slide
   * change and once more after Reveal's deferred write could have landed;
   * either way the hash is only added when Reveal left none.
   */
  private _keepFirstSlideHash() {
    const write = () => {
      if (this._exiting) return
      const indices = this._deck?.getIndices?.()
      if (indices && indices.h === 0 && !indices.v && !isSlideHash(location.hash)) {
        history.replaceState(history.state, '', `${location.pathname}${location.search}#/0`)
      }
    }
    write()
    clearTimeout(this._hashTimer)
    this._hashTimer = window.setTimeout(write, REVEAL_HASH_WRITE_WINDOW_MS)
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
        // `_transformDom` deep-clones the slides, so diagrams and math must be
        // drawn first: a clone of a block mermaid or KaTeX has not reached yet
        // stays raw source, and they then draw into the detached original.
        // One bound for both. If it runs out, KaTeX still renders the clones:
        // its render pass queries the page when it runs, not at startup.
        withTimeout(Promise.all([mermaidSettled(), mathSettled()]), DIAGRAM_WAIT_MS),
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

      // Config defaults, then the note's `slides:` frontmatter
      // (`slide-settings.ts`); a slide's own `data-*` attributes still win.
      const { config, page } = getSlideSettingSources()
      const settings = deckSettings(config, page)

      this._transformDom()
      if (this._slidecontainer) applyDefaultBackground(this._slidecontainer, settings)
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
          // `revealConfig` spreads these last: the deck settings choose the
          // look (transition, controls, progress, slide number), never these.
          await win.Reveal!.initialize(revealConfig(settings, {
            hash: true,
            history: false,
            center: true,
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
            // Reveal skips keys typed into a focused input by checking
            // `document.activeElement`, which is only the *host* when the
            // field is in a shadow root -- the review-note form's textarea,
            // the search box. So `g` (jump to slide) stole focus mid-word,
            // and space and the arrows changed slides under the reader.
            // `isInputTarget` looks through shadow roots (composedPath); an
            // open mbr overlay (review form or list, search, tasks) owns the
            // keyboard outright, Escape and Enter included. `q` is one of
            // Reveal's bindings, so this covers it too.
            keyboardCondition: (e: KeyboardEvent) => !isInputTarget(e) && !isModalOpen(),
            plugins,
          }))
          const deck = win.Reveal!
          this._deck = deck
          const fit = () => this._fitCurrentSlide()
          deck.on('ready', fit)
          deck.on('slidechanged', fit)
          deck.on('resize', fit)
          // An image (or a video's first frame) that arrives after its slide
          // was laid out changes the slide's height: re-fit and re-centre.
          // `load` does not bubble, so listen in the capture phase.
          this._slidecontainer?.addEventListener('load', fit, true)
          this._slidecontainer?.addEventListener('loadedmetadata', fit, true)
          if (!this._isPresenterView) {
            deck.addKeyBinding?.(
              { keyCode: EXIT_KEY_CODE, key: 'Q', description: 'Exit slides' },
              () => this._exitPresentation()
            )
            const keepHash = () => this._keepFirstSlideHash()
            deck.on('slidechanged', keepHash)
            keepHash()
          }
          fit()
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

    // mbr renders `loading="lazy"` images. A lazy image on a hidden slide
    // is not fetched until the slide shows, by which time Reveal has
    // centred the slide without it, so it lands low. A presentation wants
    // every picture ready before its slide anyway.
    slidesDiv.querySelectorAll('img[loading="lazy"]').forEach(img => img.removeAttribute('loading'))

    slidesDiv.querySelectorAll('section.kanban').forEach(board => groupKanbanColumns(board))

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
   * Reveal.js has no way out of a presentation (its `Esc` is the overview,
   * and a touch screen has no keys at all), and the transform has replaced
   * the page's content, so a playing deck gets a small exit button in the
   * bottom-left corner, clear of Reveal's own controls at the bottom right.
   * `reveal-slides.css` keeps it nearly invisible until hovered or focused
   * (and plainly visible on a touch screen, which cannot hover).
   */
  private _syncExitButton() {
    const wanted = this._isPlaying && !this._isPresenterView
    if (wanted && !this._exitButton) {
      const button = document.createElement('button')
      button.type = 'button'
      button.className = 'mbr-slides-exit'
      button.append(iconElement(iconX))
      button.setAttribute('aria-label', 'Exit slides')
      button.title = 'Exit slides (Q)'
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
   * `main`'s content with clones, so there is nothing to put back. The
   * reload goes to the URL without Reveal's `#/2` slide hash, or it would
   * land on it and start the presentation again (`isSlideHash`). A
   * navigation, not `replaceState` + `reload()`: Reveal defers a hash write
   * made within a second of the last one, and that timer could put the hash
   * back between the two calls (it did, on `q` right after changing slides).
   * A navigation's URL is fixed when it starts. To a URL without a fragment
   * it is a full load, and `replace` keeps it out of the history.
   */
  protected _exitPresentation() {
    this._exiting = true
    clearTimeout(this._hashTimer)
    // The reload's fresh page message resets the app anyway; say so first.
    announceSlidesState(false)
    location.replace(location.pathname + location.search)
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

/**
 * Group a `--- {.kanban}` slide's content into `.mbr-kanban-column`s inside one
 * `.mbr-kanban-board`, which `reveal-slides.css` lays out as flex columns.
 *
 * On a page the board is CSS multi-column over the section's flat children,
 * with a column break after each top-level list. Inside Reveal's scaled,
 * fixed-size slide that layout overflows and stacks (and the section's own
 * `display` is Reveal's), so the deck groups the same children explicitly: a
 * column runs up to and including the next top-level list, which is where the
 * page breaks it. Speaker notes stay where Reveal looks for them.
 */
export function groupKanbanColumns(section: Element): void {
  const doc = section.ownerDocument
  const board = doc.createElement('div')
  board.className = 'mbr-kanban-board'
  let column: HTMLElement | null = null
  for (const child of Array.from(section.children)) {
    if (child.matches('aside.notes')) continue
    if (!column) {
      column = doc.createElement('div')
      column.className = 'mbr-kanban-column'
      board.append(column)
    }
    column.append(child)
    if (child.tagName === 'UL' || child.tagName === 'OL') column = null
  }
  if (board.childElementCount > 0) section.prepend(board)
}
