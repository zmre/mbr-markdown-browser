/**
 * `<mbr-flashcard-deck>` — the full-screen review overlay (lazy chunk).
 *
 * Created by the `<mbr-flashcards>` trigger, appended to `<body>`, and removed
 * again on close. Everything stateful arrives as a property: the page's deck
 * root, and the `recordReview` writer that lives in the main bundle next to the
 * source-line cache it shares with task toggles.
 *
 * # Light DOM
 *
 * The faces are clones of the page's rendered markdown, so the deck renders
 * into its own light DOM where the page's stylesheets reach them; see
 * `styles.ts`. It is appended to `<body>` rather than rendered inside the
 * trigger because the trigger sits in the header's `<nav>`, and Pico's `nav li`
 * rules would restyle every list on every card.
 *
 * # Keys (mirroring reveal.js, which `style: slides` readers already know)
 *
 * `Space` / `→` / `PageDown` / `n` advance: on the front they flip, on the back
 * they go to the next card — except in spaced-repetition mode, where a card may
 * only be left by rating it, so on the back they do nothing but point at the
 * rating buttons. `←` / `PageUp` go back a card, `Home` / `End` jump to the
 * first / last (both In order / Random only). `Enter` or a click flips either
 * way. `1`–`4` rate (spaced repetition). `Esc` closes.
 */
import { LitElement, html, nothing, type PropertyValues, type TemplateResult } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { keyed } from 'lit/directives/keyed.js'
import {
  appendHistoryEntry,
  deckCards,
  historyEntriesOf,
  shiftSourceLines,
  type CardParts,
} from './dom.js'
import {
  RATINGS,
  RATING_LABELS,
  formatEntryTime,
  parseHistoryEntry,
  type HistoryEntry,
  type Rating,
} from './history.js'
import {
  dueThisSession,
  formatInterval,
  makeScheduler,
  planSession,
  previewDue,
  relativeTime,
  replay,
  stateName,
  type Card,
} from './session.js'
import { fitFace } from './fit.js'
import { ensureDeckStyles } from './styles.js'
import type { ReviewRecorder } from './types.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-flashcard-deck': MbrFlashcardDeckElement
  }
}

/** How the session orders and advances. */
export type DeckMode = 'order' | 'random' | 'srs'

/** Event the deck dispatches when it wants to be closed. */
export const DECK_CLOSE_EVENT = 'mbr-flashcards-close'

/** One card of the session. */
interface DeckCard {
  readonly parts: CardParts
  history: HistoryEntry[]
  /** FSRS state after `history`, or `null` for a card never reviewed. */
  state: Card | null
}

/** Elements a click on a card face must reach instead of flipping it. */
const INTERACTIVE = 'a, button, input, select, textarea, summary, video, audio, label, [contenteditable]'

/** Elements focus can rest on, for the dialog's focus trap. */
const FOCUSABLE =
  'button:not([disabled]), select:not([disabled]), a[href], input:not([disabled]), [tabindex]:not([tabindex="-1"])'

/** In-place Fisher–Yates over a copy. */
function shuffled<T>(items: readonly T[]): T[] {
  const out = items.slice()
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1))
    ;[out[i], out[j]] = [out[j], out[i]]
  }
  return out
}

/**
 * A clone of `source`'s children, safe to show a second time on the page:
 * ids would duplicate (and hijack `#fragment` links), review markers are
 * page-only decoration, and `data-mbr-line` must keep meaning "this element in
 * the page" for the line-shifting a write does.
 */
function cloneContent(source: Element): HTMLElement {
  const wrap = document.createElement('div')
  for (const child of Array.from(source.childNodes)) wrap.append(child.cloneNode(true))
  wrap.querySelectorAll('.mbr-review-marker').forEach((el) => el.remove())
  wrap.querySelectorAll('[id]').forEach((el) => el.removeAttribute('id'))
  wrap.querySelectorAll('[data-mbr-line]').forEach((el) => el.removeAttribute('data-mbr-line'))
  return wrap
}

@customElement('mbr-flashcard-deck')
export class MbrFlashcardDeckElement extends LitElement {
  /** Where the deck's definition lists live (`main#wrapper`). */
  @property({ attribute: false }) root: ParentNode = document
  /** The review writer; `null` disables spaced repetition. */
  @property({ attribute: false }) recordReview: ReviewRecorder | null = null

