/**
 * `<mbr-review-form>` — the add/edit form for one review note.
 *
 * Its own element rather than a sub-render of `<mbr-review-panel>`, because
 * `r` on a selection must be able to open it with **no panel at all**: the
 * common case is "write one note and carry on reading", and routing that
 * through the all-files overlay would put the whole review in front of someone
 * who asked to annotate one sentence.
 *
 * # It persists nothing
 *
 * The store is main-bundle state this chunk may not import (see `index.ts`), so
 * the form's entire output is one `mbr-review-save` event. Whoever opened it —
 * the trigger for a new note, the panel for an edit — owns the write. That also
 * means the form is the same code in both cases, with `existing` deciding only
 * what it starts from.
 *
 * # The suggestion prefill, and why a fallback is not an error
 *
 * A suggestion note carries replacement text, and the only sensible starting
 * point is the lines it replaces. `readSource` gets them from `/.mbr/raw`, which
 * sits behind `check_edit_access` and answers 403 unless the server was started
 * with `--edit` — so `SourceRead.exact === false`, meaning "this came from the
 * rendered text instead", is the **common** case, not a failure. It is reported
 * as a note under the box and never blocks saving: rendered text is usually
 * close enough to edit into shape, and refusing the note would be a worse
 * answer than an approximate prefill.
 */
import { LitElement, css, html, nothing, type TemplateResult } from 'lit'
import { customElement, property, query, state } from 'lit/decorators.js'
import { displayQuote } from './note-model.ts'
import { DEFAULT_NOTE_TYPE, TYPE_DEFS, isNoteType } from './types.ts'
import type { NoteAnchor, NoteType, ReviewNote, SourceRead, SourceReader } from './types.ts'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-review-form': MbrReviewFormElement
  }
}

/** `detail` of the `mbr-review-save` event. */
export interface ReviewSaveDetail {
  type: NoteType
  body: string
  /** Replacement text; `null` unless the note is a non-empty suggestion. */
  suggestion: string | null
}

@customElement('mbr-review-form')
export class MbrReviewFormElement extends LitElement {
  /**
   * Where the note goes. `null` is legitimate — a form opened with no selection
   * writes a file-level note — but then there is nothing to prefill a
   * suggestion from.
   */
  @property({ attribute: false })
  anchor: NoteAnchor | null = null

  /** The note being edited, or `null` when creating one. */
  @property({ attribute: false })
  existing: ReviewNote | null = null

  /** Reads source lines for the suggestion prefill; injected, may be absent. */
  @property({ attribute: false })
  readSource: SourceReader | null = null

  /** Type a *new* note starts on. Ignored when {@link existing} is set. */
  @property({ attribute: false })
  defaultType: NoteType = DEFAULT_NOTE_TYPE

  /**
   * Whether the store will accept a write.
   *
   * False when it was written by a newer mbr (see `ParsedEnvelope.writable`).
   * The form still opens and still reads, because seeing the note is the point
   * of opening it, but Save is refused and says why — a save that silently did
   * nothing would be worse than no button.
   */
  @property({ attribute: false })
  writable = true

  @state() private _type: NoteType = DEFAULT_NOTE_TYPE
  @state() private _body = ''
  @state() private _suggestion = ''

  /**
   * The result of the one and only `readSource` call, cached so that toggling
   * the dropdown away from `suggestion` and back does not refetch.
   */
  @state() private _source: SourceRead | null = null
  @state() private _prefilling = false

  /**
   * Whether {@link _suggestion} actually came from {@link _source}.
   *
   * Separate from `_source !== null` because editing an existing suggestion
   * leaves the author's text in place: saying "prefilled from the rendered
   * text" over text the user wrote themselves would be a lie.
   */
  @state() private _prefilled = false

  /** Guards the fetch, not the state, so an in-flight read is not started twice. */
  private _prefillStarted = false

  @query('#review-body')
  private _bodyField!: HTMLTextAreaElement

  override firstUpdated() {
    const existing = this.existing
    if (existing) {
      this._type = existing.type
      this._body = existing.body
      this._suggestion = existing.suggestion ?? ''
    } else {
      this._type = this.defaultType
    }
    // Autofocus the body: the type is a reasonable default and the body never
    // is, so the caret belongs in the only field that must be filled in.
    this._bodyField?.focus()
    if (this._type === 'suggestion') void this._ensurePrefill()
  }

