import { LitElement, css, html, nothing, type TemplateResult } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { getTasksDefaultInclude, isEditEnabled, isTasksEnabled, resolveUrl } from './shared.js'
import { isInputTarget, isModalOpen } from './mbr-keys.js'
import { getMbrAssetBase } from './dynamic-loader.js'
import { currentDocumentPath, syncDocumentTask, toggleTask } from './task-toggle.js'
import type { MbrOverlay } from './overlay.js'
import { ICON_BUTTON_CSS } from './controls.ts'
import { icon, iconListChecks } from './icons.ts'
import type { TaskToggleOutcome, TaskToggleTarget } from './tasks/types.js'

declare global {
  interface HTMLElementTagNameMap {
    'mbr-tasks': MbrTasksElement
  }
}

/** Endpoint the panel posts its queries to (server/GUI only, see server.rs). */
const TASKS_ENDPOINT = '/.mbr/tasks'

/**
 * Import the lazy task-panel chunk (`mbr-tasks.min.js`), which registers the
 * `<mbr-tasks-panel>` element. The URL is computed against the asset base so it
 * resolves in server mode from any page depth. Overridable seam so tests can
 * stub the dynamic import (happy-dom cannot execute runtime URL imports).
 *
 * This mirrors `mbr-info.ts`'s graph-chunk seam exactly — same module-level
 * importer, same once-per-page promise, same `@vite-ignore`.
 */
let importTasksChunk: () => Promise<unknown> = () => {
  const url = new URL(getMbrAssetBase() + 'components/mbr-tasks.min.js', document.baseURI).href
  return import(/* @vite-ignore */ url)
}

/** Test hook: replace the chunk importer (module-level seam). */
export function setTasksChunkImporter(importer: () => Promise<unknown>): void {
  importTasksChunk = importer
  tasksChunkPromise = null
}

/**
 * The panel's writer.
 *
 * The one thing the panel cannot do for itself is the page *behind* the
 * overlay. Every task write suppresses the live reload it would otherwise
 * trigger (see `task-toggle.ts`), which for the panel means its overlay and the
 * user's filters survive a toggle — but it also means a task from the file on
 * screen would sit there still unchecked once the panel closes.
 * `syncDocumentTask` is the other half of that bargain, and the server's new
 * source line is what lets it redraw a stamped `@done(...)` too.
 */
async function panelToggle(target: TaskToggleTarget): Promise<TaskToggleOutcome> {
  const outcome = await toggleTask(target)
  if (outcome.ok) syncDocumentTask(target.path, target.line, target.to, outcome.text)
  return outcome
}

/** Shared once-per-page promise for the chunk load; `true` when usable. */
let tasksChunkPromise: Promise<boolean> | null = null

function loadTasksChunk(): Promise<boolean> {
  if (!tasksChunkPromise) {
    tasksChunkPromise = importTasksChunk()
      .then(() => true)
      .catch((err) => {
        console.warn('Failed to load the tasks chunk:', err)
        return false // No panel this page load; the trigger stays inert.
      })
  }
  return tasksChunkPromise
}

/**
 * `<mbr-tasks>` — `list-checks` trigger for the task browser.
 *
 * Ships in the main bundle and stays tiny: a nav button plus the `t` shortcut.
 * The two-pane panel itself lives in the lazily-imported `mbr-tasks.min.js`
 * chunk, which is fetched the first time the panel is opened.
 *
 * Renders nothing at all when the task browser is unavailable — the task index
 * is built from live files, so static builds never have the endpoint (see
 * TASKS_SPEC.md "Applicability"). `_nav.html` gates the element on
 * `server_mode and tasks_enabled` too; this guard covers a stale custom
 * template.
 */
@customElement('mbr-tasks')
export class MbrTasksElement extends LitElement implements MbrOverlay {
  @state()
  private _isOpen = false

  @state()
  private _loading = false

  /** True once the lazy panel chunk has loaded (element is defined). */
  @state()
  private _chunkReady = false

  override connectedCallback() {
    super.connectedCallback()
    document.addEventListener('keydown', this._handleKeydown)
  }

  override disconnectedCallback() {
    super.disconnectedCallback()
    document.removeEventListener('keydown', this._handleKeydown)
  }

  // ========================================
  // Public Methods (the MbrOverlay contract, called from mbr-keys)
  // ========================================

  /** True while the task panel is showing (or its chunk is still loading). */
  public get isOpen(): boolean {
    return this._isOpen
  }

