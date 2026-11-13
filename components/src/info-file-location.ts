/**
 * The info panel's file button: an icon beside the Document Info section's
 * **File** label that shows the note in the platform's file manager (Finder,
 * Explorer, a freedesktop file manager), or copies its full path — GUI only.
 *
 * The server puts `window.extendedMeta.fileLocation` on a page only in GUI
 * mode (`file_location` in `mbr_server::server`), so a served or built page
 * never names a local path and this button never renders there. `reveal` says
 * whether the GUI on this platform can reveal at all; where it cannot, the
 * button copies the absolute path instead.
 *
 * The reveal itself is a wry IPC message, `mbr:reveal:` followed by the
 * note's **repository-relative** path (`extendedMeta.filePath`), which
 * `src/external_open.rs` validates and resolves inside the served repository
 * — the absolute path is never sent back, so page content cannot
 * use the channel to reveal anything outside the repository. Main bundle; no
 * module state.
 */
import { html, nothing, type TemplateResult } from 'lit'
import { ICON_SM, icon, iconCopy, iconFolderOpen } from './icons.ts'
import { writeClipboard } from './copy-content.ts'

/** Must match `IPC_REVEAL_PREFIX` in `src/external_open.rs`. */
export const IPC_REVEAL_PREFIX = 'mbr:reveal:'

/** `window.extendedMeta.fileLocation`, as the GUI server sends it. */
export interface FileLocation {
  /** The note's absolute path, as the host spells it. */
  path: string
  /** Whether the GUI can show the file in the file manager on this platform. */
  reveal: boolean
}

/** Reads `fileLocation` defensively: anything malformed is no location. */
export function fileLocationFrom(value: unknown): FileLocation | null {
  if (!value || typeof value !== 'object') return null
  const { path, reveal } = value as Record<string, unknown>
  if (typeof path !== 'string' || path.trim() === '') return null
  return { path, reveal: reveal === true }
}

/**
 * The reveal button's label, named after the platform's file manager.
 * `platform` is the user agent's platform string (a seam for tests).
 */
export function revealLabel(platform: string = navigatorPlatform()): string {
  if (/mac/i.test(platform)) return 'Show in Finder'
  if (/win/i.test(platform)) return 'Show in Explorer'
  return 'Show in file manager'
}

function navigatorPlatform(): string {
  if (typeof navigator === 'undefined') return ''
  const hinted = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform
  return hinted || navigator.platform || navigator.userAgent || ''
}

/**
 * Posts the reveal request for `relativePath`. Returns whether it was posted:
 * `window.ipc` exists only in the GUI's webview.
 */
export function postReveal(relativePath: string, ipc: Window['ipc'] = window.ipc): boolean {
  if (!relativePath || !ipc || typeof ipc.postMessage !== 'function') return false
  try {
    ipc.postMessage(IPC_REVEAL_PREFIX + relativePath)
    return true
  } catch {
    return false
  }
}

/** What the button does, and the services it needs. */
export interface FileLocationActions {
  /** Reveal the note; defaults to {@link postReveal}. */
  reveal?: (relativePath: string) => boolean
  /** Copy text; defaults to the shared clipboard writer. */
  copy?: (text: string) => Promise<boolean>
  /** Report the outcome of a copy (e.g. a toast). */
  report?: (message: string) => void
}

/**
 * The icon button beside the Document Info section's **File** label, or
 * `nothing` without a location (server mode, static builds, a template
 * without the field). It reveals the note in the file manager where the
 * platform can and there is a `relativePath` to send; otherwise it copies the
 * note's **full absolute path** (`location.path`, not the repository-relative
 * one the row shows).
 */
export function renderFileLocationButton(
  location: FileLocation | null,
  relativePath: string,
  actions: FileLocationActions = {},
): TemplateResult | typeof nothing {
  if (!location) return nothing
  const canReveal = location.reveal && relativePath !== ''
  const reveal = actions.reveal ?? postReveal
  const copy = actions.copy ?? ((text: string) => writeClipboard({ text }))
  const label = canReveal ? revealLabel() : COPY_FULL_PATH_LABEL
  const onClick = canReveal
    ? () => {
        reveal(relativePath)
      }
    : async () => {
        const ok = await copy(location.path)
        actions.report?.(ok ? 'Full path copied' : 'Could not copy the path')
      }
  return html`<button
    type="button"
    class="mbr-icon-btn sm file-location-action"
    data-action=${canReveal ? 'reveal' : 'copy'}
    title=${label}
    aria-label=${label}
    @click=${onClick}
  >${icon(canReveal ? iconFolderOpen : iconCopy, { size: ICON_SM })}</button>`
}

/** The copy button's label, where the platform cannot reveal. */
export const COPY_FULL_PATH_LABEL = 'Copy full path'
