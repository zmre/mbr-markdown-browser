/**
 * `<mbr-fm-combobox>`: a text field with a filtered suggestion list that
 * still accepts free text (ARIA 1.2 combobox, list autocomplete). Used by the
 * properties form for enum values, tags and the "+ Add property" key.
 *
 * Light DOM, like the rest of the editor: its rules are in `fm-styles.ts`.
 *
 * Events (bubbling, detail `{ value }`):
 * - `fm-combo-input` on every keystroke;
 * - `fm-combo-commit` on Enter, a picked suggestion, Tab (with
 *   `commitOnTab`, which then keeps the Tab from moving focus so the host can
 *   place it), or a `separators` character (tags: `,`); `via` says which;
 * - `fm-combo-cancel` on Escape with the list closed, when `escapeCancels`
 *   (the event is then stopped, so the editor stays open).
 * - `fm-combo-backspace-empty` on Backspace in an empty field (tags drop the last chip).
 */

import { LitElement, html, nothing, type PropertyValues } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { live } from 'lit/directives/live.js';
import { fuzzyFilter } from '../fuzzy.js';
import type { ComboOption } from './form-logic.js';

export type ComboCommitVia = 'enter' | 'pick' | 'tab' | 'separator';

export interface ComboCommitDetail {
  readonly value: string;
  readonly via: ComboCommitVia;
}

/** At most this many suggestions are drawn; filtering narrows the rest. */
const MAX_SHOWN = 50;

let nextId = 0;

@customElement('mbr-fm-combobox')
export class MbrFmCombobox extends LitElement {
  @property() value = '';
  @property({ attribute: false }) options: readonly ComboOption[] = [];
  /** The field's accessible name. */
  @property() label = '';
  @property() placeholder = '';
  /** Characters that commit the typed text (tags: `,`). */
  @property() separators = '';
  @property({ type: Boolean }) commitOnTab = false;
  @property({ type: Boolean }) escapeCancels = false;
  /** Show the list when the field is focused, before anything is typed. */
  @property({ type: Boolean }) openOnFocus = false;
  /** Leave the field empty after a commit (tags, add property). */
  @property({ type: Boolean }) clearOnCommit = false;
  @property({ type: Boolean }) disabled = false;

  @state() private _open = false;
  @state() private _active = -1;
  @state() private _query = '';

  private readonly _id = `mbr-fm-combo-${++nextId}`;

  protected override createRenderRoot(): HTMLElement {
    return this;
  }

  /** Focuses the text field. */
  override focus(): void {
    this.querySelector('input')?.focus();
  }

  /** What is typed in the field and not committed yet (trimmed). */
  get pendingText(): string {
    return (this.querySelector('input')?.value ?? this._query).trim();
  }

  /** Commits what is typed, as Enter would (a save does this for a tag not yet added). Returns whether there was any. */
  commitTyped(): boolean {
    const text = this.pendingText;
    if (!text) return false;
    this._commit(text, 'enter');
    return true;
  }

  protected override willUpdate(changed: PropertyValues<this>): void {
    if (changed.has('value')) this._query = this.value;
  }

  private _filtered(): ComboOption[] {
    const typed = this._query.trim();
    // The value itself is no suggestion once it is all that was typed.
    const matches = fuzzyFilter(
      this.options.map((o) => ({ item: o, haystacks: o.label ? [o.value, o.label] : [o.value] })),
      typed,
    );
    return matches.filter((o) => o.value !== typed || matches.length > 1).slice(0, MAX_SHOWN);
  }

  private _emit(name: string, detail: object): void {
    this.dispatchEvent(new CustomEvent(name, { detail, bubbles: true }));
  }

  private _commit(value: string, via: ComboCommitVia): void {
    this._open = false;
    this._active = -1;
    this._query = this.clearOnCommit ? '' : value;
    const input = this.querySelector('input');
    if (input) input.value = this._query;
    this._emit('fm-combo-commit', { value, via } satisfies ComboCommitDetail);
  }