  @state() private _cards: DeckCard[] = []
  @state() private _mode: DeckMode = 'random'
  @state() private _swap = false
  @state() private _queue: number[] = []
  @state() private _position = 0
  @state() private _flipped = false
  @state() private _finished = false
  /** Set while showing "nothing due": the next due date, or null if none. */
  @state() private _nothingDue: Date | null | undefined = undefined
  @state() private _historyOpen = false
  @state() private _message: string | null = null
  @state() private _writesBlocked = false
  @state() private _pending: Rating | null = null
  @state() private _announcement = ''

  /** Cards new at the start of an FSRS session and not yet rated in it. */
  private _fresh = new Set<number>()
  private _ratingCounts: Record<Rating, number> = { again: 0, hard: 0, good: 0, easy: 0 }
  private readonly _scheduler = makeScheduler()
  private _previousFocus: HTMLElement | null = null
  private _previousOverflow = ''
  private _resize: ResizeObserver | null = null
  private _fitFrame = 0
  private _focusCardNext = true
  /** Clones per `index:swap`, so re-renders reuse the same nodes. */
  private _faceCache = new Map<string, { front: HTMLElement; back: HTMLElement }>()

  // ========================================
  // Lifecycle
  // ========================================

  protected override createRenderRoot(): HTMLElement {
    return this
  }

  override connectedCallback(): void {
    super.connectedCallback()
    ensureDeckStyles()
    this._previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
    this._previousOverflow = document.documentElement.style.overflow
    document.documentElement.style.overflow = 'hidden'
    document.addEventListener('keydown', this._onKeydown)
    this.addEventListener('load', this._scheduleFit, true)

    this._cards = deckCards(this.root).map((parts) => {
      const history = parts.history ? historyEntriesOf(parts.history) : []
      return { parts, history, state: replay(this._scheduler, history) }
    })
    this._mode = this._srsUsable ? 'srs' : 'random'
    this._start(this._mode)
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    document.removeEventListener('keydown', this._onKeydown)
    this.removeEventListener('load', this._scheduleFit, true)
    this._resize?.disconnect()
    this._resize = null
    cancelAnimationFrame(this._fitFrame)
    document.documentElement.style.overflow = this._previousOverflow
    if (this._previousFocus?.isConnected) this._previousFocus.focus()
  }

  protected override firstUpdated(): void {
    const stage = this.querySelector<HTMLElement>('.mbr-fc-stage')
    if (stage && typeof ResizeObserver !== 'undefined') {
      this._resize = new ResizeObserver(this._scheduleFit)
      this._resize.observe(stage)
    }
  }

  protected override updated(changed: PropertyValues): void {
    super.updated(changed)
    this._scheduleFit()
    if (this._focusCardNext) {
      this._focusCardNext = false
      const target =
        this.querySelector<HTMLElement>('.mbr-fc-card') ??
        this.querySelector<HTMLElement>('.mbr-fc-screen button')
      target?.focus({ preventScroll: true })
    }
  }

  // ========================================
  // Session
  // ========================================

  /**
   * Spaced repetition needs a writer (the trigger supplies one only with
   * editing on and a source path) and every card addressable by its line.
   */
  private get _srsUsable(): boolean {
    return (
      this.recordReview !== null &&
      this._cards.length > 0 &&
      this._cards.every((card) => Number(card.parts.term.dataset.mbrLine) > 0)
    )
  }

  private _start(mode: DeckMode): void {
    const indices = this._cards.map((_, i) => i)
    this._mode = mode
    this._position = 0
    this._flipped = false
    this._finished = false
    this._historyOpen = false
    this._nothingDue = undefined
    this._ratingCounts = { again: 0, hard: 0, good: 0, easy: 0 }
    this._fresh = new Set()
    if (mode === 'srs') {
      const plan = planSession(
        this._scheduler,
        this._cards.map((card) => card.state),
        new Date()
      )
      this._queue = plan.queue.slice()
      this._fresh = new Set(plan.fresh)
      if (this._queue.length === 0) this._nothingDue = plan.nextDue
    } else {
      this._queue = mode === 'random' ? shuffled(indices) : indices
    }
    this._focusCardNext = true
  }

  private get _currentIndex(): number | undefined {
    return this._finished || this._nothingDue !== undefined ? undefined : this._queue[this._position]
  }

