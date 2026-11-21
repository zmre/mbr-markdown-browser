/**
 * The lazy PDF export chunk's work (`mbr-export.min.js`, entry `index.ts`):
 * finish the page for paper and wait until it may be printed
 * (design/pdf-export.md, "Page ⇄ Rust contract").
 *
 * 1. loads `/.mbr/export.css` (overridable in `.mbr/`);
 * 2. forces every `img[loading=lazy]` to load — a hidden view never lays out
 *    the viewport a lazy image waits for;
 * 3. applies the link policy (internal links would point at the exporter's
 *    ephemeral loopback server, dead once the export ends);
 * 4. waits for images (10 s cap), mermaid, KaTeX and hljs (15 s cap each);
 * 5. adds `<style id="mbr-export-page">@page { size: Win Hin; margin: M }</style>`
 *    with only what the page's own `@page` rules leave out (`page-box.ts`:
 *    a template's `@page` wins), once every stylesheet has loaded;
 * 6. waits for the fonts. A cap that runs out is a warning, never a failure.
 *
 * The result carries the page's declared sheet size (`page`), which Rust
 * prints on: WebKit's print system ignores `@page { size }`.
 *
 * Posting the result to Rust and `window.mbrExportReady` are the main-bundle
 * trigger's (`export-ready.ts`), so a chunk that fails to load is still
 * reported.
 *
 * **No `requestAnimationFrame` anywhere on this path**: a hidden webview never
 * runs one (measured, macOS), so every wait is a `setTimeout` or an event.
 *
 * Imports nothing stateful (guarded by `calendar-chunk.test.ts`): everything
 * page-scoped arrives in {@link ExportDeps}.
 */
import { EXPORT_CSS_FILE, EXPORT_PAGE_STYLE_ID, UNLINKED_CLASS } from './contract.ts'
import {
  type DeclaredPageBox,
  MARGIN_SIDES,
  type PageSize,
  type SheetLike,
  declaredPageBox,
  marginLonghands,
  mbrSheetMatcher,
} from './page-box.ts'

/** How long all of the page's images together may take. */
export const IMAGE_CAP_MS = 10_000
/** How long each of mermaid, KaTeX, hljs and the fonts may take. */
export const SETTLE_CAP_MS = 15_000

/** At most this many warnings, each at most {@link MAX_WARNING_LENGTH} long. */
const MAX_WARNINGS = 50
const MAX_WARNING_LENGTH = 300
/** Image URLs named in one "still loading" warning. */
const MAX_NAMED_IMAGES = 5

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/** What happens to links into the site (see design/pdf-export.md, "Links"). */
export type LinkPolicy = { kind: 'strip' } | { kind: 'base'; base: URL }

/**
 * What the export waits for: `page` (default), this run's message; `load`,
 * the `load` event and the fonts, posted by the script Rust injects — the
 * page's own scripts then stay out of it (`export-run.ts`).
 */
export type ReadySignal = 'page' | 'load'

/** `window.__MBR_EXPORT__`, validated. */
export interface ExportOptions {
  links: LinkPolicy
  paperWidthIn: number
  paperHeightIn: number
  margin: string
  ready: ReadySignal
}

/** What a run resolves with, and the ready message's JSON. */
export interface ExportResult {
  warnings: string[]
  /** The sheet the page's own `@page { size }` declares, when it does. */
  page?: PageSize
}

/** Thrown for a malformed flag; its message is what Rust reports. */
export class ExportOptionsError extends Error {
  override name = 'ExportOptionsError'
}

/** Larger than any real sheet; anything past it is a units mistake. */
const MAX_PAPER_IN = 200
/**
 * A CSS margin shorthand: up to four lengths. Restricted to the characters a
 * length can contain, since the value is written into a `<style>` element.
 */
const MARGIN_PATTERN = /^[0-9a-z.%\s-]{1,64}$/i

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function paperLength(value: unknown, name: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0 || value > MAX_PAPER_IN) {
    throw new ExportOptionsError(`${name} must be a positive number of inches, got ${JSON.stringify(value)}`)
  }
  return value
}

function linkPolicy(value: unknown): LinkPolicy {
  if (value === 'strip') return { kind: 'strip' }
  if (isRecord(value) && typeof value['base'] === 'string') {
    let base: URL
    try {
      base = new URL(value['base'])
    } catch {
      throw new ExportOptionsError(`links.base is not a URL: ${value['base']}`)
    }
    if ((base.protocol !== 'http:' && base.protocol !== 'https:') || !base.pathname.endsWith('/')) {
      throw new ExportOptionsError(`links.base must be an http(s) URL ending in /: ${value['base']}`)
    }
    return { kind: 'base', base }
  }
  throw new ExportOptionsError(`links must be "strip" or {"base": url}, got ${JSON.stringify(value)}`)
}

