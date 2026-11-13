/**
 * Copying content — a selection or the whole note — as its **markdown
 * source** or as **rich text** (HTML), without mbr's own page furniture.
 *
 * Shared by the selection toolbar (`<mbr-selection-actions>`), the info
 * panel's "Copy page" buttons and the GUI's Edit menu (through
 * {@link installCopyPageGlobal}). Main bundle.
 *
 * ## Markdown, or text where there is no source
 *
 * "Copy markdown" copies the note's own source: the whole body (frontmatter
 * dropped), or the lines of the blocks a selection covers, narrowed to the
 * exact span inside one block — the mapping is `source-copy.ts`. The source is
 * read from `/.mbr/raw` through `task-toggle.ts`'s shared cache
 * (`readSourceLines`), the same read a task toggle or review prefill makes, so
 * the page holds one copy of it. That endpoint exists only on a server with
 * editing on, and the selection mapping needs `data-mbr-line`; where either is
 * missing (static builds, editing off, `--no-review` pages, a selection
 * outside numbered blocks) the button says **Copy text** and copies plain
 * text instead ({@link markdownSourcePath}, {@link selectionCopyKind}). A read
 * that fails at copy time falls back to plain text too, and the toast says so.
 *
 * ## What is cleaned
 *
 * The rendered body carries things mbr adds for the reader, not the author:
 * heading permalinks (`#`), review markers, link-kind icons, flashcard progress
 * pies, the periodic-note bar, every `<mbr-*>` element (UI hosts whose shadow
 * content a clone does not carry anyway), buttons, scripts, and attributes —
 * `id`, `data-mbr-*`, Pico's `data-tooltip`, `tabindex`, `mbr-*` classes.
 * {@link cleanForCopy} removes all of it from a **clone**; the page is never
 * touched. Link and image URLs are made absolute, so a paste into another
 * application still points somewhere.
 *
 * ## Writing the clipboard
 *
 * {@link writeClipboard} tries `document.execCommand('copy')` **first**, with a
 * `copy` listener that sets exactly the flavours wanted, over a hidden
 * selection it makes and then undoes. Synchronous on purpose: it works only
 * inside the task a user gesture began (the same constraint `review/clipboard.ts`
 * documents), and an `await` on `navigator.clipboard.write` that then rejects
 * would push the fallback out of that task. The async Clipboard API is the
 * second route, for an engine whose `execCommand` is gone. In WKWebView a
 * script the app runs with `evaluateJavaScript` (the macOS Edit menu) counts
 * as a user gesture, so both routes are open there.
 *
 * A markdown copy whose source is not cached yet cannot be built inside the
 * click's task, so {@link writeClipboardLater} hands the Clipboard API a
 * `ClipboardItem` holding a *promise* — the write starts in the gesture and
 * the text arrives later. To make that the rare case, the toolbar and the info
 * panel warm the cache ({@link prefetchMarkdownSource}) as soon as they offer
 * the button.
 */

import { cachedSourceLines, currentDocumentPath, readSourceLines } from './task-toggle.ts'
import { isEditEnabled } from './shared.js'
import { pageMarkdown, selectionMarkdown, selectionSource, type SelectionSource } from './source-copy.ts'

/** Elements removed outright from a copy: mbr's own decoration. */
export const COPY_STRIP_SELECTOR = [
  '.mbr-heading-anchor',
  '.mbr-review-marker',
  '.mbr-link-icon',
  '.mbr-fc-pie',
  '.mbr-calendar-nav',
  '.sr-only',
  'script',
  'style',
  'template',
  'noscript',
  'button',
  '[data-mbr-copy-skip]',
].join(',')

/** Attributes dropped from every element of a copy. */
const STRIP_ATTRIBUTES = new Set(['id', 'tabindex', 'data-tooltip', 'data-placement', 'contenteditable'])

/** Attribute prefixes dropped from every element of a copy. */
const STRIP_ATTRIBUTE_PREFIXES = ['data-mbr-', 'data-pagefind-']

