/**
 * Mapping a note — or a selection in it — back to its **markdown source**, for
 * "Copy markdown" (`copy-content.ts`).
 *
 * Holds no state and does no I/O: the source lines come in from the caller
 * (`task-toggle.ts`'s shared `/.mbr/raw` cache), so everything here runs under
 * happy-dom against hand-built fixtures.
 *
 * ## Blocks and their lines
 *
 * In server/GUI mode the renderer puts `data-mbr-line="N"` (1-based, counted
 * in the whole file, frontmatter included) on block starts: `<p>`, headings,
 * `<li>`, `<blockquote>`, `<pre>`, `<table>`, `<dt>`, `<dd>` — see
 * `is_review_block_start` in `crates/mbr-core/src/markdown.rs`. A block's
 * source runs from its own line to the line before the next numbered block
 * that is not nested inside it (or to the end of the file), less trailing
 * blank lines. A paragraph or heading cannot hold a blank line, so it also
 * ends at the first one — which keeps an unnumbered `---` or HTML block that
 * follows it out of its lines. A `<dt>` is always one line.
 *
 * ## A selection
 *
 * A selection maps only when **all** of its text sits inside numbered blocks
 * (a contact card or a periodic-note bar has no source lines). Several blocks
 * → their lines whole. Inside one block → {@link narrowToSelection} looks for
 * the exact span of the block's source that renders as the selected text,
 * stepping over markup (emphasis, code ticks, escapes, link destinations,
 * inline tags, entities, smart punctuation), and widens it to keep emphasis
 * and links balanced. When that fails the whole block is copied.
 */

/** The attribute the renderer puts a block's source line in. */
export const SOURCE_LINE_ATTR = 'data-mbr-line'

const SOURCE_LINE_SELECTOR = `[${SOURCE_LINE_ATTR}]`

/** mbr's own furniture inside a block; its text is not the author's. */
const FURNITURE_SELECTOR = '.mbr-heading-anchor, .mbr-review-marker, .mbr-link-icon, .mbr-fc-pie, .sr-only, button'

/** An inclusive, 1-based range of source lines. */
export interface LineSpan {
  start: number
  end: number
}

/** Where a selection's markdown is, before the source has been read. */
export interface SelectionSource extends LineSpan {
  /**
   * Set when the selection lies inside one block and covers only part of it:
   * the selected text, to narrow the block's lines to.
   */
  narrowTo?: string
  /** The one block is a paragraph or heading (see {@link fitSpan}). */
  leaf?: boolean
}

// ---------------------------------------------------------------------------
// Whole note
// ---------------------------------------------------------------------------

/**
 * The note's body: `lines` without a leading YAML frontmatter block, leading
 * blank lines or trailing blank lines.
 *
 * Frontmatter is recognised the way `frontmatter.rs` does: a first line `---`
 * followed by a non-blank line, closed by a later `---` or `...` line. An
 * unclosed block is not frontmatter, so nothing is dropped.
 */
export function stripFrontmatter(lines: readonly string[]): string[] {
  let from = 0
  const first = (lines[0] ?? '').replace(/^\uFEFF/, '').trimEnd()
  if (first === '---' && lines.length > 1 && (lines[1] ?? '').trim() !== '') {
    for (let i = 1; i < lines.length; i++) {
      const line = (lines[i] ?? '').trimEnd()
      if (line === '---' || line === '...') {
        from = i + 1
        break
      }
    }
  }
  return trimBlankEdges(lines.slice(from))
}

function trimBlankEdges(lines: string[]): string[] {
  let start = 0
  let end = lines.length
  while (start < end && (lines[start] ?? '').trim() === '') start++
  while (end > start && (lines[end - 1] ?? '').trim() === '') end--
  return lines.slice(start, end)
}

/** The note's body as text: {@link stripFrontmatter}, joined. */
export function pageMarkdown(lines: readonly string[]): string {
  return stripFrontmatter(lines).join('\n')
}

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

/** The source line on `el`, or null when it carries none (or a bad one). */
export function blockLine(el: Element): number | null {
  const value = Number(el.getAttribute(SOURCE_LINE_ATTR))
  return Number.isSafeInteger(value) && value > 0 ? value : null
}

/** Every numbered block under `root`, in document order. */
export function numberedBlocks(root: ParentNode): Element[] {
  return Array.from(root.querySelectorAll(SOURCE_LINE_SELECTOR)).filter((el) => blockLine(el) !== null)
}

