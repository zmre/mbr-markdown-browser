/**
 * `<mbr-review-panel>` — every review note in the store, grouped by file.
 *
 * Lives in the lazy `mbr-review.min.js` chunk and imports nothing stateful from
 * the main bundle: the store, the URL resolver and the source reader all arrive
 * as properties, exactly as `<mbr-tasks-panel>` receives its endpoint and
 * `<mbr-mini-graph>` its services. See `index.ts` for the rule.
 *
 * # It never closes itself
 *
 * `Esc` and the close button dispatch `mbr-review-close`; the trigger owns the
 * element's lifetime. Same contract as `mbr-tasks-panel.ts`, and for the same
 * reason — the trigger is what knows whether the panel was opened by a click,
 * by a shortcut, or from a note that has just been written.
 *
 * # The markdown textarea is not a nicety
 *
 * `copyText` returns false on a non-secure origin (`mbr -s --host 0.0.0.0`
 * reached by IP has no `navigator.clipboard`, and `execCommand` may be refused
 * too). A copy button that silently does nothing there would lose the whole
 * review, so the same text is always available in a read-only textarea, which
 * a failed copy expands and selects.
 */
import { LitElement, css, html, nothing, type PropertyValues, type TemplateResult } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { safeHref } from '../safe-href.ts'
import { copyText } from './clipboard.ts'
import { formatReview } from './export-format.ts'
import './mbr-review-form.ts'
import type { ReviewSaveDetail } from './mbr-review-form.ts'
import { renderNoteCard } from './note-card.ts'
import { groupByFile, sortNotes, type NoteGroup } from './note-order.ts'
import {
  REVIEW_ANCHOR_PREFIX,
  type NoteAnchor,
  type NoteHrefResolver,
  type ReviewNote,
  type ReviewStoreApi,
  type SourceReader,
} from './types.ts'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-review-panel': MbrReviewPanelElement
  }
}

/**
 * Prefix of the element id a note's in-document marker carries.
 *
 * Exported because it is a contract with the marker layer, in the same spirit
 * as `mbr-task-N` / `mbr-marker-N`. The panel only ever *uses* it — a fragment
 * that names nothing costs a scroll and nothing else, so a marker layer that
 * has not been built yet, or a static page with no markers at all, degrades to
 * landing at the top of the file.
 */

/** How long the "Copied" confirmation stays up. */
const COPIED_MS = 2000

/** One walkable row: a file heading, or a note under it. */
type ReviewRow =
  | { kind: 'file'; groupIndex: number }
  | { kind: 'note'; groupIndex: number; noteIndex: number }

/** The flat row sequence the keyboard walks, in rendered order. */
function buildRows(groups: readonly NoteGroup[]): ReviewRow[] {
  const rows: ReviewRow[] = []
  groups.forEach((group, groupIndex) => {
    rows.push({ kind: 'file', groupIndex })
    group.notes.forEach((_note, noteIndex) => rows.push({ kind: 'note', groupIndex, noteIndex }))
  })
  return rows
}

/**
 * The true target of a keydown, seeing through shadow-root retargeting.
 *
 * The panel listens on `document`, where an event from its own shadow root is
 * retargeted to the host — so `e.target` is always `<mbr-review-panel>` and says
 * nothing about which control the user is on. Same reason `mbr-tasks-panel.ts`
 * and `mbr-keys.ts` use `composedPath`.
 */
function realTarget(e: KeyboardEvent): HTMLElement | null {
  const target = e.composedPath()[0]
  return target instanceof HTMLElement ? target : null
}

/** Whether a focused control activates itself on Enter and must keep the key. */
function ownsEnter(target: HTMLElement | null): boolean {
  if (!target) return false
  const tag = target.tagName
  return tag === 'BUTTON' || tag === 'SELECT' || tag === 'INPUT'
}

/** Whether a bare letter belongs to the focused control rather than the panel. */
function isTextEntry(target: HTMLElement | null): boolean {
  if (!target) return false
  const tag = target.tagName
  return tag === 'TEXTAREA' || tag === 'INPUT' || tag === 'SELECT'
}

@customElement('mbr-review-panel')
export class MbrReviewPanelElement extends LitElement {
  /** The note store. `null` renders the empty state rather than failing. */
  @property({ attribute: false })
  store: ReviewStoreApi | null = null