  private get _showRatings(): boolean {
    return this._mode === 'srs' && !this._writesBlocked
  }

  private _close(): void {
    this.dispatchEvent(new CustomEvent(DECK_CLOSE_EVENT, { bubbles: true, composed: true }))
  }

  private _flip(): void {
    if (this._currentIndex === undefined) return
    this._flipped = !this._flipped
    if (!this._flipped) this._historyOpen = false
    this._announcement = this._flipped ? 'Answer shown' : 'Question shown'
  }

  /** `Space` / `→`: flip a front, leave a back. */
  private _advance(): void {
    if (this._currentIndex === undefined) return
    if (!this._flipped) {
      this._flip()
    } else if (this._showRatings) {
      // A spaced-repetition card is only ever left by rating it.
      this._nudgeRatings()
      this._announcement = 'Rate this card: 1 Again, 2 Hard, 3 Good, 4 Easy'
    } else {
      this._goTo(this._position + 1)
    }
  }

  /** Replay the ratings' attention bounce, restarting it if it is running. */
  private _nudgeRatings(): void {
    const ratings = this.querySelector<HTMLElement>('.mbr-fc-ratings')
    if (!ratings) return
    ratings.classList.remove('is-nudged')
    void ratings.offsetWidth // forces a style flush, so re-adding restarts it
    ratings.classList.add('is-nudged')
  }

  /** Move to queue position `position`, or to the end-of-deck screen past it. */
  private _goTo(position: number): void {
    if (position >= this._queue.length) {
      this._finished = true
      this._announcement = 'End of deck'
    } else {
      this._finished = false
      this._position = Math.max(0, position)
      this._announcement = `Card ${this._position + 1} of ${this._queue.length}`
    }
    this._flipped = false
    this._historyOpen = false
    this._focusCardNext = true
  }

  private _back(): void {
    if (this._mode === 'srs') return
    if (this._finished) this._goTo(this._queue.length - 1)
    else if (this._position > 0) this._goTo(this._position - 1)
  }

  private async _rate(rating: Rating): Promise<void> {
    const index = this._currentIndex
    if (index === undefined || !this._flipped || !this._showRatings || this._pending) return
    const card = this._cards[index]
    const line = Number(card.parts.term.dataset.mbrLine)
    if (!this.recordReview || !(line > 0)) return

    this._pending = rating
    const outcome = await this.recordReview({ line, rating })
    this._pending = null
    if (!this.isConnected) return

    if (!outcome.ok) {
      this._message = outcome.message
      this._announcement = outcome.message
      // Every later write would be refused as well; finish the session as a
      // read-only one rather than failing card after card.
      if (outcome.kind === 'conflict') this._writesBlocked = true
      return
    }

    this._message = null
    const now = new Date()
    const entry = parseHistoryEntry(outcome.entry) ?? { at: now, rating }
    card.history = [...card.history, entry]
    card.state = replay(this._scheduler, card.history)
    // The page behind is not reloaded: renumber the lines below the insert,
    // then show the entry in the card's history there too.
    shiftSourceLines(this.root, outcome.insertedAt, outcome.insertedCount)
    appendHistoryEntry(card.parts, outcome.entry, outcome.insertedAt, outcome.line)

    this._ratingCounts = { ...this._ratingCounts, [rating]: this._ratingCounts[rating] + 1 }
    this._fresh.delete(index)
    if (card.state && dueThisSession(card.state, now)) {
      // Learn-ahead: back at the end of the queue rather than in N minutes.
      this._queue = [...this._queue, index]
    }
    this._announcement = `Saved: ${RATING_LABELS[rating]}`
    this._goTo(this._position + 1)
  }

  // ========================================
  // Keyboard
  // ========================================

