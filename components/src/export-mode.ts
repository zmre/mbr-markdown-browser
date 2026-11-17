/**
 * Export mode: the page is being rendered for a PDF (`mbr --pdf`, the GUI's
 * Save as PDF…) in a hidden webview, not shown to a reader.
 *
 * Rust injects `window.__MBR_EXPORT__` with `with_initialization_script`
 * before any page script runs (design/pdf-export.md, "Page ⇄ Rust contract").
 * This module only *reads* the flag, so the loaders (`mbr-mermaid.ts`,
 * `mbr-hljs.ts`, `mbr-katex.ts`, `mbr-genealogy.ts`) can ask it without
 * pulling in `export-ready.ts`, which does the work.
 *
 * Main bundle only: it reads a page global.
 */
import { scheduleIdleTask } from './dynamic-loader.ts'

/** The global Rust sets; named from a Rust string literal (`src/pdf_export/mod.rs`). */
export const EXPORT_FLAG_GLOBAL = '__MBR_EXPORT__'

declare global {
  interface Window {
    /**
     * Set by the PDF exporter before any page script runs. Its shape is
     * validated by `export-ready.ts`; everything else only asks whether it is
     * there.
     */
    __MBR_EXPORT__?: unknown
  }
}

/** Whether this page is being rendered for a PDF export. */
export function isExportMode(): boolean {
  const flag = window.__MBR_EXPORT__
  return flag !== undefined && flag !== null
}

/**
 * Runs `task` soon: at idle on a screen, but on the next macrotask in export
 * mode. A hidden webview gives idle callbacks no priority, so on a large page
 * `requestIdleCallback` waited out its whole timeout (~1.4 s measured for the
 * hljs pass) before content the PDF needs was drawn. Never `requestAnimationFrame`:
 * it does not fire at all in a hidden view.
 */
export function scheduleRenderTask(task: () => void): void {
  if (isExportMode()) setTimeout(task, 0)
  else scheduleIdleTask(task)
}