  /** Repo-relative source path → page URL. `null` when the URL is unknown. */
  @property({ attribute: false })
  resolveHref: NoteHrefResolver = () => null

  /** Source reader handed to the edit form's suggestion prefill. */
  @property({ attribute: false })
  readSource: SourceReader | null = null

  /**
   * Source path of the page the panel was opened from, in {@link ReviewNote.file}'s
   * string space; `null` on section and home pages.
   *
   * Decides whether activating a note navigates or scrolls: a note on the page
   * already open should not reload it.
   */
  @property({ attribute: false })
  currentFile: string | null = null

  @state() private _notes: readonly ReviewNote[] = []
  @state() private _focusRow = -1
  /** Id of the note showing its "Really delete?" step, or `null`. */
  @state() private _confirmingId: string | null = null
  /** The note being edited. Non-null exactly when the form is open. */
  @state() private _editing: ReviewNote | null = null
  @state() private _copyState: 'idle' | 'copied' | 'failed' = 'idle'

  /**
   * True once "Clear all" has been pressed and is waiting for confirmation.
   *
   * Two steps for the same reason a single note's delete has them: this is the
   * one control in the product that can destroy a whole review, and there is no
   * undo — the notes live only in this browser's localStorage.
   */
  @state() private _confirmingClear = false

  /** A clear that the store refused (a newer mbr's envelope, or disabled storage). */
  @state() private _clearFailed = false
  @state() private _showMarkdown = false

  private _unsubscribe: (() => void) | null = null
  private _copyTimeout: number | null = null

  override connectedCallback() {
    super.connectedCallback()
    document.addEventListener('keydown', this._handleKeydown)
  }

  override disconnectedCallback() {
    super.disconnectedCallback()
    document.removeEventListener('keydown', this._handleKeydown)
    this._unsubscribe?.()
    this._unsubscribe = null
    if (this._copyTimeout !== null) {
      clearTimeout(this._copyTimeout)
      this._copyTimeout = null
    }
  }

  /**
   * Attach to the injected store.
   *
   * In `updated` rather than `firstUpdated` so a store swapped in later is
   * picked up too; Lit records properties set before upgrade, so this also runs
   * on the first update for the store the trigger set before `appendChild`.
   */
  override updated(changed: PropertyValues) {
    if (!changed.has('store')) return
    this._unsubscribe?.()
    this._unsubscribe = this.store?.subscribe(() => this._refresh()) ?? null
    this._refresh()
  }

  /**
   * Re-read the store.
   *
   * The store is the single source of truth, so every write is followed by one
   * of these rather than by a local edit — which is what keeps the panel honest
   * when a write is refused (a read-only store) or coerced.
   */
  private _refresh() {
    this._notes = sortNotes(this.store?.all() ?? [])
    // An armed delete refers to a note that may no longer be there, and a
    // confirmation left over from a previous list would be aimed at whatever
    // took its place.
    this._confirmingId = null
    // Same reasoning for the clear-all confirm: another window may have
    // emptied the store while this one sat armed, and a confirm describing
    // "all 7 notes" must never survive the list changing under it.
    this._confirmingClear = false
    if (this._focusRow >= this._rows.length) this._focusRow = this._rows.length - 1
  }

  private get _writable(): boolean {
    return this.store?.writable() ?? false
  }

  private get _groups(): NoteGroup[] {
    return groupByFile(this._notes)
  }

  private get _rows(): ReviewRow[] {
    return buildRows(this._groups)
  }

  /** The note at a row, or `null` for a file heading. */
  private _noteAt(row: ReviewRow | undefined): ReviewNote | null {
    if (!row || row.kind !== 'note') return null
    return this._groups[row.groupIndex]?.notes[row.noteIndex] ?? null
  }

  private get _focusedNote(): ReviewNote | null {
    return this._noteAt(this._rows[this._focusRow])
  }

  /** The whole export, in the order the panel shows. */
  private get _markdown(): string {
    return formatReview(this._notes)
  }

  /**
   * A note's page URL with its marker fragment, or `null`.
   *
   * `null` when `resolveHref` cannot name a page for the file — a note taken in
   * one repository and read in another, or a file that has since been deleted.
   * The card renders the location as plain text rather than a dead link.
   */
  private _hrefFor(note: ReviewNote): string | null {
    const base = this.resolveHref(note.file)
    if (base === null) return null
    return safeHref(`${base}#${REVIEW_ANCHOR_PREFIX}${note.id}`)
  }

