/**
 * `<mbr-review>` — the trigger and the in-document half of review notes.
 *
 * Ships in the main bundle and stays small: the two keyboard shortcuts, the
 * floating "Add note" button that follows a selection, the chat-bubble FAB, the
 * marker layer, and the re-anchoring pass. The panel and the form live in the
 * lazily-imported `mbr-review.min.js` chunk.
 *
 * Deliberately **no button in the page header**: the entry points are `r` on a
 * selection, `R` for the list, and the FAB once there is anything to list.
 *
 * This element owns every piece of state the chunk is not allowed to hold — the
 * store, the source reader, the site index — and injects them as Lit
 * properties, because a second copy of `review-store.ts` inside the chunk would
 * carry a second cache and a second subscriber set.
 */

import { LitElement, css, html, nothing, type TemplateResult } from 'lit'
import { customElement, query, state } from 'lit/decorators.js'
import { isReviewEnabled, subscribeSiteNav } from './shared.js'
import { isInputTarget, isModalOpen } from './mbr-keys.js'
import { getMbrAssetBase, scheduleIdleTask, waitForDom } from './dynamic-loader.js'
// `RawRead` is aliased: `task-toggle.ts`'s `SourceRead` is the *transport*
// result (`ok`/`lines`/`status`), while `review/types.ts`'s is what the form
// consumes (`text`/`exact`). Two different things that wanted the same name.
import { currentDocumentPath, readSourceLines, type SourceRead as RawRead } from './task-toggle.js'
import { positionAnchored, positionAt } from './anchored-popover.js'
import type { MbrOverlay } from './overlay.js'
import {
  addNote,
  allNotes,
  applyReanchor,
  isWritable,
  notesFor,
  removeNote,
  reviewStore,
  saveNote,
  subscribe,
} from './review-store.js'
import { anchorFromSelection, lineOfOffset, reviewRoot } from './review/anchor.js'
import { indexFileOf, knownUrlPaths, resolveFileUrlPath } from './review/file-url.js'
import { ReviewMarkerLayer, markerAnchorFromHash, markerId } from './review/markers.js'
import { displayQuote } from './review/note-model.js'
import { nextAnchorState } from './review/reanchor.js'
import { buildTextIndex } from './find-in-page.js'
import { DEFAULT_NOTE_TYPE, type NoteAnchor, type NoteType, type ReviewNote } from './review/types.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-review': MbrReviewElement
  }
}

/**
 * Import the lazy panel chunk. Same seam, and the same reasons, as
 * `mbr-tasks.ts`: a runtime-computed URL so it resolves from any page depth,
 * `@vite-ignore` so vite does not code-split it at build time, and an
 * overridable module-level binding so tests can stub an import happy-dom
 * cannot execute.
 */
let importReviewChunk: () => Promise<unknown> = () => {
  const url = new URL(getMbrAssetBase() + 'components/mbr-review.min.js', document.baseURI).href
  return import(/* @vite-ignore */ url)
}

/** Test hook: replace the chunk importer (module-level seam). */
export function setReviewChunkImporter(importer: () => Promise<unknown>): void {
  importReviewChunk = importer
  reviewChunkPromise = null
}

/** Shared once-per-page promise for the chunk load; `true` when usable. */
let reviewChunkPromise: Promise<boolean> | null = null

function loadReviewChunk(): Promise<boolean> {
  if (!reviewChunkPromise) {
    reviewChunkPromise = importReviewChunk()
      .then(() => true)
      .catch((err) => {
        console.warn('Failed to load the review chunk:', err)
        return false // No panel this page load; the trigger stays inert.
      })
  }
  return reviewChunkPromise
}

@customElement('mbr-review')
export class MbrReviewElement extends LitElement implements MbrOverlay {
  @state() private _panelOpen = false
  @state() private _formOpen = false
  @state() private _loading = false
  @state() private _chunkReady = false
  @state() private _count = 0

