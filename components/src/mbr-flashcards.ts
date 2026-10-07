/**
 * `<mbr-flashcards>` — the flashcard trigger (main bundle).
 *
 * On a `type: flashcard` page (`<body class="flashcard">`) that has a
 * definition list, this renders a "Review flashcards" button in the nav (where
 * `<mbr-slides>` puts "Play Slides", and styled by that element's own sheet)
 * and claims `p`, with the same guards as slides (`isPlayKey`).
 *
 * **Deliberately tiny: it ships on every page, and almost no page is a deck.**
 * Everything else is in two lazy chunks, fetched only on flashcard pages:
 *
 * - `mbr-flashcards-reading.min.js`, imported at idle, collapses each card's
 *   `___Review History___` into a one-line summary. Separate from the deck so
 *   that *reading* a flashcard note never fetches FSRS and the overlay.
 * - `mbr-flashcards.min.js`, imported when the deck is opened: the overlay,
 *   `ts-fsrs`, and the review writer.
 *
 * The writer must use this bundle's `task-toggle.ts` / `edit-token.ts` state
 * (one source cache, one self-write window, one token), so the trigger hands
 * those functions to the chunk's `makeReviewRecorder` rather than letting the
 * chunk import a second copy. No writer (no editing, or no source path) means
 * the deck offers no spaced repetition.
 *
 * "Has a deck" is checked loosely here (any `dl > dt` in the page); the chunk
 * applies the exact rule — top-level lists only — and says "No cards" when a
 * page's only lists are nested.
 */
import { LitElement, html, nothing, type TemplateResult } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { getMbrAssetBase, scheduleIdleTask, waitForDom } from './dynamic-loader.js'
import { isEditEnabled } from './shared.js'
import { isPlayKey } from './mbr-keys.js'
import { MbrSlidesElement } from './mbr-slides.js'
import { editAuthHeaders, noteEditTokenRequired } from './edit-token.js'
import {
  TOKEN_MESSAGE,
  currentDocumentPath,
  forgetSourceLines,
  noteSelfWrite,
  readSourceLines,
} from './task-toggle.js'
import type { ReviewRecorder, ReviewServices } from './flashcards/types.js'
import type { MbrOverlay } from './overlay.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-flashcards': MbrFlashcardsElement
  }
}

/** What the chunks export that the trigger uses (`flashcards/index.ts`, `reading.ts`). */
interface ChunkModule {
  makeReviewRecorder?: (services: ReviewServices) => ReviewRecorder
  decorateAllHistories?: (root: ParentNode) => void
}

/** The properties the trigger sets on the chunk's deck element. */
interface DeckElement extends HTMLElement {
  root: ParentNode
  recordReview: ReviewRecorder | null
}

/** The page's rendered markdown (`main#wrapper`) — where the deck's lists live. */
function deckRoot(): ParentNode {
  return document.querySelector('main') ?? document.body
}

/** True on a `type: flashcard` page with a definition list to review. */
function isDeckPage(): boolean {
  return document.body.classList.contains('flashcard') && !!deckRoot().querySelector('dl > dt')
}

/**
 * Import a chunk by file name. Same seam as `mbr-review.ts`: a runtime URL that
 * resolves from any page depth, `@vite-ignore` so vite leaves it alone, and an
 * overridable binding — one for both chunks — so tests can stub what happy-dom
 * cannot import.
 */
let importChunk = (file: string): Promise<ChunkModule> =>
  import(/* @vite-ignore */ new URL(getMbrAssetBase() + 'components/' + file, document.baseURI).href)

/** Shared once-per-page load of the deck chunk; `null` when it failed. */
let deckChunk: Promise<ChunkModule | null> | null = null

/** Test hook: replace the chunk importer; `file` names the chunk wanted. */
export function setFlashcardsChunkImporter(importer: (file: string) => Promise<unknown>): void {
  importChunk = importer as typeof importChunk
  deckChunk = null
}

function chunkFailed(err: unknown): null {
  console.warn('mbr-flashcards:', err)
  return null
}

/** The writer's main-bundle state, or `null` when reviews cannot be written. */
function reviewServices(): ReviewServices | null {
  const path = currentDocumentPath()
  return isEditEnabled() && path
    ? {
        path,
        read: readSourceLines,
        forget: forgetSourceLines,
        selfWrite: noteSelfWrite,
        headers: editAuthHeaders,
        tokenRequired: noteEditTokenRequired,
        tokenMessage: TOKEN_MESSAGE,
      }
    : null
}

@customElement('mbr-flashcards')
export class MbrFlashcardsElement extends LitElement implements MbrOverlay {
  @state() private _isDeck = false
  // Not reactive: render() never reads it.
  private _isOpen = false
  private _deck: DeckElement | null = null

  override connectedCallback(): void {
    super.connectedCallback()
    document.addEventListener('keydown', this._handleKeydown)
    void waitForDom().then(() => {
      if (!this.isConnected || !isDeckPage()) return
      this._isDeck = true
      scheduleIdleTask(() => {
        importChunk('mbr-flashcards-reading.min.js')
          .then((m) => m.decorateAllHistories?.(deckRoot()))
          .catch(chunkFailed)
      })
    })
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    document.removeEventListener('keydown', this._handleKeydown)
    this.close()
  }

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
    if (!document.body.classList.contains('slides') && isPlayKey(e) && !this._isOpen && isDeckPage()) {
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
    // On failure: no deck this page load; the trigger stays inert.
    deckChunk ??= importChunk('mbr-flashcards.min.js').catch(chunkFailed)
    const chunk = await deckChunk
    // Closed (Esc) while loading, or the chunk failed: show nothing.
    if (!this._isOpen || !chunk) {
      this._isOpen = false
      return
    }

    const services = reviewServices()
    const deck = document.createElement('mbr-flashcard-deck') as DeckElement
    deck.root = deckRoot()
    // No writer means no spaced repetition: In order / Random only.
    deck.recordReview = services && chunk.makeReviewRecorder ? chunk.makeReviewRecorder(services) : null
    deck.addEventListener('mbr-flashcards-close', () => this.close())
    this._deck = deck
    document.body.append(deck)
  }

  override render(): TemplateResult | typeof nothing {
    // No loading state: the chunk is small and local, and a second press while
    // it loads is already a no-op (`_open` checks `_isOpen`). One line on
    // purpose: template whitespace is not minified.
    // prettier-ignore
    return this._isDeck ? html`<button class="play-slides-btn" @click=${() => this.open()} title="Review flashcards (P)"><span class="play-icon"></span>Review flashcards</button>` : nothing
  }

  // <mbr-slides>' own sheet, not a copy: the two buttons share the nav slot
  // and should look identical, and a second sheet would ship on every page.
  static override styles = MbrSlidesElement.styles
}
