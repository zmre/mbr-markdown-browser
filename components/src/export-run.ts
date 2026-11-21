/**
 * The export run's trigger half, shared by the main bundle's
 * `export-ready.ts` and the standalone `export-ready-standalone.ts`
 * (`mbr-export-ready.min.js`, for templates that do not load the main
 * bundle): load the `mbr-export.min.js` chunk, run it, set
 * `window.mbrExportReady`, and post exactly one `mbr:export-ready:{…}` or
 * `mbr:export-failed:<message>` through wry's `window.ipc`.
 *
 * Pure of page state: everything arrives as arguments, so both bundles keep
 * only their own loaders. The once-per-page guard is the `mbrExportReady`
 * global itself, which both bundles see: whichever starts first runs, the
 * other hands back its promise.
 */
import { EXPORT_READY_GLOBAL, IPC_EXPORT_FAILED_PREFIX, IPC_EXPORT_READY_PREFIX } from './export/contract.ts'
import type { ExportDeps, ExportResult } from './export/index.ts'

/** Longest failure message sent to Rust, which treats it as text. */
const MAX_MESSAGE_LENGTH = 300

/** What the chunk exports (`export/index.ts`). */
export interface ExportChunk {
  prepareForExport(raw: unknown, deps: ExportDeps): Promise<ExportResult>
}

declare global {
  interface Window {
    /** Defined only in export mode; resolves when the page may be printed. */
    mbrExportReady?: () => Promise<ExportResult>
  }
}

/** Posts `message` to Rust; a plain browser (no wry) has nothing listening. */
export function postToHost(message: string): void {
  const ipc = window.ipc
  if (ipc && typeof ipc.postMessage === 'function') ipc.postMessage(message)
}

function errorMessage(err: unknown): string {
  return (err instanceof Error ? err.message : String(err)).slice(0, MAX_MESSAGE_LENGTH)
}

/**
 * Whether the flag asks for `ready = "load"`: Rust's own injected script
 * then signals on `load` plus the fonts, and the page's scripts stay out of
 * it, so there is exactly one ready message.
 */
export function readyOnLoad(raw: unknown): boolean {
  return typeof raw === 'object' && raw !== null && (raw as Record<string, unknown>)['ready'] === 'load'
}

const globals = window as unknown as Record<string, unknown>

/**
 * Starts the export run, once per page: a second call (from either bundle)
 * returns the first run's promise. `null` when the flag asks for
 * `ready = "load"`.
 */
export function startExportRun(
  raw: unknown,
  deps: ExportDeps,
  loadChunk: () => Promise<ExportChunk>
): Promise<ExportResult> | null {
  const running = globals[EXPORT_READY_GLOBAL]
  if (typeof running === 'function') return (running as () => Promise<ExportResult>)()
  if (readyOnLoad(raw)) return null
  const run = loadChunk().then((chunk) => chunk.prepareForExport(raw, deps))
  globals[EXPORT_READY_GLOBAL] = () => run
  run.then(
    (result) => postToHost(IPC_EXPORT_READY_PREFIX + JSON.stringify(result)),
    (err: unknown) => postToHost(IPC_EXPORT_FAILED_PREFIX + errorMessage(err))
  )
  return run
}

/** Test hook: forget the run, so another can start. */
export function resetExportRunForTests(): void {
  delete globals[EXPORT_READY_GLOBAL]
}
