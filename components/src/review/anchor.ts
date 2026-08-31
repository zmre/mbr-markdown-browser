/**
 * Turning a reader's text selection into a note anchor.
 *
 * Reads the DOM but holds no state and mutates nothing, so it can be exercised
 * against a hand-built fixture under happy-dom.
 *
 * # Where the line numbers come from
 *
 * The renderer emits `data-mbr-line="N"` on block-level opens (`<p>`, `<h1>`-`<h6>`,
 * `<li>`, `<blockquote>`, `<pre>`, `<table>`) in server/GUI mode — see
 * `is_review_block_start` in `src/markdown.rs`. Walking up with `closest` finds
 * the innermost carrier, so a selection inside a list item reports the `<li>`'s
 * line rather than the enclosing list's.
 *
 * Blocks with no carrier exist and are not an error: a **tight** definition
 * list's `<dd>` has no `<p>` wrapper, a static build emits no attributes at all,
 * and a repository with a stale custom template may emit none either. All three
 * degrade to a file-level note rather than guessing a line.
 *
 * # Why the quote is stored in TextIndex form
 *
 * `buildTextIndex` is the vetted flattener the find bar uses. Reusing it means
 * the review feature inherits its prune list for free — `.sr-only` (the hidden
 * Pagefind title duplicate), `.mbr-heading-anchor` (the `#` permalink),
 * `.katex-mathml`, `svg` — and, more importantly, that re-anchoring later is a
 * plain verbatim `indexOf` with no normalization and no inverse mapping. See
 * `reanchor.ts`.
 */

import {
  SEARCH_ROOT_SELECTOR,
  buildTextIndex,
  chunkIndexAt,
  rangeForMatch,
  type TextIndex,
} from '../find-in-page.ts'
import { MAX_QUOTE } from './note-model.ts'
import type { NoteAnchor } from './types.ts'

/** The attribute the renderer puts a source line in. */
export const LINE_ATTR = 'data-mbr-line'

/** Selector for a block carrying a source line. */
export const LINE_SELECTOR = `[${LINE_ATTR}]`

/** The rendered body a selection has to fall inside to count. */
export function reviewRoot(doc: Document = document): HTMLElement | null {
  return doc.querySelector<HTMLElement>(SEARCH_ROOT_SELECTOR)
}

/** The element side of a node: itself, or its parent for a text node. */
function elementOf(node: Node | null): Element | null {
  if (node === null) return null
  return node.nodeType === Node.TEXT_NODE ? node.parentElement : (node as Element)
}

/** The 1-based source line on `element` or its nearest ancestor, if any. */
export function lineOfElement(element: Element | null): number | null {
  const carrier = element?.closest(LINE_SELECTOR)
  if (!carrier) return null
  const value = Number(carrier.getAttribute(LINE_ATTR))
  return Number.isSafeInteger(value) && value > 0 ? value : null
}

/**
 * The source line of the block containing `offset` in `index`.
 *
 * The lookup that `reanchor.ts` is given as its `lineAt`.
 */
export function lineOfOffset(index: TextIndex, offset: number): number | null {
  if (offset < 0 || offset >= index.text.length) return null
  const node = index.nodes[chunkIndexAt(index.starts, offset)]
  // A synthetic separator belongs to no node and therefore to no block.
  return node ? lineOfElement(node.parentElement) : null
}

/** A half-open `[start, end)` span of {@link TextIndex.text}. */
export interface OffsetSpan {
  start: number
  end: number
}

/**
 * Map a live `Range` onto offsets in `index.text`.
 *
 * An endpoint that lands in a node the index does not cover is **clamped** to
 * the nearest indexed text inside the range rather than abandoning the quote.
 * That case is ordinary, not exotic: selecting a whole heading takes in its `#`
 * permalink anchor, which is in `BLOCKED_SELECTOR` precisely so it stays out of
 * the text — so without the clamp, selecting a heading would capture nothing.
 *
 * Returns `null` only when the range contains no indexed text at all.
 */
export function offsetsForRange(index: TextIndex, range: Range): OffsetSpan | null {
  const chunkOf = nodeChunks(index)

  const start =
    offsetOf(index, chunkOf, range.startContainer, range.startOffset) ??
    clampedStart(index, range)
  const end =
    offsetOf(index, chunkOf, range.endContainer, range.endOffset) ?? clampedEnd(index, range)

  if (start === null || end === null || end <= start) return null
  return { start, end }
}

/** Offset of the first indexed text at or after the range's start. */
function clampedStart(index: TextIndex, range: Range): number | null {
  for (let i = 0; i < index.nodes.length; i++) {
    const node = index.nodes[i]
    // `comparePoint` is -1 before the range, 0 inside, 1 after.
    if (node !== null && range.comparePoint(node, node.data.length) >= 0) {
      return index.starts[i]!
    }
  }
  return null
}

/** Offset just past the last indexed text that begins at or before the range's end. */
function clampedEnd(index: TextIndex, range: Range): number | null {
  for (let i = index.nodes.length - 1; i >= 0; i--) {
    const node = index.nodes[i]
    if (node !== null && range.comparePoint(node, 0) <= 0) {
      return index.starts[i]! + node.data.length
    }
  }
  return null
}

