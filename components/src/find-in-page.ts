/**
 * Pure find-in-page primitives, kept out of `<mbr-find-bar>` so they can be
 * unit-tested and benchmarked without a browser (happy-dom has no
 * `CSS.highlights` and reports zero-size rects for every range). Same
 * pure-module precedent as `fuzzy.ts` and `sorting.ts`.
 *
 * The pipeline is: flatten a subtree into one string (`buildTextIndex`),
 * compile the user's query (`compileQuery`), scan for offsets
 * (`findMatchOffsets`), then map offsets back onto live `Range`s
 * (`rangeForMatch`). Counting and painting stay decoupled: the scan allocates
 * no `Range`s at all, so "N of M" is exact even when the caller paints only a
 * window of the matches.
 *
 * Shadow DOM needs no special-casing: `document.createTreeWalker` does not
 * descend into shadow roots, so every `<mbr-*>` element's internals are
 * invisible to the walk for free.
 */

import { escapeRegex } from './fuzzy.js';

/**
 * Container every page template renders body content into. Verified identical
 * across all seven page templates (`index.html`, `home.html`, `section.html`,
 * and friends).
 */
export const SEARCH_ROOT_SELECTOR = 'main#wrapper';

/**
 * Elements whose subtrees are pruned from the index. Each entry is
 * load-bearing:
 *
 * - `script` / `style` / `noscript` / `template` — source text, never painted.
 * - `svg` / `math` — `<text>` inside an SVG does not paint under
 *   `::highlight()`, so a Mermaid diagram would count matches nobody can see.
 * - `.sr-only` — `index.html` emits a visually hidden duplicate of the title
 *   for Pagefind; without this every title search shows a phantom first match
 *   at a zero-size rect.
 * - `.mbr-heading-anchor` — `mbr-heading-enhancer` appends a `#` permalink to
 *   every heading; without this a search for `#` matches every heading.
 * - `.katex-mathml` — KaTeX emits MathML *and* HTML for the same formula.
 * - `.mbr-review-marker` — the review feature injects one of these into a block
 *   for every anchored note. Its glyph is `::before` generated content so it is
 *   already outside the text run, which makes this belt and braces — but a
 *   marker that ever gained real text would otherwise land inside the quotes
 *   the review feature itself captures through this index, and a note would
 *   start quoting its own marker.
 * - `.sectionhidden` / `[hidden]` / `[aria-hidden="true"]` — `display: none`,
 *   so a match there has no rect to scroll to.
 */
export const BLOCKED_SELECTOR = [
  'script',
  'style',
  'noscript',
  'template',
  'svg',
  'math',
  '.sr-only',
  '.mbr-heading-anchor',
  '.mbr-review-marker',
  '.katex-mathml',
  '.sectionhidden',
  '[hidden]',
  '[aria-hidden="true"]',
].join(',');

/**
 * Tags that do NOT introduce a visual break, so their text concatenates with
 * the surrounding run: `<p>a<em>b</em>c</p>` must match "abc". Anything not
 * listed here is treated as block-level and gets a separator.
 */
const INLINE_TAGS = new Set([
  'A', 'ABBR', 'B', 'BDI', 'BDO', 'BIG', 'CITE', 'CODE', 'DATA', 'DEL', 'DFN',
  'EM', 'FONT', 'I', 'IMG', 'INS', 'KBD', 'LABEL', 'MARK', 'NOBR', 'OUTPUT',
  'PICTURE', 'Q', 'RP', 'RT', 'RUBY', 'S', 'SAMP', 'SMALL', 'SPAN', 'STRONG',
  'SUB', 'SUP', 'TIME', 'TT', 'U', 'VAR', 'WBR',
]);

/**
 * Separator pushed between two block elements. U+0000 cannot survive HTML
 * parsing (the parser rewrites it to U+FFFD) and `compileQuery` never emits
 * it, so no query can match across it. This is what stops
 * `<p>foo</p><p>bar</p>` matching "oob".
 */
const BLOCK_SEPARATOR = '\u0000';

/** Separator pushed for a `<br>`: a soft break reads as whitespace. */
const SOFT_SEPARATOR = '\n';

/** Kind of the previously pushed chunk, for separator coalescing. */
const PREV_TEXT = 0;
const PREV_SOFT = 1;
const PREV_BLOCK = 2;