function readySignal(value: unknown): ReadySignal {
  if (value === undefined || value === 'page') return 'page'
  if (value === 'load') return 'load'
  throw new ExportOptionsError(`ready must be "page" or "load", got ${JSON.stringify(value)}`)
}

/** Validates the flag Rust injected. Throws {@link ExportOptionsError}. */
export function parseExportOptions(raw: unknown): ExportOptions {
  if (!isRecord(raw)) throw new ExportOptionsError('__MBR_EXPORT__ is not an object')
  const margin = raw['margin']
  if (typeof margin !== 'string' || !MARGIN_PATTERN.test(margin.trim())) {
    throw new ExportOptionsError(`margin is not a CSS margin: ${JSON.stringify(margin)}`)
  }
  return {
    links: linkPolicy(raw['links']),
    paperWidthIn: paperLength(raw['paperWidthIn'], 'paperWidthIn'),
    paperHeightIn: paperLength(raw['paperHeightIn'], 'paperHeightIn'),
    margin: margin.trim(),
    ready: readySignal(raw['ready']),
  }
}

/** A page that declares nothing in `@page` itself. */
const NOTHING_DECLARED: DeclaredPageBox = { size: null, margins: new Set() }

/**
 * The generated `@page` rule: the configured size and margin, minus whatever
 * the page's own `@page` rules declare (those win). Orientation is already in
 * the width and height. Empty when the page declares everything.
 */
export function pageRule(options: ExportOptions, declared: DeclaredPageBox = NOTHING_DECLARED): string {
  const descriptors: string[] = []
  if (!declared.size) descriptors.push(`size: ${options.paperWidthIn}in ${options.paperHeightIn}in`)
  if (declared.margins.size === 0) {
    descriptors.push(`margin: ${options.margin}`)
  } else {
    const longhands = marginLonghands(options.margin)
    for (const side of MARGIN_SIDES) {
      if (!declared.margins.has(side)) descriptors.push(`${side}: ${longhands[side]}`)
    }
  }
  return descriptors.length === 0 ? '' : `@page { ${descriptors.join('; ')} }`
}

// ---------------------------------------------------------------------------
// Page preparation
// ---------------------------------------------------------------------------

/**
 * Adds (or replaces) the `@page` rule, last in `<head>` so it beats theme.css's;
 * none at all when the page declares every descriptor itself.
 */
export function installPageStyle(doc: Document, options: ExportOptions, declared: DeclaredPageBox = NOTHING_DECLARED): void {
  doc.getElementById(EXPORT_PAGE_STYLE_ID)?.remove()
  const rule = pageRule(options, declared)
  if (rule === '') return
  const style = doc.createElement('style')
  style.id = EXPORT_PAGE_STYLE_ID
  style.textContent = rule
  doc.head.appendChild(style)
}

/** The page's own `@page` box, from its stylesheets, mbr's own left out. */
export function documentPageBox(doc: Document, assetBase: string): DeclaredPageBox {
  const isMbrSheet = mbrSheetMatcher(assetBase, doc.baseURI, EXPORT_PAGE_STYLE_ID)
  return declaredPageBox(doc.styleSheets as unknown as ArrayLike<SheetLike>, isMbrSheet)
}

/** Makes every lazy image load now. Returns how many it changed. */
export function forceEagerImages(root: ParentNode): number {
  const lazy = Array.from(root.querySelectorAll<HTMLImageElement>('img[loading]')).filter(
    (img) => img.getAttribute('loading')?.trim().toLowerCase() === 'lazy'
  )
  for (const img of lazy) img.setAttribute('loading', 'eager')
  return lazy.length
}

/** Schemes that must never be clickable in a document handed to someone. */
const SCRIPT_SCHEMES = new Set(['javascript:', 'vbscript:', 'data:'])

/** Viewer for links out of the repository (`outside_link.rs`). */
const OUTSIDE_VIEWER_SUFFIX = '/.mbr/outside/'

/** What the policy does to one link. */
export type LinkAction = { kind: 'keep' } | { kind: 'strip' } | { kind: 'rewrite'; href: string }