const LEAF_BLOCKS = new Set(['p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6'])

/**
 * The inclusive line span of `blocks[index]`, with `lineCount` the file's
 * length when known (the end of the file is the last block's end).
 */
export function blockSpan(blocks: readonly Element[], index: number, lineCount = Number.POSITIVE_INFINITY): LineSpan | null {
  const block = blocks[index]
  if (!block) return null
  const start = blockLine(block)
  if (start === null) return null
  if (block.localName === 'dt') return { start, end: start }
  let end = lineCount
  for (let i = index + 1; i < blocks.length; i++) {
    const next = blocks[i]!
    if (block.contains(next)) continue
    const line = blockLine(next)!
    if (line > start) {
      end = line - 1
      break
    }
  }
  return { start, end: Math.max(start, end) }
}

/**
 * Trim `span` against the actual source: never past the end of the file, a
 * paragraph or heading cut at its first blank line, trailing blank lines
 * dropped. Null when the span starts past the end (a stale page).
 */
export function fitSpan(lines: readonly string[], span: LineSpan, leaf = false): LineSpan | null {
  if (span.start < 1 || span.start > lines.length) return null
  let end = Math.min(span.end, lines.length)
  if (leaf) {
    for (let line = span.start + 1; line <= end; line++) {
      if ((lines[line - 1] ?? '').trim() === '') {
        end = line - 1
        break
      }
    }
  }
  while (end > span.start && (lines[end - 1] ?? '').trim() === '') end--
  return { start: span.start, end }
}

/** The source text of `span` (inclusive, 1-based). */
export function linesOf(lines: readonly string[], span: LineSpan): string {
  return lines.slice(span.start - 1, span.end).join('\n')
}

// ---------------------------------------------------------------------------
// Selection → blocks (DOM, read-only)
// ---------------------------------------------------------------------------

function elementOf(node: Node): Element | null {
  return node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node.parentElement
}

/** The innermost numbered block holding `node`, within `root`. */
function innermostBlock(node: Node, root: Element): Element | null {
  const carrier = elementOf(node)?.closest(SOURCE_LINE_SELECTOR) ?? null
  return carrier !== null && root.contains(carrier) && blockLine(carrier) !== null ? carrier : null
}

/** The part of text node `text` that `range` selects. */
function selectedPart(range: Range, text: Text): string {
  let from = 0
  let to = text.data.length
  if (range.startContainer === text) from = range.startOffset
  if (range.endContainer === text) to = range.endOffset
  return text.data.slice(from, to)
}

/**
 * Whether every non-blank piece of text `range` selects lies inside a
 * numbered block (mbr's furniture aside). Also collects the innermost block of
 * each piece, in document order, without repeats.
 */
function selectedBlocks(range: Range, root: Element): Element[] | null {
  const ancestor = range.commonAncestorContainer
  const scope = ancestor.nodeType === Node.TEXT_NODE ? ancestor.parentNode : ancestor
  if (scope === null) return null
  const doc = root.ownerDocument
  const blocks: Element[] = []
  const visit = (text: Text): boolean => {
    if (!range.intersectsNode(text)) return true
    if (selectedPart(range, text).trim() === '') return true
    const parent = text.parentElement
    if (parent?.closest(FURNITURE_SELECTOR)) return true
    const block = innermostBlock(text, root)
    if (block === null) return false
    if (blocks[blocks.length - 1] !== block && !blocks.includes(block)) blocks.push(block)
    return true
  }
  if (scope.nodeType === Node.TEXT_NODE) {
    return visit(scope as Text) ? blocks : null
  }
  const walker = doc.createTreeWalker(scope, NodeFilter.SHOW_TEXT)
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    if (!visit(node as Text)) return null
  }
  return blocks
}

/**
 * Where the selection `range` (inside `root`) is in the note's source, or
 * null when some of its text is outside every numbered block.
 *
 * `selectedText` is the selection as plain text (what "Copy text" would
 * copy); `blockText` gives a block's whole plain text, so a selection that
 * covers all of one block copies its lines whole rather than a narrowed span
 * that would drop `## ` or `- `.
 */