  // ========================================
  // Actions
  // ========================================

  private _close() {
    this.dispatchEvent(new CustomEvent('mbr-review-close'))
  }

  /**
   * Open a note: scroll to it when it is on the page already, otherwise load
   * its page.
   *
   * The same-file case deliberately does not navigate. The panel is an overlay
   * on the document the note is about half the time, and reloading it to reach
   * a fragment would throw away the scroll position, the panel and any other
   * unsaved state on the page.
   */
  private _activate(note: ReviewNote) {
    if (note.file === this.currentFile) {
      const target = document.getElementById(`${REVIEW_ANCHOR_PREFIX}${note.id}`)
      if (target && typeof target.scrollIntoView === 'function') {
        target.scrollIntoView({ block: 'center' })
      }
      this._close()
      return
    }
    const href = this._hrefFor(note)
    if (href !== null) window.location.assign(href)
  }

  /**
   * Edit a note — by delegation if the host wants it, in place otherwise.
   *
   * The `mbr-review-edit` event is **cancelable**, and cancelling it is how a
   * host says "I own the editor". A trigger that already renders its own
   * `<mbr-review-form>` — because `r` on a selection must open one with no panel
   * at all — should claim the edit here, so that both routes share one form
   * instance and one save path. A panel used on its own, or under a host that
   * does not listen, still edits in place rather than doing nothing at all.
   */
  private _startEdit(note: ReviewNote) {
    if (!this._writable) return
    this._confirmingId = null
    const claimed = !this.dispatchEvent(
      new CustomEvent<ReviewNote>('mbr-review-edit', {
        detail: note,
        bubbles: true,
        composed: true,
        cancelable: true,
      })
    )
    if (claimed) return
    this._editing = note
  }

  private _closeForm() {
    this._editing = null
  }

  private _handleSave(e: CustomEvent<ReviewSaveDetail>) {
    const editing = this._editing
    if (!editing || !this.store) {
      this._closeForm()
      return
    }
    const { type, body, suggestion } = e.detail
    this.store.save({ ...editing, type, body, suggestion, updatedAt: Date.now() })
    this._closeForm()
    this._refresh()
  }

  private _deleteNote(note: ReviewNote) {
    if (!this._writable) return
    this.store?.remove(note.id)
    this._confirmingId = null
    this._refresh()
  }

  /**
   * Copy the export.
   *
   * A failure is not reported as an error and then dropped: it expands the
   * markdown box and selects it, so the very next keystroke a user would try
   * (`⌘/Ctrl+C`) does the right thing.
   */
  private async _copy(): Promise<void> {
    const ok = await copyText(this._markdown)
    if (this._copyTimeout !== null) {
      clearTimeout(this._copyTimeout)
      this._copyTimeout = null
    }
    if (!ok) {
      this._copyState = 'failed'
      this._showMarkdown = true
      await this.updateComplete
      const area = this.shadowRoot?.querySelector<HTMLTextAreaElement>('#review-markdown')
      area?.focus()
      area?.select()
      return
    }
    this._copyState = 'copied'
    this._copyTimeout = window.setTimeout(() => {
      this._copyTimeout = null
      this._copyState = 'idle'
    }, COPIED_MS)
  }

  // ========================================
  // Keyboard
  // ========================================