  /** The anchor a suggestion reads from: the live one, else the edited note's. */
  private get _location(): { file: string; line: number | null; endLine: number | null } | null {
    if (this.anchor) return this.anchor
    const existing = this.existing
    return existing ? { file: existing.file, line: existing.line, endLine: existing.endLine } : null
  }

  /** The quote shown read-only above a suggestion's replacement box. */
  private get _quote(): string {
    return displayQuote(this.anchor?.quote ?? this.existing?.quote ?? null)
  }

  /**
   * Fetch the source lines once.
   *
   * The result is applied to the textarea only when it is empty, so an edit of
   * an existing suggestion keeps what the author wrote — and so does a new note
   * whose author started typing before the read landed.
   */
  private async _ensurePrefill(): Promise<void> {
    if (this._prefillStarted) return
    this._prefillStarted = true

    const read = this.readSource
    const location = this._location
    if (!read || !location) return

    this._prefilling = true
    try {
      const source = await read(location.file, location.line, location.endLine)
      this._source = source
      if (this._suggestion.length === 0 && source.text.length > 0) {
        this._suggestion = source.text
        this._prefilled = true
      }
    } catch {
      // A prefill is a convenience. Failing to get one leaves an empty box,
      // which is exactly what the user would have had without the feature.
    } finally {
      this._prefilling = false
    }
  }

  private _setType(value: string) {
    // Validated rather than cast: the `<select>`'s value is a string as far as
    // the DOM is concerned, and `isNoteType` is the same check the store uses.
    if (!isNoteType(value)) return
    this._type = value
    if (value === 'suggestion') void this._ensurePrefill()
  }

  /** Save is refused for an empty body — unless the suggestion carries the note. */
  private get _canSave(): boolean {
    if (!this.writable) return false
    return this._body.trim().length > 0 || this._type === 'suggestion'
  }

  private _save() {
    if (!this._canSave) return
    const suggestion = this._type === 'suggestion' ? this._suggestion : ''
    const detail: ReviewSaveDetail = {
      type: this._type,
      body: this._body,
      suggestion: suggestion.length > 0 ? suggestion : null,
    }
    this.dispatchEvent(
      new CustomEvent<ReviewSaveDetail>('mbr-review-save', {
        detail,
        bubbles: true,
        composed: true,
      })
    )
  }

  private _cancel() {
    this.dispatchEvent(new CustomEvent('mbr-review-cancel', { bubbles: true, composed: true }))
  }

