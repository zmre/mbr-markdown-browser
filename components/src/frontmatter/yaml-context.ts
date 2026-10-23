/**
 * Where a cursor is in YAML source, as completion needs to know it: the path
 * of the map whose key is being typed, or the path of the value being typed,
 * plus the span a completion replaces.
 *
 * Pure: a source string and its Lezer YAML tree in, a plain record out (no
 * editor state, no DOM), so it is tested on cursor positions directly.
 *
 * The tree (`@lezer/yaml`, what `syntaxTree(state)` holds) gives the
 * structure: which flow collection the cursor is inside, which block
 * collections are open at the previous line, the keys and indexes above them.
 * The cursor's own line is read as text, because that is the line being typed
 * and so usually the one that does not parse yet (a bare `sta` under a map is
 * an error node; a blank indented line belongs to no node at all). A line
 * joins the innermost collection open at the end of the previous content
 * line whose items start in its column, or, failing that, becomes the first
 * item of a key left empty on the line above (`generated:` then `  by`).
 */

import type { SyntaxNode, Tree } from '@lezer/common';
import type { FmPath } from './model.js';

/**
 * - `key`: a key in the map at `path`;
 * - `value`: the value at `path`;
 * - `item`: a block sequence item at `path` (`- te`), which may be a scalar
 *   or the first key of a map item, so both kinds of suggestion apply.
 */
export type YamlRole = 'key' | 'value' | 'item';

/** An unclosed `[[` before the cursor, inside a value. */
export interface WikilinkContext {
  /** Where `[[` starts (or the quote a completion adds goes). */
  readonly from: number;
  /** End of the span a completion replaces (past `]]` when it is already there). */
  readonly to: number;
  /** Text typed after `[[`. */
  readonly query: string;
  /** The quote the link is inside (its own or the one `open` adds): escape for it. */
  readonly quote: '"' | "'" | null;
  /**
   * Written around `[[Title]]`: a link typed at the start of an unquoted
   * value is quoted (`"[[Title]]"`), since unquoted it is a nested list;
   * one inside an open quote closes it when the line has no closing quote.
   */
  readonly open: string;
  readonly close: string;
}

export interface YamlContext {
  readonly role: YamlRole;
  readonly path: FmPath;
  /** Start of the typed text (inside the quotes for a quoted value). */
  readonly from: number;
  /** End of the span a completion replaces (the rest of the key or value). */
  readonly to: number;
  /** The text from `from` to the cursor. */
  readonly prefix: string;
  /** The value's opening quote, when it has one. */
  readonly quote: '"' | "'" | null;
  /** Inside a flow collection (`[…]`/`{…}`): unquoted text cannot hold `,[]{}`. */
  readonly flow: boolean;
  /** For `key`: a `:` already follows on the line, so only the key is replaced. */
  readonly hasColon: boolean;
  /** Column of the key or item being typed (for inserting nested lines). */
  readonly column: number;
  readonly wikilink: WikilinkContext | null;
}

// ============================================================================
// Tree helpers
// ============================================================================

const SEQUENCES = new Set(['BlockSequence', 'FlowSequence']);
const BLOCK_COLLECTIONS = new Set(['BlockMapping', 'BlockSequence']);

function children(node: SyntaxNode): SyntaxNode[] {
  const out: SyntaxNode[] = [];
  for (let c = node.firstChild; c; c = c.nextSibling) out.push(c);
  return out;
}

function lineStartOf(text: string, pos: number): number {
  return text.lastIndexOf('\n', pos - 1) + 1;
}

function columnOf(text: string, pos: number): number {
  return pos - lineStartOf(text, pos);
}

/** A key's text, unquoted. */
export function keyText(raw: string): string {
  const t = raw.trim();
  if (t.startsWith('"')) {
    try {
      return JSON.parse(t.endsWith('"') && t.length > 1 ? t : `${t}"`) as string;
    } catch {
      return t.slice(1, t.endsWith('"') ? -1 : undefined);
    }
  }
  if (t.startsWith("'")) return t.slice(1, t.endsWith("'") && t.length > 1 ? -1 : undefined).replace(/''/g, "'");
  return t;
}

