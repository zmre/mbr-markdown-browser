/**
 * PDF export trigger (main bundle, no custom element), and the GUI's
 * editor-state relay.
 *
 * When Rust has set `window.__MBR_EXPORT__` (`export-mode.ts`), this loads the
 * lazy `mbr-export.min.js` chunk (`export/`), which finishes the page for
 * paper, and posts exactly one `mbr:export-ready:{"warnings":[…]}` or
 * `mbr:export-failed:<message>` through wry's `window.ipc` when it is done —
 * or when the chunk could not be loaded. `window.mbrExportReady()` returns the
 * same promise. Kept to the flag check and the hand-over because every page
 * pays for the main bundle and almost none is ever exported.
 *
 * Everything page-scoped the chunk needs (the loaders' settle promises, the
 * asset base, the stylesheet loader) is injected from here: the chunk imports
 * nothing stateful.
 *
 * In GUI windows it also relays the editor's open/closed state to Rust, which
 * greys out Save as PDF… while the editor is open (an export reloads the page
 * from disk, so unsaved edits would silently be missing).
 */
import { getMbrAssetBase, loadCss, waitForDom } from './dynamic-loader.ts'
import { loadExportChunk } from './export-chunk.ts'
import {
  EXPORT_READY_GLOBAL,
  IPC_EDITOR_STATE_PREFIX,
  IPC_EXPORT_FAILED_PREFIX,
  IPC_EXPORT_READY_PREFIX,
} from './export/contract.ts'
import type { ExportDeps } from './export/index.ts'
import { isExportMode } from './export-mode.ts'
import { hljsSettled } from './mbr-hljs.ts'
import { mathSettled } from './mbr-katex.ts'
import { mermaidSettled } from './mbr-mermaid.ts'
import { EDITOR_STATE_EVENT } from './mbr-editor.ts'
import { isGuiMode } from './shared.ts'

/** Longest failure message sent to Rust, which treats it as text. */
const MAX_MESSAGE_LENGTH = 300

declare global {
  interface Window {
    /** Defined only in export mode; resolves when the page may be printed. */
    mbrExportReady?: () => Promise<{ warnings: string[] }>
  }
}

/** Posts `message` to Rust; a plain browser (no wry) has nothing listening. */
function postToHost(message: string): void {
  const ipc = window.ipc
  if (ipc && typeof ipc.postMessage === 'function') ipc.postMessage(message)
}

function errorMessage(err: unknown): string {
  return (err instanceof Error ? err.message : String(err)).slice(0, MAX_MESSAGE_LENGTH)
}

/** This page's services, for the chunk. */
export function exportDeps(): ExportDeps {
  return {
    doc: document,
    pageUrl: () => new URL(window.location.href),
    waitForDom,
    loadCss,
    assetBase: getMbrAssetBase,
    settled: { mermaid: mermaidSettled, math: mathSettled, hljs: hljsSettled },
  }
}

/** The run, once per page. */
let exportRun: Promise<{ warnings: string[] }> | null = null

/**
 * Starts the export run (once): installs `window.mbrExportReady`, loads the
 * chunk, and posts exactly one ready/failed message when it ends.
 */
export function startExportReady(raw: unknown, deps: ExportDeps = exportDeps()): Promise<{ warnings: string[] }> {
  if (exportRun) return exportRun
  const run = loadExportChunk().then((chunk) => chunk.prepareForExport(raw, deps))
  exportRun = run
  ;(window as unknown as Record<string, unknown>)[EXPORT_READY_GLOBAL] = () => run
  run.then(
    (result) => postToHost(IPC_EXPORT_READY_PREFIX + JSON.stringify(result)),
    (err: unknown) => postToHost(IPC_EXPORT_FAILED_PREFIX + errorMessage(err))
  )
  return run
}

/** Test hook: forget the run, so another can start. */
export function resetExportReadyForTests(): void {
  exportRun = null
  delete (window as unknown as Record<string, unknown>)[EXPORT_READY_GLOBAL]
}

/**
 * Relays every `EDITOR_STATE_EVENT` to Rust as `mbr:editor-state:open|closed`,
 * in GUI windows only. Returns the uninstaller.
 */
export function installEditorStateRelay(): () => void {
  if (!isGuiMode()) return () => {}
  const relay = (event: Event) => {
    const open = (event as CustomEvent<{ open?: unknown }>).detail?.open === true
    postToHost(`${IPC_EDITOR_STATE_PREFIX}${open ? 'open' : 'closed'}`)
  }
  document.addEventListener(EDITOR_STATE_EVENT, relay)
  return () => document.removeEventListener(EDITOR_STATE_EVENT, relay)
}

installEditorStateRelay()
if (isExportMode()) void startExportReady(window.__MBR_EXPORT__).catch(() => {})
