/**
 * The in-document half of the review feature: a marker per anchored note, and
 * a wash over the text each one covers.
 *
 * # Why both a marker element and CSS.highlights
 *
 * `::highlight()` is **paint only** — no hit testing, no events, no focus. So it
 * cannot be the thing a reader hovers or tabs to, and it cannot represent a
 * note at all when the quoted text is gone or when the note is about the file
 * as a whole. An injected element can do all three, which is why the marker is
 * the source of truth and the highlight is decoration. An engine without the
 * Custom Highlight API loses the wash and keeps every marker.
 *
 * Conversely the wash must NOT be an injected `<mark>`: wrapping a quote splits
 * text nodes, which invalidates `find-in-page.ts`'s `TextIndex` and every
 * enhancer's cached node references. `CSS.highlights` mutates no DOM at all.
 *
 * The marker is deliberately shaped like the existing `>>>` marginalia
 * (`theme.css`): a small glyph in the flow, with the note itself in a popover
 * on hover or focus.
 *
 * # Hovering the passage
 *
 * The washed passage shows the same card on hover, although a highlight takes
 * no events: one `pointermove` listener on the root (attached only while some
 * quote is painted, so a page without notes pays nothing) checks, at most once
 * per animation frame, whether the pointer is inside a painted range's client
 * rects (`passageHitsAt`). Nothing is attached per note. Touch has no hover,
 * so a tap on the passage (a touch `pointerup` that selected nothing) opens the
 * card pinned, like a tap on the marker. That is the touch route while a slide
 * deck plays, where `reveal-slides.css` hides the markers.
 *
 * # Why the glyph is generated content
 *
 * `.mbr-review-marker` has **no text content**; its glyph comes from a
 * `::before` rule in `theme.css`. A real text node inside the block would join
 * the block's text run, so selecting a paragraph would copy its markers too and
 * every `textContent` consumer would see them — the same trap
 * `mbr-heading-enhancer.ts` documents for the `#` permalink. The class is also
 * in `find-in-page.ts`'s `BLOCKED_SELECTOR` as belt and braces.
 */

import { buildTextIndex, rangeForMatch, type TextIndex } from '../find-in-page.ts'
import { LINE_SELECTOR, lineOfOffset } from './anchor.ts'
import { quoteMatches, pickNearest } from './reanchor.ts'
import { REVIEW_ANCHOR_PREFIX, noteTypeIcon, type ReviewNote } from './types.ts'

/** Class on every injected marker. Mirrored in `theme.css` and `BLOCKED_SELECTOR`. */
export const MARKER_CLASS = 'mbr-review-marker'

/** Highlight registry names. `mbr-find` and `mbr-find-active` are the find bar's. */
export const HIGHLIGHT_ALL = 'mbr-review'
export const HIGHLIGHT_ACTIVE = 'mbr-review-active'

/** Element id for a note's marker, so `#mbr-review-<id>` deep-links to it. */
export function markerId(note: ReviewNote): string {
  return `${REVIEW_ANCHOR_PREFIX}${note.id}`
}

/** The id in a `#mbr-review-…` fragment, or null. */
export function markerAnchorFromHash(hash: string): string | null {
  const match = new RegExp(`^#?(${REVIEW_ANCHOR_PREFIX}(.+))$`).exec(hash)
  return match ? match[1]! : null
}

/**
 * Feature-detect the Custom Highlight API.
 *
 * Resolved per call rather than once at module load so a test can stub it —
 * the same seam `mbr-find-bar.ts` uses, and the reason its own tests can cover
 * both the supported and unsupported paths.
 */
function highlightApi(): { registry: HighlightRegistry; Ctor: typeof Highlight } | null {
  const registry = (globalThis as { CSS?: { highlights?: HighlightRegistry } }).CSS?.highlights
  const Ctor = (globalThis as { Highlight?: typeof Highlight }).Highlight
  return registry && typeof Ctor === 'function' ? { registry, Ctor } : null
}

/** The smallest box `passageHitsAt` compares a pointer against. */
export interface HitRect {
  readonly left: number
  readonly top: number
  readonly right: number
  readonly bottom: number
}

/** One painted quote: the boxes it occupies on screen, and what it stands for. */
export interface PassageBoxes<T> {
  readonly rects: ArrayLike<HitRect>
  readonly value: T
}

/** What the pointer is over: every note whose quote is under it, and the line box hit. */
export interface PassageHover {
  readonly notes: ReviewNote[]
  readonly rect: DOMRect
}

/**
 * Every value whose boxes contain the point, in order, with the first box
 * hit. `slop` widens each box a little so the gaps between a wrapped quote's
 * lines do not flicker the card.
 */