/** Attributes holding a URL, made absolute in a rich copy. */
const URL_ATTRIBUTES = ['href', 'src', 'poster']

/** What a copy button asks for. */
export type CopyFormat = 'markdown' | 'rich'

/** What actually went on the clipboard: markdown can degrade to text. */
export type CopiedAs = 'markdown' | 'text' | 'rich'

/** The outcome of a copy. */
export interface CopyOutcome {
  ok: boolean
  as: CopiedAs
}

/** What a copy puts on the clipboard. `html` is absent for a text-only copy. */
export interface CopyPayload {
  text: string
  html?: string
}

/** The flavours a rendered fragment is copied as. */
type RenderedFormat = 'text' | 'rich'

/**
 * Remove mbr's furniture from `root` **in place** (pass a clone).
 *
 * `baseUrl` resolves relative `href`/`src`/`poster` values; omit it to leave
 * them as they are.
 */
export function cleanForCopy(root: ParentNode, baseUrl?: string): void {
  for (const el of Array.from(root.querySelectorAll(COPY_STRIP_SELECTOR))) el.remove()
  // UI hosts (`<mbr-info>`, `<mbr-contact-backlinks>`, …): their content
  // lives in a shadow root, so a clone of one is an empty, meaningless tag.
  for (const el of Array.from(root.querySelectorAll('*'))) {
    if (el.localName.startsWith('mbr-')) el.remove()
  }
  for (const el of Array.from(root.querySelectorAll('*'))) cleanAttributes(el, baseUrl)
}

function cleanAttributes(el: Element, baseUrl: string | undefined): void {
  for (const name of el.getAttributeNames()) {
    if (STRIP_ATTRIBUTES.has(name) || STRIP_ATTRIBUTE_PREFIXES.some((p) => name.startsWith(p))) {
      el.removeAttribute(name)
    }
  }
  const classes = Array.from(el.classList).filter((c) => c.startsWith('mbr-'))
  if (classes.length > 0) el.classList.remove(...classes)
  if (el.getAttribute('class') === '') el.removeAttribute('class')
  if (baseUrl === undefined) return
  for (const attr of URL_ATTRIBUTES) {
    const value = el.getAttribute(attr)
    if (value === null || value.startsWith('#')) continue
    try {
      el.setAttribute(attr, new URL(value, baseUrl).href)
    } catch {
      // Not a URL a base can fix; leave it as written.
    }
  }
}

/** Blocks separated from their neighbours by a blank line. */
const PARAGRAPH_BLOCKS = new Set([
  'p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'pre', 'blockquote', 'table', 'ul', 'ol', 'dl',
  'figure', 'hr', 'section', 'article', 'header', 'footer', 'details', 'aside',
])

/** Blocks that start and end a line, with no blank line around them. */
const LINE_BLOCKS = new Set([
  'li', 'div', 'tr', 'dt', 'dd', 'figcaption', 'summary', 'nav', 'main', 'address', 'caption',
  'thead', 'tbody', 'tfoot',
])

/** Tags whose text is never part of a copy. */
const SKIPPED_TEXT = new Set(['script', 'style', 'template', 'noscript'])

/**
 * Plain text for a (cleaned) tree, the way a reader sees it: paragraphs and
 * headings separated by a blank line, list items and table rows one per line,
 * table cells by a tab, whitespace collapsed outside `<pre>`.
 *
 * Computed from the tree rather than with `innerText`, which needs the nodes
 * laid out — a detached clone is not, and attaching one to measure it would
 * restyle a whole note's worth of elements for a copy.
 */