  private _onKeydown = (e: KeyboardEvent): void => {
    if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return
    const target = e.composedPath()[0]
    const tag = target instanceof HTMLElement ? target.tagName : ''
    const onControl = tag === 'BUTTON' || tag === 'SELECT' || tag === 'A' || tag === 'SUMMARY'
    const typing = tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT'

    if (e.key === 'Escape') {
      e.preventDefault()
      this._close()
      return
    }
    if (e.key === 'Tab') {
      this._trapFocus(e)
      return
    }
    if (typing) return

    const ordered = this._mode !== 'srs'
    switch (e.key) {
      case ' ':
      case 'Spacebar':
        // A focused button keeps its own Space.
        if (onControl) return
        e.preventDefault()
        this._advance()
        return
      case 'Enter':
        if (onControl) return
        e.preventDefault()
        this._flip()
        return
      case 'ArrowRight':
      case 'PageDown':
      case 'n':
        e.preventDefault()
        this._advance()
        return
      case 'ArrowLeft':
      case 'PageUp':
        if (!ordered) return
        e.preventDefault()
        this._back()
        return
      case 'Home':
        if (!ordered || this._queue.length === 0) return
        e.preventDefault()
        this._goTo(0)
        return
      case 'End':
        if (!ordered || this._queue.length === 0) return
        e.preventDefault()
        this._goTo(this._queue.length - 1)
        return
      case '1':
      case '2':
      case '3':
      case '4':
        if (!this._flipped || !this._showRatings) return
        e.preventDefault()
        void this._rate(RATINGS[Number(e.key) - 1])
        return
      default:
        return
    }
  }

  /** Keep Tab inside the dialog. */
  private _trapFocus(e: KeyboardEvent): void {
    const focusable = Array.from(this.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
      (el) => !el.closest('[inert]') && el.getClientRects().length > 0
    )
    if (focusable.length === 0) return
    const first = focusable[0]
    const last = focusable[focusable.length - 1]
    const active = document.activeElement
    if (e.shiftKey && (active === first || !this.contains(active))) {
      e.preventDefault()
      last.focus()
    } else if (!e.shiftKey && (active === last || !this.contains(active))) {
      e.preventDefault()
      first.focus()
    }
  }

  // ========================================
  // Fit to screen
  // ========================================

  private _scheduleFit = (): void => {
    cancelAnimationFrame(this._fitFrame)
    this._fitFrame = requestAnimationFrame(() => {
      for (const face of Array.from(this.querySelectorAll<HTMLElement>('.mbr-fc-face'))) {
        const content = face.querySelector<HTMLElement>('.mbr-fc-content')
        if (content) fitFace(face, content)
      }
    })
  }

  // ========================================
  // Rendering
  // ========================================

  private _faces(index: number): { front: HTMLElement; back: HTMLElement } {
    const key = `${index}:${this._swap}`
    let faces = this._faceCache.get(key)
    if (!faces) {
      const { parts } = this._cards[index]
      const question = cloneContent(parts.term)
      const answer = document.createElement('div')
      for (const dd of parts.answers) {
        const one = cloneContent(dd)
        one.className = 'mbr-fc-answer'
        answer.append(one)
      }
      faces = this._swap ? { front: answer, back: question } : { front: question, back: answer }
      this._faceCache.set(key, faces)
    }
    return faces
  }

  private _counter(): string {
    if (this._cards.length === 0) return ''
    if (this._mode === 'srs') {
      const remaining = this._finished ? [] : this._queue.slice(this._position)
      const fresh = remaining.filter((i) => this._fresh.has(i)).length
      return `Due ${remaining.length - fresh} · New ${fresh}`
    }
    if (this._finished) return `${this._queue.length} / ${this._queue.length}`
    return `${this._position + 1} / ${this._queue.length}`
  }

  private _renderBar(): TemplateResult {
    return html`
      <header class="mbr-fc-bar">
        <h2 class="mbr-fc-title" id="mbr-fc-title">Flashcards</h2>
        <span class="mbr-fc-counter" aria-label="Progress">${this._counter()}</span>
        <span class="mbr-fc-spacer"></span>
        <button
          type="button"
          class="secondary outline mbr-fc-swap"
          aria-pressed=${this._swap ? 'true' : 'false'}
          title="Show the answer first"
          @click=${this._toggleSwap}
        >
          <span aria-hidden="true">⇄</span><span class="mbr-fc-swap-label"> Swap sides</span>
        </button>
        <select aria-label="Review mode" @change=${this._onModeChange}>
          <option value="order" ?selected=${this._mode === 'order'}>In order</option>
          <option value="random" ?selected=${this._mode === 'random'}>Random</option>
          ${this._srsUsable
            ? html`<option value="srs" ?selected=${this._mode === 'srs'}>
                Spaced repetition (FSRS)
              </option>`
            : nothing}
        </select>
        <button
          type="button"
          class="mbr-fc-close"
          aria-label="Close flashcards (Esc)"
          title="Close (Esc)"
          @click=${this._close}
        >
          ×
        </button>
      </header>
    `
  }