export function selectionSource(
  range: Range,
  root: Element,
  selectedText: string,
  blockText: (block: Element) => string,
): SelectionSource | null {
  if (range.collapsed || !root.contains(range.commonAncestorContainer)) return null
  const selected = selectedBlocks(range, root)
  if (selected === null || selected.length === 0) return null
  const all = numberedBlocks(root)
  let start = Number.POSITIVE_INFINITY
  let end = 0
  for (const block of selected) {
    const span = blockSpan(all, all.indexOf(block))
    if (span === null) return null
    start = Math.min(start, span.start)
    end = Math.max(end, span.end)
  }
  // One block, or a block and blocks nested in it (a list item and its
  // loose paragraph): the outermost decides.
  const outer = selected.filter((b) => !selected.some((other) => other !== b && other.contains(b)))
  if (outer.length === 1) {
    const block = outer[0]!
    const wanted = collapse(selectedText)
    if (wanted !== '' && wanted !== collapse(blockText(block))) {
      return { start, end, narrowTo: wanted, ...leafFlag(block) }
    }
    return { start, end, ...leafFlag(block) }
  }
  return { start, end }
}

/** Marks a span that belongs to a paragraph or heading (see {@link fitSpan}). */
function leafFlag(block: Element): { leaf?: boolean } {
  return LEAF_BLOCKS.has(block.localName) ? { leaf: true } : {}
}

/**
 * The markdown for a {@link selectionSource} result, given the file's lines;
 * null when the lines no longer fit (the file is shorter than the page).
 */
export function selectionMarkdown(lines: readonly string[], where: SelectionSource): string | null {
  const span = fitSpan(lines, where, where.leaf === true)
  if (span === null) return null
  const source = linesOf(lines, span)
  if (where.narrowTo !== undefined) {
    const narrowed = narrowToSelection(source, where.narrowTo)
    if (narrowed !== null) return source.slice(narrowed.start, narrowed.end)
  }
  return source
}

function collapse(text: string): string {
  return text.replace(/\s+/g, ' ').trim()
}

// ---------------------------------------------------------------------------
// Narrowing (pure string work)
// ---------------------------------------------------------------------------

/** A half-open `[start, end)` span of a string. */
export interface CharSpan {
  start: number
  end: number
}

/** Characters markdown uses as markup, stepped over when they do not match. */
const MARKUP = new Set(['*', '_', '`', '~', '=', '\\', '[', ']', '!', '^', '<', '>', '#', '|'])

/** Characters standing for a line break, a container prefix or a cell edge. */
const GAP = new Set([' ', '\t', '\n', '\r', '>', '|'])

const ENTITIES: Record<string, string> = {
  amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ', ndash: '–', mdash: '—', hellip: '…',
  lsquo: "'", rsquo: "'", ldquo: '"', rdquo: '"', copy: '©', reg: '®', trade: '™',
}

/** Typographic forms a renderer with smart punctuation produces. */
function plainChar(ch: string): string {
  switch (ch) {
    case '‘':
    case '’':
    case '‚':
      return "'"
    case '“':
    case '”':
    case '„':
      return '"'
    case '\u00a0':
      return ' '
    default:
      return ch
  }
}