  private _handleKeydown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      if (this._editing) {
        this._closeForm()
        return
      }
      // Back out of the clear-all confirm before closing the panel: Esc is the
      // reflex for "no", and closing the panel outright would leave the reader
      // unsure whether they had just deleted everything.
      if (this._confirmingClear) {
        this._confirmingClear = false
        return
      }
      this._close()
      return
    }

    if (e.ctrlKey && !e.metaKey) {
      switch (e.key.toLowerCase()) {
        case 'n':
          e.preventDefault()
          this._moveFocus(1)
          return
        case 'p':
          e.preventDefault()
          this._moveFocus(-1)
          return
        case 'd':
          e.preventDefault()
          this._scrollList(0.5)
          return
        case 'u':
          e.preventDefault()
          this._scrollList(-0.5)
          return
        case 'f':
          e.preventDefault()
          this._scrollList(1)
          return
        case 'b':
          e.preventDefault()
          this._scrollList(-1)
          return
      }
      // Everything else with Ctrl belongs to the browser — Ctrl+C above all,
      // which is the manual copy the failed-copy path just set up.
      return
    }

    if (e.metaKey || e.altKey) return

    // The same two exemptions the task panel documents: a focused <select> owns
    // its arrow keys, and buttons, selects and inputs own Enter.
    const target = realTarget(e)
    if (target?.tagName === 'SELECT' && e.key.startsWith('Arrow')) return
    if (e.key === 'Enter' && ownsEnter(target)) return

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault()
        this._moveFocus(1)
        return
      case 'ArrowUp':
        e.preventDefault()
        this._moveFocus(-1)
        return
      case 'Enter': {
        const note = this._focusedNote
        if (!note) return
        e.preventDefault()
        this._activate(note)
        return
      }
    }

    // Bare letters are the panel's only unmodified single-key commands, so they
    // are the ones that must yield: to a text field, and to the form, where
    // every letter is content. The form stops `Esc` itself; these never reach
    // it because they are refused here.
    if (this._editing || isTextEntry(target)) return

    switch (e.key) {
      case 'e': {
        const note = this._focusedNote
        if (!note) return
        e.preventDefault()
        this._startEdit(note)
        return
      }
      case 'd': {
        const note = this._focusedNote
        if (!note || !this._writable) return
        e.preventDefault()
        // The keyboard route uses the card's own two-step confirmation rather
        // than a second one: `d` arms, `d` again deletes. Deleting a note on a
        // single keypress would be one fat finger away from losing a comment
        // with no undo anywhere in the feature.
        if (this._confirmingId === note.id) {
          this._deleteNote(note)
        } else {
          this._confirmingId = note.id
        }
        return
      }
      case 'c':
        e.preventDefault()
        void this._copy()
        return
    }
  }

  private _moveFocus(delta: number) {
    const rows = this._rows
    if (rows.length === 0) {
      this._focusRow = -1
      return
    }
    this._focusRow = Math.max(-1, Math.min(this._focusRow + delta, rows.length - 1))
    // A confirmation is about the row it was armed on; walking away from that
    // row disarms it rather than leaving a primed button behind.
    this._confirmingId = null
    void this.updateComplete.then(() => {
      const focused = this.shadowRoot?.querySelector('.focused')
      if (focused && typeof focused.scrollIntoView === 'function') {
        focused.scrollIntoView({ block: 'nearest' })
      }
    })
  }

  private _scrollList(pages: number) {
    const list = this.shadowRoot?.querySelector('.review-list')
    if (!list) return
    // Full pages keep 50px of context, matching the task panel and mbr-browse.
    const amount =
      Math.abs(pages) >= 1
        ? Math.sign(pages) * Math.max(list.clientHeight - 50, 0)
        : list.clientHeight * pages
    list.scrollBy({ top: amount, behavior: 'smooth' })
  }

  // ========================================
  // Render
  // ========================================

  override render() {
    return html`
      <div class="review-backdrop" @click=${() => this._close()}></div>
      <div class="review-container" role="dialog" aria-label="Review notes">
        ${this._renderHeader()}
        ${this.store !== null && !this._writable ? this._renderReadOnlyBanner() : nothing}
        ${this._editing ? this._renderForm(this._editing) : nothing}
        <div class="review-list">${this._renderList()}</div>
        ${this._renderMarkdown()} ${this._renderFooter()}
      </div>
    `
  }

  private _renderHeader(): TemplateResult {
    const count = this._notes.length
    return html`
      <header class="review-header">
        <h2>Review notes</h2>
        <span class="review-count">${count} note${count === 1 ? '' : 's'}</span>
        <button
          class="review-action"
          ?disabled=${count === 0}
          @click=${() => void this._copy()}
          title="Copy the whole review as markdown"
        >
          Copy as markdown
        </button>
        ${this._renderClearAll(count)}
        <button class="review-close" @click=${() => this._close()} aria-label="Close">✕</button>
        ${this._clearFailed
          ? html`<p class="copy-status failed" role="status"
              >Could not clear the notes — this browser refused the write.</p
            >`
          : nothing}
        ${this._copyState === 'idle'
          ? nothing
          : html`<p
              class="copy-status ${this._copyState}"
              role="status"
              >${this._copyState === 'copied'
                ? 'Copied'
                : 'Copy failed — select and copy manually (⌘/Ctrl+C)'}</p
            >`}
      </header>
    `
  }

  /**
   * "Clear all", as a two-step confirm rendered in place.
   *
   * In place rather than `window.confirm`, so it matches the note cards' own
   * confirm and cannot be suppressed by a browser's "prevent additional
   * dialogs" checkbox — which, once ticked, would make the button silently
   * destroy the review with no prompt at all.
   *
   * The confirm states the count, because "clear all" from a filtered-looking
   * list is exactly where someone expects it to mean "the ones I can see".
   * It clears every note in every file, so it says so.
   */
  private _renderClearAll(count: number): TemplateResult | typeof nothing {
    if (count === 0 || !this._writable) return nothing

    if (!this._confirmingClear) {
      return html`
        <button
          class="review-action review-danger"
          @click=${() => (this._confirmingClear = true)}
          title="Delete every review note, in every file"
        >
          Clear all
        </button>
      `
    }

    return html`
      <span class="clear-confirm" role="group" aria-label="Confirm clearing all notes">
        <span class="clear-confirm-text"
          >Delete all ${count} note${count === 1 ? '' : 's'}? This cannot be undone — copy
          the review first if you want to keep it.</span
        >
        <button class="review-action review-danger" @click=${() => this._clearAll()}>
          Delete
        </button>
        <button class="review-action" @click=${() => (this._confirmingClear = false)}>
          Cancel
        </button>
      </span>
    `
  }

  /**
   * Delete every note.
   *
   * Deliberately does NOT copy the review to the clipboard first. That was the
   * first version and it was wrong twice over: it silently replaces whatever
   * the reader had on their clipboard, and it made the clear conditional on a
   * clipboard write that fails outright on a non-secure origin — refusing an
   * action the user explicitly asked for. The confirm points at "Copy as
   * markdown" instead and lets them decide.
   */
  private _clearAll(): void {
    this._confirmingClear = false
    this._confirmingId = null
    this._focusRow = 0
    this._clearFailed = this.store?.clear() === false
    this._refresh()
  }

  /**
   * The store came from a newer mbr.
   *
   * Prominent rather than a footnote: everything still reads, so without this
   * the only symptom would be an Edit button that has quietly disappeared.
   */
  private _renderReadOnlyBanner(): TemplateResult {
    return html`
      <p class="review-banner" role="status">
        These notes were written by a newer version of mbr. They are shown read-only so this build
        cannot overwrite fields it does not understand — copying still works.
      </p>
    `
  }

  private _renderForm(note: ReviewNote): TemplateResult {
    const anchor: NoteAnchor = {
      file: note.file,
      line: note.line,
      endLine: note.endLine,
      quote: note.quote,
    }
    return html`
      <div class="review-form-slot">
        <mbr-review-form
          .anchor=${anchor}
          .existing=${note}
          .readSource=${this.readSource}
          .defaultType=${note.type}
          @mbr-review-save=${(e: CustomEvent<ReviewSaveDetail>) => this._handleSave(e)}
          @mbr-review-cancel=${() => this._closeForm()}
        ></mbr-review-form>
      </div>
    `
  }

  private _renderList(): TemplateResult | TemplateResult[] {
    const groups = this._groups
    if (groups.length === 0) {
      return html`<div class="review-empty">
        No review notes yet — select some text and press <kbd>r</kbd>.
      </div>`
    }

    const rows = this._rows
    // One pass over the row list keeps the rendered order and the keyboard's
    // idea of it identical by construction — the same arrangement as the task
    // panel's results list.
    return rows.map((row, index) =>
      row.kind === 'file'
        ? this._renderFileHeading(groups[row.groupIndex], index)
        : this._renderNoteRow(groups[row.groupIndex].notes[row.noteIndex], index)
    )
  }

  private _renderFileHeading(group: NoteGroup, rowIndex: number): TemplateResult {
    return html`
      <h3
        class="file-heading ${rowIndex === this._focusRow ? 'focused' : ''}"
        @mouseenter=${() => (this._focusRow = rowIndex)}
      >
        <span class="file-name">${group.file}</span>
        <span class="file-count">${group.notes.length}</span>
      </h3>
    `
  }

  private _renderNoteRow(note: ReviewNote, rowIndex: number): TemplateResult {
    return renderNoteCard({
      note,
      focused: rowIndex === this._focusRow,
      href: this._hrefFor(note),
      editable: this._writable,
      confirming: this._confirmingId === note.id,
      onFocus: () => (this._focusRow = rowIndex),
      onNavigate: () => this._activate(note),
      onEdit: () => this._startEdit(note),
      onDelete: () => (this._confirmingId = note.id),
      onConfirmDelete: () => this._deleteNote(note),
      onCancelDelete: () => (this._confirmingId = null),
    })
  }

  /**
   * The export, verbatim, in a read-only textarea.
   *
   * Always rendered behind a disclosure rather than only on failure, so the
   * control a user needs after a failed copy is one they have already seen.
   */
  private _renderMarkdown(): TemplateResult {
    return html`
      <div class="markdown-pane">
        <button
          class="markdown-toggle"
          aria-expanded=${this._showMarkdown}
          @click=${() => (this._showMarkdown = !this._showMarkdown)}
        >
          ${this._showMarkdown ? '▼' : '▶'} Show markdown
        </button>
        ${this._showMarkdown
          ? html`<textarea
              id="review-markdown"
              readonly
              rows="8"
              aria-label="Review as markdown"
              .value=${this._markdown}
            ></textarea>`
          : nothing}
      </div>
    `
  }

  private _renderFooter(): TemplateResult {
    return html`
      <div class="review-footer">
        <span class="footer-hint">
          <kbd>^n</kbd><kbd>^p</kbd> navigate <kbd>↵</kbd> open <kbd>e</kbd> edit <kbd>d</kbd>
          delete <kbd>c</kbd> copy <kbd>^d</kbd><kbd>^u</kbd> scroll <kbd>esc</kbd> close
        </span>
      </div>
    `
  }

  // ========================================
  // Styles
  // ========================================

  static override styles = css`
    :host {
      display: contents;
    }

    /* ---- Shell: the same backdrop/container rungs as <mbr-tasks-panel>,
     * which can never be open at the same time as this one. ---- */

    .review-backdrop {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.4);
      z-index: 1000;
      animation: fadeIn 0.2s ease;
    }

    @keyframes fadeIn {
      from {
        opacity: 0;
      }
      to {
        opacity: 1;
      }
    }

    .review-container {
      position: fixed;
      left: 0;
      top: 0;
      height: 100%;
      width: 560px;
      max-width: 100vw;
      display: flex;
      flex-direction: column;
      overflow: hidden;
      z-index: 1001;
      background: var(--pico-card-background-color, #f8f9fa);
      color: var(--pico-color, #333);
      border-right: 1px solid var(--pico-muted-border-color, #eee);
      animation: slideIn 0.25s ease;
    }

    @keyframes slideIn {
      from {
        transform: translateX(-100%);
      }
      to {
        transform: translateX(0);
      }
    }

    /* ---- Header ---- */

    .review-header {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 0.5rem;
      padding: 0.6rem 0.75rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      background: var(--pico-background-color, #fff);
      flex-shrink: 0;
    }

    .review-header h2 {
      margin: 0;
      font-size: 0.85rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      color: var(--pico-muted-color, #5c6b73);
    }

    .review-count {
      flex: 1;
      font-size: 0.72rem;
      color: var(--pico-muted-color, #5c6b73);
    }

    .review-action {
      margin: 0;
      font-size: 0.78rem;
      padding: 0.25rem 0.6rem;
      border-radius: 4px;
      cursor: pointer;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
    }

    .review-action[disabled] {
      opacity: 0.55;
      cursor: not-allowed;
    }

    /* Destructive, so it reads as destructive — but only in its colour. It is
     * not given visual weight over Copy, which is the action a reader wants
     * far more often. */
    .review-danger {
      color: var(--pico-del-color, #dc3545);
      border-color: var(--pico-del-color, #dc3545);
    }

    .review-danger:hover {
      background: var(--pico-del-color, #dc3545);
      color: var(--pico-primary-inverse, #fff);
    }

    .clear-confirm {
      display: flex;
      align-items: center;
      gap: 0.4rem;
      flex-wrap: wrap;
    }

    .clear-confirm-text {
      font-size: 0.78rem;
      color: var(--pico-del-color, #dc3545);
    }

    .review-close {
      background: transparent;
      border: none;
      cursor: pointer;
      font-size: 1rem;
      line-height: 1;
      padding: 0.3rem 0.4rem;
      border-radius: 4px;
      color: var(--pico-muted-color, #5c6b73);
    }

    .copy-status {
      flex-basis: 100%;
      margin: 0;
      font-size: 0.72rem;
      color: var(--pico-muted-color, #5c6b73);
    }

    .copy-status.failed {
      color: var(--mbr-review-issue-color, #c62828);
    }

    .review-banner {
      margin: 0;
      padding: 0.5rem 0.75rem;
      font-size: 0.76rem;
      color: var(--pico-color, #333);
      background: var(--pico-background-color, #fff);
      border-bottom: 1px solid var(--mbr-review-suggestion-color, #f57c00);
      border-inline-start: 4px solid var(--mbr-review-suggestion-color, #f57c00);
      flex-shrink: 0;
    }

    .review-form-slot {
      padding: 0.5rem 0.75rem;
      border-bottom: 1px solid var(--pico-muted-border-color, #eee);
      flex-shrink: 0;
    }

    /* ---- List ---- */

    .review-list {
      flex: 1;
      overflow-y: auto;
      padding: 0.25rem 0.75rem 0.75rem;
    }

    .review-empty {
      padding: 2rem 1rem;
      text-align: center;
      color: var(--pico-muted-color, #5c6b73);
    }

    .file-heading {
      display: flex;
      align-items: baseline;
      gap: 0.4rem;
      margin: 0.7rem 0 0.25rem;
      padding: 0.2rem 0.3rem;
      border-radius: 4px;
      font-size: 0.78rem;
      font-weight: 600;
      color: var(--pico-muted-color, #5c6b73);
    }

    .file-name {
      flex: 1;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    }

    .file-count {
      font-size: 0.7rem;
      font-variant-numeric: tabular-nums;
    }

    .file-heading.focused,
    .note-card.focused {
      outline: 2px solid var(--pico-primary, #0d6efd);
      outline-offset: -2px;
    }

    /* ---- Note cards ----
     *
     * A deliberate mirror of what templates/theme.css gives the in-document
     * review markers. Those rules cannot cross into this shadow root, but the
     * --mbr-review-* custom properties they are built from DO inherit across
     * the boundary, so restating the rules against the same tokens keeps the
     * panel and the page looking like one product — theme switches and the
     * dark-mode overrides included. Every var() carries a fallback so the panel
     * still renders correctly against a repository's own older theme.css.
     */

    .note-card {
      display: flex;
      flex-direction: column;
      gap: 0.3rem;
      margin-bottom: 0.35rem;
      padding: 0.45rem 0.55rem;
      border-radius: 5px;
      cursor: pointer;
      background: var(--pico-background-color, #fff);
      border-inline-start: 3px solid var(--pico-muted-border-color, #eee);
    }

    .note-card:hover {
      background: var(--pico-secondary-background, #f0f0f0);
    }

    .note-card[data-type='issue'] {
      border-inline-start-color: var(--mbr-review-issue-color, #c62828);
    }
    .note-card[data-type='suggestion'] {
      border-inline-start-color: var(--mbr-review-suggestion-color, #f57c00);
    }
    .note-card[data-type='note'] {
      border-inline-start-color: var(--mbr-review-note-color, #1976d2);
    }
    .note-card[data-type='praise'] {
      border-inline-start-color: var(--mbr-review-praise-color, #388e3c);
    }
    .note-card[data-type='question'] {
      border-inline-start-color: var(--mbr-review-question-color, #7b1fa2);
    }
    .note-card[data-type='insight'] {
      border-inline-start-color: var(--mbr-review-insight-color, #0288d1);
    }

    .note-head {
      display: flex;
      align-items: baseline;
      flex-wrap: wrap;
      gap: 0.4rem;
    }

    .note-badge {
      display: inline-flex;
      align-items: baseline;
      gap: 0.25rem;
      font-size: 0.7rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.04em;
    }

    .note-badge[data-type='issue'] {
      color: var(--mbr-review-issue-color, #c62828);
    }
    .note-badge[data-type='suggestion'] {
      color: var(--mbr-review-suggestion-color, #f57c00);
    }
    .note-badge[data-type='note'] {
      color: var(--mbr-review-note-color, #1976d2);
    }
    .note-badge[data-type='praise'] {
      color: var(--mbr-review-praise-color, #388e3c);
    }
    .note-badge[data-type='question'] {
      color: var(--mbr-review-question-color, #7b1fa2);
    }
    .note-badge[data-type='insight'] {
      color: var(--mbr-review-insight-color, #0288d1);
    }

    /* The icon arrives as an inline --mbr-review-icon custom property naming
     * one of theme.css's six mask URIs, so the artwork is written down once
     * and the panel matches the in-document markers. Custom properties cross
     * the shadow boundary; the rules using them do not, hence this. Painted
     * with background-color so the badge's own per-type colour drives it. */
    /* The icon is a real <svg> from icon-svg.ts, the same call the
     * in-document markers make. It strokes with currentColor, so the badge's
     * per-type colour above is all this has to set. */
    .note-icon {
      display: inline-flex;
      align-items: center;
    }

    .note-icon > svg {
      display: block;
      width: 1em;
      height: 1em;
    }

    .note-loc {
      flex: 1;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
      font-size: 0.72rem;
      color: var(--pico-muted-color, #5c6b73);
      text-decoration: none;
    }

    .note-stale {
      font-size: 0.68rem;
      padding: 0.05em 0.5em;
      border-radius: 0.5em;
      color: var(--mbr-review-stale-color, var(--pico-muted-color, #5c6b73));
      border: 1px solid var(--mbr-review-stale-color, var(--pico-muted-color, #5c6b73));
      white-space: nowrap;
    }

    .note-actions {
      display: inline-flex;
      gap: 0.25rem;
    }

    .note-action {
      margin: 0;
      background: transparent;
      border: 1px solid transparent;
      border-radius: 3px;
      cursor: pointer;
      font-size: 0.7rem;
      padding: 0.1rem 0.4rem;
      color: var(--pico-muted-color, #5c6b73);
    }

    .note-action:hover {
      border-color: var(--pico-muted-border-color, #ccc);
      color: var(--pico-color, #333);
    }

    .note-action.danger {
      color: var(--mbr-review-issue-color, #c62828);
      border-color: var(--mbr-review-issue-color, #c62828);
    }

    /* Two lines of context, no more: the quote identifies the note's place, and
     * a long selection would push the comment itself off screen. */
    .note-quote {
      margin: 0;
      padding: 0 0 0 0.5rem;
      border-inline-start: 2px solid var(--pico-muted-border-color, #eee);
      font-size: 0.76rem;
      color: var(--pico-muted-color, #5c6b73);
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      display: -webkit-box;
      -webkit-line-clamp: 2;
      line-clamp: 2;
      -webkit-box-orient: vertical;
      overflow: hidden;
    }

    .note-body {
      font-size: 0.85rem;
      white-space: pre-wrap;
      overflow-wrap: anywhere;
    }

    .note-suggestion {
      margin: 0;
      padding: 0.35rem 0.5rem;
      border-radius: 4px;
      background: var(--pico-card-background-color, #f8f9fa);
      overflow-x: auto;
    }

    .note-suggestion code {
      font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
      font-size: 0.76rem;
      white-space: pre-wrap;
    }

    /* ---- Markdown pane ---- */

    .markdown-pane {
      display: flex;
      flex-direction: column;
      gap: 0.3rem;
      padding: 0.4rem 0.75rem;
      border-top: 1px solid var(--pico-muted-border-color, #eee);
      background: var(--pico-background-color, #fff);
      flex-shrink: 0;
    }

    .markdown-toggle {
      align-self: flex-start;
      margin: 0;
      background: transparent;
      border: none;
      cursor: pointer;
      font-size: 0.72rem;
      padding: 0.1rem 0;
      color: var(--pico-muted-color, #5c6b73);
    }

    #review-markdown {
      width: 100%;
      box-sizing: border-box;
      margin: 0;
      resize: vertical;
      font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
      font-size: 0.72rem;
      padding: 0.35rem 0.5rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
    }

    /* ---- Footer ---- */

    .review-footer {
      padding: 0.4rem 0.75rem;
      border-top: 1px solid var(--pico-muted-border-color, #eee);
      background: var(--pico-background-color, #fff);
      flex-shrink: 0;
    }

    .footer-hint {
      font-size: 0.7rem;
      color: var(--pico-muted-color, #5c6b73);
    }

    kbd {
      display: inline-block;
      padding: 0.05rem 0.3rem;
      margin-right: 0.1rem;
      font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
      font-size: 0.68rem;
      color: var(--pico-color, #333);
      background: var(--pico-secondary-background, #f5f5f5);
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 3px;
    }

    @media (max-width: 620px) {
      .review-container {
        width: 100vw;
      }
    }
  `
}
