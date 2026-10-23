import { LitElement, html, css, nothing, type TemplateResult } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { getTagSources, isEditEnabled } from './shared.js';
import { isInputTarget, isModalOpen } from './mbr-keys.js';
import {
  getEditToken,
  isEditTokenRequired,
  noteEditTokenRequired,
  setEditToken,
} from './edit-token.js';
import { isSocketReachable } from './mbr-live-reload.js';
import { noteSelfWrite, readSourceLines } from './task-toggle.js';
import { ICON_BUTTON_CSS } from './controls.ts';
import { icon, iconPencil } from './icons.ts';

declare global {
  interface HTMLElementTagNameMap {
    'mbr-editor': MbrEditorElement;
  }
}

/**
 * The part of the lazily-loaded editor chunk this trigger uses (erased at
 * compile time). Narrowed to the exports it calls, so a test can stand in for
 * the chunk without reproducing the rest of the module.
 */
type EditorModule = Pick<typeof import('./editor-crepe.js'), 'openEditor' | 'createNote'>;

/**
 * Fragment that opens the editor on arrival (`createNote` navigates to a new
 * note with it). Same value as `EDIT_ON_ARRIVAL_HASH` in the chunk, which this
 * bundle cannot import.
 */
export const EDIT_ON_ARRIVAL_HASH = '#mbr-edit';

/** Runtime URL of the separately-built Crepe editor chunk (server mode only). */
const EDITOR_CHUNK_URL = '/.mbr/components/mbr-editor.min.js';

/**
 * Import the editor chunk. Overridable seam so tests can stub the dynamic
 * import — happy-dom cannot execute a runtime URL import — mirroring
 * `setTasksChunkImporter` in `mbr-tasks.ts` and the graph seam in `mbr-info.ts`.
 */
let importEditorChunk: () => Promise<EditorModule> = () =>
  import(/* @vite-ignore */ EDITOR_CHUNK_URL) as Promise<EditorModule>;

/**
 * Dispatched on `document` with `detail: { open: boolean }` when the editor
 * opens (ready) and closes. The iOS app bridge (`app-bridge.ts`) relays it so
 * the native bar can step aside while the full-screen editor is up.
 */
export const EDITOR_STATE_EVENT = 'mbr-editor-state';

/** Test hook: replace the chunk importer (module-level seam). */
export function setEditorChunkImporter(importer: () => Promise<EditorModule>): void {
  importEditorChunk = importer;
}

/** Tell the app bridge (and anyone else) whether the editor is on screen. */
function announceEditorState(open: boolean): void {
  document.dispatchEvent(new CustomEvent(EDITOR_STATE_EVENT, { detail: { open } }));
}

/** Percent-decodes a URL path, tolerating a malformed escape sequence. */
function safeDecode(path: string): string {
  try {
    return decodeURIComponent(path);
  } catch {
    return path;
  }
}

/**
 * The repo-relative folder a "New note" started on this page should go in: a
 * note's own folder, else the folder listing's path (`''` at the root).
 */
export function newNoteFolder(markdownSource: string | null, pathname: string): string {
  if (markdownSource) {
    const slash = markdownSource.lastIndexOf('/');
    return slash < 0 ? '' : markdownSource.slice(0, slash);
  }
  return safeDecode(pathname).replace(/^\/+|\/+$/g, '');
}

/**
 * Lightweight edit trigger: a pencil button (next to the info button) that,
 * when activated, lazy-loads the heavy Crepe editor chunk and opens the editing
 * modal for the current markdown file. The element is also on folder listings,
 * where it shows no pencil but still offers {@link openNew} ("New note") to the
 * iOS app's menu.
 */
@customElement('mbr-editor')
export class MbrEditorElement extends LitElement {
  @state()
  private _isOpen = false;

  @state()
  private _loading = false;

  /** A create flow (picker + POST) is in progress. */
  private _creating = false;