/** The index of a sequence `Item` among its siblings. */
function itemIndex(item: SyntaxNode): number {
  let i = 0;
  for (let s = item.prevSibling; s; s = s.prevSibling) if (s.name === 'Item') i++;
  return i;
}

/** A FlowMapping that error recovery (or `[a: b]`) made without `{`. */
function isImplicitFlowMap(node: SyntaxNode): boolean {
  return node.name === 'FlowMapping' && node.firstChild?.name !== '{';
}

/**
 * The path of the value `node` is (or is inside): every enclosing `Pair`
 * contributes its key, every sequence `Item` its index. `null` when a pair
 * on the way has no key.
 */
export function pathOf(text: string, node: SyntaxNode): (string | number)[] | null {
  const path: (string | number)[] = [];
  let from: SyntaxNode | null = null;
  for (let n: SyntaxNode | null = node; n; from = n, n = n.parent) {
    if (n.name === 'Pair') {
      // Inside the pair's key (error recovery nests `{t` in one), not its value.
      if (from?.name === 'Key') continue;
      const key = n.getChild('Key');
      if (!key) return null;
      path.unshift(keyText(text.slice(key.from, key.to)));
    } else if (n.name === 'Item' && n.parent && SEQUENCES.has(n.parent.name)) {
      path.unshift(itemIndex(n));
    }
  }
  return path;
}

/** Where a block collection's items start: its first pair's key, or its first `-`. */
function blockColumn(text: string, coll: SyntaxNode): number | null {
  const first = children(coll).find((c) => (coll.name === 'BlockMapping' ? c.name === 'Pair' : c.name === '-'));
  return first ? columnOf(text, first.from) : null;
}

/**
 * Whether a pair's value is still to come at the line starting `lineStart`:
 * nothing after its `:`, or only a plain scalar that starts on that line
 * (`generated:\n  a` parses as `generated: "a"` while a key is typed).
 */
function pairOpensAt(pair: SyntaxNode, lineStart: number): boolean {
  const kids = children(pair);
  const colon = kids.findIndex((c) => c.name === ':');
  if (colon < 0) return false;
  const value = kids[colon + 1];
  return !value || (value.name === 'Literal' && value.from >= lineStart && kids.length === colon + 2);
}

// ============================================================================
// Line text helpers
// ============================================================================