export function plainTextOf(root: Node): string {
  let out = ''
  // Newlines owed before the next text: 1 = new line, 2 = blank line.
  let owed = 0

  const needBreak = (n: number): void => {
    if (out.length > 0) owed = Math.max(owed, n)
  }
  const emit = (text: string): void => {
    if (text.length === 0) return
    if (owed > 0) {
      out = out.replace(/[ \t]+$/, '') + '\n'.repeat(owed)
      owed = 0
    }
    out += text
  }

  const walk = (node: Node, inPre: boolean): void => {
    if (node.nodeType === Node.TEXT_NODE) {
      const data = (node as Text).data
      if (inPre) {
        emit(data)
        return
      }
      const collapsed = data.replace(/\s+/g, ' ')
      const atLineStart = owed > 0 || out.length === 0 || out.endsWith('\n') || out.endsWith('\t')
      emit(atLineStart || out.endsWith(' ') ? collapsed.replace(/^ /, '') : collapsed)
      return
    }
    if (node.nodeType !== Node.ELEMENT_NODE && node.nodeType !== Node.DOCUMENT_FRAGMENT_NODE) return
    const tag = node.nodeType === Node.ELEMENT_NODE ? (node as Element).localName : ''
    if (SKIPPED_TEXT.has(tag)) return
    if (tag === 'br') {
      out = out.replace(/[ \t]+$/, '')
      emit('\n')
      return
    }
    if (tag === 'img') {
      const alt = (node as Element).getAttribute('alt')
      if (alt) emit(alt)
      return
    }
    const gap = PARAGRAPH_BLOCKS.has(tag) ? 2 : LINE_BLOCKS.has(tag) ? 1 : 0
    if (gap > 0) needBreak(gap)
    const pre = inPre || tag === 'pre'
    const children = Array.from(node.childNodes)
    children.forEach((child, i) => {
      walk(child, pre)
      // A tab between cells, never after the last one in a row.
      const childTag = child.nodeType === Node.ELEMENT_NODE ? (child as Element).localName : ''
      if ((childTag === 'td' || childTag === 'th') && i < children.length - 1 && owed === 0) {
        out = out.replace(/ +$/, '') + '\t'
      }
    })
    if (gap > 0) needBreak(gap)
  }

  walk(root, false)
  return out.replace(/[ \t]+$/gm, '').trim()
}

/** Serialize a (cleaned) tree's children as HTML. */
export function htmlOf(root: ParentNode): string {
  const holder = document.createElement('div')
  for (const child of Array.from(root.childNodes)) holder.appendChild(child.cloneNode(true))
  return holder.innerHTML.trim()
}

/**
 * Inline and structural elements a selection inside them would lose when
 * cloned: `Range.cloneContents` starts at the common ancestor's *children*, so
 * three words inside `<strong>` copy as plain text, and two list items copy
 * without their `<ul>`.
 */
const WRAPPING_ANCESTORS = new Set([
  'strong', 'b', 'em', 'i', 'u', 's', 'del', 'ins', 'mark', 'code', 'kbd', 'sub', 'sup', 'a',
  'ul', 'ol', 'dl', 'table', 'thead', 'tbody', 'tfoot', 'tr', 'pre', 'blockquote',
])

/**
 * The contents of `range` as a fragment, re-wrapped in the inline and
 * structural ancestors it sits inside (up to, never including, `root`).
 */
export function fragmentForRange(range: Range, root: Node | null = null): DocumentFragment {
  let fragment = range.cloneContents()
  let ancestor: Node | null = range.commonAncestorContainer
  if (ancestor.nodeType !== Node.ELEMENT_NODE) ancestor = ancestor.parentNode
  while (ancestor !== null && ancestor !== root && ancestor.nodeType === Node.ELEMENT_NODE) {
    const el = ancestor as Element
    if (!WRAPPING_ANCESTORS.has(el.localName)) break
    const shell = el.cloneNode(false) as Element
    shell.appendChild(fragment)
    fragment = document.createDocumentFragment()
    fragment.appendChild(shell)
    ancestor = el.parentNode
  }
  return fragment
}