  private _renderCard(index: number): unknown {
    const { front, back } = this._faces(index)
    const flipped = this._flipped
    const label = `Card ${this._position + 1} of ${this._queue.length}, ${
      flipped ? 'answer' : 'question'
    } side`
    return keyed(
      `${this._position}:${index}:${this._swap}`,
      html`
        <div class="mbr-fc-slot">
          <div
            class="mbr-fc-card ${flipped ? 'is-flipped' : ''}"
            tabindex="0"
            role="group"
            aria-roledescription="flashcard"
            aria-label=${label}
            @click=${this._onCardClick}
          >
            <section class="mbr-fc-face mbr-fc-front" ?inert=${flipped} aria-hidden=${flipped ? 'true' : 'false'}>
              <span class="mbr-fc-side">${this._swap ? 'Answer' : 'Question'}</span>
              <div class="mbr-fc-content">${front}</div>
            </section>
            <section class="mbr-fc-face mbr-fc-back" ?inert=${!flipped} aria-hidden=${flipped ? 'false' : 'true'}>
              <span class="mbr-fc-side">${this._swap ? 'Question' : 'Answer'}</span>
              <div class="mbr-fc-content">${back}</div>
            </section>
          </div>
        </div>
      `
    )
  }

  private _renderNothingDue(next: Date | null): TemplateResult {
    return html`
      <div class="mbr-fc-screen" role="status">
        <h3>Nothing due</h3>
        <p>
          ${next
            ? html`Next card due ${relativeTime(next, new Date())} (${next.toLocaleString(undefined, {
                dateStyle: 'medium',
                timeStyle: 'short',
              })}).`
            : 'No card has a review scheduled.'}
        </p>
        <div class="mbr-fc-actions">
          <button type="button" @click=${() => this._start('random')}>Review anyway (random)</button>
          <button type="button" class="secondary outline" @click=${this._close}>Close</button>
        </div>
      </div>
    `
  }

  private _renderFinished(): TemplateResult {
    const reviewed = RATINGS.reduce((sum, r) => sum + this._ratingCounts[r], 0)
    const breakdown = RATINGS.filter((r) => this._ratingCounts[r] > 0)
      .map((r) => `${RATING_LABELS[r]} ${this._ratingCounts[r]}`)
      .join(' · ')
    return html`
      <div class="mbr-fc-screen" role="status">
        <h3>${this._mode === 'srs' ? 'Session complete' : 'End of deck'}</h3>
        <p>
          ${this._mode === 'srs'
            ? reviewed > 0
              ? `${reviewed} review${reviewed === 1 ? '' : 's'} saved: ${breakdown}.`
              : 'No reviews were saved.'
            : `You went through all ${this._queue.length} card${this._queue.length === 1 ? '' : 's'}.`}
        </p>
        <div class="mbr-fc-actions">
          <button type="button" @click=${() => this._start(this._mode)}>
            ${this._mode === 'srs' ? 'Check for more' : 'Restart'}
          </button>
          <button type="button" class="secondary outline" @click=${this._close}>Close</button>
        </div>
      </div>
    `
  }

  private _renderControls(index: number): TemplateResult {
    const card = this._cards[index]
    if (!this._flipped) {
      return html`
        ${this._mode !== 'srs'
          ? html`<button type="button" class="secondary outline" ?disabled=${this._position === 0} @click=${this._back}>
              ← Previous
            </button>`
          : nothing}
        <button type="button" class="mbr-fc-primary" @click=${this._flip}>Show answer</button>
        ${this._mode !== 'srs'
          ? html`<button type="button" class="secondary outline" @click=${() => this._goTo(this._position + 1)}>
              Next →
            </button>`
          : nothing}
      `
    }
    const history = card.history.length
      ? html`<button
          type="button"
          class="mbr-fc-history-toggle"
          aria-expanded=${this._historyOpen ? 'true' : 'false'}
          @click=${() => (this._historyOpen = !this._historyOpen)}
        >
          History (${card.history.length})
        </button>`
      : nothing
    if (this._showRatings) {
      const now = new Date()
      const due = previewDue(this._scheduler, card.state, now)
      return html`
        <div class="mbr-fc-ratings" role="group" aria-label="Rate your recall">
          ${RATINGS.map(
            (rating, i) => html`
              <button
                type="button"
                class="mbr-fc-rate-${rating} ${this._pending === rating ? 'is-pending' : ''}"
                ?disabled=${this._pending !== null}
                aria-label="${i + 1}: ${RATING_LABELS[rating]}, next review in ${formatInterval(now, due[rating])}"
                @click=${() => void this._rate(rating)}
              >
                <span><kbd>${i + 1}</kbd>${RATING_LABELS[rating]}</span>
                <small>${formatInterval(now, due[rating])}</small>
              </button>
            `
          )}
        </div>
        ${history}
      `
    }
    return html`
      <button type="button" class="secondary outline" ?disabled=${this._position === 0} @click=${this._back}>
        ← Previous
      </button>
      <button type="button" class="mbr-fc-primary" @click=${() => this._goTo(this._position + 1)}>Next →</button>
      ${history}
    `
  }

