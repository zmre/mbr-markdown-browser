import { LitElement, html, nothing } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { isGuiMode } from './shared.ts'

/**
 * Prefix of the IPC message that asks the GUI to show its Open… dialog
 * starting at an outside link's folder. Must stay identical to
 * `IPC_OPEN_OUTSIDE_PREFIX` in `src/external_open.rs`, its only reader.
 *
 * The message only chooses where the native panel starts: the host never
 * opens, reads or reports anything because of it. What the reader picks in the
 * panel opens like Open…/Cmd+O.
 */
export const IPC_OPEN_OUTSIDE_PREFIX = 'mbr:open-outside:'

/**
 * The target named by the page's `?path=` (see `mbr_core::outside_link`), or
 * `null` when there is none. `URLSearchParams` undoes the percent-encoding.
 */
export function outsidePathFromSearch(search: string): string | null {
  const path = new URLSearchParams(search).get('path')
  return path && path.trim() !== '' ? path : null
}

/**
 * Posts the Open… request. Returns whether it was posted: `window.ipc` exists
 * only in the GUI.
 */
export function postOpenOutside(path: string): boolean {
  const ipc = window.ipc
  if (!ipc || typeof ipc.postMessage !== 'function') return false
  try {
    ipc.postMessage(IPC_OPEN_OUTSIDE_PREFIX + path)
    return true
  } catch {
    return false
  }
}

/**
 * The body of `/.mbr/outside/`: where a link out of the repository goes and
 * why it is not shown. The path is drawn as text only (Lit escapes it), in
 * the light DOM so Pico styles the buttons.
 */
@customElement('mbr-outside-link')
export class MbrOutsideLinkElement extends LitElement {
  @state() private _status = ''

  private readonly _path = outsidePathFromSearch(window.location.search)

  protected override createRenderRoot(): HTMLElement {
    return this
  }

  private async _copy(): Promise<void> {
    if (!this._path) return
    try {
      await navigator.clipboard.writeText(this._path)
      this._status = 'Copied.'
    } catch {
      this._selectPath()
      this._status = 'Press Cmd/Ctrl+C to copy the selected path.'
    }
  }

  private _selectPath(): void {
    const code = this.querySelector('code.mbr-outside-path')
    const selection = window.getSelection()
    if (!code || !selection) return
    const range = document.createRange()
    range.selectNodeContents(code)
    selection.removeAllRanges()
    selection.addRange(range)
  }

  private _open(): void {
    if (!this._path) return
    this._status = postOpenOutside(this._path)
      ? 'Choose the file or folder to open in the dialog.'
      : 'Opening is not available here.'
  }

  protected override render() {
    if (!this._path) {
      return html`<p>This link does not say which file it points to.</p>`
    }
    const gui = isGuiMode()
    return html`
      <p>
        This note links to <code class="mbr-outside-path">${this._path}</code>, which is
        outside the folder this mbr is serving, so it can't be shown here.
      </p>
      ${gui
        ? html`<p>
            <strong>Open…</strong> shows a file dialog in that folder. Whatever you choose
            there opens in this window, in place of the current folder.
          </p>`
        : nothing}
      <p class="mbr-outside-actions">
        <button type="button" class="secondary" @click=${() => void this._copy()}>
          Copy path
        </button>
        ${gui ? html`<button type="button" @click=${() => this._open()}>Open…</button>` : nothing}
        <button type="button" class="outline secondary" @click=${() => history.back()}>
          Back
        </button>
      </p>
      <p class="mbr-outside-status" role="status" aria-live="polite">${this._status}</p>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-outside-link': MbrOutsideLinkElement
  }
}
