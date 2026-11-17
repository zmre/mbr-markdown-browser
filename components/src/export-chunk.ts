/**
 * Single-flight loader for the lazy `mbr-export.min.js` chunk (`export/`),
 * which finishes a page for a PDF export. `calendar-chunk.ts`'s pattern.
 *
 * MAIN BUNDLE. Only `export-ready.ts` loads it, and only when Rust set
 * `window.__MBR_EXPORT__`, so no reader's page ever fetches it.
 */
import { getMbrAssetBase } from './dynamic-loader.ts'
import type { ExportDeps } from './export/index.ts'

/** What the chunk exports (`export/index.ts`). */
export interface ExportChunk {
  prepareForExport(raw: unknown, deps: ExportDeps): Promise<{ warnings: string[] }>
}

const defaultImporter = (): Promise<unknown> => {
  const url = new URL(getMbrAssetBase() + 'components/mbr-export.min.js', document.baseURI).href
  return import(/* @vite-ignore */ url)
}

let importExportChunk: () => Promise<unknown> = defaultImporter

/** Shared once-per-page promise. */
let exportChunkPromise: Promise<ExportChunk> | null = null

/** Test hook: replace the chunk importer (module-level seam). */
export function setExportChunkImporter(importer: (() => Promise<unknown>) | null): void {
  importExportChunk = importer ?? defaultImporter
  exportChunkPromise = null
}

/**
 * Loads the export chunk once per page. Unlike the other loaders this one
 * **rejects** when the import fails: an export without its chunk cannot
 * succeed, and the trigger reports the reason to Rust.
 */
export function loadExportChunk(): Promise<ExportChunk> {
  exportChunkPromise ??= importExportChunk().then((module) => module as ExportChunk)
  return exportChunkPromise
}