/**
 * The policy for one `href` on the page at `page`. Pure, for the tests.
 *
 * - `#fragment` (and a same-origin link to this very page with a fragment):
 *   kept as `#fragment`, the only internal link that survives the export.
 * - `/.mbr/outside/` (a path outside the repository): stripped either way.
 * - Same origin: stripped, or rebased onto `base` keeping path, query and
 *   fragment (`/a/b/?q#f` → `<base>a/b/?q#f`).
 * - `javascript:`/`vbscript:`/`data:`: stripped.
 * - Anything else (http(s), mailto, tel, app schemes): kept.
 */
export function linkAction(rawHref: string, page: URL, policy: LinkPolicy): LinkAction {
  if (rawHref.startsWith('#')) return { kind: 'keep' }
  let url: URL
  try {
    url = new URL(rawHref, page)
  } catch {
    return { kind: 'strip' }
  }
  if (SCRIPT_SCHEMES.has(url.protocol)) return { kind: 'strip' }
  if (url.origin !== page.origin) return { kind: 'keep' }
  if (url.pathname.endsWith(OUTSIDE_VIEWER_SUFFIX)) return { kind: 'strip' }
  if (url.hash && url.pathname === page.pathname && url.search === page.search) {
    return { kind: 'rewrite', href: url.hash }
  }
  if (policy.kind === 'strip') return { kind: 'strip' }
  const relative = `${url.pathname.replace(/^\/+/, '')}${url.search}${url.hash}`
  return { kind: 'rewrite', href: new URL(relative, policy.base).href }
}

/** Applies {@link linkAction} to every `a[href]` under `root`. Idempotent. */
export function applyLinkPolicy(root: ParentNode, page: URL, policy: LinkPolicy): void {
  for (const anchor of root.querySelectorAll<HTMLAnchorElement>('a[href]')) {
    const action = linkAction(anchor.getAttribute('href') ?? '', page, policy)
    if (action.kind === 'strip') {
      anchor.removeAttribute('href')
      anchor.removeAttribute('target')
      anchor.classList.add(UNLINKED_CLASS)
    } else if (action.kind === 'rewrite') {
      anchor.setAttribute('href', action.href)
    }
  }
}

// ---------------------------------------------------------------------------
// Waiting
// ---------------------------------------------------------------------------

/** One macrotask. */
function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0))
}

/** Whether `promise` settled (either way) within `ms`. Never rejects. */
export function settlesWithin(promise: Promise<unknown>, ms: number): Promise<boolean> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(false), ms)
    const done = () => {
      clearTimeout(timer)
      resolve(true)
    }
    promise.then(done, done)
  })
}

function imageUrl(img: HTMLImageElement): string {
  return img.currentSrc || img.getAttribute('src') || ''
}

/** Settles once `img` has loaded or failed; reports a failure in `warnings`. */
function imageDone(img: HTMLImageElement, warnings: string[]): Promise<void> {
  const failed = () => warnings.push(`image failed to load: ${imageUrl(img)}`)
  if (img.complete) {
    if (img.naturalWidth === 0) failed()
    return Promise.resolve()
  }
  return new Promise((resolve) => {
    img.addEventListener('load', () => resolve(), { once: true })
    img.addEventListener(
      'error',
      () => {
        failed()
        resolve()
      },
      { once: true }
    )
  })
}

/**
 * Waits for `images` (those with a URL) until `deadline` (`Date.now()` ms);
 * warns about each that failed and, once, about those still loading then.
 */