export function passageHitsAt<T>(
  x: number,
  y: number,
  passages: readonly PassageBoxes<T>[],
  slop = 2
): { values: T[]; rect: HitRect | null } {
  const values: T[] = []
  let first: HitRect | null = null
  for (const passage of passages) {
    for (let i = 0; i < passage.rects.length; i++) {
      const box = passage.rects[i]!
      if (x >= box.left - slop && x <= box.right + slop && y >= box.top - slop && y <= box.bottom + slop) {
        values.push(passage.value)
        first ??= box
        break
      }
    }
  }
  return { values, rect: first }
}

/** A painted quote, kept so the pointer can be hit-tested against it. */
interface Passage {
  range: Range
  note: ReviewNote
}

/** A group of notes sharing one anchor, and the element they hang from. */
interface MarkerGroup {
  key: string
  line: number | null
  notes: ReviewNote[]
  host: Element
}

/**
 * Draws review markers into the rendered page.
 *
 * Instantiated by `<mbr-review>` rather than being a module singleton, so two
 * instances (a test, a second mount) cannot fight over the same DOM.
 */
export class ReviewMarkerLayer {
  private readonly root: HTMLElement
  private onActivate: (notes: ReviewNote[], marker: HTMLElement, pinned: boolean) => void
  private onPassageHover: ((hover: PassageHover | null) => void) | null
  private onPassageTap: ((hover: PassageHover) => void) | null
  private passages: Passage[] = []
  private pointer: { x: number; y: number } | null = null
  private hoverFrame = 0
  /** Ids of the notes under the pointer at the last check, joined. */
  private hoverKey = ''

  constructor(
    root: HTMLElement,
    onActivate: (notes: ReviewNote[], marker: HTMLElement, pinned: boolean) => void,
    onPassageHover: ((hover: PassageHover | null) => void) | null = null,
    onPassageTap: ((hover: PassageHover) => void) | null = null
  ) {
    this.root = root
    this.onActivate = onActivate
    this.onPassageHover = onPassageHover
    this.onPassageTap = onPassageTap
  }

  /** The notes whose painted quote contains the viewport point, if any. */
  notesAt(x: number, y: number): PassageHover | null {
    const hit = passageHitsAt(
      x,
      y,
      this.passages.map((passage) => ({ rects: passage.range.getClientRects(), value: passage.note }))
    )
    if (hit.values.length === 0 || hit.rect === null) return null
    const { left, top, right, bottom } = hit.rect
    return { notes: hit.values, rect: new DOMRect(left, top, right - left, bottom - top) }
  }

  private onPointerMove = (e: PointerEvent): void => {
    if (e.pointerType === 'touch') return
    this.pointer = { x: e.clientX, y: e.clientY }
    if (this.hoverFrame === 0) {
      this.hoverFrame = requestAnimationFrame(() => {
        this.hoverFrame = 0
        this.checkHover()
      })
    }
  }

  /** A finger lifted on a passage, without having selected text there. */
  private onPointerUp = (e: PointerEvent): void => {
    if (e.pointerType !== 'touch' || this.onPassageTap === null) return
    const selection = window.getSelection()
    if (selection && selection.rangeCount > 0 && !selection.isCollapsed) return
    const hover = this.notesAt(e.clientX, e.clientY)
    if (hover !== null) this.onPassageTap(hover)
  }

  private onPointerLeave = (): void => {
    this.pointer = null
    this.checkHover()
  }

  /** Report a change in which notes' passages are under the pointer. */
  private checkHover(): void {
    const hover = this.pointer ? this.notesAt(this.pointer.x, this.pointer.y) : null
    const key = hover ? hover.notes.map((note) => note.id).join(' ') : ''
    if (key === this.hoverKey) return
    this.hoverKey = key
    this.onPassageHover?.(hover)
  }

  private listenForHover(): void {
    if (this.passages.length === 0) return
    if (this.onPassageHover !== null) {
      this.root.addEventListener('pointermove', this.onPointerMove, { passive: true })
      this.root.addEventListener('pointerleave', this.onPointerLeave)
    }
    if (this.onPassageTap !== null) this.root.addEventListener('pointerup', this.onPointerUp)
  }

  private stopListeningForHover(): void {
    this.root.removeEventListener('pointermove', this.onPointerMove)
    this.root.removeEventListener('pointerleave', this.onPointerLeave)
    this.root.removeEventListener('pointerup', this.onPointerUp)
    if (this.hoverFrame !== 0) cancelAnimationFrame(this.hoverFrame)
    this.hoverFrame = 0
    this.pointer = null
    this.passages = []
    if (this.hoverKey !== '') {
      this.hoverKey = ''
      this.onPassageHover?.(null)
    }
  }

