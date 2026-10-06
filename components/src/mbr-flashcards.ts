/**
 * `<mbr-flashcards>` — the flashcard trigger (main bundle).
 *
 * On a `type: flashcard` page (`<body class="flashcard">`) with at least one
 * top-level definition list, this renders a "Review flashcards" button in the
 * nav (where `<mbr-slides>` puts "Play Slides") and claims `p`, the same key
 * and guards as slides (`isPlayKey`). Opening it lazy-loads
 * `mbr-flashcards.min.js` and appends a `<mbr-flashcard-deck>` overlay to
 * `<body>`.
 *
 * It also does the one thing every flashcard page needs without the chunk:
 * collapse each card's `___Review History___` into a one-line summary
 * (`flashcards/dom.ts::decorateHistory`). That runs in static builds too.
 *
 * Kept small on purpose — the overlay, FSRS (`ts-fsrs`) and the fitting logic
 * are all in the chunk. What lives here is what the chunk may not hold: the
 * writer (`recordReview`, which shares `task-toggle.ts`'s caches) and the
 * decision whether spaced repetition is available at all.
 */
import { LitElement, css, html, nothing, type TemplateResult } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { getMbrAssetBase, waitForDom } from './dynamic-loader.js'
import { isEditEnabled } from './shared.js'
import { isPlayKey } from './mbr-keys.js'
import { currentDocumentPath } from './task-toggle.js'
import { recordReview } from './flashcard-review.js'
import { decorateAllHistories, hasDeck } from './flashcards/dom.js'
import type { ReviewRecorder } from './flashcards/types.js'
import type { MbrOverlay } from './overlay.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-flashcards': MbrFlashcardsElement
  }
}

/** Body class `type: flashcard` produces (`templates.rs::body_class_list`). */
const FLASHCARD_CLASS = 'flashcard'

/** Event the deck dispatches to be closed (`flashcards/mbr-flashcard-deck.ts`). */
const DECK_CLOSE_EVENT = 'mbr-flashcards-close'

/** The properties the trigger sets on the chunk's deck element. */
interface DeckElement extends HTMLElement {
  root: ParentNode
  recordReview: ReviewRecorder | null
  srsAvailable: boolean
}

/** The page's rendered markdown — where the deck's lists live. */
function deckRoot(): ParentNode {
  return document.querySelector('main#wrapper') ?? document.querySelector('main') ?? document.body
}

/** True on a flashcard page that has something to review. */
function isDeckPage(): boolean {
  return document.body.classList.contains(FLASHCARD_CLASS) && hasDeck(deckRoot())
}

/**
 * Import the lazy deck chunk. Same seam as `mbr-review.ts`: a runtime URL that
 * resolves from any page depth, `@vite-ignore` so vite leaves it alone, and an
 * overridable binding so tests can stub what happy-dom cannot import.
 */
let importFlashcardsChunk: () => Promise<unknown> = () => {
  const url = new URL(getMbrAssetBase() + 'components/mbr-flashcards.min.js', document.baseURI).href
  return import(/* @vite-ignore */ url)
}

/** Test hook: replace the chunk importer (module-level seam). */
export function setFlashcardsChunkImporter(importer: () => Promise<unknown>): void {
  importFlashcardsChunk = importer
  flashcardsChunkPromise = null
}

/** Shared once-per-page promise for the chunk load; `true` when usable. */
let flashcardsChunkPromise: Promise<boolean> | null = null

function loadFlashcardsChunk(): Promise<boolean> {
  if (!flashcardsChunkPromise) {
    flashcardsChunkPromise = importFlashcardsChunk()
      .then(() => true)
      .catch((err) => {
        console.warn('Failed to load the flashcards chunk:', err)
        return false // No deck this page load; the trigger stays inert.
      })
  }
  return flashcardsChunkPromise
}

@customElement('mbr-flashcards')
export class MbrFlashcardsElement extends LitElement implements MbrOverlay {
  @state() private _isDeck = false
  @state() private _isOpen = false
  @state() private _loading = false

  private _deck: DeckElement | null = null

  override connectedCallback(): void {
    super.connectedCallback()
    document.addEventListener('keydown', this._handleKeydown)
    waitForDom()
      .then(() => {
        if (!this.isConnected || !document.body.classList.contains(FLASHCARD_CLASS)) return
        decorateAllHistories(deckRoot())
        this._isDeck = isDeckPage()
      })
      .catch((err) => console.error('[mbr-flashcards] Error:', err))
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    document.removeEventListener('keydown', this._handleKeydown)
    this.close()
  }

  // ========================================
  // MbrOverlay
  // ========================================

  /** True while the deck is showing, or its chunk is loading. */
  public get isOpen(): boolean {
    return this._isOpen
  }

  public open(): void {
    void this._open()
  }

  public close(): void {
    this._isOpen = false
    this._deck?.remove()
    this._deck = null
  }

  private _handleKeydown = (e: KeyboardEvent): void => {
    // A page that is both slides and flashcards keeps `p` for the slides.
    if (document.body.classList.contains('slides')) return
    if (isPlayKey(e) && !this._isOpen && isDeckPage()) {
      e.preventDefault()
      void this._open()
    }
  }

  private async _open(): Promise<void> {
    // Checked here rather than trusting `_isDeck`, which is set a microtask
    // late: an `isOpen` reporting true with nothing on screen would make
    // `isModalOpen()` swallow every bare-letter shortcut on the page.
    if (this._isOpen || !isDeckPage()) return
    this._isOpen = true
    this._loading = true
    let ready = false
    try {
      ready = await loadFlashcardsChunk()
    } finally {
      this._loading = false
    }
    // Closed (Esc) while loading, or the chunk failed: show nothing.
    if (!this._isOpen || !ready) {
      this._isOpen = false
      return
    }

    const deck = document.createElement('mbr-flashcard-deck') as DeckElement
    deck.root = deckRoot()
    deck.recordReview = recordReview
    deck.srsAvailable = isEditEnabled() && currentDocumentPath() !== null
    deck.addEventListener(DECK_CLOSE_EVENT, () => this.close())
    this._deck = deck
    document.body.append(deck)
  }

  override render(): TemplateResult | typeof nothing {
    if (!this._isDeck) return nothing
    // A dimmed button stands in for a spinner: the chunk is small and local,
    // so the loading state is rarely visible at all.
    return html`<button
      @click=${() => this.open()}
      ?disabled=${this._loading}
      aria-busy=${this._loading ? 'true' : 'false'}
      aria-label="Review flashcards (P)"
      title="Review the flashcards on this page (P)"
    ><i></i><span>Review flashcards</span></button>`
  }

  // Same metrics as <mbr-slides>' button, so the nav reads the same. Written
  // compactly: Lit `css` text is not minified, and this ships on every page.
  static override styles = css`
    :host { display: contents }
    button { display: flex; align-items: center; gap: .4rem; padding: .35rem .7rem; border: none;
      border-radius: 4px; cursor: pointer; font-size: .85rem; font-weight: 500; white-space: nowrap;
      background: var(--pico-primary-background, #1095c1); color: var(--pico-primary-inverse, #fff);
      transition: background .15s ease, transform .1s ease }
    button:hover { background: var(--pico-primary-hover-background, #0d7a9c); transform: translateY(-1px) }
    button:disabled { opacity: .7; cursor: wait }
    i { border-left: 8px solid currentColor; border-block: 5px solid transparent }
    @media (max-width: 576px) { span { display: none } }
  `
}
