/**
 * PDF export readiness for a page that does not load mbr's main bundle: a
 * chrome-free custom page template (an invoice, a letter) that includes
 * `{% include "/.mbr/_export_ready.html" %}` or
 * `<script defer src="/.mbr/components/mbr-export-ready.min.js">`.
 *
 * Without it such a page never posts `mbr:export-ready:` and the export waits
 * out its whole timeout. With it the page runs the same export as every
 * other page (`export-run.ts` + the `mbr-export.min.js` chunk: export.css,
 * eager images, the link policy, the `@page` rule, then the waits), with
 * services of its own in place of the main bundle's:
 *
 * - the asset base comes from this script's own URL (`…/.mbr/components/…`),
 *   so it works at any depth, in a static build and on a host's server;
 * - diagrams: when the page loads mermaid itself (`window.mermaid`), wait
 *   until every `.mermaid` block is drawn (`data-processed` or an `<svg>`);
 *   KaTeX and highlight.js render synchronously when a page uses them, and
 *   the fonts KaTeX needs are waited for at the end like on any page.
 *
 * Costs nothing on an ordinary view: without `window.__MBR_EXPORT__` it does
 * nothing at all. On a page that also has the main bundle, the first run
 * started wins (`export-run.ts`'s guard), so there is one ready message.
 *
 * Built alone as an IIFE (`vite.export-ready.config.ts`); must import
 * nothing stateful (`shared.ts`, `dynamic-loader.ts`, the loaders), which
 * `calendar-chunk.test.ts` guards.
 */
import type { ExportDeps } from './export/index.ts'
import { type ExportChunk, startExportRun } from './export-run.ts'

/** How often the diagram wait looks again. Never `requestAnimationFrame`. */
const MERMAID_POLL_MS = 50

/** This script's file, relative to the asset base. */
export const STANDALONE_SCRIPT_PATH = 'components/mbr-export-ready.min.js'

/**
 * The `/.mbr/` asset base from this script's own URL (it lives at
 * `<base>components/mbr-export-ready.min.js`); `/.mbr/` when it is unknown.
 */
export function assetBaseFromScript(scriptSrc: string | null | undefined): string {
  if (!scriptSrc) return '/.mbr/'
  try {
    const url = new URL(scriptSrc)
    return url.pathname.endsWith(`/${STANDALONE_SCRIPT_PATH}`) ? new URL('../', url).href : '/.mbr/'
  } catch {
    return '/.mbr/'
  }
}

export function waitForDom(doc: Document = document): Promise<void> {
  if (doc.readyState !== 'loading') return Promise.resolve()
  return new Promise((resolve) => doc.addEventListener('DOMContentLoaded', () => resolve(), { once: true }))
}

/** Adds a stylesheet (once) and settles when it has loaded or failed. */
export function loadStylesheet(href: string, doc: Document = document): Promise<void> {
  const absolute = new URL(href, doc.baseURI).href
  const present = Array.from(doc.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"]')).some(
    (link) => link.href === absolute
  )
  if (present) return Promise.resolve()
  return new Promise((resolve, reject) => {
    const link = doc.createElement('link')
    link.rel = 'stylesheet'
    link.href = href
    link.addEventListener('load', () => resolve(), { once: true })
    link.addEventListener('error', () => reject(new Error(`failed to load ${href}`)), { once: true })
    doc.head.appendChild(link)
  })
}

/**
 * Settles once every `.mermaid` block on the page is drawn, when the page
 * loads mermaid itself; at once when it does not (nothing will draw them).
 * The export caps the wait (15 s) with a warning.
 */
export function domMermaidSettled(doc: Document = document, win: object = window, pollMs = MERMAID_POLL_MS): Promise<void> {
  if (!('mermaid' in win)) return Promise.resolve()
  const pending = () =>
    Array.from(doc.querySelectorAll('.mermaid')).some((el) => !el.hasAttribute('data-processed') && !el.querySelector('svg'))
  return new Promise((resolve) => {
    const check = () => (pending() ? setTimeout(check, pollMs) : resolve())
    check()
  })
}

/** The services a page without the main bundle gives the export chunk. */
export function standaloneDeps(assetBase: string): ExportDeps {
  return {
    doc: document,
    pageUrl: () => new URL(window.location.href),
    waitForDom: () => waitForDom(document),
    loadCss: (href) => loadStylesheet(href, document),
    assetBase: () => assetBase,
    settled: {
      mermaid: () => domMermaidSettled(document, window),
      math: () => Promise.resolve(),
      hljs: () => Promise.resolve(),
    },
  }
}

/** Imports the chunk from the asset base. */
export function chunkImporter(assetBase: string): () => Promise<ExportChunk> {
  let chunk: Promise<ExportChunk> | null = null
  return () => {
    const url = new URL(`${assetBase}components/mbr-export.min.js`, document.baseURI).href
    chunk ??= (import(/* @vite-ignore */ url) as Promise<unknown>).then((module) => module as ExportChunk)
    return chunk
  }
}

/**
 * Starts the export when Rust set `window.__MBR_EXPORT__`: after the DOM is
 * parsed and one more macrotask, so the main bundle (a module script, run
 * before `DOMContentLoaded`) starts first when it is on the page too.
 * Returns the run, or `null` when there is nothing to do.
 */
export async function bootStandaloneExport(
  scriptSrc: string | null | undefined,
  importer?: () => Promise<ExportChunk>,
  deps: (assetBase: string) => ExportDeps = standaloneDeps
): Promise<unknown> {
  const flag = window.__MBR_EXPORT__
  if (flag === undefined || flag === null) return null
  const assetBase = assetBaseFromScript(scriptSrc)
  await waitForDom(document)
  await new Promise((resolve) => setTimeout(resolve, 0))
  const run = startExportRun(flag, deps(assetBase), importer ?? chunkImporter(assetBase))
  return run ? run.catch(() => null) : null
}