  /** Anchor for the note being written, or null for a file-level note. */
  @state() private _draftAnchor: NoteAnchor | null = null
  /** The note being edited, or null when creating. */
  @state() private _editing: ReviewNote | null = null

  /** Viewport rect of the live selection, when there is a reviewable one. */
  @state() private _selectionRect: DOMRect | null = null

  /**
   * Viewport rect the note form anchors to: the selection being commented on,
   * or the marker of the note being edited.
   *
   * Kept separately from `_selectionRect` because opening the form clears the
   * selection highlight but must not lose the place the reader was looking at.
   */
  private _formRect: DOMRect | null = null

  @query('.form-anchor')
  private _formAnchor!: HTMLElement | null

  private _markers: ReviewMarkerLayer | null = null
  private _unsubscribeStore: (() => void) | null = null
  private _unsubscribeNav: (() => void) | null = null
  private _selectionFrame = 0
  private _urlPaths: ReadonlySet<string> = new Set()
  private _indexFile = 'index.md'
  /** Marker whose card was opened by a click, and so stays up. */
  private _pinnedMarker: HTMLElement | null = null
  /** Set while Esc returns focus to a marker, so the card does not reopen. */
  private _suppressActivate = false
  private _popover: HTMLDivElement | null = null
  private _hideTimer: number | undefined

  // ========================================
  // Lifecycle
  // ========================================

  override connectedCallback(): void {
    super.connectedCallback()
    if (!isReviewEnabled()) return

    document.addEventListener('keydown', this._handleKeydown)
    document.addEventListener('selectionchange', this._onSelectionChange)
    window.addEventListener('scroll', this._onViewportChange, { passive: true })
    window.addEventListener('resize', this._onViewportChange)
    window.addEventListener('hashchange', this._revealFromHash)
    document.addEventListener('pointerdown', this._onDocumentPointerDown)

    this._unsubscribeStore = subscribe(() => this._onStoreChanged())
    this._unsubscribeNav = subscribeSiteNav((navState) => {
      if (!navState.data) return
      this._urlPaths = knownUrlPaths(navState.data)
      this._indexFile = indexFileOf(navState.data)
    })

    // Markers are page decoration, not first paint. Same idle scheduling as
    // `mbr-footnote-preview.ts`.
    void waitForDom()
      .then(() => scheduleIdleTask(() => this._install()))
      .catch((err) => console.warn('Review marker setup failed:', err))
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback()
    document.removeEventListener('keydown', this._handleKeydown)
    document.removeEventListener('selectionchange', this._onSelectionChange)
    window.removeEventListener('scroll', this._onViewportChange)
    window.removeEventListener('resize', this._onViewportChange)
    window.removeEventListener('hashchange', this._revealFromHash)
    document.removeEventListener('pointerdown', this._onDocumentPointerDown)
    this._unsubscribeStore?.()
    this._unsubscribeNav?.()
    this._markers?.clear()
    this._popover?.remove()
    this._popover = null
    if (this._selectionFrame) cancelAnimationFrame(this._selectionFrame)
  }

  /**
   * First paint of the markers, plus the re-anchoring pass.
   *
   * Bails when the element has been disconnected since the idle task was
   * queued. `scheduleIdleTask` cannot be cancelled, and `reviewRoot()` is a
   * document-level query, so without this guard a removed element still
   * injects markers and still writes re-anchored lines to the store —
   * observable as a live element's own pass finding the work already done.
   */
  private _install(): void {
    if (!this.isConnected) return
    const root = reviewRoot()
    if (root === null) return
    this._markers = new ReviewMarkerLayer(root, (notes, marker, pinned) =>
      this._showPopover(notes, marker, pinned)
    )
    this._reanchor()
    this._redraw()
    this._revealFromHash()
  }

  // ========================================
  // MbrOverlay
  // ========================================

