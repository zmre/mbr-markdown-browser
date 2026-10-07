/**
 * `<mbr-folder-picker>` — the folder button beside the search panel's
 * "Current folder only" option, and the filterable folder list it opens.
 *
 * CHUNK ONLY (`mbr-search-extras.min.js`): imports nothing stateful. The folder
 * list arrives as a property from `<mbr-search>`; a choice leaves as a
 * `mbr-folder-pick` event (`detail.folder`), bubbling and composed.
 *
 * Keyboard model: focus stays in the filter field and the highlighted row is
 * announced via `aria-activedescendant` — the same arrow/Enter model as the
 * search results. Escape is stopped here so it closes only the picker; the
 * search modal's document-level Escape handler never sees it.
 */
import { LitElement, html, css, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'

/** Most folders listed at once; the filter narrows the rest. */
const MAX_LISTED_FOLDERS = 200

export interface FolderPickEventDetail {
  folder: string
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-folder-picker': MbrFolderPickerElement
  }
}

@customElement('mbr-folder-picker')
export class MbrFolderPickerElement extends LitElement {
  /** Every folder that holds notes (see `foldersFromUrlPaths`). */
  @property({ attribute: false })
  folders: string[] = []

  @state()
  private _open = false

  @state()
  private _filter = ''

  @state()
  private _index = 0

  /** True while the folder list is showing. */
  get isOpen(): boolean {
    return this._open
  }

  /** Open the list and focus its filter field. */
  open(): void {
    this._open = true
    this._filter = ''
    this._index = 0
    void this.updateComplete.then(() => this.renderRoot.querySelector<HTMLInputElement>('input')?.focus())
  }

  /** Close the list; `refocus` returns focus to the folder button. */
  close(refocus = true): void {
    if (!this._open) return
    this._open = false
    if (refocus) {
      void this.updateComplete.then(() => this.renderRoot.querySelector<HTMLButtonElement>('button')?.focus())
    }
  }

  private _filtered(): string[] {
    const needle = this._filter.trim().toLowerCase()
    const matches = needle ? this.folders.filter((f) => f.toLowerCase().includes(needle)) : this.folders
    return matches.slice(0, MAX_LISTED_FOLDERS)
  }

  private _choose(folder: string): void {
    this.close(false)
    this.dispatchEvent(
      new CustomEvent<FolderPickEventDetail>('mbr-folder-pick', {
        detail: { folder },
        bubbles: true,
        composed: true,
      })
    )
  }

  private _onKeydown(e: KeyboardEvent): void {
    const folders = this._filtered()
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      const step = e.key === 'ArrowDown' ? 1 : -1
      this._index = Math.max(0, Math.min(this._index + step, folders.length - 1))
      void this.updateComplete.then(() =>
        this.renderRoot.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' })
      )
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const folder = folders[this._index]
      if (folder) this._choose(folder)
    } else if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      this.close()
    }
  }

  private _renderList() {
    const folders = this._filtered()
    const active = Math.min(this._index, folders.length - 1)
    return html`
      <div class="picker" @click=${(e: Event) => e.stopPropagation()}>
        <input
          type="text"
          placeholder="Filter folders…"
          aria-label="Filter folders"
          role="combobox"
          aria-expanded="true"
          aria-controls="folders"
          aria-activedescendant=${active >= 0 ? `folder-${active}` : ''}
          autocomplete="off"
          spellcheck="false"
          .value=${this._filter}
          @input=${(e: Event) => {
            this._filter = (e.target as HTMLInputElement).value
            this._index = 0
          }}
          @keydown=${this._onKeydown}
        />
        <ul id="folders" role="listbox" aria-label="Folders">
          ${folders.length === 0
            ? html`<li class="empty" role="presentation">No folders match</li>`
            : folders.map(
                (folder, i) => html`<li
                  id=${`folder-${i}`}
                  class="option"
                  role="option"
                  aria-selected=${i === active ? 'true' : 'false'}
                  title=${folder}
                  @mousedown=${(e: Event) => e.preventDefault()}
                  @click=${() => this._choose(folder)}
                >
                  ${folder}
                </li>`
              )}
        </ul>
      </div>
    `
  }

  override render() {
    return html`
      <button
        type="button"
        title="Choose a folder to search in"
        aria-label="Choose a folder to search in"
        aria-haspopup="listbox"
        aria-expanded=${this._open ? 'true' : 'false'}
        @click=${() => (this._open ? this.close() : this.open())}
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path>
        </svg>
      </button>
      ${this._open ? this._renderList() : nothing}
    `
  }

  static override styles = css`
    :host {
      position: relative;
      display: inline-flex;
    }

    button {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      width: 1.5rem;
      height: 1.5rem;
      margin: 0;
      padding: 0;
      border: 1px solid transparent;
      border-radius: 4px;
      background: transparent;
      color: var(--pico-muted-color, #666);
      cursor: pointer;
    }

    button:hover,
    button[aria-expanded='true'] {
      color: var(--pico-color, #333);
      border-color: var(--pico-muted-border-color, #ccc);
    }

    button:focus-visible {
      outline: 2px solid var(--pico-primary, #0172ad);
      outline-offset: 1px;
    }

    .picker {
      position: absolute;
      top: calc(100% + 0.35rem);
      left: -12rem;
      z-index: 5;
      width: min(22rem, 80vw);
      padding: 0.4rem;
      border: 1px solid var(--pico-muted-border-color, #ddd);
      border-radius: 8px;
      background: var(--pico-background-color, #fff);
      box-shadow: 0 10px 30px -8px rgba(0, 0, 0, 0.3);
    }

    input {
      width: 100%;
      box-sizing: border-box;
      margin: 0 0 0.35rem;
      padding: 0.3rem 0.5rem;
      font-size: 0.8rem;
      border: 1px solid var(--pico-muted-border-color, #ccc);
      border-radius: 4px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
    }

    ul {
      list-style: none;
      margin: 0;
      padding: 0;
      max-height: 14rem;
      overflow-y: auto;
    }

    li {
      margin: 0;
      padding: 0.25rem 0.5rem;
      border-radius: 4px;
      font-family: var(--pico-font-family-monospace, monospace);
      font-size: 0.78rem;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      list-style: none;
    }

    .option {
      cursor: pointer;
      color: var(--pico-color, #333);
    }

    .option:hover {
      background: color-mix(in srgb, var(--pico-primary, #0172ad) 8%, transparent);
    }

    .option[aria-selected='true'] {
      background: color-mix(in srgb, var(--pico-primary, #0172ad) 18%, transparent);
    }

    .empty {
      color: var(--pico-muted-color, #888);
      font-family: inherit;
    }
  `
}