export async function waitForImages(
  images: readonly HTMLImageElement[],
  deadline: number,
  capMs: number,
  warnings: string[]
): Promise<void> {
  const pending = new Set(images.filter((img) => imageUrl(img) !== ''))
  const all = Promise.all(
    [...pending].map((img) => imageDone(img, warnings).then(() => void pending.delete(img)))
  )
  const finished = await settlesWithin(all, Math.max(0, deadline - Date.now()))
  if (finished || pending.size === 0) return
  const named = [...pending].slice(0, MAX_NAMED_IMAGES).map(imageUrl)
  const more = pending.size > named.length ? ` and ${pending.size - named.length} more` : ''
  warnings.push(
    `${pending.size} image(s) still loading after ${capMs / 1000} s: ${named.join(', ')}${more}`
  )
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

/**
 * Everything the run needs from the main bundle, injected by the trigger
 * (`export-ready.ts`): this chunk must not import a stateful module (the
 * loaders' settle promises are per-page state, and `dynamic-loader.ts`
 * imports `shared.ts`).
 */
export interface ExportDeps {
  doc: Document
  pageUrl: () => URL
  /** `dynamic-loader.ts`'s DOM-ready wait (main bundle). */
  waitForDom: () => Promise<void>
  /** `dynamic-loader.ts`'s stylesheet loader (main bundle). */
  loadCss: (href: string) => Promise<void>
  /** Where `/.mbr/` assets are at this page's depth. */
  assetBase: () => string
  /** The main bundle's loaders' settle promises. */
  settled: { mermaid: () => Promise<void>; math: () => Promise<void>; hljs: () => Promise<void> }
  /**
   * The page's own `@page` box. Defaults to {@link documentPageBox}; tests
   * stand it in (happy-dom drops `@page` rules).
   */
  pageBox?: () => DeclaredPageBox
  /** Defaults to {@link IMAGE_CAP_MS} / {@link SETTLE_CAP_MS}; tests shorten them. */
  imageCapMs?: number
  settleCapMs?: number
}

/** Every `<img>` in the document (happy-dom has no `document.images`). */
function allImages(doc: Document): HTMLImageElement[] {
  return Array.from(doc.querySelectorAll('img'))
}

/** `text`, cut to {@link MAX_WARNING_LENGTH}: it crosses into Rust as text. */
export function boundedText(text: string): string {
  return text.length > MAX_WARNING_LENGTH ? `${text.slice(0, MAX_WARNING_LENGTH - 1)}…` : text
}

/** Four decimals: a hundredth of a millimetre, as Rust writes the flag. */
function roundedSize(size: PageSize): PageSize {
  const round = (inches: number) => Math.round(inches * 10_000) / 10_000
  return { widthIn: round(size.widthIn), heightIn: round(size.heightIn) }
}

/** Trims and caps warnings: they cross into Rust, which treats them as text. */
function boundedWarnings(warnings: readonly string[]): string[] {
  return warnings.slice(0, MAX_WARNINGS).map(boundedText)
}

/**
 * Prepares the page and waits until it is ready to print. Rejects only for
 * a malformed flag or an unexpected error; every cap is a warning.
 */
export async function prepareForExport(raw: unknown, deps: ExportDeps): Promise<ExportResult> {
  const options = parseExportOptions(raw)
  const { doc } = deps
  const imageCapMs = deps.imageCapMs ?? IMAGE_CAP_MS
  const settleCapMs = deps.settleCapMs ?? SETTLE_CAP_MS
  const warnings: string[] = []
  const deadline = Date.now() + imageCapMs

  await deps.waitForDom()
  // Let the loaders' own DOM-ready callbacks claim their work first, so the
  // `*Settled()` promises below are the real ones.
  await tick()

  const cssUrl = `${deps.assetBase()}${EXPORT_CSS_FILE}`
  const css = deps.loadCss(cssUrl).catch(() => {
    warnings.push(`export stylesheet failed to load: ${cssUrl}`)
  })

  forceEagerImages(doc)
  applyLinkPolicy(doc, deps.pageUrl(), options.links)

  const settle = async (name: string, settled: () => Promise<void>) => {
    if (!(await settlesWithin(settled(), settleCapMs))) {
      warnings.push(`${name} not finished after ${settleCapMs / 1000} s`)
    }
  }
  await Promise.all([
    css,
    waitForImages(allImages(doc), deadline, imageCapMs, warnings),
    settle('diagrams (mermaid)', deps.settled.mermaid),
    settle('math (KaTeX)', deps.settled.math),
    settle('code highlighting', deps.settled.hljs),
  ])

  // Rendering can add links and images (a diagram's links, a contact
  // portrait); catch them with whatever is left of the image budget.
  const seen = new Set(allImages(doc))
  forceEagerImages(doc)
  applyLinkPolicy(doc, deps.pageUrl(), options.links)
  const late = allImages(doc).filter((img) => !seen.has(img))
  if (late.length > 0) await waitForImages(late, deadline, imageCapMs, warnings)

  // Every stylesheet has loaded by now, so the page's own `@page` rules can
  // be read: what they declare wins over the configured paper and margin.
  const declared = deps.pageBox ? deps.pageBox() : documentPageBox(doc, deps.assetBase())
  installPageStyle(doc, options, declared)

  // Last: KaTeX's fonts are only requested once math has rendered.
  const fonts = (doc as Document & { fonts?: FontFaceSet }).fonts
  if (fonts) await settle('fonts', () => fonts.ready.then(() => undefined))

  const result: ExportResult = { warnings: boundedWarnings(warnings) }
  if (declared.size) result.page = roundedSize(declared.size)
  return result
}