/** Characters accepted between two query words. Mirrors `\s` plus NBSP. */
const SEPARATOR_SOURCE = '[ \\t\\r\\n\\f\\v\\u00a0]+';

/** Splits a query into words on any run of whitespace. */
const QUERY_SPLIT = /[\s\u00a0]+/;

/** Extra breathing room above and below the scrolled-to match, in pixels. */
const SCROLL_PADDING = 24;

/**
 * A subtree flattened into one searchable string.
 *
 * `text` is the concatenation of every accepted `Text` node's `data`, verbatim
 * — no normalization — plus synthetic separators. Because nothing is rewritten,
 * an offset into `text` maps back to a `(Text, offset)` pair exactly, with no
 * inverse mapping to get wrong.
 *
 * Chunk `i` covers `text[starts[i] .. starts[i+1])` and belongs to `nodes[i]`;
 * `nodes[i]` is `null` for a synthetic separator.
 */
export interface TextIndex {
  text: string;
  nodes: (Text | null)[];
  starts: Int32Array;
}

/** Offsets of every match found, plus the exact total (see {@link findMatchOffsets}). */
export interface MatchOffsets {
  /** Match start offsets into {@link TextIndex.text}, at most `cap` of them. */
  starts: Int32Array;
  /** Match end offsets (exclusive), parallel to {@link MatchOffsets.starts}. */
  ends: Int32Array;
  /** Exact match count, even when more than `cap` matches were found. */
  total: number;
}

/**
 * Blocked elements are rejected at ELEMENT level, which prunes the whole
 * subtree in O(1). The obvious alternative — a `SHOW_TEXT`-only walk plus
 * `parentElement.closest(BLOCKED_SELECTOR)` — is O(depth) *per text node*.
 */
const WALK_FILTER: NodeFilter = {
  acceptNode(node: Node): number {
    if (node.nodeType === Node.ELEMENT_NODE && (node as Element).matches(BLOCKED_SELECTOR)) {
      return NodeFilter.FILTER_REJECT;
    }
    return NodeFilter.FILTER_ACCEPT;
  },
};

/**
 * Flatten `root` into a {@link TextIndex}.
 *
 * Known limitation: separators are emitted when *entering* a block element, so
 * a bare text node that directly follows a block (`<p>a</p>b`) merges with it.
 * Rendered markdown never produces that shape — block elements are siblings of
 * block elements — and detecting subtree exit costs an O(depth) check per node.
 */
export function buildTextIndex(root: ParentNode): TextIndex {
  const parts: string[] = [];
  const nodes: (Text | null)[] = [];
  const offsets: number[] = [];
  let offset = 0;
  // Starting at PREV_BLOCK suppresses a pointless leading separator.
  let previous = PREV_BLOCK;

  const push = (chunk: string, node: Text | null): void => {
    parts.push(chunk);
    nodes.push(node);
    offsets.push(offset);
    offset += chunk.length;
  };

  const walker = document.createTreeWalker(
    root,
    NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT,
    WALK_FILTER,
  );

  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    if (node.nodeType === Node.TEXT_NODE) {
      const text = node as Text;
      // Empty text nodes would create zero-width chunks, which would give two
      // chunks the same start offset and break the binary search.
      if (text.data.length === 0) continue;
      push(text.data, text);
      previous = PREV_TEXT;
      continue;
    }

    const tag = (node as Element).tagName;
    if (tag === 'BR') {
      // A hard break reads as whitespace, so "foo<br>bar" still matches the
      // query "foo bar" — unlike a block boundary, which must not match.
      if (previous === PREV_TEXT) {
        push(SOFT_SEPARATOR, null);
        previous = PREV_SOFT;
      }
    } else if (!INLINE_TAGS.has(tag) && previous !== PREV_BLOCK) {
      push(BLOCK_SEPARATOR, null);
      previous = PREV_BLOCK;
    }
  }

  return { text: parts.join(''), nodes, starts: Int32Array.from(offsets) };
}