  public open(): void {
    void this._open()
  }

  public close(): void {
    this._isOpen = false
  }

  public toggle(): void {
    if (this._isOpen) {
      this.close()
    } else {
      this.open()
    }
  }

  /**
   * Lowercase `t` opens the panel. Verified free: `mbr-keys` claims only
   * `Shift+T` (table of contents).
   *
   * Deliberately open-only rather than a toggle, and guarded exactly like
   * `<mbr-editor>`'s `e`: once the panel is open `isInputTarget` is true for the
   * filter field, so `t` must reach the field as a literal character. `Esc` and
   * the trigger button close it.
   */
  private _handleKeydown = (e: KeyboardEvent) => {
    if (
      e.key === 't' &&
      !e.ctrlKey &&
      !e.metaKey &&
      !e.altKey &&
      !e.shiftKey &&
      isTasksEnabled() &&
      !this._isOpen &&
      !this._loading &&
      !isInputTarget(e) &&
      !isModalOpen()
    ) {
      e.preventDefault()
      void this._open()
    }
  }

  private async _open(): Promise<void> {
    // Refuse to "open" when there is nothing to show: `render()` returns
    // nothing without the endpoint, and an `isOpen` that reported true anyway
    // would make `isModalOpen()` suppress every bare-letter shortcut on the
    // page behind an invisible panel.
    if (this._isOpen || !isTasksEnabled()) return
    // Show the overlay immediately: the chunk download is usually instant, but
    // a cold cache should still give feedback on the very first open.
    this._isOpen = true
    if (this._chunkReady) return

    this._loading = true
    try {
      this._chunkReady = await loadTasksChunk()
    } finally {
      this._loading = false
    }
    // A failed import leaves nothing to show; don't strand an empty backdrop.
    if (!this._chunkReady) {
      this._isOpen = false
    }
  }

  private _renderTrigger(): TemplateResult {
    return html`
      <button
        class="tasks-trigger mbr-icon-btn"
        @click=${() => this.toggle()}
        aria-label="Open the task browser"
        aria-expanded=${this._isOpen ? 'true' : 'false'}
        title="Tasks (T)"
        ?aria-busy=${this._loading}
      >
        ${icon(iconListChecks)}
      </button>
    `
  }

  private _renderLoading(): TemplateResult {
    return html`
      <div class="tasks-loading-backdrop" aria-live="polite">
        <div class="tasks-loading-box" role="status">
          <span class="tasks-spinner" aria-hidden="true"></span>
          Loading tasks…
        </div>
      </div>
    `
  }

  override render() {
    // The nav template already gates on `server_mode and tasks_enabled`; this
    // second guard keeps a stale custom `_nav.html` from rendering a button
    // whose endpoint answers 404.
    if (!isTasksEnabled()) return nothing

    return html`
      ${this._renderTrigger()}
      ${this._loading ? this._renderLoading() : nothing}
      ${this._isOpen && this._chunkReady
        ? html`
            <mbr-tasks-panel
              .endpoint=${TASKS_ENDPOINT}
              .resolveHref=${resolveUrl}
              .editEnabled=${isEditEnabled()}
              .defaultInclude=${getTasksDefaultInclude()}
              .toggleTask=${panelToggle}
              .currentPath=${currentDocumentPath()}
              @mbr-tasks-close=${() => this.close()}
            ></mbr-tasks-panel>
          `
        : nothing}
    `
  }

  static override styles = [ICON_BUTTON_CSS, css`
    :host {
      display: contents;
    }


    .tasks-loading-backdrop {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.35);
      display: flex;
      align-items: center;
      justify-content: center;
      z-index: 1200;
    }

    .tasks-loading-box {
      display: flex;
      align-items: center;
      gap: 0.6rem;
      padding: 0.9rem 1.4rem;
      border-radius: 8px;
      background: var(--pico-background-color, #fff);
      color: var(--pico-color, #333);
      box-shadow: 0 10px 30px rgba(0, 0, 0, 0.2);
    }

    .tasks-spinner {
      width: 1.1rem;
      height: 1.1rem;
      border: 2px solid var(--pico-muted-border-color, #ccc);
      border-top-color: var(--pico-primary, #0172ad);
      border-radius: 50%;
      animation: mbr-tasks-spin 0.7s linear infinite;
    }

    @keyframes mbr-tasks-spin {
      to {
        transform: rotate(360deg);
      }
    }
  `]
}