  /**
   * `Ctrl`/`Cmd+Enter` saves, `Esc` cancels.
   *
   * Bound to the element rather than to `document`: a keydown inside the shadow
   * root is composed and bubbles out to the host, so this sees every key the
   * form has focus for and none that it does not — which is what keeps the
   * panel's bare-letter shortcuts from firing while someone types a note.
   */
  private _handleKeydown = (e: KeyboardEvent) => {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault()
      this._save()
      return
    }
    if (e.key === 'Escape') {
      e.preventDefault()
      // Stop it here so the panel's document listener does not also act: both
      // would close the form, but only one of them should decide.
      e.stopPropagation()
      this._cancel()
    }
  }

  override connectedCallback() {
    super.connectedCallback()
    this.addEventListener('keydown', this._handleKeydown)
  }

  override disconnectedCallback() {
    super.disconnectedCallback()
    this.removeEventListener('keydown', this._handleKeydown)
  }

  override render() {
    return html`
      <form
        class="review-form"
        aria-label=${this.existing ? 'Edit review note' : 'Add review note'}
        @submit=${(e: Event) => {
          e.preventDefault()
          this._save()
        }}
      >
        <label class="field">
          <span class="field-label">Type</span>
          <select
            id="review-type"
            aria-label="Note type"
            .value=${this._type}
            @change=${(e: Event) => this._setType((e.target as HTMLSelectElement).value)}
          >
            ${TYPE_DEFS.map(
              (def) => html`
                <option value=${def.id} ?selected=${this._type === def.id}>
                  ${def.label} — ${def.description}
                </option>
              `
            )}
          </select>
        </label>

        <label class="field">
          <span class="field-label">Comment</span>
          <textarea
            id="review-body"
            rows="4"
            spellcheck="true"
            placeholder="What should the author know?"
            .value=${this._body}
            @input=${(e: Event) => (this._body = (e.target as HTMLTextAreaElement).value)}
          ></textarea>
        </label>

        ${this._type === 'suggestion' ? this._renderSuggestion() : nothing}

        ${this.writable
          ? nothing
          : html`<p class="readonly-note" role="status">
              These notes were written by a newer version of mbr, so this build will not overwrite
              them.
            </p>`}

        <div class="form-actions">
          <span class="form-hint"><kbd>⌘/^↵</kbd> save <kbd>esc</kbd> cancel</span>
          <button type="button" class="secondary" @click=${() => this._cancel()}>Cancel</button>
          <button type="submit" class="primary" ?disabled=${!this._canSave}>Save</button>
        </div>
      </form>
    `
  }

  private _renderSuggestion(): TemplateResult {
    const quote = this._quote
    return html`
      <div class="suggestion">
        ${quote.length > 0
          ? html`<div class="field">
              <span class="field-label">Selected text</span>
              <blockquote class="suggestion-quote">${quote}</blockquote>
            </div>`
          : nothing}
        <label class="field">
          <span class="field-label">Replace with</span>
          <textarea
            id="review-suggestion"
            rows="4"
            spellcheck="false"
            aria-busy=${this._prefilling ? 'true' : 'false'}
            placeholder=${this._prefilling ? 'Reading the source…' : 'Replacement text'}
            .value=${this._suggestion}
            @input=${(e: Event) => (this._suggestion = (e.target as HTMLTextAreaElement).value)}
          ></textarea>
        </label>
        ${this._prefilled && this._source && !this._source.exact
          ? html`<p class="suggestion-note">
              Prefilled from the rendered text — the file's source isn't readable (start mbr with
              <code>--edit</code>).
            </p>`
          : nothing}
      </div>
    `
  }

  static override styles = css`
    :host {
      display: contents;
    }

    .review-form {
      display: flex;
      flex-direction: column;
      gap: 0.6rem;
      padding: 0.75rem;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
      border: 1px solid var(--pico-muted-border-color, #eee);
      border-radius: 6px;
    }

    .field {
      display: flex;
      flex-direction: column;
      gap: 0.2rem;
      margin: 0;
    }

    .field-label {
      font-size: 0.7rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      color: var(--pico-muted-color, #5c6b73);
    }

    select,
    textarea {
      margin: 0;
      width: 100%;
      box-sizing: border-box;
      font-size: 0.85rem;
      padding: 0.35rem 0.5rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
    }

    textarea {
      resize: vertical;
      font-family: inherit;
    }

    #review-suggestion {
      font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
      font-size: 0.8rem;
    }

    .suggestion {
      display: flex;
      flex-direction: column;
      gap: 0.4rem;
      border-inline-start: 3px solid var(--mbr-review-suggestion-color, #f57c00);
      padding-inline-start: 0.5rem;
    }

    .suggestion-quote {
      margin: 0;
      padding: 0.3rem 0.5rem;
      font-size: 0.8rem;
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      color: var(--pico-muted-color, #5c6b73);
      background: var(--pico-card-background-color, #f8f9fa);
      border-radius: 4px;
    }

    /* A fallback prefill is normal, not an error, so it is muted body copy —
     * no red, no icon, no border. */
    .suggestion-note {
      margin: 0;
      font-size: 0.72rem;
      color: var(--pico-muted-color, #5c6b73);
    }

    .suggestion-note code {
      font-size: 0.95em;
    }

    .readonly-note {
      margin: 0;
      font-size: 0.72rem;
      color: var(--mbr-review-issue-color, #c62828);
    }

    .form-actions {
      display: flex;
      align-items: center;
      gap: 0.4rem;
    }

    .form-hint {
      flex: 1;
      font-size: 0.68rem;
      color: var(--pico-muted-color, #5c6b73);
    }

    button {
      margin: 0;
      font-size: 0.82rem;
      padding: 0.3rem 0.8rem;
      border-radius: 4px;
      cursor: pointer;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
    }

    button.primary {
      background: var(--pico-primary, #0d6efd);
      border-color: var(--pico-primary, #0d6efd);
      color: var(--pico-primary-inverse, #fff);
    }

    button[disabled] {
      opacity: 0.55;
      cursor: not-allowed;
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
  `
}
