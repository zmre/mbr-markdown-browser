/**
 * The editor's safety net (TODO "Editor: cover mbr's markdown extensions",
 * item 7): a save rewrites only the top-level blocks the reader changed.
 *
 * Crepe saves by serializing the whole document, so anything the serializer
 * spells differently from the author — a construct no plugin covers yet, a
 * cosmetic choice the plugins do not keep — would be rewritten in every block
 * of the file on any save. Instead:
 *
 * - **At open** ({@link prepareBaseline}): the loaded body (`original`) and its
 *   serialization (`baseline`) are split into top-level blocks with the
 *   editor's own parser, and each baseline block is paired with the original
 *   block it came from — by text where they are equal (the parity harness
 *   keeps that the common case), by meaning (each block parsed alone) or by
 *   position between two such anchors otherwise.
 * - **At save** ({@link minimalDiffBody}): the current serialization is split
 *   the same way and matched to the baseline's blocks by text (a longest
 *   common subsequence). A block that still serializes exactly as it did at
 *   open was not edited, and is written as the original source; the others
 *   are written as serialized. Separators between two original blocks that
 *   were neighbours stay as they were.
 * - **Checked**: the result is split again, and must give back exactly the
 *   blocks it was assembled from (an original block can end in a way that
 *   swallows what follows it — an unclosed fence, a lazy line). If not, the
 *   whole serialization is saved instead, as before.
 *
 * {@link rewrittenLines} reports what the round trip changes, for the notice
 * the editor shows at open.
 */

/** The parts of an mdast root this module reads. */
export interface ParsedRoot {
  children: { position?: { start: { offset?: number }; end: { offset?: number } } }[];
}

/** A document cut at its top-level blocks: `leading`, then blocks with `gaps` between them, then `trailing`. */
export interface BlockSplit {
  leading: string;
  blocks: string[];
  /** `gaps[i]` lies between `blocks[i]` and `blocks[i + 1]`. */
  gaps: string[];
  trailing: string;
  /** 1-based first line of each block. */
  lines: number[];
}

/** Splits `markdown` at the top-level blocks `parse` finds; `null` without positions. */
export function splitBlocks(markdown: string, parse: (markdown: string) => ParsedRoot): BlockSplit | null {
  const ranges: [number, number][] = [];
  for (const child of parse(markdown).children) {
    const start = child.position?.start.offset;
    const end = child.position?.end.offset;
    if (start === undefined || end === undefined) return null;
    const previous = ranges[ranges.length - 1];
    if (previous && start < previous[1]) return null;
    ranges.push([start, end]);
  }
  if (ranges.length === 0) return { leading: markdown, blocks: [], gaps: [], trailing: '', lines: [] };
  const lineAt = lineCounter(markdown);
  return {
    leading: markdown.slice(0, ranges[0][0]),
    blocks: ranges.map(([start, end]) => markdown.slice(start, end)),
    gaps: ranges.slice(1).map(([start], i) => markdown.slice(ranges[i][1], start)),
    trailing: markdown.slice(ranges[ranges.length - 1][1]),
    lines: ranges.map(([start]) => lineAt(start)),
  };
}

/** Offsets to 1-based line numbers, for increasing offsets. */
function lineCounter(text: string): (offset: number) => number {
  let line = 1;
  let at = 0;
  return (offset) => {
    for (; at < offset; at++) if (text[at] === '\n') line++;
    return line;
  };
}

/**
 * Pairs of indices `[i, j]` with `a[i]` equal to `b[j]` (by `equal`), in
 * order, forming a longest common subsequence. A shared prefix and suffix are
 * matched first, so a small edit to a long document costs little.
 */
export function matchSequences<T>(a: readonly T[], b: readonly T[], equal: (x: T, y: T) => boolean = Object.is): [number, number][] {
  let head = 0;
  while (head < a.length && head < b.length && equal(a[head], b[head])) head++;
  let tail = 0;
  while (tail < a.length - head && tail < b.length - head && equal(a[a.length - 1 - tail], b[b.length - 1 - tail])) tail++;
  const pairs: [number, number][] = [];
  for (let i = 0; i < head; i++) pairs.push([i, i]);
  const n = a.length - head - tail;
  const m = b.length - head - tail;
  if (n > 0 && m > 0) {
    const width = m + 1;
    const lcs = new Uint32Array((n + 1) * width);
    for (let i = n - 1; i >= 0; i--) {
      for (let j = m - 1; j >= 0; j--) {
        lcs[i * width + j] = equal(a[head + i], b[head + j])
          ? lcs[(i + 1) * width + j + 1] + 1
          : Math.max(lcs[(i + 1) * width + j], lcs[i * width + j + 1]);
      }
    }
    let i = 0;
    let j = 0;
    while (i < n && j < m) {
      if (equal(a[head + i], b[head + j])) {
        pairs.push([head + i, head + j]);
        i++;
        j++;
      } else if (lcs[(i + 1) * width + j] >= lcs[i * width + j + 1]) {
        i++;
      } else {
        j++;
      }
    }
  }
  for (let k = tail; k > 0; k--) pairs.push([a.length - k, b.length - k]);
  return pairs;
}

