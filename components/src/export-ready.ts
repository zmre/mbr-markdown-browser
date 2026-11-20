/**
 * PDF export trigger (main bundle, no custom element), and the GUI's
 * editor-state relay.
 *
 * When Rust has set `window.__MBR_EXPORT__` (`export-mode.ts`), this loads the
 * lazy `mbr-export.min.js` chunk (`export/`), which finishes the page for
 * paper, and posts exactly one `mbr:export-ready:{"warnings":[…]}` or
 * `mbr:export-failed:<message>` through wry's `window.ipc` when it is done —
 * or when the chunk could not be loaded. `window.mbrExportReady()` returns the
 * same promise. The run itself is `export-run.ts`, shared with the standalone
 * `mbr-export-ready.min.js` that templates without the main bundle include.
 * Kept to the flag check and the hand-over because every page pays for the
 * main bundle and almost none is ever exported.
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
import { IPC_EDITOR_STATE_PREFIX } from './export/contract.ts'
import type { ExportDeps, ExportResult } from './export/index.ts'
import { isExportMode } from './export-mode.ts'
import { postToHost, resetExportRunForTests, startExportRun } from './export-run.ts'
import { hljsSettled } from './mbr-hljs.ts'
import { mathSettled } from './mbr-katex.ts'
import { mermaidSettled } from './mbr-mermaid.ts'
import { EDITOR_STATE_EVENT } from './mbr-editor.ts'
import { isGuiMode } from './shared.ts'

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

/**
 * Starts the export run (once per page): installs `window.mbrExportReady`,
 * loads the chunk, and posts exactly one ready/failed message when it ends.
 * `null` when the flag asks for `ready = "load"` (Rust's script signals).
 */
export function startExportReady(raw: unknown, deps: ExportDeps = exportDeps()): Promise<ExportResult> | null {
  return startExportRun(raw, deps, loadExportChunk)
}

/** Test hook: forget the run, so another can start. */
export function resetExportReadyForTests(): void {
  resetExportRunForTests()
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
if (isExportMode()) void startExportReady(window.__MBR_EXPORT__)?.catch(() => {})
