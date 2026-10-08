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
 * they go to the next card — except in the rated modes (spaced repetition,
 * Concentric), where a card may only be left by rating it, so on the back they
 * do nothing but point at the rating buttons. `←` / `PageUp` go back a card,
 * `Home` / `End` jump to the first / last (both In order / Random only).
 * `Enter` or a click flips either way. `1`–`4` rate (rated modes). `Esc` closes
 * the help, the Cards box or the section-filter popover if one has focus, else
 * the deck.
 *
 * The deck controls have keys too — `c` / `o` / `r` / `s` switch mode, `w`
 * swaps sides, `f` opens the filter, `+` / `-` resize Concentric's stack, `#`
 * focuses the Cards box — and `?` lists every one ({@link SHORTCUT_GROUPS}, the
 * single table the help is drawn from).
 *
 * The listener is on `window` in the **capture** phase, so it runs before the
 * page's own document-level shortcuts (`mbr-keys.ts` toggles its help on `?`
 * without asking whether an overlay is open), and every key it sees stops
 * there: while the deck is open, nothing behind it reacts to the keyboard.
 * Modified keys are never handled, only stopped, so `Cmd`/`Ctrl` combinations
 * keep their browser meaning.
 *
 * # Concentric
 *
 * A small stack that grows as it is learned (`concentric.ts`). It needs no
 * writer: with one (editing on) each rating is saved exactly as spaced
 * repetition saves it; without one — or once a conflict or a 403 has stopped
 * writes —
 * ratings live in this session only (`card.history` in memory, no write, no
 * line shifting).
 *
 * # Section filter
 *
 * The funnel narrows the deck to the cards under chosen headings
 * (`dom.ts::cardSections`). Not persisted: every open starts on all cards.
 */
import { LitElement, html, nothing, type PropertyValues, type TemplateResult } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { keyed } from 'lit/directives/keyed.js'
import { live } from 'lit/directives/live.js'
import {
  appendHistoryEntry,
  cardSections,
  deckCards,
  historyEntriesOf,
  shiftSourceLines,
  type CardParts,
  type CardSection,
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
import {
  CONCENTRIC_DEFAULT_SIZE,
  displaySize,
  rateConcentric,
  resizeConcentric,
  setConcentricEligible,
  startConcentric,
  type ConcentricState,
} from './concentric.js'
import { fitFace } from './fit.js'
import { shuffle } from './random.js'
import { ensureDeckStyles } from './styles.js'
import type { ReviewRecorder } from './types.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-flashcard-deck': MbrFlashcardDeckElement
  }
}

/** How the session orders and advances. */
export type DeckMode = 'order' | 'random' | 'concentric' | 'srs'

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

/** Where Concentric puts a card back, as the small text on its rating button. */
const CONCENTRIC_HINTS: Readonly<Record<Rating, string>> = {
  again: 'soon',
  hard: 'midway',
  good: 'later',
  easy: 'last',
}

/** One row of the help: the keys as shown, and what they do. */
interface Shortcut {
  readonly keys: readonly string[]
  readonly action: string
}

/**
 * Every deck key, grouped as the `?` help shows them. The help is rendered from
 * this table, so it cannot drift from itself; `_onKeydown` is the other half
 * and `mbr-flashcard-deck.test.ts` presses every key listed here.
 */
