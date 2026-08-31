/**
 * One review note, as rendered in the all-files panel.
 *
 * A plain render function rather than its own custom element, for the same
 * reason as `tasks/task-card.ts`: the panel already owns a shadow root, and
 * keeping the card there means one copy of the `--mbr-review-*` styles instead
 * of one per card.
 *
 * # The type glyph is generated content
 *
 * The type icon is a real `<svg>` built by `icon-svg.ts` — the same call the
 * in-document markers make, so the panel and the page cannot drift apart. Lit
 * renders a `Node` handed to a child expression directly, so no `unsafeSVG`
 * and no artwork restated here.
 *
 * The icon inherits `currentColor` from the badge's per-type colour,
 * never as a text node — so a reader selecting a card's text, and anything that
 * walks `textContent`, sees the label and not the glyph. That is done here by
 * handing the glyph to CSS through an inline `--mbr-review-icon` custom
 * property rather than by restating the six glyphs in `static styles`: the
 * colour rules already have to be per-type, but the glyph does not, and a second
 * copy of it could drift from `TYPE_DEFS`.
 *
 * # Delete confirms, and the confirmation is the panel's state
 *
 * Deleting is one keystroke away (`d`) and has no undo, so the button arms a
 * second one rather than firing. The armed note is held by the panel, not by
 * the card, because the card is a function with no state of its own and because
 * only the panel knows which note the keyboard is on.
 */
import { html, nothing, type TemplateResult } from 'lit'
import { formatLocation } from './export-format.ts'
import { displayQuote } from './note-model.ts'
import { createIconSvg } from './icon-svg.ts'
import { typeDef } from './types.ts'
import type { ReviewNote } from './types.ts'

/**
 * Wording for a note whose anchor no longer matches the document, or `null`
 * when it still does. `exact` and a file-level note's `null` both read as "no
 * badge", which is why this returns a string rather than taking a boolean.
 */
function staleLabel(note: ReviewNote): string | null {
  if (note.anchorState === 'moved') return 'moved'
  if (note.anchorState === 'lost') return 'text not found'
  return null
}

export interface NoteCardOptions {
  note: ReviewNote
  focused: boolean
  /** Page URL for the note's file, or `null` when it cannot be resolved. */
  href: string | null
  /**
   * Whether Edit and Delete are offered at all. False when the store was
   * written by a newer mbr — no control is shown that cannot work, the same
   * bargain `task-card.ts` makes with a marker's checkbox.
   */
  editable: boolean
  /** Whether this card is showing its "Really delete?" step. */
  confirming: boolean
  onFocus: () => void
  onNavigate: (e: Event) => void
  onEdit: () => void
  /** Arms the confirm step. Never deletes. */
  onDelete: () => void
  /** The second press/click: this one actually deletes. */
  onConfirmDelete: () => void
  onCancelDelete: () => void
}

export function renderNoteCard(options: NoteCardOptions): TemplateResult {
  const { note, focused, href, editable, confirming } = options
  const def = typeDef(note.type)
  const quote = displayQuote(note.quote)
  const stale = staleLabel(note)

  return html`
    <article
      class="note-card ${focused ? 'focused' : ''}"
      data-note-id=${note.id}
      data-type=${note.type}
      @mouseenter=${options.onFocus}
      @click=${options.onNavigate}
    >
      <header class="note-head">
        <span class="note-badge" data-type=${note.type}>
          ${def
            ? html`<span class="note-icon" aria-hidden="true"
                >${createIconSvg(def.id)}</span
              >`
            : nothing}
          <span class="note-type-label">${def?.label ?? note.type}</span>
        </span>
        ${href === null
          ? html`<span class="note-loc">${formatLocation(note)}</span>`
          : html`<a
              class="note-loc"
              href=${href}
              @click=${(e: Event) => e.stopPropagation()}
              >${formatLocation(note)}</a
            >`}
        ${stale !== null
          ? html`<span class="note-stale" title="The quoted text has changed since this note"
              >${stale}</span
            >`
          : nothing}
        ${editable ? renderActions(options, confirming) : nothing}
      </header>
      ${quote.length > 0 ? html`<blockquote class="note-quote">${quote}</blockquote>` : nothing}
      ${note.body.length > 0 ? html`<div class="note-body">${note.body}</div>` : nothing}
      ${renderSuggestion(note)}
    </article>
  `
}

/**
 * Edit/Delete, or the armed confirmation that replaces them.
 *
 * Every button stops propagation: the card behind them navigates on click, and
 * pressing Delete must not also open the note's page.
 */
function renderActions(options: NoteCardOptions, confirming: boolean): TemplateResult {
  const stop = (e: Event) => e.stopPropagation()
  if (confirming) {
    return html`
      <span class="note-actions">
        <button
          class="note-action danger"
          @click=${(e: Event) => {
            stop(e)
            options.onConfirmDelete()
          }}
        >
          Really delete?
        </button>
        <button
          class="note-action"
          @click=${(e: Event) => {
            stop(e)
            options.onCancelDelete()
          }}
        >
          Cancel
        </button>
      </span>
    `
  }
  return html`
    <span class="note-actions">
      <button
        class="note-action"
        title="Edit this note"
        @click=${(e: Event) => {
          stop(e)
          options.onEdit()
        }}
      >
        Edit
      </button>
      <button
        class="note-action"
        title="Delete this note"
        @click=${(e: Event) => {
          stop(e)
          options.onDelete()
        }}
      >
        Delete
      </button>
    </span>
  `
}

/**
 * A suggestion's replacement text.
 *
 * Gated on the type as well as on emptiness: `suggestion` survives a type
 * change (so switching away and back does not lose what was typed), and only a
 * suggestion note exports it.
 */
function renderSuggestion(note: ReviewNote): TemplateResult | typeof nothing {
  if (note.type !== 'suggestion') return nothing
  const text = note.suggestion ?? ''
  if (text.length === 0) return nothing
  return html`<pre class="note-suggestion"><code>${text}</code></pre>`
}
