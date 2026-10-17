/**
 * Only the backslash escapes a paragraph needs.
 *
 * remark-stringify escapes by pattern: a `*` with spaces on both sides, a `[`
 * anywhere in text, a `&` before a letter — whatever *could* start a construct
 * somewhere. So `2 * 3` saves as `2 \* 3`, `see [a] here` as `see \[a] here`
 * and `AT&T` as `AT\&T`, though none of them is anything but text. Here the
 * printed paragraph (or heading, or term) is checked instead: each escape is
 * dropped in turn, the result re-parsed with the editor's own processor (every
 * syntax plugin included), and the drop kept only when the tree is unchanged.
 * Escapes the text needs (`\*not emphasis\*`, `\[[not a wikilink]]`, `\$5`)
 * change the tree when dropped and stay.
 *
 * Context the block cannot see on its own is supplied around it: the
 * document's link reference and footnote definitions (a `[a]` is a link when
 * `[a]: …` exists), and the container it sits in, as a `- ` or `> ` prefix on
 * its lines (`[ ] x` is a task first in a list item, `[!NOTE]` an alert first
 * in a quote). Adding a container is conservative: a paragraph that is not its
 * item's first is checked as if it were, which can only keep more escapes.
 *
 * Cost: one parse per escape, of one block, at save time — and only for blocks
 * with escapes. A block with more than {@link MAX_CANDIDATES} keeps them all.
 */
import { defaultHandlers, type Info, type State } from 'mdast-util-to-markdown';
import type { Parents, Root } from 'mdast';
import type { Ctx } from '@milkdown/kit/ctx';
import { remarkCtx } from '@milkdown/kit/core';
import { $remark } from '@milkdown/kit/utils';
import { addRemarkExtensions, LAZY_LINE, withoutLazyMarks, type MdNode } from './remark-support.js';

/** Beyond this many escapes in one block, leave the block as printed. */
export const MAX_CANDIDATES = 32;