/** `Text` node -> its chunk index, built once per call. */
function nodeChunks(index: TextIndex): Map<Text, number> {
  const map = new Map<Text, number>()
  index.nodes.forEach((node, i) => {
    // A node appears at most once, but guard anyway: the first chunk is the one
    // whose `starts` entry the offsets are measured from.
    if (node !== null && !map.has(node)) map.set(node, i)
  })
  return map
}

/**
 * The index offset for a `(container, offset)` DOM position.
 *
 * A position may name a text node directly, or an element plus a child index —
 * `Selection` produces both, depending on how the drag ended.
 */
function offsetOf(
  index: TextIndex,
  chunkOf: Map<Text, number>,
  container: Node,
  offset: number
): number | null {
  if (container.nodeType === Node.TEXT_NODE) {
    const chunk = chunkOf.get(container as Text)
    if (chunk === undefined) return null
    return index.starts[chunk]! + Math.min(offset, (container as Text).data.length)
  }

  // An element position: resolve to the first indexed text node at or after it.
  const child = container.childNodes[offset] ?? null
  const text = child ? firstIndexedText(child, chunkOf) : lastIndexedTextIn(container, chunkOf)
  if (text === null) return null
  const chunk = chunkOf.get(text)
  if (chunk === undefined) return null
  return child ? index.starts[chunk]! : index.starts[chunk]! + text.data.length
}

/** The first indexed `Text` at or inside `node`, in document order. */
function firstIndexedText(node: Node, chunkOf: Map<Text, number>): Text | null {
  if (node.nodeType === Node.TEXT_NODE && chunkOf.has(node as Text)) return node as Text
  for (const child of Array.from(node.childNodes)) {
    const found = firstIndexedText(child, chunkOf)
    if (found !== null) return found
  }
  return null
}

/** The last indexed `Text` inside `node`, in document order. */
function lastIndexedTextIn(node: Node, chunkOf: Map<Text, number>): Text | null {
  const children = Array.from(node.childNodes)
  for (let i = children.length - 1; i >= 0; i--) {
    const found = lastIndexedTextIn(children[i]!, chunkOf)
    if (found !== null) return found
  }
  return node.nodeType === Node.TEXT_NODE && chunkOf.has(node as Text) ? (node as Text) : null
}

/**
 * Move an end point sitting at offset 0 back to the end of the previous text.
 *
 * A selection dragged to the end of a paragraph very often ends at
 * `(nextBlock, 0)` — a position that is *before* any of the next block's text,
 * but still resolves to the next block's element and so to the next block's
 * line. Left alone it inflates `endLine` by one on a large fraction of ordinary
 * selections, which is the single likeliest off-by-one in this feature.
 *
 * Returns the element the end point should be attributed to.
 */
export function endPointElement(range: Range): Element | null {
  if (range.endOffset !== 0) return elementOf(range.endContainer)

  const walker = document.createTreeWalker(
    range.commonAncestorContainer,
    NodeFilter.SHOW_TEXT
  )
  let last: Text | null = null
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    const text = node as Text
    if (text.data.length === 0) continue
    // Strictly before the end point.
    if (range.comparePoint(text, text.data.length) <= 0) last = text
    else break
  }
  return last !== null ? last.parentElement : elementOf(range.endContainer)
}

/**
 * Resolve a selection to a note anchor, or `null` when it is not reviewable.
 *
 * `null` means "make a file-level note instead", not "fail": a collapsed
 * selection is the plain `r` shortcut, and a selection outside the rendered
 * body (the nav, the sidebar, a panel's shadow root) is not about the document.
 */
export function anchorFromRange(
  range: Range,
  file: string,
  root: HTMLElement | null = reviewRoot()
): NoteAnchor | null {
  if (root === null || range.collapsed) return null
  // A `Selection` cannot span into a shadow root in a way `contains` accepts,
  // so this rejects every `<mbr-*>` element's internals for free.
  if (!root.contains(range.commonAncestorContainer)) return null

  const startLine = lineOfElement(elementOf(range.startContainer))
  const endLine = lineOfElement(endPointElement(range))

  const index = buildTextIndex(root)
  const span = offsetsForRange(index, range)
  const quote = span === null ? null : index.text.slice(span.start, span.end).slice(0, MAX_QUOTE)

  const line = startLine
  // Defensive: `getRangeAt` returns document order, so an inverted pair should
  // be impossible — but an inverted one would export as `12-4`.
  const resolvedEnd = line !== null && endLine !== null && endLine > line ? endLine : null

  return { file, line, endLine: resolvedEnd, quote: quote && quote.length > 0 ? quote : null }
}

/**
 * The anchor for the reader's current selection, if it is reviewable.
 *
 * Convenience over {@link anchorFromRange} for the trigger element.
 */
export function anchorFromSelection(file: string, selection: Selection | null): NoteAnchor | null {
  if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return null
  return anchorFromRange(selection.getRangeAt(0), file)
}

/** Re-derive a live `Range` for a stored quote, for highlight painting. */
export function rangeForQuote(index: TextIndex, start: number, length: number): Range | null {
  return rangeForMatch(index, start, start + length)
}