  /**
   * Rebuild every marker from scratch.
   *
   * A full re-render rather than a diff: there are a handful of nodes, and
   * diffing injected light-DOM against notes that may each have moved is
   * precisely where idempotency bugs live. `clear()` first means running twice
   * is the same as running once.
   */
  render(notes: readonly ReviewNote[]): void {
    this.clear()
    if (notes.length === 0) return

    const index = buildTextIndex(this.root)
    for (const group of this.groups(notes)) {
      this.inject(group)
    }
    this.paint(notes, index)
    this.listenForHover()
  }

  /** Remove every marker and drop the highlights. */
  clear(): void {
    this.stopListeningForHover()
    this.root.querySelectorAll(`.${MARKER_CLASS}`).forEach((marker) => marker.remove())
    const api = highlightApi()
    if (api) {
      api.registry.delete(HIGHLIGHT_ALL)
      api.registry.delete(HIGHLIGHT_ACTIVE)
    }
  }

  /**
   * One marker per distinct anchor, not per note.
   *
   * Three notes on the same line would otherwise stack three glyphs; the
   * popover lists them instead.
   */
  private groups(notes: readonly ReviewNote[]): MarkerGroup[] {
    const byKey = new Map<string, MarkerGroup>()
    for (const note of notes) {
      const host = this.hostFor(note)
      if (host === null) continue
      const key = note.line === null ? '__file__' : String(note.line)
      const existing = byKey.get(key)
      if (existing) {
        existing.notes.push(note)
      } else {
        byKey.set(key, { key, line: note.line, notes: [note], host })
      }
    }
    return [...byKey.values()]
  }

  /**
   * The element a note's marker hangs from.
   *
   * A file-level note, and a note whose line no longer exists in this render,
   * both fall back to the first block in the body — so a note is never silently
   * invisible just because the document moved underneath it.
   */
  private hostFor(note: ReviewNote): Element | null {
    if (note.line !== null) {
      const exact = this.root.querySelector(`[data-mbr-line="${note.line}"]`)
      if (exact) return exact
    }
    return this.root.querySelector(LINE_SELECTOR) ?? this.root.firstElementChild
  }

  private inject(group: MarkerGroup): void {
    const marker = document.createElement('span')
    marker.className = MARKER_CLASS
    // `data-mbr-review-type` drives the marker's colour from theme.css; the
    // artwork is the inline <svg> appended below. An <svg> contributes nothing
    // to `textContent`, so the marker stays textless and cannot be selected,
    // copied, or captured inside a note's own quote.
    marker.dataset.mbrReviewType = group.notes[0]!.type
    marker.dataset.mbrReviewCount = String(group.notes.length)
    marker.id = markerId(group.notes[0]!)
    marker.tabIndex = 0
    marker.setAttribute('role', 'button')
    const label =
      group.notes.length === 1
        ? `Review note: ${group.notes[0]!.type}`
        : `${group.notes.length} review notes`
    marker.setAttribute('aria-label', label)
    if (group.notes.some((note) => note.anchorState === 'lost')) {
      marker.dataset.mbrReviewStale = 'true'
    }

    // A click is reported as *pinned*: the card it opens must survive the
    // pointer leaving the marker, or the Edit and Delete buttons on it are
    // unreachable. Hover and focus stay transient.
    marker.addEventListener('mouseenter', () => this.onActivate(group.notes, marker, false))
    marker.addEventListener('focus', () => this.onActivate(group.notes, marker, false))
    marker.addEventListener('click', () => this.onActivate(group.notes, marker, true))
    marker.addEventListener('keydown', (e) => {
      // The marker is `role="button"` and focusable, so it owes the keyboard
      // the same activation a button gives.
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault()
        this.onActivate(group.notes, marker, true)
      }
    })

    const icon = noteTypeIcon(group.notes[0]!.type)
    if (icon !== null) marker.appendChild(icon)

    group.host.appendChild(marker)
  }

  /**
   * Paint the quoted text of every note that can still be located.
   *
   * Ranges are built once and handed to the registry in a single `set` — the
   * engine revalidates every live range on every DOM mutation, so one
   * registration of N ranges is very different from N registrations.
   */
  private paint(notes: readonly ReviewNote[], index: TextIndex): void {
    // The ranges are located even without the Highlight API: they are also
    // what a hover over the passage is tested against.
    const api = highlightApi()

    const ranges: Range[] = []
    for (const note of notes) {
      if (!note.quote) continue
      const matches = quoteMatches(index.text, note.quote)
      if (matches.starts.length === 0) continue
      const best = pickNearest(
        matches.starts.map((start) => ({ start, line: lineOfOffset(index, start) })),
        note.line
      )
      if (best === null) continue
      const range = rangeForMatch(index, best.start, best.start + matches.length)
      if (range) {
        ranges.push(range)
        this.passages.push({ range, note })
      }
    }

    if (api && ranges.length > 0) {
      api.registry.set(HIGHLIGHT_ALL, new api.Ctor(...ranges))
    }
  }
}