export const SHORTCUT_GROUPS: readonly { readonly title: string; readonly shortcuts: readonly Shortcut[] }[] = [
  {
    title: 'Navigation',
    shortcuts: [
      { keys: ['Space', '→', 'PageDown', 'n'], action: 'Flip the card, then go to the next one' },
      { keys: ['Enter'], action: 'Flip the card either way' },
      { keys: ['←', 'PageUp'], action: 'Previous card (In order, Random)' },
      { keys: ['Home', 'End'], action: 'First / last card (In order, Random)' },
    ],
  },
  {
    title: 'Rating',
    shortcuts: [
      { keys: ['1'], action: 'Again' },
      { keys: ['2'], action: 'Hard' },
      { keys: ['3'], action: 'Good' },
      { keys: ['4'], action: 'Easy' },
    ],
  },
  {
    title: 'Deck controls',
    shortcuts: [
      { keys: ['c'], action: 'Concentric (FSRS)' },
      { keys: ['o'], action: 'In order' },
      { keys: ['r'], action: 'Random' },
      { keys: ['s'], action: 'Spaced repetition (FSRS), when editing is on' },
      { keys: ['w'], action: 'Swap sides' },
      { keys: ['f'], action: 'Filter by section: ↑ / ↓ move, Space toggles, a picks all, Esc closes' },
      { keys: ['+', '-'], action: 'One more / one fewer card in play (Concentric)' },
      { keys: ['#'], action: 'Edit the number of cards (Concentric); Enter or Esc leaves it' },
      { keys: ['?'], action: 'Show or hide this help' },
      { keys: ['Esc'], action: 'Close the help or the filter, else the deck' },
    ],
  },
]

/** Mode keys, and the mode each selects. */
const MODE_KEYS: Readonly<Record<string, DeckMode>> = { c: 'concentric', o: 'order', r: 'random', s: 'srs' }

/** Labels for the mode dropdown and the announcement a mode key makes. */
const MODE_LABELS: Readonly<Record<DeckMode, string>> = {
  order: 'In order',
  random: 'Random',
  concentric: 'Concentric (FSRS)',
  srs: 'Spaced repetition (FSRS)',
}