/** ASCII punctuation: what a backslash can escape in CommonMark. */
const ESCAPABLE = /[!-/:-@[-`{-~]/;

/** A function from markdown to a comparable form of its tree. */
export type Canonical = (markdown: string) => string;

/** Escapes in `printed`: offsets of backslashes before ASCII punctuation. */
function escapesIn(printed: string): number[] {
  const offsets: number[] = [];
  for (let i = 0; i < printed.length - 1; i++) {
    if (printed[i] !== '\\') continue;
    if (ESCAPABLE.test(printed[i + 1])) offsets.push(i);
    i++; // `\\`: the second backslash is escaped, not escaping.
  }
  return offsets;
}

/** Above this many cells, {@link authoredEscapes} compares escaped characters instead of aligning. */
const MAX_ALIGNMENT_CELLS = 1_000_000;

/**
 * Which of `escapes` (offsets in `printed`) the author wrote: the backslashes
 * a longest common subsequence of `printed` and `authored` pairs with a
 * backslash in `authored`. `authored` may be stale (the block was edited);
 * the alignment only has to be good, not exact — a wrong pairing keeps an
 * escape that was safe to drop, never drops one that is needed.
 */
export function authoredEscapes(printed: string, authored: string, escapes: readonly number[]): Set<number> {
  const kept = new Set<number>();
  const n = printed.length;
  const m = authored.length;
  if (n * m > MAX_ALIGNMENT_CELLS) {
    for (const at of escapes) if (authored.includes(printed.slice(at, at + 2))) kept.add(at);
    return kept;
  }
  // lcs[i][j]: LCS length of printed[i..] and authored[j..].
  const width = m + 1;
  const lcs = new Uint32Array((n + 1) * width);
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i * width + j] =
        printed[i] === authored[j]
          ? lcs[(i + 1) * width + j + 1] + 1
          : Math.max(lcs[(i + 1) * width + j], lcs[i * width + j + 1]);
    }
  }
  const wanted = new Set(escapes);
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (printed[i] === authored[j]) {
      if (wanted.has(i)) kept.add(i);
      i++;
      j++;
    } else if (lcs[(i + 1) * width + j] >= lcs[i * width + j + 1]) {
      i++;
    } else {
      j++;
    }
  }
  return kept;
}

/**
 * `printed` without every backslash escape whose removal leaves
 * `canonical(wrap(text))` unchanged — except those the author wrote
 * (`authored`, the block's source when it was loaded), which stay. Right to
 * left, so earlier offsets stay valid; each removal is checked against the
 * original tree, so the result always parses as `printed` does.
 */
export function minimizeEscapes(
  printed: string,
  canonical: Canonical,
  wrap: (text: string) => string = (t) => t,
  authored?: string,
): string {
  const escapes = escapesIn(printed);
  const keep = authored === undefined ? new Set<number>() : authoredEscapes(printed, authored, escapes);
  const candidates = escapes.filter((at) => !keep.has(at));
  if (candidates.length === 0 || candidates.length > MAX_CANDIDATES) return printed;
  const reference = canonical(wrap(printed));
  let current = printed;
  for (let c = candidates.length - 1; c >= 0; c--) {
    const at = candidates[c];
    const attempt = current.slice(0, at) + current.slice(at + 1);
    if (canonical(wrap(attempt)) === reference) current = attempt;
  }
  return current;
}

/**
 * The tree as JSON, without positions (which an escape shifts) and recorded
 * source styles, and with inline content as ProseMirror holds it: runs of
 * text and inline nodes, each with its set of marks ({@link markRuns}). So
 * `[**x**](u)` and `**[x](u)**`, `**a**[**b**](u)` and `**a[b](u)**`, or
 * `**__x__**` and `**x**` read the same — they draw the same, and the editor prints one for the
 * other — while any edit a reader can see changes a run.
 */
function treeKey(tree: unknown): string {
  const replacer = (key: string, value: unknown): unknown => {
    // `mbrImageBlockPositions`: positions too (`images.ts`, on the root).
    if (key === 'position' || key === 'mbrStyle' || key === 'mbrImageBlockPositions') return undefined;
    const node = value as MdNode | null;
    if (!node || typeof node !== 'object' || Array.isArray(node) || !node.children?.some((child) => MARK_TYPES.has(child.type))) {
      return value;
    }
    return { ...node, children: markRuns(node.children) };
  };
  return JSON.stringify(tree, replacer);
}

/** Inline wrappers that are marks in the editor. */
const MARK_TYPES = new Set(['emphasis', 'strong', 'delete', 'link', 'linkReference', 'mbrLinkReference']);

/** A mark wrapper without its content or its spelling (`marker`: `*` or `_`), as a comparable label. */
function markLabel(wrapper: MdNode): string {
  return JSON.stringify({ ...wrapper, children: undefined, position: undefined, mbrStyle: undefined, marker: undefined });
}

/** Phrasing as runs: every leaf with the set of marks around it (sorted labels), adjacent texts with one set merged. */
function markRuns(children: readonly MdNode[], marks: readonly string[] = [], out: MdNode[] = []): MdNode[] {
  for (const child of children) {
    if (MARK_TYPES.has(child.type)) {
      markRuns(child.children ?? [], [...marks, markLabel(child)], out);
      continue;
    }
    // A set: `**__x__**` draws as `**x**`, and ProseMirror holds it so.
    const sorted = [...new Set(marks)].sort().join('|');
    const previous = out[out.length - 1] as (MdNode & { marks?: string }) | undefined;
    if (child.type === 'text' && previous?.type === 'text' && previous.marks === sorted) {
      out[out.length - 1] = { ...previous, value: (previous.value ?? '') + (child.value ?? '') } as MdNode;
    } else {
      out.push({ ...child, marks: sorted } as MdNode);
    }
  }
  return out;
}

/** Definitions (`[a]: …`, `[^a]: …`) of the document being printed, per serialization. */
const definitionsByState = new WeakMap<State, string>();

function collectDefinitions(tree: MdNode): string {
  const lines: string[] = [];
  const visit = (node: MdNode): void => {
    if (node.type === 'mbrDefinition' || node.type === 'definition') {
      const label = /^\[((?:[^\]\\]|\\.)+)\]:/.exec(String(node.value ?? ''))?.[1] ?? (node.label as string | undefined);
      if (label) lines.push(`[${label}]: /`);
    } else if (node.type === 'footnoteDefinition' && typeof node.label === 'string') {
      lines.push(`[^${node.label}]: x`);
    }
    for (const child of node.children ?? []) visit(child);
  };
  visit(tree);
  return lines.join('\n');
}

/** The containers around the block being printed, outermost first, as prefixes. */
function containerWrap(state: State): (text: string) => string {
  const prefixes: { first: string; rest: string }[] = [];
  for (const construct of state.stack) {
    if (construct === 'blockquote') prefixes.push({ first: '> ', rest: '> ' });
    else if (construct === 'listItem') prefixes.push({ first: '- ', rest: '  ' });
  }
  return (text) => {
    let out = text;
    for (const { first, rest } of [...prefixes].reverse()) {
      out = out
        .split('\n')
        // A lazy line (`authored-style.ts`) is written without the containers' prefixes.
        .map((line, i) => (line.startsWith(LAZY_LINE) ? line : (i === 0 ? first : rest) + line))
        .join('\n');
    }
    return withoutLazyMarks(out);
  };
}

interface MinimizeOptions {
  /** Set by {@link minimalEscapesPlugin}: the editor's parse, as a comparable key. */
  mbrCanonical?: Canonical;
}

/**
 * {@link minimizeEscapes} for a block printed in `state`, in its context;
 * `authored` is the block's source as loaded (`mbrStyle.source`), if known.
 * Unchanged when the plugin is not installed (a printer used on its own).
 */
export function minimizeBlockEscapes(printed: string, state: State, authored?: string): string {
  const canonical = (state.options as MinimizeOptions).mbrCanonical;
  if (!canonical) return printed;
  const definitions = definitionsByState.get(state) ?? '';
  const wrap = containerWrap(state);
  const suffix = definitions ? `\n\n${definitions}\n` : '\n';
  return minimizeEscapes(printed, canonical, (text) => wrap(text) + suffix, authored);
}

/**
 * Whether two spellings of the block being printed in `state` (a paragraph,
 * a heading) parse to the same tree in its context — the same wrap as
 * {@link minimizeBlockEscapes}. `false` without the plugin, unless equal.
 */
export function sameBlockMeaning(state: State, a: string, b: string): boolean {
  if (a === b) return true;
  const canonical = (state.options as MinimizeOptions).mbrCanonical;
  if (!canonical) return false;
  const definitions = definitionsByState.get(state) ?? '';
  const wrap = containerWrap(state);
  const suffix = definitions ? `\n\n${definitions}\n` : '\n';
  return canonical(wrap(a) + suffix) === canonical(wrap(b) + suffix);
}

/**
 * Whether two printings of a phrasing block read the same (`a\|b` and `a|b`
 * outside a table, `AT\&T` and `AT&T`). `false` without the plugin, unless
 * the strings are equal.
 */
export function samePhrasing(state: State, a: string, b: string): boolean {
  if (a === b) return true;
  const canonical = (state.options as MinimizeOptions).mbrCanonical;
  return canonical !== undefined && canonical(`${a}\n`) === canonical(`${b}\n`);
}

/** Records the document's definitions for {@link minimizeBlockEscapes}, then prints it. */
function printRoot(node: Root, parent: Parents | undefined, state: State, info: Info): string {
  definitionsByState.set(state, collectDefinitions(node as unknown as MdNode));
  // Lazy-line marks (`authored-style.ts`) have done their work in the containers.
  return withoutLazyMarks(defaultHandlers.root(node, parent, state, info));
}

/** The parts of a unified processor {@link canonicalOf} uses. */
export interface Parser {
  parse(markdown: string): unknown;
  runSync(tree: never, file: string): unknown;
}

/** `markdown` parsed (and transformed) by `remark`, as a comparable key: what it means, not how it is spelled. */
export function canonicalOf(remark: Parser): Canonical {
  return (markdown) => treeKey(remark.runSync(remark.parse(markdown) as never, markdown));
}

/** {@link canonicalOf} the editor's processor, looked up when called (it is built after the plugins). */
function canonicalFor(ctx: Ctx): Canonical {
  return (markdown) => canonicalOf(ctx.get(remarkCtx) as unknown as Parser)(markdown);
}

/**
 * Installs the check: the editor's parse as `mbrCanonical` (read when a block
 * is printed, by then the processor is complete) and the `root` handler that
 * collects definitions. Block printers call {@link minimizeBlockEscapes}:
 * `definition-list-syntax.ts`'s paragraph and term, `authored-style.ts`'s
 * heading.
 */
export const minimalEscapesPlugin = $remark('mbrMinimalEscapes', (ctx) => function minimalEscapes(this: unknown) {
  addRemarkExtensions(this, {
    toMarkdown: { mbrCanonical: canonicalFor(ctx), handlers: { root: printRoot } },
  });
});