/** What the editor knew about the body when it opened. */
export interface SaveBaseline {
  original: BlockSplit;
  baseline: BlockSplit;
  /** `toOriginal[i]`: the original block baseline block `i` came from, if known. */
  toOriginal: (number | undefined)[];
}

/**
 * Pairs `baseline`'s blocks with `original`'s (see the module docs).
 * `canonical` maps one block's markdown to a comparable form of its tree.
 * `null` when either cannot be split.
 */
export function prepareBaseline(
  original: string,
  baseline: string,
  parse: (markdown: string) => ParsedRoot,
  canonical: (markdown: string) => string,
): SaveBaseline | null {
  const o = splitBlocks(original, parse);
  const b = splitBlocks(baseline, parse);
  if (!o || !b) return null;
  const toOriginal: (number | undefined)[] = new Array(b.blocks.length).fill(undefined);
  const anchors = matchSequences(o.blocks, b.blocks);
  // Between two anchors (and at the ends): blocks the serializer changed.
  const bounds: [number, number][] = [[-1, -1], ...anchors, [o.blocks.length, b.blocks.length]];
  for (let k = 0; k < bounds.length - 1; k++) {
    const [oi, bi] = bounds[k];
    if (k > 0) toOriginal[bi] = oi;
    const [oNext, bNext] = bounds[k + 1];
    const oGap = range(oi + 1, oNext);
    const bGap = range(bi + 1, bNext);
    if (oGap.length === 0 || bGap.length === 0) continue;
    if (oGap.length === bGap.length) {
      oGap.forEach((oIndex, n) => (toOriginal[bGap[n]] = oIndex));
      continue;
    }
    const oKeys = oGap.map((i) => canonical(o.blocks[i]));
    const bKeys = bGap.map((i) => canonical(b.blocks[i]));
    for (const [x, y] of matchSequences(oKeys, bKeys)) toOriginal[bGap[y]] = oGap[x];
  }
  return { original: o, baseline: b, toOriginal };
}

function range(from: number, to: number): number[] {
  return Array.from({ length: Math.max(0, to - from) }, (_, i) => from + i);
}

/** The body to save: `current` with every unedited block written as it was loaded. */
export function minimalDiffBody(base: SaveBaseline, current: string, parse: (markdown: string) => ParsedRoot): string {
  return minimalDiff(base, current, parse) ?? current;
}

/** {@link minimalDiffBody}, `null` when it falls back to `current`. */
export function minimalDiff(base: SaveBaseline, current: string, parse: (markdown: string) => ParsedRoot): string | null {
  const c = splitBlocks(current, parse);
  if (!c || c.blocks.length === 0) return null;
  const { original: o, baseline: b, toOriginal } = base;
  const origin: (number | undefined)[] = new Array(c.blocks.length).fill(undefined);
  for (const [bi, ci] of matchSequences(b.blocks, c.blocks)) origin[ci] = toOriginal[bi];
  const pieces = c.blocks.map((block, j) => (origin[j] === undefined ? block : o.blocks[origin[j] as number]));
  const first = origin[0];
  const last = origin[origin.length - 1];
  let out = first === 0 ? o.leading : c.leading;
  pieces.forEach((piece, j) => {
    if (j > 0) {
      const before = origin[j - 1];
      const here = origin[j];
      out += before !== undefined && here === before + 1 ? o.gaps[before] : c.gaps[j - 1];
    }
    out += piece;
  });
  out += last === o.blocks.length - 1 ? o.trailing : c.trailing;
  const check = splitBlocks(out, parse);
  if (!check || check.blocks.length !== pieces.length || check.blocks.some((block, j) => block !== pieces[j])) return null;
  return out;
}

/** 1-based inclusive line ranges of the original body. */
export interface LineRange {
  first: number;
  last: number;
}

/**
 * What the round trip changes, as line ranges of the original body:
 * `ifEdited` — blocks the editor prints differently, saved as written unless
 * edited; `onSave` — blocks it could not pair at all, which any save
 * rewrites.
 */
export function rewrittenLines(base: SaveBaseline): { ifEdited: LineRange[]; onSave: LineRange[] } {
  const { original: o, baseline: b, toOriginal } = base;
  const paired = new Map<number, number>();
  toOriginal.forEach((oi, bi) => {
    if (oi !== undefined) paired.set(oi, bi);
  });
  const ifEdited: LineRange[] = [];
  const onSave: LineRange[] = [];
  o.blocks.forEach((block, oi) => {
    const bi = paired.get(oi);
    if (bi !== undefined && b.blocks[bi] === block) return;
    const first = o.lines[oi];
    const range = { first, last: first + block.split('\n').length - 1 };
    (bi === undefined ? onSave : ifEdited).push(range);
  });
  return { ifEdited, onSave };
}

/** `3`, `3–5`, `3–5, 9`: line ranges for a message. */
export function formatLineRanges(ranges: readonly LineRange[], offset = 0): string {
  return ranges
    .map(({ first, last }) => (first === last ? `${first + offset}` : `${first + offset}–${last + offset}`))
    .join(', ');
}