  /**
   * True while this element owns the keyboard.
   *
   * Covers the **form** as well as the panel: the form's `<select>` and its
   * buttons are not `isInputTarget`, so without this a bare `j` would scroll
   * the page behind an open form. It stays false when the feature is off — an
   * `isOpen` that reported true behind an invisible overlay would make
   * `isModalOpen()` suppress every bare-letter shortcut on the page.
   *
   * `_loading` is deliberately NOT part of it, the same way `<mbr-tasks>`
   * reports only `_isOpen`: the flag it would add is already implied, since
   * both `_openPanel` and `_openForm` set their own state before awaiting the
   * chunk. Including it would also make `close()` fail to close — the load
   * outlives the click that cancelled it.
   */
  public get isOpen(): boolean {
    return this._panelOpen || this._formOpen
  }

  public open(): void {
    void this._openPanel()
  }

  public close(): void {
    this._panelOpen = false
    this._formOpen = false
    this._draftAnchor = null
    this._editing = null
  }

  // ========================================
  // Keyboard
  // ========================================

  /**
   * `r` adds a note, `R` opens the list.
   *
   * The six-part guard is `mbr-tasks.ts`'s, and both are **open-only rather
   * than toggles** for its documented reason: once the form is open its
   * textarea makes `isInputTarget` true, so `r` has to reach it as a literal
   * character. `Esc` and the buttons close.
   *
   * Note the `!shiftKey` / `shiftKey` split — the same asymmetry `mbr-keys.ts`
   * uses for `f` and `F`.
   */
  private _handleKeydown = (e: KeyboardEvent): void => {
    // Esc closes a pinned note card. Checked before every other guard: the
    // card can be open while a marker holds focus, and Esc must reach it.
    if (e.key === 'Escape' && this._pinnedMarker !== null) {
      e.preventDefault()
      const marker = this._pinnedMarker
      this._hidePopover()
      // Returning focus to the marker would re-fire its `focus` handler and
      // reopen the card Esc just closed. Suppress that one activation; the
      // next hover or click behaves normally.
      this._suppressActivate = true
      marker.focus()
      window.setTimeout(() => (this._suppressActivate = false), 0)
      return
    }

    if (e.ctrlKey || e.metaKey || e.altKey) return
    if (!isReviewEnabled() || this.isOpen || this._loading) return
    if (isInputTarget(e) || isModalOpen()) return

    if (e.key === 'r' && !e.shiftKey) {
      e.preventDefault()
      void this._openForm(anchorFromSelection(this._file() ?? '', window.getSelection()))
      return
    }
    if (e.key === 'R' && e.shiftKey) {
      e.preventDefault()
      void this._openPanel()
    }
  }

  // ========================================
  // Selection tracking
  // ========================================

  /**
   * Track the selection so the "Add note" button can follow it.
   *
   * Coalesced through `requestAnimationFrame`: `selectionchange` fires for
   * every pointer move of a drag-select, and each handler measures a rect.
   */
  private _onSelectionChange = (): void => {
    if (this._selectionFrame) cancelAnimationFrame(this._selectionFrame)
    this._selectionFrame = requestAnimationFrame(() => {
      this._selectionFrame = 0
      this._measureSelection()
    })
  }

  private _onViewportChange = (): void => {
    if (this._selectionRect !== null) this._measureSelection()
  }

  /** Rect of the live selection right now, or null. */
  private _currentSelectionRect(): DOMRect | null {
    const selection = window.getSelection()
    if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return null
    const rect = selection.getRangeAt(0).getBoundingClientRect()
    return rect.width === 0 && rect.height === 0 ? null : rect
  }

  private _measureSelection(): void {
    if (this.isOpen) {
      this._selectionRect = null
      return
    }
    const file = this._file()
    const selection = window.getSelection()
    if (file === null || anchorFromSelection(file, selection) === null) {
      this._selectionRect = null
      return
    }
    const rect = selection!.getRangeAt(0).getBoundingClientRect()
    // happy-dom, and a detached range, report all zeros.
    this._selectionRect = rect.width === 0 && rect.height === 0 ? null : rect
  }