  override connectedCallback() {
    super.connectedCallback();
    document.addEventListener('keydown', this._handleKeydown);
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', this._openOnArrival, { once: true });
    } else {
      this._openOnArrival();
    }
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener('keydown', this._handleKeydown);
    document.removeEventListener('DOMContentLoaded', this._openOnArrival);
  }

  /**
   * `#mbr-edit` on arrival (a note just created) opens the editor, and the
   * fragment is dropped first so a reload or a shared link does not reopen it.
   */
  private _openOnArrival = () => {
    if (location.hash !== EDIT_ON_ARRIVAL_HASH || !this.canEdit) return;
    history.replaceState(history.state, '', location.pathname + location.search);
    // The note was usually created a moment ago, and the watcher's echo of
    // that create can reach this page seconds later; `<mbr-live-reload>` would
    // then reload it and close the editor the reader is already typing in
    // (seen in WebKit: create, arrive, editor up, reload ~1s later). Register
    // the file as this page's own write and prime its source cache, so an
    // echo is settled by content (`isOwnWriteEcho`): only a real change on
    // disk — the reader's own save included — reloads.
    const source = this._markdownSource();
    if (source) {
      noteSelfWrite(source);
      void readSourceLines(source);
    }
    void this._open();
  };

  private _handleKeydown = (e: KeyboardEvent) => {
    // "e" opens the editor, but only when it's safe to hijack the key: not
    // while typing in an input (isInputTarget uses composedPath so inputs
    // inside shadow DOMs are detected) and not while a modal is open.
    if (
      e.key === 'e' &&
      !e.ctrlKey &&
      !e.metaKey &&
      !e.altKey &&
      this.canEdit &&
      !this._isOpen &&
      !this._loading &&
      !isInputTarget(e) &&
      !isModalOpen()
    ) {
      e.preventDefault();
      void this._open();
    }
  };

  /**
   * Opens the editor, exactly as `e` and the pencil button do. Public for the
   * iOS app's menu (`app-bridge.ts`); a no-op where {@link canEdit} is false.
   */
  public open(): void {
    if (this.canEdit) void this._open();
  }

  /** Whether this page can be edited: editing is on and it has a source file. */
  public get canEdit(): boolean {
    return isEditEnabled() && this._markdownSource() !== null;
  }

  /**
   * Whether "New note" is available here: editing is on. Unlike {@link canEdit}
   * it needs no source file, so a folder listing offers it too.
   */
  public get canCreate(): boolean {
    return isEditEnabled();
  }

  /**
   * Starts "New note": loads the editor chunk (with the loading overlay), asks
   * for a path prefilled with this page's folder, creates the file and opens it
   * in the editor. Public for the iOS app's menu; a no-op where
   * {@link canCreate} is false.
   */
  public openNew(): void {
    if (this.canCreate) void this._openNew();
  }

  private async _openNew() {
    if (this._isOpen || this._loading || this._creating) return;
    this._creating = true;
    this._loading = true;
    try {
      const mod = await importEditorChunk();
      // The picker is the feedback from here on.
      this._loading = false;
      await mod.createNote({
        folder: newNoteFolder(this._markdownSource(), location.pathname),
        token: getEditToken(),
        onAuthRequired: () => noteEditTokenRequired(),
      });
    } catch (err) {
      console.error('Failed to start a new note:', err);
    } finally {
      this._loading = false;
      this._creating = false;
    }
  }

  private _markdownSource(): string | null {
    const source = window.frontmatter?.['markdown_source'];
    return typeof source === 'string' && source.length > 0 ? source : null;
  }

  private async _open() {
    if (this._isOpen || this._loading) return;
    const source = this._markdownSource();
    if (!source) return;

    const encoded = source.split('/').map(encodeURIComponent).join('/');
    const rawUrl = `/.mbr/raw/${encoded}`;
    const saveUrl = `/.mbr/edit/${encoded}`;

    // Show an immediate loading overlay: the editor chunk is large and its
    // download/parse can take a moment, so give instant feedback on click.
    this._loading = true;
    try {
      const mod = await importEditorChunk();
      await mod.openEditor({
        rawUrl,
        saveUrl,
        filePath: source,
        // The chunk holds no token state of its own: it borrows what the page
        // knows and hands back whatever the user types, so the single
        // in-memory copy in `edit-token.ts` stays the only one. Nothing here
        // reaches `localStorage` or `sessionStorage`.
        token: getEditToken(),
        tokenRequired: isEditTokenRequired(),
        onToken: (token: string) => setEditToken(token),
        // The properties editor's tag fields; the chunk cannot read the page config itself.
        tagSources: getTagSources().map((s) => ({ field: s.field, label: s.label, label_plural: s.labelPlural })),
        // Fired once the editor modal is on screen: hand off from our spinner.
        onReady: () => {
          this._isOpen = true;
          this._loading = false;
          announceEditorState(true);
        },
        onClose: () => {
          this._isOpen = false;
          this._loading = false;
          announceEditorState(false);
        },
        // Where a live-reload socket exists it reloads the page on the write.
        // Under the iOS app's `mbr:` scheme none can, so reload here.
        onSaved: () => {
          if (!isSocketReachable(location.protocol)) location.reload();
        },
      });
    } catch (err) {
      console.error('Failed to load the editor:', err);
    } finally {
      this._loading = false;
    }
  }

  private _renderLoading(): TemplateResult {
    return html`
      <div class="edit-loading-backdrop" aria-live="polite">
        <div class="edit-loading-box" role="status">
          <span class="edit-spinner" aria-hidden="true"></span>
          Loading editor…
        </div>
      </div>
    `;
  }

  private _renderTrigger(): TemplateResult {
    return html`
      <button
        class="edit-trigger mbr-icon-btn"
        @click=${() => void this._open()}
        aria-label="Edit this page"
        title="Edit (E)"
        ?aria-busy=${this._loading}
      >
        ${icon(iconPencil)}
      </button>
    `;
  }

  override render() {
    // The pencil only where there is a file to edit (a folder listing carries
    // the element for "New note" alone); the overlay serves both flows.
    if (!isEditEnabled()) return html``;
    return html`
      ${this.canEdit ? this._renderTrigger() : nothing}
      ${this._loading ? this._renderLoading() : nothing}
    `;
  }

  static override styles = [ICON_BUTTON_CSS, css`
    :host {
      display: contents;
    }

    /* Instant loading overlay shown while the editor chunk downloads */
    .edit-loading-backdrop {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.5);
      z-index: 2000;
      display: flex;
      align-items: center;
      justify-content: center;
    }
    .edit-loading-box {
      display: flex;
      align-items: center;
      gap: 0.6rem;
      padding: 1rem 1.5rem;
      border-radius: 8px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
      box-shadow: 0 12px 40px rgba(0, 0, 0, 0.35);
    }
    .edit-spinner {
      width: 1rem;
      height: 1rem;
      border: 2px solid var(--pico-muted-border-color, #ccc);
      border-top-color: var(--pico-primary, #0172ad);
      border-radius: 50%;
      animation: mbr-editor-spin 0.7s linear infinite;
    }
    @keyframes mbr-editor-spin {
      to {
        transform: rotate(360deg);
      }
    }
  `];
}