/** Index of the first `:` that ends a key in `s` (followed by a space or the end), outside quotes and `[[`. */
function keyColon(s: string): number {
  let quote: string | null = null;
  for (let i = 0; i < s.length; i++) {
    const ch = s[i];
    if (quote) {
      if (ch === quote) quote = null;
      continue;
    }
    if ((ch === '"' || ch === "'") && (i === 0 || /[\s[{,]/.test(s[i - 1]))) quote = ch;
    else if (ch === '[' && s[i + 1] === '[') return -1;
    else if (ch === ':' && (i + 1 === s.length || s[i + 1] === ' ' || s[i + 1] === '\t')) return i;
    else if (ch === '#' && (i === 0 || /\s/.test(s[i - 1]))) return -1;
  }
  return -1;
}

/** Whether the line text before the cursor ends inside a comment. */
function inComment(before: string): boolean {
  let quote: string | null = null;
  for (let i = 0; i < before.length; i++) {
    const ch = before[i];
    if (quote) {
      if (ch === quote) quote = null;
    } else if (ch === '"' || ch === "'") {
      if (i === 0 || /[\s[{,:]/.test(before[i - 1])) quote = ch;
    } else if (ch === '#' && (i === 0 || /\s/.test(before[i - 1]))) {
      return true;
    }
  }
  return false;
}

/** Index of the quote closing a scalar opened by `q`, in `s` (after the opening quote), or -1. */
function closingQuote(s: string, q: '"' | "'"): number {
  for (let i = 0; i < s.length; i++) {
    if (q === '"' && s[i] === '\\') i++;
    else if (s[i] === q) {
      if (q === "'" && s[i + 1] === "'") i++;
      else return i;
    }
  }
  return -1;
}

/** The end of the scalar text that starts at the cursor (`after` is the rest of the line). */
function valueEnd(pos: number, after: string, quote: '"' | "'" | null, flow: boolean): number {
  if (quote) {
    const close = closingQuote(after, quote);
    if (close >= 0) return pos + close;
  }
  const stop = flow ? after.search(/[,\]}]|\s#/) : after.search(/\s#/);
  const rest = stop < 0 ? after : after.slice(0, stop);
  return pos + rest.trimEnd().length;
}

// ============================================================================
// The context
// ============================================================================

interface Core {
  readonly role: YamlRole;
  readonly path: (string | number)[];
  /** Where the value or key text starts (before any quote). */
  readonly textFrom: number;
  readonly flow: boolean;
  readonly column: number;
}

/**
 * The completion context at `pos` in `text`, whose YAML tree is `tree`;
 * `null` where nothing should be suggested (a comment, a block scalar's
 * content, right after `:`, an indentation no collection matches).
 */
export function yamlContextAt(text: string, tree: Tree, pos: number): YamlContext | null {
  const lineStart = lineStartOf(text, pos);
  const nl = text.indexOf('\n', pos);
  const lineEnd = nl < 0 ? text.length : nl;
  const before = text.slice(lineStart, pos);
  if (inComment(before)) return null;
  for (let n: SyntaxNode | null = tree.resolveInner(pos, -1); n; n = n.parent) {
    if (n.name === 'BlockLiteralContent' || (n.name === 'BlockLiteral' && pos > n.from)) return null;
  }

  const after = text.slice(pos, lineEnd);
  const wiki = wikilinkAt(text, tree, pos, lineStart, after);
  if (wiki) return wiki;

  const core = coreAt(text, tree, pos);
  if (!core) return null;
  const raw = text.slice(core.textFrom, pos);
  if (core.role === 'key') {
    const tail = after.match(/^[^\s:#,\]}]*/)![0];
    const hasColon = /^[^#\n]*?:(\s|$)/.test(after);
    return {
      role: 'key',
      path: core.path,
      from: core.textFrom,
      to: pos + tail.length,
      prefix: raw,
      quote: null,
      flow: core.flow,
      hasColon,
      column: core.column,
      wikilink: null,
    };
  }
  // A value: anchors, tags, aliases, block scalar headers and the space after
  // a flow collection are not completed (inside one is `flowCore`'s).
  if (/^[&*!|>[{]/.test(raw)) return null;
  const quote = raw[0] === '"' || raw[0] === "'" ? (raw[0] as '"' | "'") : null;
  const from = quote ? core.textFrom + 1 : core.textFrom;
  if (quote && closingQuote(text.slice(from, pos), quote) >= 0) return null; // past the closing quote
  return {
    role: core.role,
    path: core.path,
    from,
    to: valueEnd(pos, after, quote, core.flow),
    prefix: text.slice(from, pos),
    quote,
    flow: core.flow,
    hasColon: false,
    column: core.column,
    wikilink: null,
  };
}

/**
 * An unclosed `[[` on the line before the cursor, in a value. The structure
 * comes from where the `[[` starts, because unquoted `[[Al` is YAML for a
 * nested list and the tree at the cursor is inside it.
 */
function wikilinkAt(text: string, tree: Tree, pos: number, lineStart: number, after: string): YamlContext | null {
  const before = text.slice(lineStart, pos);
  const at = before.lastIndexOf('[[');
  if (at < 0 || before.indexOf(']]', at) >= 0) return null;
  const start = lineStart + at;
  const outer = coreAt(text, tree, start);
  if (!outer || outer.role === 'key') return null;
  const lead = text.slice(outer.textFrom, start);
  const opened = lead[0] === '"' || lead[0] === "'" ? (lead[0] as '"' | "'") : null;
  if (opened && closingQuote(lead.slice(1), opened) >= 0) return null; // after the closing quote
  // A name has no flow punctuation unless quoted: `[[1, 2], [3` is a nested list.
  const query = text.slice(start + 2, pos);
  if (!(opened ? /^[^[\]\n]*$/ : /^[^[\]{},\n]*$/).test(query)) return null;
  // Mid-way through an unquoted flow value a `[` cannot be written bare.
  if (!opened && lead !== '' && outer.flow) return null;
  const close = after.indexOf(']]');
  const to = close >= 0 && !/["',\n]/.test(after.slice(0, close)) ? pos + close + 2 : pos;
  const quote = opened ?? (lead === '' ? '"' : null);
  const wikilink: WikilinkContext = {
    from: start,
    to,
    query,
    quote,
    open: !opened && lead === '' ? '"' : '',
    close: opened ? (text[to] === opened ? '' : opened) : lead === '' ? '"' : '',
  };
  return {
    role: outer.role,
    path: outer.path,
    from: start,
    to,
    prefix: text.slice(start, pos),
    quote: opened,
    flow: outer.flow,
    hasColon: false,
    column: outer.column,
    wikilink,
  };
}

/** Role, path and text start at `pos`, from the flow collection around it or the block structure. */
function coreAt(text: string, tree: Tree, pos: number): Core | null {
  return flowCore(text, tree, pos) ?? blockCore(text, tree, pos);
}

/** Inside `[…]` or `{…}`: the innermost real flow collection holding `pos`. */
function flowCore(text: string, tree: Tree, pos: number): Core | null {
  let flow: SyntaxNode | null = null;
  for (let n: SyntaxNode | null = tree.resolveInner(pos, -1); n; n = n.parent) {
    if ((n.name === 'FlowSequence' || n.name === 'FlowMapping') && !isImplicitFlowMap(n)) {
      const open = n.firstChild;
      const close = n.lastChild;
      const closed = close && close !== open && (close.name === ']' || close.name === '}');
      if (open && pos >= open.to && (!closed || pos <= close.from)) {
        flow = n;
        break;
      }
    }
  }
  if (!flow) return null;
  const base = pathOf(text, flow);
  if (!base) return null;
  // The current entry starts after the last `[`, `{` or `,` of this collection before the cursor.
  let segStart = flow.firstChild!.to;
  let index = 0;
  for (const c of children(flow)) {
    if (c.to > pos) break;
    if (c.name === ',') {
      segStart = c.to;
      index++;
    }
  }
  const seg = text.slice(segStart, pos);
  const lead = seg.length - seg.trimStart().length;
  const entryFrom = segStart + lead;
  const column = columnOf(text, entryFrom);
  if (flow.name === 'FlowSequence') {
    // `[a: b]` is a one-pair map item; its key is not completed.
    if (keyColon(seg.trimStart()) >= 0) return null;
    return { role: 'value', path: [...base, index], textFrom: entryFrom, flow: true, column };
  }
  const entry = seg.trimStart();
  const colon = keyColon(entry);
  if (colon < 0) return { role: 'key', path: base, textFrom: entryFrom, flow: true, column };
  const key = keyText(entry.slice(0, colon));
  const valueText = entry.slice(colon + 1);
  if (valueText === '') return null;
  const valueFrom = entryFrom + colon + 1 + (valueText.length - valueText.trimStart().length);
  return { role: 'value', path: [...base, key], textFrom: valueFrom, flow: true, column };
}

/** The end of the last line before `lineStart` with content (not blank, not only a comment). */
function previousContentEnd(text: string, lineStart: number): number | null {
  let end = lineStart - 1;
  while (end > 0) {
    const start = lineStartOf(text, end);
    const line = text.slice(start, end);
    if (line.trim() !== '' && !/^\s*#/.test(line)) return start + line.trimEnd().length;
    end = start - 1;
  }
  return null;
}

/**
 * The block collection a line whose first token is at `column` continues:
 * the innermost one open at the end of the previous content line whose items
 * start in that column (a sequence for a `-` line, a map otherwise), or the
 * value of a key left empty above it. `null` when none fits.
 */
function blockParent(
  text: string,
  tree: Tree,
  lineStart: number,
  column: number,
  dash: boolean,
): { path: (string | number)[]; seq: SyntaxNode | null } | null {
  const prevEnd = previousContentEnd(text, lineStart);
  if (prevEnd === null) return column === 0 ? { path: [], seq: dash ? rootSequence(tree) : null } : null;
  // The collections open at the end of the previous line, and (when the
  // line already parses) those around its own first token: a key's value
  // collection starts after the key's `:`, so it is no ancestor of it.
  const ancestors: SyntaxNode[] = [];
  const first = lineStart + column;
  const own = first < text.length && text[first] !== '\n' ? tree.resolveInner(first, 1) : null;
  for (const start of [own, tree.resolveInner(prevEnd, -1)]) {
    for (let n: SyntaxNode | null = start; n; n = n.parent) if (!ancestors.includes(n)) ancestors.push(n);
  }
  for (const n of ancestors) {
    if (!BLOCK_COLLECTIONS.has(n.name) || blockColumn(text, n) !== column) continue;
    if ((n.name === 'BlockSequence') !== dash) {
      // A `- ` line in a map's column: a sequence written flush under a key (`tags:\n- a`).
      if (dash && n.name === 'BlockMapping') break;
      return null;
    }
    const path = pathOf(text, n);
    return path ? { path, seq: dash ? n : null } : null;
  }
  // The first item under a key left empty on a line above.
  for (const n of ancestors) {
    if (n.name !== 'Pair' || n.parent?.name !== 'BlockMapping' || !pairOpensAt(n, lineStart)) continue;
    const keyCol = columnOf(text, n.from);
    if (column > keyCol || (dash && column === keyCol)) {
      const path = pathOf(text, n);
      return path ? { path, seq: null } : null;
    }
    return null;
  }
  return null;
}

function rootSequence(tree: Tree): SyntaxNode | null {
  const doc = tree.topNode.getChild('Document');
  const seq = doc?.getChild('BlockSequence');
  return seq ?? null;
}

/** Block context from the cursor's line: its indentation, `- ` markers and a `key:`. */
function blockCore(text: string, tree: Tree, pos: number): Core | null {
  const lineStart = lineStartOf(text, pos);
  const before = text.slice(lineStart, pos);
  const lead = /^([ \t]*)((?:-[ \t]+)*)/.exec(before)!;
  const indent = lead[1].length;
  const dashes = lead[2] ? lead[2].match(/-[ \t]+/g)!.length : 0;
  const restFrom = lineStart + lead[0].length;
  const rest = text.slice(restFrom, pos);
  if (/^-$/.test(rest)) return null; // a `-` not yet followed by a space

  const parent = blockParent(text, tree, lineStart, indent, dashes > 0);
  if (!parent) return null;
  const path = [...parent.path];
  if (dashes > 0) {
    const dashPos = lineStart + indent;
    const index = parent.seq ? children(parent.seq).filter((c) => c.name === '-' && c.from < dashPos).length : 0;
    path.push(index);
    for (let i = 1; i < dashes; i++) path.push(0);
  }
  const column = restFrom - lineStart;
  const colon = keyColon(rest);
  if (colon >= 0) {
    const valueText = rest.slice(colon + 1);
    if (valueText === '') return null; // right after `:`
    path.push(keyText(rest.slice(0, colon)));
    const valueFrom = restFrom + colon + 1 + (valueText.length - valueText.trimStart().length);
    return { role: 'value', path, textFrom: valueFrom, flow: false, column };
  }
  if (dashes > 0) return { role: 'item', path, textFrom: restFrom, flow: false, column };
  return { role: 'key', path, textFrom: restFrom, flow: false, column };
}