  // ========================================
  // Notes
  // ========================================

  private _file(): string | null {
    return currentDocumentPath()
  }

  private _onStoreChanged(): void {
    this._count = allNotes().length
    this._redraw()
  }

  private _redraw(): void {
    const file = this._file()
    this._count = allNotes().length
    if (this._markers && file !== null) this._markers.render(notesFor(file))
  }

  /**
   * Re-locate every note on this page whose quoted text may have moved.
   *
   * Runs on load and after any store change, both of which hand us a fresh DOM.
   * Nothing incremental is tracked because nothing needs to be: `<mbr-live-reload>`
   * reloads the page when the file changes on disk, and the WYSIWYG editor's
   * save lands on a fresh render too.
   */
  private _reanchor(): void {
    const file = this._file()
    const root = reviewRoot()
    if (file === null || root === null) return

    const notes = notesFor(file).filter((note) => note.quote !== null)
    if (notes.length === 0) return

    const index = buildTextIndex(root)
    const updates = new Map<string, Partial<ReviewNote>>()
    for (const note of notes) {
      const next = nextAnchorState(note, index.text, (offset) => lineOfOffset(index, offset))
      if (
        next.line !== note.line ||
        next.endLine !== note.endLine ||
        next.anchorState !== note.anchorState
      ) {
        updates.set(note.id, next)
      }
    }
    // One write and one notification, and none at all in the common case where
    // nothing moved.
    applyReanchor(updates)
  }

  // ========================================
  // Panel and form
  // ========================================

  private async _openPanel(): Promise<void> {
    if (this._panelOpen || !isReviewEnabled()) return
    this._selectionRect = null
    this._panelOpen = true
    if (await this._ensureChunk()) return
    this._panelOpen = false
  }

  private async _openForm(anchor: NoteAnchor | null): Promise<void> {
    if (this._formOpen || !isReviewEnabled()) return
    const file = this._file()
    if (file === null) return
    // Remember where the reader was looking BEFORE clearing the selection
    // highlight, so the form can open next to the text being commented on.
    this._formRect = this._selectionRect ?? this._currentSelectionRect()
    this._selectionRect = null
    this._draftAnchor = anchor ?? { file, line: null, endLine: null, quote: null }
    this._editing = null
    this._formOpen = true
    if (await this._ensureChunk()) return
    this._formOpen = false
    this._draftAnchor = null
  }

  private async _editNote(note: ReviewNote): Promise<void> {
    const marker = document.getElementById(markerId(note))
    this._formRect = marker?.getBoundingClientRect() ?? this._formRect
    this._editing = note
    this._draftAnchor = {
      file: note.file,
      line: note.line,
      endLine: note.endLine,
      quote: note.quote,
    }
    this._formOpen = true
    if (await this._ensureChunk()) return
    this._formOpen = false
    this._editing = null
  }

  /** Load the chunk if needed; true when the UI can render. */
  private async _ensureChunk(): Promise<boolean> {
    if (this._chunkReady) return true
    this._loading = true
    try {
      this._chunkReady = await loadReviewChunk()
    } finally {
      this._loading = false
    }
    return this._chunkReady
  }

  private _onSave(e: CustomEvent<{ type: NoteType; body: string; suggestion: string | null }>): void {
    const anchor = this._draftAnchor
    const existing = this._editing
    const detail = e.detail

    if (existing !== null) {
      saveNote({ ...existing, ...detail, updatedAt: Date.now() })
    } else if (anchor !== null) {
      addNote({
        file: anchor.file,
        line: anchor.line,
        endLine: anchor.endLine,
        quote: anchor.quote,
        type: detail.type,
        body: detail.body,
        suggestion: detail.suggestion,
      })
    }

    this._formOpen = false
    this._draftAnchor = null
    this._editing = null
    this._redraw()
  }