/** The section filter's funnel. */
const FUNNEL = html`<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" focusable="false">
  <path d="M3 4h18l-7 8.5V19l-4 2v-8.5z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" />
</svg>`

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
  /** Concentric's growth threshold (`flashcards_concentric_threshold`). */
  @property({ attribute: false }) concentricThreshold = 0.7

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
  @state() private _concentric: ConcentricState | null = null
  /** Ids of the headings the filter keeps; empty means every card. */
  @state() private _filter: ReadonlySet<string> = new Set()
  @state() private _filterOpen = false
  @state() private _helpOpen = false

  /** Cards new at the start of an FSRS session and not yet rated in it. */
  private _fresh = new Set<number>()
  private _ratingCounts: Record<Rating, number> = { again: 0, hard: 0, good: 0, easy: 0 }
  private readonly _scheduler = makeScheduler()
  private _previousFocus: HTMLElement | null = null
  /** Where focus goes back to when the help or the filter popover closes. */
  private _helpReturnFocus: HTMLElement | null = null
  private _filterReturnFocus: HTMLElement | null = null
  private _previousOverflow = ''
  private _resize: ResizeObserver | null = null
  private _fitFrame = 0
  private _focusCardNext = true
  /** Clones per `index:swap`, so re-renders reuse the same nodes. */
  private _faceCache = new Map<string, { front: HTMLElement; back: HTMLElement }>()
  /**
   * Headings the filter offers: those holding some, but not all, cards (one
   * holding every card would filter nothing).
   */
  private _sections: readonly CardSection[] = []

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
    window.addEventListener('keydown', this._onKeydown, true)
    document.addEventListener('pointerdown', this._onPointerDown, true)
    this.addEventListener('load', this._scheduleFit, true)

    this._cards = deckCards(this.root).map((parts) => {
      const history = parts.history ? historyEntriesOf(parts.history) : []
      return { parts, history, state: replay(this._scheduler, history) }
    })
    this._sections = cardSections(
      this.root,
      this._cards.map((card) => card.parts.term)
    ).sections.filter((section) => section.cards.length < this._cards.length)
    // Concentric whenever there is something to review, edit mode or not: it
    // works session-only without a writer and saves each rating with one.
    this._mode = this._cards.length > 0 ? 'concentric' : 'random'
    this._start(this._mode)
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    window.removeEventListener('keydown', this._onKeydown, true)
    document.removeEventListener('pointerdown', this._onPointerDown, true)
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
    // The mode select follows `_mode` by property: once the reader has used the
    // dropdown, an option's `selected` attribute no longer moves it, and a mode
    // key changes the mode from outside it.
    const select = this.querySelector<HTMLSelectElement>('.mbr-fc-bar select')
    if (select && select.value !== this._mode) select.value = this._mode
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

  /** The cards the section filter admits, in document order. */
  private _eligible(): number[] {
    if (this._filter.size === 0) return this._cards.map((_, i) => i)
    const kept = new Set<number>()
    for (const section of this._sections) {
      if (this._filter.has(section.id)) section.cards.forEach((i) => kept.add(i))
    }
    return Array.from(kept).sort((a, b) => a - b)
  }

  private _start(mode: DeckMode): void {
    const indices = this._eligible()
    this._mode = mode
    this._position = 0
    this._flipped = false
    this._finished = false
    this._historyOpen = false
    this._nothingDue = undefined
    this._ratingCounts = { again: 0, hard: 0, good: 0, easy: 0 }
    this._fresh = new Set()
    if (mode === 'srs') {
      // Planned over the eligible cards only, then mapped back to deck indices.
      const plan = planSession(
        this._scheduler,
        indices.map((i) => this._cards[i].state),
        new Date(),
        Math.random
      )
      this._queue = plan.queue.map((k) => indices[k])
      this._fresh = new Set(Array.from(plan.fresh, (k) => indices[k]))
      if (this._queue.length === 0) this._nothingDue = plan.nextDue
    } else if (mode === 'concentric') {
      this._queue = []
      this._concentric = startConcentric({
        latest: this._cards.map((card) => card.history[card.history.length - 1]?.rating ?? null),
        eligible: indices,
        size: this._concentric?.target ?? CONCENTRIC_DEFAULT_SIZE,
        threshold: this.concentricThreshold,
        rng: Math.random,
      })
    } else {
      this._queue = mode === 'random' ? shuffle(indices, Math.random) : indices
    }
    this._focusCardNext = true
  }

  private get _currentIndex(): number | undefined {
    if (this._finished || this._nothingDue !== undefined) return undefined
    return this._mode === 'concentric' ? this._concentric?.stack[0] : this._queue[this._position]
  }

  /**
   * Rated modes show the rating buttons. Concentric always does: after a
   * conflict it carries on session-only, where spaced repetition (whose whole
   * point is the saved schedule) falls back to plain paging.
   */
  private get _showRatings(): boolean {
    return (this._mode === 'srs' && !this._writesBlocked) || this._mode === 'concentric'
  }

  /** In order / Random: cards are paged, not rated, so back and jumps work. */
  private get _ordered(): boolean {
    return this._mode === 'order' || this._mode === 'random'
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
    if (!this._ordered) return
    if (this._finished) this._goTo(this._queue.length - 1)
    else if (this._position > 0) this._goTo(this._position - 1)
  }

  private async _rate(rating: Rating): Promise<void> {
    const index = this._currentIndex
    if (index === undefined || !this._flipped || !this._showRatings || this._pending) return
    const card = this._cards[index]
    const line = Number(card.parts.term.dataset.mbrLine)
    const write = this.recordReview !== null && !this._writesBlocked && line > 0
    // Spaced repetition only ever rates through the writer; Concentric rates
    // session-only when there is none.
    if (this._mode === 'srs' && !write) return

    let entry: HistoryEntry = { at: new Date(), rating }
    let saved = false
    if (write && this.recordReview) {
      this._pending = rating
      const outcome = await this.recordReview({ line, rating })
      this._pending = null
      if (outcome.ok) {
        // The page behind is not reloaded: renumber the lines below the
        // insert, then show the entry in the card's history there too. This
        // runs before the `isConnected` check on purpose: the writer has
        // already spliced the inserted lines into the shared source-line
        // cache, so a deck closed mid-write must still bring the page's line
        // numbers in step, or a later task toggle below the card addresses
        // the wrong line (and the server, seeing `expected` match, accepts it).
        shiftSourceLines(this.root, outcome.insertedAt, outcome.insertedCount)
        appendHistoryEntry(card.parts, outcome.entry, outcome.insertedAt, outcome.line)
      }
      if (!this.isConnected) return

      if (!outcome.ok) {
        this._message = outcome.message
        this._announcement = outcome.message
        // Every later write would be refused as well (a stale file, or a 403
        // that no retry can fix); finish the session as a read-only one rather
        // than failing — and re-showing the notice — card after card. An
        // `auth` or `other` failure does not block: the next rating tries
        // again, so saving resumes once a token is entered or the network is
        // back.
        if (outcome.kind === 'conflict' || outcome.kind === 'refused') this._writesBlocked = true
        // Spaced repetition keeps the card so the rating can be retried.
        // Concentric keeps going with this rating, session-only — it never
        // needed the writer, and stalling on a 401 would leave the default
        // mode stuck on a token-protected server.
        if (this._mode !== 'concentric') return
      } else {
        this._message = null
        saved = true
        entry = parseHistoryEntry(outcome.entry) ?? entry
      }
    }

    // Kept sorted, as `parseHistory` returns it: FSRS refuses to replay a
    // review older than the one before it, and a clock behind the note's
    // latest entry (or a hand-edited future date) would otherwise throw here.
    card.history = [...card.history, entry].sort((a, b) => a.at.getTime() - b.at.getTime())
    card.state = replay(this._scheduler, card.history)
    this._ratingCounts = { ...this._ratingCounts, [rating]: this._ratingCounts[rating] + 1 }
    const verb = saved ? 'Saved' : 'Rated'

    if (this._mode === 'concentric') {
      this._rateConcentric(rating, verb)
      return
    }
    this._fresh.delete(index)
    if (card.state && dueThisSession(card.state, entry.at)) {
      // Learn-ahead: back at the end of the queue rather than in N minutes.
      this._queue = [...this._queue, index]
    }
    this._announcement = `${verb}: ${RATING_LABELS[rating]}`
    this._goTo(this._position + 1)
  }

  /** Concentric's half of a rating: reinsert, maybe grow, show the next front. */
  private _rateConcentric(rating: Rating, verb: string): void {
    const before = this._concentric
    if (!before) return
    const next = rateConcentric(before, rating, Math.random)
    this._concentric = next
    // `_position` counts turns here, so the next card's slot animates in even
    // when it is the same card again.
    this._position++
    this._flipped = false
    this._historyOpen = false
    this._focusCardNext = true
    if (next.mastered) {
      this._finished = true
      this._announcement = 'Stack mastered'
    } else if (next.stack.length > before.stack.length) {
      this._announcement = `${verb}: ${RATING_LABELS[rating]}. ${next.stack.length - before.stack.length} more cards folded in`
    } else {
      this._announcement = `${verb}: ${RATING_LABELS[rating]}`
    }
  }

  /** Keep practising a mastered stack: same cards, same order. */
  private _keepPractising = (): void => {
    this._finished = false
    this._focusCardNext = true
  }

  private _onSizeChange = (e: Event): void => {
    this._resizeStack(Number((e.target as HTMLInputElement).value))
  }

  /** Concentric's stack to `size` cards: the Cards box, `+` and `-` all land here. */
  private _resizeStack(size: number): void {
    if (!this._concentric || !Number.isFinite(size)) return
    this._concentric = resizeConcentric(this._concentric, size, Math.random)
    this._finished = false
    this._announcement = `${this._concentric.stack.length} cards in play`
  }

  /** The modes the dropdown offers right now. */
  private _modeAvailable(mode: DeckMode): boolean {
    if (mode === 'srs') return this._srsUsable
    if (mode === 'concentric') return this._cards.length > 0
    return true
  }

  /** A mode key: switch if the mode is offered and not already on. */
  private _switchMode(mode: DeckMode): boolean {
    if (!this._modeAvailable(mode) || this._mode === mode) return false
    this._start(mode)
    this._announcement = `${MODE_LABELS[mode]} mode`
    return true
  }

  // ========================================
  // Section filter
  // ========================================

  private _toggleFilter = (): void => {
    this._filterOpen = !this._filterOpen
    this._filterReturnFocus = null
  }

  /** `f`: open the popover with focus on its first checkbox, remembering where focus was. */
  private _openFilterFromKeyboard(): void {
    this._filterReturnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
    this._filterOpen = true
    void this.updateComplete.then(() => this._filterChecks()[0]?.focus())
  }

  /**
   * Close the popover. Focus returns to the funnel when it was opened there;
   * when `f` opened it, to where `f` was pressed — or, since choosing sections
   * restarts the deck and replaces the card element, to the card now showing.
   */
  private _closeFilter(): void {
    this._filterOpen = false
    const back = this._filterReturnFocus
    this._filterReturnFocus = null
    void this.updateComplete.then(() => {
      const funnel = this.querySelector<HTMLElement>('.mbr-fc-filter-btn')
      const target = !back
        ? funnel
        : back.isConnected && this.contains(back)
          ? back
          : (this.querySelector<HTMLElement>('.mbr-fc-card') ?? funnel)
      target?.focus({ preventScroll: true })
    })
  }

  private _filterChecks(): HTMLInputElement[] {
    return Array.from(this.querySelectorAll<HTMLInputElement>('.mbr-fc-filter-pop input[type="checkbox"]'))
  }

  /** Keys inside the popover; true when the key was used. */
  private _onFilterKey(e: KeyboardEvent, target: EventTarget | undefined): boolean {
    const checks = this._filterChecks()
    const at = checks.indexOf(target as HTMLInputElement)
    const focusAt = (i: number) => checks[(i + checks.length) % checks.length]?.focus()
    switch (e.key) {
      case 'ArrowDown':
        focusAt(at + 1)
        return true
      case 'ArrowUp':
        focusAt(at - 1)
        return true
      case 'Home':
        focusAt(0)
        return true
      case 'End':
        focusAt(checks.length - 1)
        return true
      case 'a':
        this._applyFilter(new Set())
        return true
      case 'f':
        this._closeFilter()
        return true
      default:
        return false
    }
  }

  private _onPointerDown = (e: Event): void => {
    if (!this._filterOpen) return
    const wrap = this.querySelector('.mbr-fc-filter')
    if (wrap && e.target instanceof Node && !wrap.contains(e.target)) this._filterOpen = false
  }

  private _toggleSection(id: string, on: boolean): void {
    const next = new Set(this._filter)
    if (on) next.add(id)
    else next.delete(id)
    this._applyFilter(next)
  }

  /**
   * Narrow the deck: In order / Random restart on the eligible cards, spaced
   * repetition re-plans over them, Concentric swaps cards in and out of its
   * stack and keeps its size.
   */
  private _applyFilter(filter: ReadonlySet<string>): void {
    this._filter = filter
    if (this._mode === 'concentric' && this._concentric) {
      const front = this._concentric.stack[0]
      this._concentric = setConcentricEligible(this._concentric, this._eligible(), Math.random)
      this._finished = false
      if (this._concentric.stack[0] !== front) {
        this._position++
        this._flipped = false
        this._historyOpen = false
      }
    } else {
      this._start(this._mode)
      // Focus stays in the popover while the reader is still choosing.
      this._focusCardNext = false
    }
    const count = this._eligible().length
    this._announcement = `${count} card${count === 1 ? '' : 's'} selected`
  }

  // ========================================
  // Help
  // ========================================

  private _openHelp(): void {
    this._helpReturnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
    this._helpOpen = true
    void this.updateComplete.then(() => this.querySelector<HTMLElement>('.mbr-fc-help')?.focus())
  }

  private _closeHelp = (): void => {
    this._helpOpen = false
    const back = this._helpReturnFocus
    this._helpReturnFocus = null
    void this.updateComplete.then(() => {
      if (back?.isConnected) back.focus({ preventScroll: true })
      else this.querySelector<HTMLElement>('.mbr-fc-card')?.focus({ preventScroll: true })
    })
  }

  // ========================================
  // Keyboard
  // ========================================

  private _onKeydown = (e: KeyboardEvent): void => {
    // Nothing behind an open deck reacts to the keyboard (see the class doc).
    // Stopping propagation leaves default actions alone: Space still toggles a
    // focused checkbox, digits still reach the Cards box.
    e.stopPropagation()
    if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return
    if (this._handleKey(e)) e.preventDefault()
  }

  /** The deck's half of a key press; true when it was used. */
  private _handleKey(e: KeyboardEvent): boolean {
    const target = e.composedPath()[0]
    const el = target instanceof HTMLElement ? target : null
    const tag = el?.tagName ?? ''
    const onControl = tag === 'BUTTON' || tag === 'SELECT' || tag === 'A' || tag === 'SUMMARY'
    const typing = tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT'
    const inSizeBox = !!el?.closest('.mbr-fc-size')
    const inFilter = !!el?.closest('.mbr-fc-filter-pop')

    if (this._helpOpen) {
      if (e.key === 'Escape' || e.key === '?') {
        this._closeHelp()
        return true
      }
      if (e.key === 'Tab') this._trapFocus(e, this.querySelector<HTMLElement>('.mbr-fc-help') ?? this)
      // Everything else waits until the help is closed.
      return false
    }

    if (e.key === 'Escape') {
      // Innermost first: the Cards box, then the popover, then the deck.
      if (inSizeBox) this._leaveSizeBox(el as HTMLInputElement, false)
      else if (this._filterOpen) this._closeFilter()
      else this._close()
      return true
    }
    if (e.key === 'Tab') {
      this._trapFocus(e, this)
      return false
    }
    if (inSizeBox) {
      if (e.key !== 'Enter') return false
      this._leaveSizeBox(el as HTMLInputElement, true)
      return true
    }
    if (inFilter && this._onFilterKey(e, target)) return true
    if (typing) return false

    const ordered = this._ordered
    switch (e.key) {
      case ' ':
      case 'Spacebar':
        // A focused button keeps its own Space.
        if (onControl) return false
        this._advance()
        return true
      case 'Enter':
        if (onControl) return false
        this._flip()
        return true
      case 'ArrowRight':
      case 'PageDown':
      case 'n':
        this._advance()
        return true
      case 'ArrowLeft':
      case 'PageUp':
        if (!ordered) return false
        this._back()
        return true
      case 'Home':
        if (!ordered || this._queue.length === 0) return false
        this._goTo(0)
        return true
      case 'End':
        if (!ordered || this._queue.length === 0) return false
        this._goTo(this._queue.length - 1)
        return true
      case '1':
      case '2':
      case '3':
      case '4':
        if (!this._flipped || !this._showRatings) return false
        void this._rate(RATINGS[Number(e.key) - 1])
        return true
      case 'c':
      case 'o':
      case 'r':
      case 's':
        return this._switchMode(MODE_KEYS[e.key])
      case 'w':
        this._toggleSwap()
        return true
      case 'f':
        if (this._sections.length < 2) return false
        if (this._filterOpen) this._closeFilter()
        else this._openFilterFromKeyboard()
        return true
      case '+':
      case '=':
      case '-': {
        if (this._mode !== 'concentric' || !this._concentric) return false
        this._resizeStack(displaySize(this._concentric) + (e.key === '-' ? -1 : 1))
        return true
      }
      case '#': {
        const box = this.querySelector<HTMLInputElement>('.mbr-fc-size input')
        if (!box) return false
        box.focus()
        box.select()
        return true
      }
      case '?':
        this._openHelp()
        return true
      default:
        return false
    }
  }

  /** Enter applies what was typed, Esc puts the box back; both return to the card. */
  private _leaveSizeBox(box: HTMLInputElement, apply: boolean): void {
    if (apply) this._resizeStack(Number(box.value))
    else if (this._concentric) box.value = String(displaySize(this._concentric))
    this.querySelector<HTMLElement>('.mbr-fc-card')?.focus({ preventScroll: true })
  }

  /** Keep Tab inside `scope`: the dialog, or the help while it is open. */
  private _trapFocus(e: KeyboardEvent, scope: HTMLElement): void {
    const focusable = Array.from(scope.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
      (el) => !el.closest('[inert]') && el.getClientRects().length > 0
    )
    if (focusable.length === 0) return
    const first = focusable[0]
    const last = focusable[focusable.length - 1]
    const active = document.activeElement
    if (e.shiftKey && (active === first || !scope.contains(active))) {
      e.preventDefault()
      last.focus()
    } else if (!e.shiftKey && (active === last || !scope.contains(active))) {
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
    if (this._mode === 'concentric' && this._concentric) {
      return `${this._concentric.stack.length} in play · ${this._concentric.pool.length} left`
    }
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
        ${this._renderFilter()}
        ${this._mode === 'concentric' && this._concentric
          ? html`<label class="mbr-fc-size">
              Cards
              <input
                type="number"
                min="1"
                max=${this._concentric.eligible.length}
                aria-label="Cards in play"
                .value=${live(String(displaySize(this._concentric)))}
                @change=${this._onSizeChange}
              />
            </label>`
          : nothing}
        <select aria-label="Review mode" @change=${this._onModeChange}>
          ${(['order', 'random', 'concentric', 'srs'] as const)
            .filter((mode) => this._modeAvailable(mode))
            .map(
              (mode) => html`<option value=${mode} ?selected=${this._mode === mode}>${MODE_LABELS[mode]}</option>`
            )}
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

  private _renderFilter(): TemplateResult | typeof nothing {
    if (this._sections.length < 2) return nothing
    const count = this._filter.size
    const minLevel = Math.min(...this._sections.map((s) => s.level))
    return html`
      <div class="mbr-fc-filter">
        <button
          type="button"
          class="secondary outline mbr-fc-filter-btn ${count > 0 ? 'is-active' : ''}"
          aria-haspopup="true"
          aria-expanded=${this._filterOpen ? 'true' : 'false'}
          aria-label="Filter by section"
          title=${count > 0 ? `Filter by section (${count} selected)` : 'Filter by section'}
          @click=${this._toggleFilter}
        >
          ${FUNNEL}${count > 0 ? html`<span class="mbr-fc-filter-badge" aria-hidden="true"></span>` : nothing}
        </button>
        ${this._filterOpen
          ? html`<div class="mbr-fc-filter-pop" role="group" aria-label="Sections">
              <label>
                <input
                  type="checkbox"
                  .checked=${live(count === 0)}
                  @change=${() => this._applyFilter(new Set())}
                />
                All sections
              </label>
              ${this._sections.map(
                (section) => html`<label style="--mbr-fc-depth: ${section.level - minLevel}">
                  <input
                    type="checkbox"
                    .checked=${live(this._filter.has(section.id))}
                    @change=${(e: Event) =>
                      this._toggleSection(section.id, (e.target as HTMLInputElement).checked)}
                  />
                  <span>${section.text}</span>
                  <small>${section.cards.length}</small>
                </label>`
              )}
            </div>`
          : nothing}
      </div>
    `
  }

  private _renderCard(index: number): unknown {
    const { front, back } = this._faces(index)
    const flipped = this._flipped
    const side = flipped ? 'answer' : 'question'
    const label =
      this._mode === 'concentric'
        ? `Card in play, ${side} side`
        : `Card ${this._position + 1} of ${this._queue.length}, ${side} side`
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

  private _renderMastered(): TemplateResult {
    const inPlay = this._concentric?.stack.length ?? 0
    return html`
      <div class="mbr-fc-screen" role="status">
        <h3>Stack mastered</h3>
        <p>
          All ${inPlay} card${inPlay === 1 ? ' is' : 's are'} in play, and the last pass met the bar.
        </p>
        <div class="mbr-fc-actions">
          <button type="button" @click=${this._keepPractising}>Keep practising</button>
          <button type="button" class="secondary outline" @click=${() => this._start('concentric')}>
            New stack
          </button>
          <button type="button" class="secondary outline" @click=${this._close}>Close</button>
        </div>
      </div>
    `
  }

  private _renderFinished(): TemplateResult {
    if (this._mode === 'concentric') return this._renderMastered()
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
        ${this._ordered
          ? html`<button type="button" class="secondary outline" ?disabled=${this._position === 0} @click=${this._back}>
              ← Previous
            </button>`
          : nothing}
        <button type="button" class="mbr-fc-primary" @click=${this._flip}>Show answer</button>
        ${this._ordered
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
      // Spaced repetition says when the card is due next; Concentric, where in
      // the stack it goes back.
      const now = new Date()
      const due = this._mode === 'srs' ? previewDue(this._scheduler, card.state, now) : null
      const hint = (rating: Rating) =>
        due
          ? { text: formatInterval(now, due[rating]), spoken: `next review in ${formatInterval(now, due[rating])}` }
          : { text: CONCENTRIC_HINTS[rating], spoken: `back ${CONCENTRIC_HINTS[rating]}` }
      return html`
        <div class="mbr-fc-ratings" role="group" aria-label="Rate your recall">
          ${RATINGS.map((rating, i) => {
            const { text, spoken } = hint(rating)
            return html`
              <button
                type="button"
                class="mbr-fc-rate-${rating} ${this._pending === rating ? 'is-pending' : ''}"
                ?disabled=${this._pending !== null}
                aria-label="${i + 1}: ${RATING_LABELS[rating]}, ${spoken}"
                @click=${() => void this._rate(rating)}
              >
                <span><kbd>${i + 1}</kbd>${RATING_LABELS[rating]}</span>
                <small>${text}</small>
              </button>
            `
          })}
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
    if (this._showRatings) {
      return html`<p class="mbr-fc-hints" aria-hidden="true">
        <kbd>Space</kbd> flip · <kbd>1</kbd>–<kbd>4</kbd> rate · <kbd>Esc</kbd> close · <kbd>?</kbd> for shortcuts
      </p>`
    }
    return html`<p class="mbr-fc-hints" aria-hidden="true">
      <kbd>Space</kbd>/<kbd>→</kbd> flip, then next · <kbd>←</kbd> back · <kbd>Enter</kbd> flip ·
      <kbd>Esc</kbd> close · <kbd>?</kbd> for shortcuts
    </p>`
  }

  private _renderHelp(): TemplateResult | typeof nothing {
    if (!this._helpOpen) return nothing
    return html`
      <div class="mbr-fc-help-backdrop" @click=${(e: Event) => e.target === e.currentTarget && this._closeHelp()}>
        <div class="mbr-fc-help" role="dialog" aria-modal="true" aria-labelledby="mbr-fc-help-title" tabindex="-1">
          <header>
            <h3 id="mbr-fc-help-title">Keyboard shortcuts</h3>
            <button type="button" class="secondary outline" aria-label="Close shortcuts (Esc)" @click=${this._closeHelp}>
              Close
            </button>
          </header>
          ${SHORTCUT_GROUPS.map(
            (group) => html`
              <h4>${group.title}</h4>
              <table>
                <tbody>
                  ${group.shortcuts.map(
                    (shortcut) => html`<tr>
                      <th scope="row">
                        ${shortcut.keys.map((k, i) => html`${i > 0 ? ' ' : ''}<kbd>${k}</kbd>`)}
                      </th>
                      <td>${shortcut.action}</td>
                    </tr>`
                  )}
                </tbody>
              </table>
            `
          )}
        </div>
      </div>
    `
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
        ${this._renderHelp()}
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