  private _onInput(e: Event): void {
    const input = e.target as HTMLInputElement;
    let text = input.value;
    if (this.separators) {
      const at = [...text].findIndex((c) => this.separators.includes(c));
      if (at >= 0) {
        const head = text.slice(0, at).trim();
        if (head) {
          this._commit(head, 'separator');
          return;
        }
        text = text.slice(at + 1);
        input.value = text;
      }
    }
    this._query = text;
    this._open = true;
    this._active = -1;
    this._emit('fm-combo-input', { value: text });
  }

  private _onKeydown(e: KeyboardEvent): void {
    const shown = this._open ? this._filtered() : [];
    const input = e.target as HTMLInputElement;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (!this._open) {
        this._open = true;
        this._active = -1;
        return;
      }
      if (shown.length === 0) return;
      const step = e.key === 'ArrowDown' ? 1 : -1;
      if (this._active < 0) this._active = step > 0 ? 0 : shown.length - 1;
      else this._active = (this._active + step + shown.length) % shown.length;
      return;
    }
    if (e.key === 'Enter') {
      if (e.isComposing) return;
      e.preventDefault();
      const picked = this._active >= 0 ? shown[this._active] : undefined;
      this._commit(picked ? picked.value : input.value.trim(), picked ? 'pick' : 'enter');
      return;
    }
    if (e.key === 'Tab' && this.commitOnTab && !e.shiftKey && input.value.trim() !== '') {
      e.preventDefault();
      const picked = this._active >= 0 ? shown[this._active] : undefined;
      this._commit(picked ? picked.value : input.value.trim(), 'tab');
      return;
    }
    if (e.key === 'Escape') {
      if (this._open && shown.length > 0) {
        e.preventDefault();
        e.stopPropagation();
        this._open = false;
        this._active = -1;
      } else if (this.escapeCancels) {
        e.preventDefault();
        e.stopPropagation();
        this._open = false;
        this._emit('fm-combo-cancel', {});
      }
      return;
    }
    if (e.key === 'Backspace' && input.value === '') this._emit('fm-combo-backspace-empty', {});
  }

  private _onFocus(): void {
    if (this.openOnFocus) this._open = true;
  }

  private _onBlur(): void {
    this._open = false;
    this._active = -1;
  }

  protected override render() {
    const shown = this._open ? this._filtered() : [];
    const listId = `${this._id}-list`;
    const activeId = this._active >= 0 && this._active < shown.length ? `${this._id}-opt-${this._active}` : '';
    return html`<div class="fm-combo">
      <input
        type="text"
        class="fm-in"
        role="combobox"
        autocomplete="off"
        spellcheck="false"
        aria-autocomplete="list"
        aria-label=${this.label}
        aria-expanded=${shown.length > 0 ? 'true' : 'false'}
        aria-controls=${listId}
        aria-activedescendant=${activeId || nothing}
        placeholder=${this.placeholder || nothing}
        ?disabled=${this.disabled}
        .value=${live(this._query)}
        @input=${this._onInput}
        @keydown=${this._onKeydown}
        @focus=${this._onFocus}
        @blur=${this._onBlur}
      />
      <ul class="fm-combo-list" role="listbox" id=${listId} aria-label=${this.label} ?hidden=${shown.length === 0}>
        ${shown.map(
          (o, i) =>
            html`<li
              role="option"
              id=${`${this._id}-opt-${i}`}
              class=${i === this._active ? 'active' : ''}
              aria-selected=${i === this._active ? 'true' : 'false'}
              @mousedown=${(e: Event) => e.preventDefault()}
              @click=${() => this._commit(o.value, 'pick')}
            >
              <span class="fm-combo-value">${o.label ?? o.value}</span>
              ${o.detail ? html`<span class="fm-combo-detail">${o.detail}</span>` : nothing}
            </li>`,
        )}
      </ul>
    </div>`;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-fm-combobox': MbrFmCombobox;
  }
}