  /**
   * Take over editing from the panel.
   *
   * `mbr-review-edit` is dispatched cancelable, and the panel falls back to an
   * inline form of its own if nobody claims it. **`preventDefault()` is what
   * claims it** — without this call both forms open at once, each with its own
   * idea of what is being edited. One form instance, one save path.
   */
  private _onPanelEdit = (e: CustomEvent<ReviewNote>): void => {
    e.preventDefault()
    void this._editNote(e.detail)
  }

  private _onCancelForm(): void {
    this._formOpen = false
    this._draftAnchor = null
    this._editing = null
  }

  /**
   * Read the source lines a suggestion should start from.
   *
   * Falls back to the rendered quote when `/.mbr/raw` is unavailable — it sits
   * behind `check_edit_access`, so a 403 is the ordinary answer on a server
   * started without `--edit`, not a failure. The form says which one it got.
   */
  private _readSource = async (
    file: string,
    line: number | null,
    endLine: number | null
  ): Promise<{ text: string; exact: boolean }> => {
    const fallback = displayQuote(this._draftAnchor?.quote ?? null)
    if (line === null) return { text: fallback, exact: false }

    let read: RawRead
    try {
      read = await readSourceLines(file)
    } catch {
      return { text: fallback, exact: false }
    }
    if (!read.ok) return { text: fallback, exact: false }

    const slice = read.lines.slice(line - 1, (endLine ?? line))
    if (slice.length === 0) return { text: fallback, exact: false }
    return { text: slice.join('\n'), exact: true }
  }

  private _resolveHref = (file: string): string | null => {
    return resolveFileUrlPath(file, this._indexFile, this._urlPaths)
  }

  /**
   * Place the note form beside the text it is about.
   *
   * Runs after every update rather than once on open: the chunk loads
   * asynchronously, so the form does not exist on the update that opened it,
   * and its height changes when the type switches to `suggestion` and the code
   * box appears.
   */
  override updated(): void {
    const anchor = this._formAnchor
    if (!anchor || !this._formOpen) return
    if (this._formRect) {
      positionAt(this._formRect, anchor)
    } else {
      // A file-level note with nothing selected has no place on the page to
      // point at; sit above the FAB, out of the way of the text.
      anchor.style.left = ''
      anchor.style.top = ''
      anchor.classList.add('form-corner')
    }
  }

  // ========================================
  // In-document popover
  // ========================================

  private _getPopover(): HTMLDivElement {
    if (this._popover) return this._popover
    const el = document.createElement('div')
    el.className = 'mbr-review-popover'
    el.setAttribute('role', 'tooltip')
    el.style.display = 'none'
    el.addEventListener('mouseenter', () => this._cancelHide())
    el.addEventListener('mouseleave', () => this._scheduleHide())
    document.body.appendChild(el)
    this._popover = el
    return el
  }

  /**
   * Show the note card for a marker.
   *
   * `pinned` is a click rather than a hover. A pinned card stays up until it is
   * dismissed deliberately: hover semantics make the Edit and Delete buttons
   * almost unusable, because the pointer has to cross the gap between the
   * marker and the card, and on a touch device there is no hover at all.
   * Clicking the same marker again closes it.
   */
  private _showPopover(notes: ReviewNote[], marker: HTMLElement, pinned = false): void {
    // A focus restored by Esc must not reopen what Esc closed.
    if (!pinned && this._suppressActivate) return

    if (pinned && this._pinnedMarker === marker) {
      this._hidePopover()
      return
    }

    this._cancelHide()
    const popover = this._getPopover()
    popover.replaceChildren(...notes.map((note) => this._popoverCard(note)))
    popover.style.display = 'block'
    positionAnchored(marker, popover)

    this._pinnedMarker = pinned ? marker : null
    if (!pinned) {
      marker.addEventListener('mouseleave', () => this._scheduleHide(), { once: true })
      marker.addEventListener('blur', () => this._scheduleHide(), { once: true })
    }
  }