/**
 * Compile a user query into a global `RegExp`, or `null` when the query is
 * blank.
 *
 * Words are joined with a flexible whitespace separator rather than searching a
 * normalized copy of the text: markdown soft-wraps, so `<p>hello\nworld</p>` is
 * one text node containing a newline, and "hello world" has to find it. Doing it
 * in the pattern keeps `TextIndex.text` offsets 1:1 with `Text.data` — a
 * normalized buffer would need an inverse mapping, which is the single most
 * bug-prone part of any find implementation.
 *
 * Escaping is mandatory: the input is text a reader typed, not a pattern.
 */
export function compileQuery(query: string, caseSensitive: boolean): RegExp | null {
  const words = query.trim().split(QUERY_SPLIT).filter((word) => word.length > 0);
  if (words.length === 0) return null;
  return new RegExp(words.map(escapeRegex).join(SEPARATOR_SOURCE), caseSensitive ? 'g' : 'gi');
}

/**
 * Scan `index.text` for every match of `re`, storing at most `cap` offsets but
 * always counting them all.
 *
 * No `Range` is allocated here, which is why "N of M" stays exact on a document
 * with more matches than anyone would ever want painted.
 */
export function findMatchOffsets(index: TextIndex, re: RegExp, cap: number): MatchOffsets {
  const starts: number[] = [];
  const ends: number[] = [];
  let total = 0;

  re.lastIndex = 0;
  for (let match = re.exec(index.text); match !== null; match = re.exec(index.text)) {
    if (match[0].length === 0) {
      // A zero-length match leaves lastIndex where it was; without this nudge
      // the loop spins forever. `compileQuery` cannot produce one, but a
      // caller-supplied pattern can.
      re.lastIndex++;
      continue;
    }
    total++;
    if (starts.length < cap) {
      starts.push(match.index);
      ends.push(match.index + match[0].length);
    }
  }

  return { starts: Int32Array.from(starts), ends: Int32Array.from(ends), total };
}

/**
 * Turn a `[start, end)` offset pair into a live `Range`, or `null` if the pair
 * is out of bounds or lands on a synthetic separator.
 *
 * Matches that span several text nodes work natively — which is precisely why
 * the Custom Highlight API beats wrapping hits in `<mark>`: no DOM mutation, no
 * split text nodes, and none of the enhancers' cached node references are
 * invalidated.
 */
export function rangeForMatch(index: TextIndex, start: number, end: number): Range | null {
  if (start < 0 || end <= start || end > index.text.length) return null;

  const startChunk = chunkIndexAt(index.starts, start);
  const endChunk = chunkIndexAt(index.starts, end - 1);
  const startNode = index.nodes[startChunk];
  const endNode = index.nodes[endChunk];
  if (!startNode || !endNode) return null;

  const range = document.createRange();
  range.setStart(startNode, start - index.starts[startChunk]);
  range.setEnd(endNode, end - index.starts[endChunk]);
  return range;
}

/**
 * Scroll `range` into view, leaving `topInset` pixels clear at the top for the
 * find bar itself. No-ops when the range is already comfortably on screen.
 */
export function scrollRangeIntoView(range: Range, topInset: number): void {
  const rect = range.getBoundingClientRect();
  // A detached range — and every range under happy-dom — reports all zeros.
  if (rect.width === 0 && rect.height === 0) return;

  const top = topInset + SCROLL_PADDING;
  const bottom = window.innerHeight - SCROLL_PADDING;
  if (rect.top >= top && rect.bottom <= bottom) return;

  // 'auto', not 'smooth' (a deliberate divergence from mbr-keys' scrolling):
  // holding the find-next key queues smooth animations that fight each other.
  window.scrollTo({ top: Math.max(0, window.scrollY + rect.top - top), behavior: 'auto' });
}

/**
 * Index of the chunk containing `offset`: the last chunk starting at or before it.
 *
 * Exported for `review/anchor.ts`, which maps a selection to offsets and back
 * the other way. Duplicating this search there would mean two copies of the
 * same off-by-one-prone `(low + high + 1) >> 1` bias, with nothing forcing them
 * to agree about which chunk owns a boundary offset.
 */
export function chunkIndexAt(starts: Int32Array, offset: number): number {
  let low = 0;
  let high = starts.length - 1;
  while (low < high) {
    const mid = (low + high + 1) >> 1;
    if (starts[mid] <= offset) {
      low = mid;
    } else {
      high = mid - 1;
    }
  }
  return low;
}