const ENTITY = /^&(#\d+|#[xX][0-9a-fA-F]+|[A-Za-z][A-Za-z0-9]*);/
const HTML_TAG = /^<\/?[A-Za-z][A-Za-z0-9-]*(?:\s[^<>]*)?\/?>/
const HTML_COMMENT = /^<!--[\s\S]*?-->/

function decodeEntity(body: string): string | null {
  if (body.startsWith('#')) {
    const code = body[1] === 'x' || body[1] === 'X' ? parseInt(body.slice(2), 16) : parseInt(body.slice(1), 10)
    return Number.isFinite(code) && code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : null
  }
  return ENTITIES[body] ?? null
}

/**
 * How many source characters at `k` render as the selected character `ch`
 * (0 when none do).
 */
function matchChar(source: string, k: number, ch: string): number {
  const want = plainChar(ch)
  const at = source[k]
  if (at === undefined) return 0
  // An entity first: `&amp;` renders as `&`, which would otherwise match the
  // `&` alone and strand `amp;`.
  if (at === '&') {
    const m = ENTITY.exec(source.slice(k, k + 12))
    const decoded = m ? decodeEntity(m[1]!) : null
    if (m && decoded !== null) return plainChar(decoded) === want ? m[0].length : 0
  }
  if (plainChar(at) === want) return 1
  if (want === '–' && source.startsWith('--', k) && !source.startsWith('---', k)) return 2
  if (want === '—' && source.startsWith('---', k)) return 3
  if (want === '…' && source.startsWith('...', k)) return 3
  return 0
}

/**
 * How many source characters at `k` are markup that renders as nothing, or 0.
 * A link's `](destination)`, a wikilink's `[[target|`, an inline tag and an
 * HTML comment are stepped over whole.
 */
function skipMarkup(source: string, k: number): number {
  const at = source[k]
  if (at === undefined) return 0
  if (at === ']') {
    const next = source[k + 1]
    if (next === '(') {
      const close = matchingClose(source, k + 1, '(', ')')
      return close === -1 ? 1 : close + 1 - k
    }
    if (next === '[') {
      const close = source.indexOf(']', k + 2)
      return close === -1 ? 1 : close + 1 - k
    }
    return 1
  }
  if (at === '[' && source[k + 1] === '[') {
    const close = source.indexOf(']]', k + 2)
    const bar = source.indexOf('|', k + 2)
    if (close !== -1 && bar !== -1 && bar < close) return bar + 1 - k
    return 2
  }
  if (at === '<') {
    const rest = source.slice(k, k + 512)
    const m = HTML_COMMENT.exec(rest) ?? HTML_TAG.exec(rest)
    if (m) return m[0].length
    return 1
  }
  return MARKUP.has(at) ? 1 : 0
}

function matchingClose(source: string, open: number, o: string, c: string): number {
  let depth = 0
  for (let i = open; i < source.length; i++) {
    const ch = source[i]
    if (ch === '\\') {
      i++
      continue
    }
    if (ch === '\n' && source[i + 1] === '\n') return -1
    if (ch === o) depth++
    else if (ch === c && --depth === 0) return i
  }
  return -1
}

function isSpace(ch: string | undefined): boolean {
  return ch === undefined || /\s/.test(ch)
}

/**
 * Match all of `wanted` (whitespace already collapsed) starting exactly at
 * `from`; the end of the match, or -1.
 */
function matchFrom(source: string, from: number, wanted: string): number {
  let k = from
  let j = 0
  while (j < wanted.length) {
    const ch = wanted[j]!
    if (ch === ' ') {
      // A space stands for any run of whitespace, line breaks with their
      // container prefixes (`> `), table pipes, or a `\` hard break.
      const before = k
      while (k < source.length && (GAP.has(source[k]!) || (source[k] === '\\' && source[k + 1] === '\n'))) k++
      if (k === before) {
        // Markup may sit before the gap (`**a** b`, `[a](u) b`).
        let m = k
        let step: number
        while ((step = skipMarkup(source, m)) > 0) m += step
        if (m === k || !GAP.has(source[m] ?? '')) return -1
        k = m
        continue
      }
      j++
      continue
    }
    const n = matchChar(source, k, ch)
    if (n > 0) {
      k += n
      j++
      continue
    }
    const skip = skipMarkup(source, k)
    if (skip === 0) return -1
    // An escaped character is literal: `\*` renders as `*`.
    if (source[k] === '\\' && k + 1 < source.length && matchChar(source, k + 1, ch) > 0) {
      k += 1 + matchChar(source, k + 1, ch)
      j++
      continue
    }
    k += skip
  }
  return k
}

/**
 * The span of `source` that renders as `selected` (whitespace collapsed),
 * widened to keep markup balanced — or null when no span matches.
 */
export function narrowToSelection(source: string, selected: string): CharSpan | null {
  const wanted = collapse(selected)
  if (wanted === '') return null
  for (let s = 0; s < source.length; s++) {
    if (matchChar(source, s, wanted[0]!) === 0) continue
    const e = matchFrom(source, s, wanted)
    if (e !== -1) return balanceSpan(source, { start: s, end: e })
  }
  return null
}

const EMPHASIS = new Set(['*', '_', '~', '=', '`'])

function isEscaped(source: string, i: number): boolean {
  let n = 0
  for (let k = i - 1; k >= 0 && source[k] === '\\'; k--) n++
  return n % 2 === 1
}

/** A run of one emphasis character, `[start, end)`. */
interface Run {
  ch: string
  start: number
  end: number
}

function runsIn(source: string, from: number, to: number): Run[] {
  const runs: Run[] = []
  for (let i = from; i < to; ) {
    const ch = source[i]!
    if (EMPHASIS.has(ch) && !isEscaped(source, i)) {
      let j = i
      while (j < to && source[j] === ch) j++
      runs.push({ ch, start: i, end: j })
      i = j
    } else i++
  }
  return runs
}

/**
 * Widen `span` so the markup it holds is balanced:
 *
 * 1. absorb wrappers around it — `**x**`, `` `x` ``, `[x](u)`, `![x](u)`;
 * 2. an emphasis run opened inside and closed outside extends the end to its
 *    closer, one closed inside and opened outside extends the start;
 * 3. likewise an unmatched `[` or `]`.
 */
export function balanceSpan(source: string, span: CharSpan): CharSpan {
  let { start, end } = span

  // 1. Wrappers.
  for (;;) {
    const before = source[start - 1]
    if (before !== undefined && EMPHASIS.has(before) && source[end] === before && !isEscaped(source, start - 1)) {
      start--
      end++
      continue
    }
    if (before === '[' && source[end] === ']' && !isEscaped(source, start - 1)) {
      start--
      end += Math.max(1, skipMarkup(source, end))
      if (source[start - 1] === '!') start--
      continue
    }
    break
  }

  // 2. Emphasis runs, paired by character and length. Inside a code span
  //    nothing else counts.
  const open: Run[] = []
  const unmatchedClosers: Run[] = []
  let code: Run | null = null
  for (const run of runsIn(source, start, end)) {
    const len = run.end - run.start
    if (code !== null) {
      if (run.ch === '`' && len === code.end - code.start) code = null
      continue
    }
    const canOpen = !isSpace(source[run.end])
    const canClose = !isSpace(source[run.start - 1])
    const top = open.length > 0 ? open[open.length - 1]! : null
    if (canClose && top !== null && top.ch === run.ch && top.end - top.start === len) {
      open.pop()
    } else if (run.ch === '`') {
      code = run
    } else if (canOpen) {
      open.push(run)
    } else if (canClose) {
      unmatchedClosers.push(run)
    }
  }
  if (code !== null) open.push(code)
  for (const run of open) {
    const len = run.end - run.start
    const closer = runsIn(source, end, source.length).find((r) => r.ch === run.ch && r.end - r.start === len)
    if (closer) end = Math.max(end, closer.end)
  }
  for (const run of unmatchedClosers) {
    const len = run.end - run.start
    const opener = runsIn(source, 0, start).reverse().find((r) => r.ch === run.ch && r.end - r.start === len)
    if (opener) start = Math.min(start, opener.start)
  }

  // 3. Brackets.
  let depth = 0
  let firstUnclosed = -1
  for (let i = start; i < end; i++) {
    if (isEscaped(source, i)) continue
    if (source[i] === '[') {
      if (depth === 0) firstUnclosed = i
      depth++
    } else if (source[i] === ']') {
      if (depth > 0) depth--
      else {
        // Closed inside, opened before: find its `[`.
        let d = 0
        for (let k = start - 1; k >= 0; k--) {
          if (isEscaped(source, k)) continue
          if (source[k] === ']') d++
          else if (source[k] === '[' && d-- === 0) {
            start = source[k - 1] === '!' ? k - 1 : k
            break
          }
        }
      }
    }
  }
  if (depth > 0 && firstUnclosed !== -1) {
    const close = matchingClose(source, firstUnclosed, '[', ']')
    if (close !== -1) end = Math.max(end, close + skipMarkup(source, close))
  }
  // A link closed inside whose destination is outside: take the destination.
  if (source[end - 1] === ']' && (source[end] === '(' || source[end] === '[')) {
    end = end - 1 + skipMarkup(source, end - 1)
  }
  return enclosingLink(source, { start, end }) ?? { start, end }
}

/**
 * The whole link when `span` lies inside a link's text — selecting a word of
 * `[the docs](u)` copies the link, URL and all — else null. Looks back only
 * within the paragraph (no blank line).
 */
function enclosingLink(source: string, span: CharSpan): CharSpan | null {
  let depth = 0
  for (let k = span.start - 1; k >= 0; k--) {
    if (source[k] === '\n' && source[k - 1] === '\n') return null
    if (isEscaped(source, k)) continue
    if (source[k] === ']') depth++
    else if (source[k] === '[') {
      if (depth > 0) {
        depth--
        continue
      }
      const close = matchingClose(source, k, '[', ']')
      if (close === -1 || close < span.end - 1) return null
      if (source[close + 1] !== '(' && source[close + 1] !== '[') return null
      const start = source[k - 1] === '!' ? k - 1 : k
      return { start, end: close + skipMarkup(source, close) }
    }
  }
  return null
}