/** The payload for a cleaned fragment in `format`. */
export function payloadFor(fragment: DocumentFragment, format: RenderedFormat, baseUrl?: string): CopyPayload {
  cleanForCopy(fragment, baseUrl)
  const text = plainTextOf(fragment)
  return format === 'rich' ? { text, html: htmlOf(fragment) } : { text }
}

/** Selector for the rendered note: the same root find-in-page and review use. */
export const COPY_ROOT_SELECTOR = 'main#wrapper'

/** The rendered note on this page, if there is one. */
export function copyRoot(doc: Document = document): HTMLElement | null {
  return doc.querySelector<HTMLElement>(COPY_ROOT_SELECTOR)
}

/**
 * The live selection's range when it is a non-empty selection inside the
 * note, else null. Selections in the nav, a sidebar or a shadow root are not
 * the note's content.
 */
export function selectionRangeInRoot(selection: Selection | null, root: Node | null = copyRoot()): Range | null {
  if (root === null || !selection || selection.rangeCount === 0 || selection.isCollapsed) return null
  const range = selection.getRangeAt(0)
  if (range.collapsed || !root.contains(range.commonAncestorContainer)) return null
  return range
}

/**
 * The rendered payload (plain text, or text + HTML) for the live selection, or
 * null when nothing in the note is selected.
 */
export function selectionPayload(format: RenderedFormat, selection: Selection | null = window.getSelection()): CopyPayload | null {
  const root = copyRoot()
  const range = selectionRangeInRoot(selection, root)
  if (range === null) return null
  return payloadFor(fragmentForRange(range, root), format, document.baseURI)
}

/** The rendered payload for the whole note, or null on a page without one. */
export function documentPayload(format: RenderedFormat, doc: Document = document): CopyPayload | null {
  const root = copyRoot(doc)
  if (root === null) return null
  const fragment = doc.createDocumentFragment()
  for (const child of Array.from(root.childNodes)) fragment.appendChild(child.cloneNode(true))
  return payloadFor(fragment, format, doc.baseURI)
}

/**
 * Put `payload` on the clipboard; true when it got there.
 *
 * See the module comment for why `execCommand` goes first.
 */
export async function writeClipboard(payload: CopyPayload): Promise<boolean> {
  if (copyWithExecCommand(payload)) return true
  try {
    if (!globalThis.isSecureContext || !navigator.clipboard) return false
    if (payload.html !== undefined && typeof ClipboardItem === 'function' && navigator.clipboard.write) {
      await navigator.clipboard.write([
        new ClipboardItem({
          'text/html': new Blob([payload.html], { type: 'text/html' }),
          'text/plain': new Blob([payload.text], { type: 'text/plain' }),
        }),
      ])
      return true
    }
    if (navigator.clipboard.writeText) {
      await navigator.clipboard.writeText(payload.text)
      return true
    }
  } catch {
    // Denied, or no user activation left; the caller reports the failure.
  }
  return false
}

/**
 * The synchronous route: a `copy` listener that sets the flavours, run by
 * `execCommand('copy')` over a hidden selection.
 *
 * The selection is needed because WebKit disables the copy command — and so
 * never fires the event — when nothing is selected. The reader's own
 * selection is put back afterwards.
 */
export function copyWithExecCommand(payload: CopyPayload, doc: Document = document): boolean {
  if (typeof doc.execCommand !== 'function') return false
  let wrote = false
  const onCopy = (e: ClipboardEvent): void => {
    if (!e.clipboardData) return
    e.clipboardData.setData('text/plain', payload.text)
    if (payload.html !== undefined) e.clipboardData.setData('text/html', payload.html)
    e.preventDefault()
    wrote = true
  }

  const selection = doc.getSelection()
  const saved = selection ? Array.from({ length: selection.rangeCount }, (_, i) => selection.getRangeAt(i)) : []
  const holder = doc.createElement('div')
  holder.textContent = payload.text || ' '
  holder.setAttribute('aria-hidden', 'true')
  holder.style.cssText = 'position:fixed;top:0;left:-9999px;opacity:0;user-select:text;-webkit-user-select:text;white-space:pre'
  doc.body.appendChild(holder)
  doc.addEventListener('copy', onCopy, true)
  try {
    if (selection) {
      const range = doc.createRange()
      range.selectNodeContents(holder)
      selection.removeAllRanges()
      selection.addRange(range)
    }
    const ok = doc.execCommand('copy')
    return ok && wrote
  } catch {
    return false
  } finally {
    doc.removeEventListener('copy', onCopy, true)
    holder.remove()
    if (selection) {
      selection.removeAllRanges()
      for (const range of saved) selection.addRange(range)
    }
  }
}