  private _renderHistory(index: number): TemplateResult | typeof nothing {
    if (!this._historyOpen || !this._flipped) return nothing
    const card = this._cards[index]
    const state = card.state
    return html`
      <div class="mbr-fc-history-panel" role="region" aria-label="Review history">
        ${state
          ? html`<p>
              ${stateName(state)} · due ${state.due.toLocaleString(undefined, {
                dateStyle: 'medium',
                timeStyle: 'short',
              })}
              · stability ${state.stability.toFixed(1)}d · difficulty ${state.difficulty.toFixed(1)}
            </p>`
          : nothing}
        <table>
          <thead>
            <tr><th scope="col">Reviewed</th><th scope="col">Rating</th></tr>
          </thead>
          <tbody>
            ${card.history
              .slice()
              .reverse()
              .map(
                (entry) =>
                  html`<tr><td>${formatEntryTime(entry.at)}</td><td>${RATING_LABELS[entry.rating]}</td></tr>`
              )}
          </tbody>
        </table>
      </div>
    `
  }

  private _renderHints(): TemplateResult {
    if (this._mode === 'srs' && !this._writesBlocked) {
      return html`<p class="mbr-fc-hints" aria-hidden="true">
        <kbd>Space</kbd> flip · <kbd>1</kbd>–<kbd>4</kbd> rate · <kbd>Esc</kbd> close
      </p>`
    }
    return html`<p class="mbr-fc-hints" aria-hidden="true">
      <kbd>Space</kbd>/<kbd>→</kbd> flip, then next · <kbd>←</kbd> back · <kbd>Enter</kbd> flip ·
      <kbd>Esc</kbd> close
    </p>`
  }

  override render(): TemplateResult {
    const index = this._currentIndex
    let body: unknown
    if (this._cards.length === 0) {
      body = html`<div class="mbr-fc-screen"><h3>No cards</h3><p>This page has no definition lists to review.</p></div>`
    } else if (this._nothingDue !== undefined) {
      body = this._renderNothingDue(this._nothingDue)
    } else if (this._finished || index === undefined) {
      body = this._renderFinished()
    } else {
      body = this._renderCard(index)
    }
    return html`
      <div class="mbr-fc-overlay" role="dialog" aria-modal="true" aria-labelledby="mbr-fc-title">
        ${this._renderBar()}
        ${this._message
          ? html`<div class="mbr-fc-banner" role="alert">
              <span>${this._message}</span>
              <button type="button" class="secondary outline" @click=${() => (this._message = null)}>
                Dismiss
              </button>
            </div>`
          : nothing}
        <div class="mbr-fc-stage">${body}</div>
        ${index !== undefined ? this._renderHistory(index) : nothing}
        <footer class="mbr-fc-controls">${index !== undefined ? this._renderControls(index) : nothing}</footer>
        ${this._renderHints()}
        <div class="mbr-fc-sr-only" aria-live="polite">${this._announcement}</div>
      </div>
    `
  }

  // ========================================
  // Handlers
  // ========================================

  private _onCardClick = (e: MouseEvent): void => {
    const target = e.target
    if (target instanceof Element && target.closest(INTERACTIVE)) return
    // A text selection is a reader copying an answer, not asking to flip.
    if (window.getSelection()?.toString()) return
    this._flip()
  }

  private _toggleSwap = (): void => {
    this._swap = !this._swap
    this._flipped = false
    this._historyOpen = false
  }

  private _onModeChange = (e: Event): void => {
    const value = (e.target as HTMLSelectElement).value as DeckMode
    this._start(value)
  }
}
