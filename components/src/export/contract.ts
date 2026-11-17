/**
 * The names of the PDF export's page ⇄ Rust contract (design/pdf-export.md).
 * Every one is also a string literal in `src/pdf_export/mod.rs`;
 * `export-ready.test.ts` pins both sides.
 *
 * Pure constants, so both the main-bundle trigger (`export-ready.ts`) and the
 * lazy chunk (`export/index.ts`) import them; each bundle keeps only what it
 * uses.
 */

/** `window.mbrExportReady(): Promise<{warnings: string[]}>`. */
export const EXPORT_READY_GLOBAL = 'mbrExportReady'
/** Followed by `JSON.stringify({warnings})`. */
export const IPC_EXPORT_READY_PREFIX = 'mbr:export-ready:'
/** Followed by a plain-text message. */
export const IPC_EXPORT_FAILED_PREFIX = 'mbr:export-failed:'
/** Followed by `open` or `closed`. */
export const IPC_EDITOR_STATE_PREFIX = 'mbr:editor-state:'
/** The generated `@page` rule's `<style>` element. */
export const EXPORT_PAGE_STYLE_ID = 'mbr-export-page'
/** Served at `/.mbr/export.css`; compiled in, overridable in `.mbr/`. */
export const EXPORT_CSS_FILE = 'export.css'
/** Put on a link whose `href` the policy removed (styled by export.css). */
export const UNLINKED_CLASS = 'mbr-export-unlinked'