/**
 * Write a payload that is still being built: the Clipboard API is called now,
 * inside the gesture, with a `ClipboardItem` whose text is a promise. Where
 * that is not available the payload is awaited and written the ordinary way,
 * which works wherever the browser keeps the gesture alive across the wait.
 */
export async function writeClipboardLater(pending: Promise<CopyPayload>): Promise<boolean> {
  if (globalThis.isSecureContext && typeof ClipboardItem === 'function' && navigator.clipboard?.write) {
    try {
      const blob = pending.then((p) => new Blob([p.text], { type: 'text/plain' }))
      await navigator.clipboard.write([new ClipboardItem({ 'text/plain': blob })])
      return true
    } catch {
      // Refused or unsupported: try the synchronous route below.
    }
  }
  return writeClipboard(await pending)
}

// ---------------------------------------------------------------------------
// Markdown source
// ---------------------------------------------------------------------------

/**
 * The repository path whose source "Copy markdown" reads, or null where the
 * page cannot read it: a static build (no server), a server with editing off
 * (`/.mbr/raw` is an editing endpoint), or a page that is not a note.
 */
export function markdownSourcePath(): string | null {
  if (!window.__MBR_CONFIG__?.serverMode || !isEditEnabled()) return null
  return currentDocumentPath()
}

const prefetched = new Set<string>()

/**
 * Start reading the note's source into the shared cache, once per page, so a
 * later "Copy markdown" can write synchronously.
 */
export function prefetchMarkdownSource(): void {
  const path = markdownSourcePath()
  if (path === null || prefetched.has(path) || cachedSourceLines(path) !== null) return
  prefetched.add(path)
  void readSourceLines(path).catch(() => {})
}

/** Forget which notes were prefetched (tests). */
export function resetCopyPrefetch(): void {
  prefetched.clear()
}

/** A block's whole plain text, as "Copy text" would copy it. */
function renderedBlockText(block: Element): string {
  const fragment = document.createDocumentFragment()
  fragment.appendChild(block.cloneNode(true))
  cleanForCopy(fragment)
  return plainTextOf(fragment)
}

/** Where the live selection is in the source, when it can be mapped at all. */
function liveSelectionSource(range: Range, root: Element, selectedText: string): SelectionSource | null {
  return selectionSource(range, root, selectedText, renderedBlockText)
}

/**
 * What the toolbar's copy button would copy for the live selection:
 * `'markdown'` when the page can read its source and the selection maps to
 * it, `'text'` otherwise, null with no selection in the note.
 */
export function selectionCopyKind(selection: Selection | null = window.getSelection()): 'markdown' | 'text' | null {
  const root = copyRoot()
  const range = selectionRangeInRoot(selection, root)
  if (range === null || root === null) return null
  if (markdownSourcePath() === null) return 'text'
  const text = payloadFor(fragmentForRange(range, root), 'text').text
  return liveSelectionSource(range, root, text) === null ? 'text' : 'markdown'
}

/**
 * Copy something as markdown: `build` turns the source lines into the text
 * (null when they do not fit), `fallback` is the plain text to copy instead.
 * Synchronous when the source is cached; otherwise read through the shared
 * cache and written with {@link writeClipboardLater}.
 */
