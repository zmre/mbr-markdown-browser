/**
 * Entry of the lazy `mbr-export.min.js` chunk, imported by the main bundle's
 * `export-ready.ts`, or by the standalone `mbr-export-ready.min.js` on a page
 * that does not load the main bundle, only when Rust set
 * `window.__MBR_EXPORT__`.
 */
export { prepareForExport } from './prepare.ts'
export type { ExportDeps, ExportResult } from './prepare.ts'