  /** Dismiss a pinned card on a click anywhere outside it. */
  private _onDocumentPointerDown = (e: MouseEvent): void => {
    if (this._pinnedMarker === null) return
    const path = e.composedPath()
    if (path.includes(this._pinnedMarker)) return
    if (this._popover && path.includes(this._popover)) return
    this._hidePopover()
  }

  /** Plain DOM rather than Lit: the popover is a `body >` child, not ours. */
  private _popoverCard(note: ReviewNote): HTMLElement {
    const card = document.createElement('div')
    card.className = 'mbr-review-popover-card'

    const head = document.createElement('div')
    head.className = 'mbr-review-popover-head'
    head.textContent = note.type.toUpperCase()
    if (note.anchorState === 'lost') head.textContent += ' · text not found'
    card.appendChild(head)

    const body = document.createElement('div')
    body.className = 'mbr-review-popover-body'
    body.textContent = note.body
    card.appendChild(body)

    if (note.type === 'suggestion' && note.suggestion) {
      const pre = document.createElement('pre')
      pre.textContent = note.suggestion
      card.appendChild(pre)
    }

    const actions = document.createElement('div')
    actions.className = 'mbr-review-popover-actions'
    const edit = document.createElement('button')
    edit.type = 'button'
    edit.textContent = 'Edit'
    edit.addEventListener('click', () => {
      this._hidePopover()
      void this._editNote(note)
    })
    const del = document.createElement('button')
    del.type = 'button'
    del.textContent = 'Delete'
    del.addEventListener('click', () => {
      // One click, but the popover only appears on a deliberate hover or focus
      // of a marker, and the note is recoverable from nowhere — so confirm.
      if (window.confirm('Delete this review note?')) {
        removeNote(note.id)
        this._hidePopover()
        this._redraw()
      }
    })
    actions.append(edit, del)
    card.appendChild(actions)
    return card
  }

  private _scheduleHide(): void {
    if (this._pinnedMarker !== null) return
    this._cancelHide()
    this._hideTimer = window.setTimeout(() => this._hidePopover(), 150)
  }

  private _cancelHide(): void {
    if (this._hideTimer !== undefined) {
      clearTimeout(this._hideTimer)
      this._hideTimer = undefined
    }
  }

  private _hidePopover(): void {
    this._cancelHide()
    this._pinnedMarker = null
    if (this._popover) this._popover.style.display = 'none'
  }

  /** Scroll to and flash a `#mbr-review-<id>` deep link. */
  private _revealFromHash = (): void => {
    const id = markerAnchorFromHash(window.location.hash)
    if (id === null) return
    const marker = document.getElementById(id)
    if (marker === null) return
    marker.scrollIntoView({ block: 'center' })
    marker.focus()
  }

  // ========================================
  // Render
  // ========================================

  private _renderSelectionButton(): TemplateResult | typeof nothing {
    const rect = this._selectionRect
    if (rect === null) return nothing
    // Above the selection when there is room, otherwise below it.
    const top = rect.top > 44 ? rect.top - 40 : rect.bottom + 8
    const left = Math.max(8, Math.min(rect.left, window.innerWidth - 130))
    return html`
      <button
        class="selection-button"
        style="top:${top}px;left:${left}px"
        @mousedown=${(e: Event) => e.preventDefault()}
        @click=${() => void this._openForm(anchorFromSelection(this._file() ?? '', window.getSelection()))}
        title="Add a review note (r)"
      >
        Add note
      </button>
    `
  }

  private _renderFab(): TemplateResult | typeof nothing {
    if (this._count === 0 || this.isOpen) return nothing
    return html`
      <button
        class="review-fab"
        @click=${() => void this._openPanel()}
        aria-label="Open review notes (R)"
        title="Review notes (R)"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="20"
          height="20"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"></path>
        </svg>
        <span class="fab-count">${this._count}</span>
      </button>
    `
  }

