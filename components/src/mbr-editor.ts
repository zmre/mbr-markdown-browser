import { LitElement, html, css, nothing, type TemplateResult } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { isEditEnabled } from './shared.js';
import { isInputTarget, isModalOpen } from './mbr-keys.js';

declare global {
  interface HTMLElementTagNameMap {
    'mbr-editor': MbrEditorElement;
  }
}

/** Type of the lazily-loaded editor chunk (erased at compile time). */
type EditorModule = typeof import('./editor-crepe.js');

/** Runtime URL of the separately-built Crepe editor chunk (server mode only). */
const EDITOR_CHUNK_URL = '/.mbr/components/mbr-editor.min.js';

/**
 * Lightweight edit trigger: a pencil button (next to the info button) that,
 * when activated, lazy-loads the heavy Crepe editor chunk and opens the editing
 * modal for the current markdown file. Only rendered when editing is enabled.
 */
@customElement('mbr-editor')
export class MbrEditorElement extends LitElement {
  @state()
  private _isOpen = false;

  @state()
  private _loading = false;

  override connectedCallback() {
    super.connectedCallback();
    document.addEventListener('keydown', this._handleKeydown);
  }

  override disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener('keydown', this._handleKeydown);
  }

  private _handleKeydown = (e: KeyboardEvent) => {
    // "e" opens the editor, but only when it's safe to hijack the key: not
    // while typing in an input (isInputTarget uses composedPath so inputs
    // inside shadow DOMs are detected) and not while a modal is open.
    if (
      e.key === 'e' &&
      !e.ctrlKey &&
      !e.metaKey &&
      !e.altKey &&
      isEditEnabled() &&
      !this._isOpen &&
      !this._loading &&
      !isInputTarget(e) &&
      !isModalOpen()
    ) {
      e.preventDefault();
      void this._open();
    }
  };

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
      const mod = (await import(/* @vite-ignore */ EDITOR_CHUNK_URL)) as EditorModule;
      await mod.openEditor({
        rawUrl,
        saveUrl,
        filePath: source,
        // Fired once the editor modal is on screen: hand off from our spinner.
        onReady: () => {
          this._isOpen = true;
          this._loading = false;
        },
        onClose: () => {
          this._isOpen = false;
          this._loading = false;
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
        class="edit-trigger"
        @click=${() => void this._open()}
        aria-label="Edit this page"
        title="Edit (e)"
        ?aria-busy=${this._loading}
      >
        <span class="edit-icon">✎</span>
      </button>
    `;
  }

  override render() {
    // The button only makes sense when editing is enabled; the template also
    // gates the element, but guard here too for robustness.
    if (!isEditEnabled()) return html``;
    return html`
      ${this._renderTrigger()}
      ${this._loading ? this._renderLoading() : nothing}
    `;
  }

  static override styles = css`
    :host {
      display: contents;
    }
    .edit-trigger {
      cursor: pointer;
      width: 2rem;
      height: 2rem;
      padding: 0;
      display: flex;
      align-items: center;
      justify-content: center;
      border-radius: 4px;
      border: none;
      background: transparent;
      transition: background 0.15s ease;
    }
    .edit-trigger:hover {
      border: 1px solid var(--pico-contrast-hover-border, rgba(0, 0, 0, 0.05));
    }
    .edit-icon {
      font-size: 1.1rem;
      line-height: 1;
      font-style: normal;
      color: var(--pico-color, #333);
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
  `;
}