async function copyMarkdown(
  path: string | null,
  build: ((lines: readonly string[]) => string | null) | null,
  fallback: CopyPayload,
): Promise<CopyOutcome> {
  if (path === null || build === null) {
    return { ok: await writeClipboard(fallback), as: 'text' }
  }
  const cached = cachedSourceLines(path)
  if (cached !== null) {
    const text = build(cached)
    const payload = text === null ? fallback : { text }
    return { ok: await writeClipboard(payload), as: text === null ? 'text' : 'markdown' }
  }
  let as: CopiedAs = 'text'
  const pending = readSourceLines(path)
    .then((read) => (read.ok ? build(read.lines) : null))
    .catch(() => null)
    .then((text): CopyPayload => {
      if (text === null) return fallback
      as = 'markdown'
      return { text }
    })
  const ok = await writeClipboardLater(pending)
  // The Clipboard API may finish before it has read the promise; `as` is
  // known once the text is.
  await pending
  return { ok, as }
}

/** Copy the live selection; `ok` is false when there is none or the write failed. */
export async function copySelection(format: CopyFormat): Promise<CopyOutcome> {
  if (format === 'rich') {
    const payload = selectionPayload('rich')
    return { ok: payload === null ? false : await writeClipboard(payload), as: 'rich' }
  }
  const root = copyRoot()
  const range = selectionRangeInRoot(window.getSelection(), root)
  if (range === null || root === null) return { ok: false, as: 'text' }
  // Everything that reads the selection happens now, before any wait.
  const fallback = payloadFor(fragmentForRange(range, root), 'text', document.baseURI)
  const path = markdownSourcePath()
  const where = path === null ? null : liveSelectionSource(range, root, fallback.text)
  return copyMarkdown(path, where === null ? null : (lines) => selectionMarkdown(lines, where), fallback)
}

/** Copy the whole note; `ok` is false on a page without one or when the write failed. */
export async function copyDocument(format: CopyFormat): Promise<CopyOutcome> {
  if (format === 'rich') {
    const payload = documentPayload('rich')
    return { ok: payload === null ? false : await writeClipboard(payload), as: 'rich' }
  }
  const fallback = documentPayload('text')
  if (fallback === null) return { ok: false, as: 'text' }
  return copyMarkdown(markdownSourcePath(), pageMarkdown, fallback)
}

/**
 * Name of the global the GUI's Edit menu calls (`src/browser.rs`,
 * `COPY_PAGE_RICH_SCRIPT` / `COPY_PAGE_MARKDOWN_SCRIPT`). **Referenced from a
 * Rust string literal**: renaming it cannot fail at compile time, which is why
 * `copy-content.test.ts` and a browser.rs unit test both pin the spelling.
 */
export const COPY_PAGE_GLOBAL = 'mbrCopyPage'

/** What the menu calls: copy the note, toast the outcome. */
export type CopyPageFunction = (format: CopyFormat) => Promise<boolean>

/**
 * Install `window.mbrCopyPage(format)` (`'markdown'` or `'rich'`). `report`
 * is told the outcome (the selection toolbar passes its toast); a menu has no
 * other way to say a copy failed.
 */
export function installCopyPageGlobal(report: (outcome: CopyOutcome) => void = () => {}): void {
  const fn: CopyPageFunction = async (format) => {
    const outcome = await copyDocument(format === 'rich' ? 'rich' : 'markdown')
    report(outcome)
    return outcome.ok
  }
  ;(window as unknown as Record<string, unknown>)[COPY_PAGE_GLOBAL] = fn
}

/** The toast text for a finished copy. */
export function copyOutcomeMessage(outcome: CopyOutcome, what: 'selection' | 'page'): string {
  if (!outcome.ok) return 'Copy failed'
  const kind = outcome.as === 'rich' ? 'rich text' : outcome.as
  return what === 'page' ? `Page copied as ${kind}` : `Copied as ${kind}`
}