  override render(): TemplateResult | typeof nothing {
    // The template gates on `server_mode and review_enabled`; this second guard
    // covers a stale custom `_footer.html`.
    if (!isReviewEnabled()) return nothing

    return html`
      ${this._renderSelectionButton()}
      ${this._renderFab()}
      ${this._loading
        ? html`<div class="review-loading" role="status">Loading review notes…</div>`
        : nothing}
      ${this._panelOpen && this._chunkReady
        ? html`
            <mbr-review-panel
              .store=${reviewStore}
              .resolveHref=${this._resolveHref}
              .readSource=${this._readSource}
              .currentFile=${this._file()}
              @mbr-review-close=${() => this.close()}
              @mbr-review-edit=${this._onPanelEdit}
            ></mbr-review-panel>
          `
        : nothing}
      ${this._formOpen && this._chunkReady
        ? html`
            <div class="form-anchor">
            <mbr-review-form
              .anchor=${this._draftAnchor}
              .existing=${this._editing}
              .readSource=${this._readSource}
              .defaultType=${DEFAULT_NOTE_TYPE}
              .writable=${isWritable()}
              @mbr-review-save=${this._onSave}
              @mbr-review-cancel=${() => this._onCancelForm()}
            ></mbr-review-form>
            </div>
          `
        : nothing}
    `
  }

  static override styles = css`
    :host {
      display: contents;
    }

    /* Follows the selection, so it must sit above the content but below any
     * panel this element itself opens. */
    .selection-button {
      position: fixed;
      z-index: 850;
      padding: 0.3rem 0.7rem;
      font-size: 0.85rem;
      line-height: 1.2;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 6px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
      box-shadow: 0 6px 18px -8px rgba(0, 0, 0, 0.45);
      cursor: pointer;
      white-space: nowrap;
    }

    .review-fab {
      position: fixed;
      right: 1rem;
      bottom: 1rem;
      z-index: 800;
      width: 3rem;
      height: 3rem;
      display: flex;
      align-items: center;
      justify-content: center;
      border: none;
      border-radius: 50%;
      background: var(--pico-primary, #0172ad);
      color: var(--pico-primary-inverse, #fff);
      box-shadow: 0 8px 20px -6px rgba(0, 0, 0, 0.45);
      cursor: pointer;
    }

    .fab-count {
      position: absolute;
      top: -0.2rem;
      right: -0.2rem;
      min-width: 1.25rem;
      height: 1.25rem;
      padding: 0 0.25rem;
      display: flex;
      align-items: center;
      justify-content: center;
      font-size: 0.7rem;
      font-weight: 700;
      border-radius: 0.75rem;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
      border: 1px solid var(--pico-muted-border-color, #ccc);
    }

    /*
     * The note form is fixed-position and carries the card chrome.
     *
     * mbr-review-form is display:contents and styles only its own fields,
     * because the panel embeds it inline in its own layout. Rendered
     * standalone from here it would otherwise lay out in normal flow at the
     * very end of the document -- which is where mbr-review sits in
     * _footer.html -- so autofocusing its textarea scrolled the reader to the
     * bottom of the page, away from the text they were commenting on.
     */
    .form-anchor {
      position: fixed;
      z-index: 1100;
      width: min(420px, calc(100vw - 2rem));
      max-height: 80vh;
      overflow-y: auto;
      padding: 0.15rem;
      background: var(--pico-background-color, #fff);
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 8px;
      box-shadow: 0 12px 32px -12px rgba(0, 0, 0, 0.45);
    }

    /* No selection to point at: park it clear of the floating action button. */
    .form-anchor.form-corner {
      right: 1rem;
      bottom: 5rem;
    }

    .review-loading {
      position: fixed;
      inset: 0;
      z-index: 1200;
      display: flex;
      align-items: center;
      justify-content: center;
      background: rgba(0, 0, 0, 0.35);
      color: var(--pico-color, #333);
    }

    @media print {
      .selection-button,
      .review-fab,
      .review-loading {
        display: none !important;
      }
    }
  `
}
