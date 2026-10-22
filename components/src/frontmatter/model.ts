/**
 * The frontmatter model: one parsed `yaml.Document` plus the exact text it
 * was parsed from, and pure edit operations that patch that text in place.
 *
 * The form and the YAML tab are two views of this model. A save writes
 * `model.text`, so the bytes it contains are the whole contract:
 *
 * - **Surgical.** Every operation splices the source text at the edited
 *   node's own range (`node.range`, plus the CST indicator tokens kept by
 *   `keepSourceTokens`). Bytes outside that span never change: comments, key
 *   order, quoting, flow style and blank lines elsewhere survive by
 *   construction.
 * - **Verified.** After each splice the text is re-parsed and its JS value is
 *   compared with what the operation means (the same operation applied to the
 *   old JS value). On any mismatch, or a parse error, the operation is redone
 *   through the `yaml` document API and the whole block is re-serialized
 *   (`String(doc)`), and the result is flagged {@link FrontmatterModel.normalized}
 *   so the editor can say so. The fallback is verified too; failing that, the
 *   operation throws and the model is unchanged.
 * - **Never over broken YAML.** A model with parse errors refuses every
 *   operation ({@link FrontmatterEditError} `parse-errors`): only the YAML tab
 *   may edit text that did not parse.
 *
 * Scalars are parsed with yaml's default (YAML 1.2 core) schema, which agrees
 * with the renderer's `yaml_rust2` on what a plain scalar means: `2026-10-21`
 * is a string, `yes` is a string, `1e3` is a number.
 */

import {
  isAlias,
  isMap,
  isPair,
  isScalar,
  isSeq,
  parseDocument,
  stringify,
  visit,
  type Document,
  type Node as YamlNode,
  type Pair,
  type Scalar,
  type YAMLMap,
  type YAMLSeq,
} from 'yaml';

// ============================================================================
// Types
// ============================================================================

/** A frontmatter scalar as the editor sees it (YAML 1.2 core schema). */
export type FmScalar = string | number | boolean | null;
/** Any frontmatter value: JSON-shaped. */
export type FmValue = FmScalar | FmValue[] | { [key: string]: FmValue };
/** A path into the frontmatter: map keys are strings, sequence indexes numbers. */
export type FmPath = readonly (string | number)[];

/** A parse error, 1-based line and column within the frontmatter text. */
export interface FrontmatterError {
  readonly code: string;
  readonly message: string;
  readonly line: number;
  readonly col: number;
}

/** One splice of the text: `old.slice(0, start) + text + old.slice(end)`. */
export interface TextEdit {
  readonly start: number;
  readonly end: number;
  readonly text: string;
}

export interface FrontmatterModel {
  /** The YAML text (no fences), exactly what a save writes. */
  readonly text: string;
  /** `text` parsed with `keepSourceTokens`. Treat as read-only. */
  readonly doc: Document.Parsed;
  /** Parse errors; when non-empty every edit operation refuses. */
  readonly errors: readonly FrontmatterError[];
  /**
   * Sticky: some operation since open could not be applied surgically and the
   * whole block was re-serialized by the `yaml` package (formatting outside
   * the edited value may have changed).
   */
  readonly normalized: boolean;
  /** The splice the last operation made, `null` on open or after a normalizing fallback. */
  readonly lastEdit: TextEdit | null;
  /**
   * Scalars (by `JSON.stringify(path)`) authored plain, or empty, that a
   * {@link setScalar} had to double-quote (`"hello "` while a space is
   * typed). They go back to plain once the value no longer needs quotes; a
   * scalar the author quoted is never in here, so it stays quoted.
   */
  readonly forcedQuotes: ReadonlySet<string>;
}

export type FrontmatterEditErrorReason = 'parse-errors' | 'not-found' | 'wrong-kind' | 'exists' | 'invalid';

/** Why an operation refused. The model passed in is unchanged. */
export class FrontmatterEditError extends Error {
  readonly reason: FrontmatterEditErrorReason;
  constructor(reason: FrontmatterEditErrorReason, message: string) {
    super(message);
    this.name = 'FrontmatterEditError';
    this.reason = reason;
  }
}

export type NodeKind = 'scalar' | 'map' | 'seq' | 'alias' | 'missing';

/** What the form needs to know about the value at a path. */
export interface NodeInfo {
  readonly kind: NodeKind;
  /** A flow collection (`[a, b]`, `{a: 1}`). */
  readonly flow: boolean;
  /** The scalar's source style: `PLAIN`, `QUOTE_SINGLE`, `QUOTE_DOUBLE`, `BLOCK_LITERAL`, `BLOCK_FOLDED`. */
  readonly scalarStyle?: Scalar.Type;
  /** An explicit tag on this node (`!!str 12`, `!custom x`). */
  readonly tag?: string;
  /** An anchor on this node (`&x`). */
  readonly anchor?: string;
  /** Any comment on or inside this value (trailing, before or between items). */
  readonly hasComments: boolean;
  /** Any anchor, alias, explicit tag or merge key (`<<`) on or inside this value. */
  readonly hasYamlFeatures: boolean;
  /**
   * The form cannot edit this value in place without losing something the
   * author wrote, so it should show a preview and "Edit as YAML": an alias,
   * anchor, tag or merge key anywhere in it, or a collection holding a
   * comment the operations cannot keep in its place (see
   * {@link keepsComment}). A trailing comment on the same line as a scalar
   * (`mobile: "+1 555"  # cell`), or after a flow collection, does not count:
   * {@link setScalar} keeps it, and adding, removing or renaming a sibling
   * never touches its line.
   */
  readonly needsYaml: boolean;
}

/** One child of a collection, for the form's rows. */
export interface ModelEntry {
  /** The map key, or the sequence index. */
  readonly key: string | number;
  readonly path: FmPath;
  readonly value: FmValue;
  readonly info: NodeInfo;
}

// ============================================================================
// Opening and reading
// ============================================================================

function parse(text: string): Document.Parsed {
  return parseDocument(text, { keepSourceTokens: true, prettyErrors: true });
}

function errorsOf(doc: Document.Parsed, text: string): FrontmatterError[] {
  const errors = doc.errors.map((e) => ({
    code: e.code,
    message: e.message.split('\n')[0],
    line: e.linePos?.[0].line ?? 1,
    col: e.linePos?.[0].col ?? 1,
  }));
  if (errors.length > 0) return errors;
  // Not parse errors to `yaml`, but the value cannot be read (`toJS` throws): as broken as one.
  const alias = unresolvedAlias(doc);
  if (alias) return [errorAt(text, alias.range?.[0] ?? 0, 'BAD_ALIAS', `The alias *${alias.source} names no anchor before it`)];
  const read = readJs(doc);
  if ('error' in read) return [errorAt(text, firstAliasAt(doc), 'BAD_VALUE', read.error.message)];
  return [];
}

function errorAt(text: string, at: number, code: string, message: string): FrontmatterError {
  const lineStartAt = text.lastIndexOf('\n', at - 1) + 1;
  return { code, message, line: text.slice(0, at).split('\n').length, col: at - lineStartAt + 1 };
}

/** Where the first alias is (`0` when there is none): where an unreadable value most likely goes wrong. */
function firstAliasAt(doc: Document): number {
  let at = 0;
  visit(doc, {
    Alias(_key, node) {
      at = node.range?.[0] ?? 0;
      return visit.BREAK;
    },
  });
  return at;
}

type JsRead = { readonly value: FmValue } | { readonly error: Error };

/**
 * `doc.toJS()`, once per document: a model and its document never change, and
 * every row, hint and render reads the value. The value is frozen, since it is
 * shared. `toJS` can throw where `yaml` reported no error (aliases nested past
 * `maxAliasCount`, the "billion laughs"); that is cached too, and
 * {@link errorsOf} reports it, so such a model is read-only like a broken one.
 */
const jsCache = new WeakMap<Document, JsRead>();

function readJs(doc: Document): JsRead {
  let read = jsCache.get(doc);
  if (!read) {
    try {
      read = { value: deepFreeze((doc.toJS() as FmValue | undefined) ?? {}) };
    } catch (err) {
      read = { error: err instanceof Error ? err : new Error(String(err)) };
    }
    jsCache.set(doc, read);
  }
  return read;
}

function deepFreeze<T extends FmValue>(v: T): T {
  if (v !== null && typeof v === 'object' && !Object.isFrozen(v)) {
    for (const child of Object.values(v)) deepFreeze(child);
    Object.freeze(v);
  }
  return v;
}

/** An alias whose anchor is not defined before it, or `null`. */
export function unresolvedAlias(doc: Document): { source: string; range?: [number, number, number] } | null {
  let found: { source: string; range?: [number, number, number] } | null = null;
  visit(doc, {
    Alias(_key, node) {
      if (node.resolve(doc) === undefined) {
        found = { source: node.source, range: node.range ?? undefined };
        return visit.BREAK;
      }
      return undefined;
    },
  });
  return found;
}

/** Opens `text` (YAML without fences, LF line endings) as a model. */
export function openFrontmatter(text: string): FrontmatterModel {
  const doc = parse(text);
  return { text, doc, errors: errorsOf(doc, text), normalized: false, lastEdit: null, forcedQuotes: NO_PATHS };
}

const NO_PATHS: ReadonlySet<string> = new Set();

/**
 * The model's whole value; an empty or comment-only block reads as `{}`, and
 * so does one that cannot be read (see {@link readJs}; it has `errors`).
 * Frozen and shared: copy before changing it.
 */
export function toValue(model: FrontmatterModel): FmValue {
  const read = readJs(model.doc);
  return 'value' in read ? read.value : {};
}

interface Located {
  /** The node at the path (`null` only for a pair with no value node). */
  readonly node: YamlNode | null;
  /** The collection holding it, `null` at the root. */
  readonly parent: YAMLMap | YAMLSeq | null;
  /** The pair holding it when the parent is a map. */
  readonly pair: Pair | null;
  /** Its index among the parent's items. */
  readonly index: number;
}

function keyString(pair: Pair): string | undefined {
  const key = pair.key;
  if (isScalar(key)) return String(key.value);
  return undefined;
}

function locate(doc: Document.Parsed, path: FmPath): Located | undefined {
  let found: Located = { node: (doc.contents as YamlNode | null) ?? null, parent: null, pair: null, index: 0 };
  for (const seg of path) {
    const node = found.node;
    if (isMap(node) && typeof seg === 'string') {
      const index = node.items.findIndex((p) => keyString(p) === seg);
      if (index < 0) return undefined;
      const pair = node.items[index];
      found = { node: (pair.value as YamlNode | null) ?? null, parent: node, pair, index };
    } else if (isSeq(node) && typeof seg === 'number') {
      if (!Number.isInteger(seg) || seg < 0 || seg >= node.items.length) return undefined;
      found = { node: (node.items[seg] as YamlNode | null) ?? null, parent: node, pair: null, index: seg };
    } else {
      return undefined;
    }
  }
  return found;
}

/** The JS value at `path`, or `undefined` when there is none. */
export function getIn(model: FrontmatterModel, path: FmPath): FmValue | undefined {
  let value: FmValue | undefined = toValue(model);
  for (const seg of path) {
    if (Array.isArray(value) && typeof seg === 'number') value = value[seg];
    else if (value && typeof value === 'object' && !Array.isArray(value) && typeof seg === 'string' && Object.prototype.hasOwnProperty.call(value, seg)) value = value[seg];
    else return undefined;
  }
  return value;
}

function hasOwnComment(n: { comment?: string | null; commentBefore?: string | null }): boolean {
  return Boolean(n.comment) || Boolean(n.commentBefore);
}

function nodeInfoOf(node: YamlNode): { hasComments: boolean } {
  let comments = false;
  visit(node, {
    Node(_key, n) {
      if (hasOwnComment(n)) comments = true;
    },
  });
  return { hasComments: comments };
}

/** Describes the value at `path` (see {@link NodeInfo}). */
export function nodeInfo(model: FrontmatterModel, path: FmPath): NodeInfo {
  const loc = locate(model.doc, path);
  if (!loc) {
    return { kind: 'missing', flow: false, hasComments: false, hasYamlFeatures: false, needsYaml: false };
  }
  const node = loc.node;
  if (!node) {
    return { kind: 'scalar', flow: false, hasComments: false, hasYamlFeatures: false, needsYaml: false };
  }
  let comments = false;
  let features = false;
  const self = node as YamlNode & { anchor?: string; tag?: string; flow?: boolean };
  if (isAlias(node)) {
    return { kind: 'alias', flow: false, hasComments: hasOwnComment(node), hasYamlFeatures: true, needsYaml: true };
  }
  let unplaceable = false;
  visit(node, {
    Node(_key, n) {
      const props = n as YamlNode & { anchor?: string; tag?: string };
      if (isAlias(n) || props.anchor || props.tag) features = true;
      if (hasOwnComment(n)) {
        comments = true;
        if (!keepsComment(model.text, n)) unplaceable = true;
      }
    },
    Pair(_key, p) {
      if (isScalar(p.key) && p.key.value === '<<') features = true;
    },
  });
  const kind: NodeKind = isMap(node) ? 'map' : isSeq(node) ? 'seq' : 'scalar';
  const nested = kind !== 'scalar';
  return {
    kind,
    flow: Boolean(self.flow),
    scalarStyle: isScalar(node) ? node.type : undefined,
    tag: self.tag,
    anchor: self.anchor,
    hasComments: comments,
    hasYamlFeatures: features,
    needsYaml: features || (nested && unplaceable),
  };
}

/**
 * Whether every edit operation keeps `node`'s comment where it is, so a
 * collection holding it can still be edited in the form: a comment on the
 * same line after a one-line scalar (`work: x  # desk`, or an empty value's
 * `key: # later`) or after a flow collection (`tags: [a, b]  # mine`), with
 * no comment lines before it. A scalar edit keeps it ({@link setScalar}
 * rewrites only the token, or carries it over); a sibling's add, remove or
 * rename never touches its line; and removing the node removes its own line,
 * comment included, as the user asked.
 *
 * Anything else may be orphaned or dropped: comment lines between items
 * (they would end up above the wrong item, or go with a rewritten
 * collection), a comment after `key:` above a block collection (rewritten
 * when its last item goes), a comment inside a flow collection, and a block
 * scalar's header comment.
 */
function keepsComment(text: string, node: YamlNode): boolean {
  const n = node as YamlNode & { commentBefore?: string | null; comment?: string | null };
  if (n.commentBefore || !n.comment) return false;
  const flowCollection = (isMap(node) || isSeq(node)) && isFlow(node);
  if (!flowCollection && !(isScalar(node) && !node.type?.startsWith('BLOCK'))) return false;
  const [, valueEnd] = rangeOf(node);
  return /^[ \t]*#[^\n]*(\n|$)/.test(text.slice(valueEnd)) && !/\n/.test(n.comment);
}

/** The children of the map or sequence at `path` (empty for anything else). */
export function entries(model: FrontmatterModel, path: FmPath): ModelEntry[] {
  const loc = locate(model.doc, path);
  const node = loc?.node;
  const value = getIn(model, path);
  if (isMap(node) && value && typeof value === 'object' && !Array.isArray(value)) {
    return node.items.flatMap((pair) => {
      const key = keyString(pair);
      if (key === undefined) return [];
      const childPath = [...path, key];
      return [{ key, path: childPath, value: value[key] ?? null, info: nodeInfo(model, childPath) }];
    });
  }
  if (isSeq(node) && Array.isArray(value)) {
    return value.map((item, i) => {
      const childPath = [...path, i];
      return { key: i, path: childPath, value: item, info: nodeInfo(model, childPath) };
    });
  }
  return [];
}

// ============================================================================
// Text geometry
// ============================================================================

function lineStart(text: string, pos: number): number {
  return text.lastIndexOf('\n', pos - 1) + 1;
}

function column(text: string, pos: number): number {
  return pos - lineStart(text, pos);
}

/** `pos` if it starts a line, else the start of the next line (or the end of text). */
function nextLineStart(text: string, pos: number): number {
  if (pos === 0 || text[pos - 1] === '\n') return pos;
  const nl = text.indexOf('\n', pos);
  return nl === -1 ? text.length : nl + 1;
}

/**
 * Where a node's own text ends. A block collection's range runs on over the
 * comment lines and indentation that follow its last item, which belong to
 * whatever comes next; those are left out (a scalar's are not, since a block
 * scalar's content lines may start with `#`). The result is the start of the
 * line after the node.
 */
function nodeTextEnd(text: string, node: YamlNode): number {
  const [start, , nodeEnd] = rangeOf(node);
  let end = nodeEnd;
  if (isMap(node) || isSeq(node)) {
    const ls = lineStart(text, end);
    if (ls > start && onlySpaces(text.slice(ls, end))) end = ls;
    const firstLine = lineStart(text, start);
    while (end > start && text[end - 1] === '\n') {
      const prev = lineStart(text, end - 1);
      if (prev <= firstLine || !/^[ \t]*(#.*)?$/.test(text.slice(prev, end - 1))) break;
      end = prev;
    }
  }
  return nextLineStart(text, Math.max(end, start));
}

/** `end` with one final line break excluded. */
function withoutFinalNewline(text: string, start: number, end: number): number {
  return end > start && text[end - 1] === '\n' ? end - 1 : end;
}

function onlySpaces(s: string): boolean {
  return /^[ \t]*$/.test(s);
}

interface CstToken {
  type: string;
  offset: number;
  source?: string;
}

interface CstCollection {
  items?: { start?: CstToken[]; sep?: CstToken[] }[];
}

function srcItem(coll: YAMLMap | YAMLSeq, index: number): { start?: CstToken[]; sep?: CstToken[] } | undefined {
  const tok = (coll as unknown as { srcToken?: CstCollection }).srcToken;
  return tok?.items?.[index];
}

function rangeOf(node: YamlNode): [number, number, number] {
  const r = (node as unknown as { range?: [number, number, number] }).range;
  if (!r) throw new FrontmatterEditError('invalid', 'node without a source range');
  return r;
}

function isFlow(coll: YAMLMap | YAMLSeq): boolean {
  return Boolean((coll as unknown as { flow?: boolean }).flow);
}

/**
 * Where a block item begins: the `-` of a sequence item, or the key (or the
 * `?` of an explicit key) of a map pair.
 */
function blockItemStart(coll: YAMLMap | YAMLSeq, index: number): number {
  const ind = srcItem(coll, index)?.start?.find((t) => t.type === 'seq-item-ind' || t.type === 'explicit-key-ind');
  if (ind) return ind.offset;
  const item = coll.items[index];
  if (isPair(item)) {
    if (isScalar(item.key) || isMap(item.key) || isSeq(item.key)) return rangeOf(item.key as YamlNode)[0];
  } else if (item) {
    return rangeOf(item as YamlNode)[0];
  }
  throw new FrontmatterEditError('invalid', `cannot find item ${index} in the source`);
}

/** The offset just after a pair's `:` (or a block sequence item's `-`). */
function afterIndicator(coll: YAMLMap | YAMLSeq, index: number): number {
  const item = srcItem(coll, index);
  if (isMap(coll)) {
    const ind = item?.sep?.find((t) => t.type === 'map-value-ind');
    if (ind) return ind.offset + 1;
    const pair = coll.items[index];
    // `{a}` / `? a` with no `:`: the value goes after the key.
    return rangeOf(pair.key as YamlNode)[1];
  }
  const ind = item?.start?.find((t) => t.type === 'seq-item-ind');
  if (ind) return ind.offset + 1;
  return rangeOf(coll.items[index] as YamlNode)[0];
}

/** Where a block item ends: the start of the line after its last byte. */
function blockItemEnd(text: string, coll: YAMLMap | YAMLSeq, index: number): number {
  const item = coll.items[index];
  const value = (isPair(item) ? item.value : item) as YamlNode | null;
  let end: number;
  if (value && rangeOf(value)[2] > rangeOf(value)[0]) {
    end = nodeTextEnd(text, value);
  } else {
    end = afterIndicator(coll, index);
  }
  return nextLineStart(text, end);
}

/** The end of a flow item's value (its last byte, no comments or commas). */
function flowItemEnd(coll: YAMLMap | YAMLSeq, index: number): number {
  const item = coll.items[index];
  if (isPair(item)) {
    const v = item.value as YamlNode | null;
    if (v && rangeOf(v)[1] > rangeOf(v)[0]) return rangeOf(v)[1];
    return rangeOf(item.key as YamlNode)[1];
  }
  return rangeOf(item as YamlNode)[1];
}

function flowItemStart(coll: YAMLMap | YAMLSeq, index: number): number {
  const item = coll.items[index];
  return isPair(item) ? rangeOf(item.key as YamlNode)[0] : rangeOf(item as YamlNode)[0];
}

interface Style {
  /** Indentation step for nested block collections. */
  readonly indent: number;
  /** Whether a block sequence under a key is indented (`key:\n  - a`) or not (`key:\n- a`). */
  readonly indentSeq: boolean;
}

/** The document's own indentation habits, from its first nested block collections. */
function detectStyle(model: FrontmatterModel): Style {
  let indent: number | undefined;
  let indentSeq: boolean | undefined;
  const text = model.text;
  visit(model.doc, {
    Pair(_key, pair) {
      const value = pair.value;
      if (!isScalar(pair.key) || (!isMap(value) && !isSeq(value)) || isFlow(value) || value.items.length === 0) return;
      const keyCol = column(text, rangeOf(pair.key as YamlNode)[0]);
      const childCol = column(text, blockItemStart(value, 0));
      if (isMap(value) && indent === undefined && childCol > keyCol) indent = childCol - keyCol;
      if (isSeq(value) && indentSeq === undefined) {
        indentSeq = childCol > keyCol;
        if (indent === undefined && childCol > keyCol) indent = childCol - keyCol;
      }
      if (indent !== undefined && indentSeq !== undefined) return visit.BREAK;
      return undefined;
    },
  });
  return { indent: indent ?? 2, indentSeq: indentSeq ?? true };
}

// ============================================================================
// Formatting values
// ============================================================================

type Ctx = 'block' | 'flow' | 'key';

function parsesTo(src: string, value: FmValue, ctx: Ctx): boolean {
  const wrapped = ctx === 'flow' ? `[${src}]` : ctx === 'key' ? `${src}: x` : `k: ${src}`;
  const doc = parseDocument(wrapped);
  if (doc.errors.length > 0 || doc.warnings.length > 0) return false;
  const js = doc.toJS() as unknown;
  if (ctx === 'flow') return Array.isArray(js) && js.length === 1 && jsEqual(js[0], value);
  if (ctx === 'key') {
    const keys = js && typeof js === 'object' ? Object.keys(js) : [];
    return keys.length === 1 && keys[0] === value;
  }
  return Boolean(js && typeof js === 'object') && jsEqual((js as Record<string, unknown>).k, value);
}

function doubleQuoted(s: string): string {
  return JSON.stringify(s);
}

/**
 * One line of YAML for `value` in context, in `style` when that is still
 * valid there (plain stays plain, quotes keep their kind), else double-quoted.
 * Returns `null` for a string that should become a block scalar.
 */
function inlineScalar(value: FmScalar, ctx: Ctx, style?: Scalar.Type): string | null {
  if (value === null) return ctx === 'block' ? '' : 'null';
  if (typeof value !== 'string') {
    const out = stringify(value).trimEnd();
    return out;
  }
  const multiline = value.includes('\n');
  if (multiline && ctx === 'block' && style !== 'QUOTE_DOUBLE' && style !== 'QUOTE_SINGLE') return null;
  if (style === 'QUOTE_SINGLE' && !multiline) {
    const single = `'${value.replace(/'/g, "''")}'`;
    if (parsesTo(single, value, ctx)) return single;
  }
  if (style === 'QUOTE_DOUBLE') return doubleQuoted(value);
  if (!multiline && value === value.trim() && value !== '' && parsesTo(value, value, ctx)) return value;
  return doubleQuoted(value);
}

/**
 * A literal block scalar for a multi-line string (header line first, content
 * lines at `contentCol`), ending in a line break; `null` when a literal block
 * cannot carry it without an indentation indicator.
 */
function literalBlock(value: string, contentCol: number, header?: string): string | null {
  if (/^[ \t\n]/.test(value) || /[\r\u0085\u2028\u2029]/.test(value)) return null;
  const chomp = value.endsWith('\n\n') ? '+' : value.endsWith('\n') ? '' : '-';
  const content = chomp === '-' ? value : value.slice(0, -1);
  const pad = ' '.repeat(contentCol);
  const lines = content.split('\n').map((l) => (l === '' ? '' : pad + l));
  return `|${chomp}${header ?? ''}\n${lines.join('\n')}\n`;
}

function stringifyBlock(value: FmValue, style: Style): string {
  // An empty value is written `key:` / `-`, as a new top-level property is.
  return stringify(value, { indent: style.indent, indentSeq: style.indentSeq, lineWidth: 0, minContentWidth: 0, nullStr: '' })
    .replace(/\n$/, '')
    .replace(/^( *-) $/gm, '$1');
}

/** `value` in flow style on one line (`[a, b]`, `{k: v}`), the spelling hand-written YAML uses. */
function stringifyFlow(value: FmValue): string {
  if (Array.isArray(value)) return `[${value.map(stringifyFlow).join(', ')}]`;
  if (value !== null && typeof value === 'object') {
    const pairs = Object.entries(value).map(([k, v]) => `${inlineScalar(k, 'key', 'PLAIN') ?? doubleQuoted(k)}: ${stringifyFlow(v)}`);
    return `{${pairs.join(', ')}}`;
  }
  return inlineScalar(value, 'flow') ?? doubleQuoted(String(value));
}

function isScalarValue(v: FmValue): v is FmScalar {
  return v === null || typeof v !== 'object';
}

function indentLines(lines: readonly string[], col: number): string {
  const pad = ' '.repeat(col);
  return lines.map((l) => (l === '' ? '' : pad + l)).join('\n');
}

/**
 * The text that follows a block indicator (`key:` or `-`) for `value`, with
 * no final line break: ` inline`, or a line break and lines indented to
 * `childCol` (`compact` puts a nested map/seq's first line on the indicator
 * line, as `- a: 1` does).
 */
function afterBlockIndicator(value: FmValue, childCol: number, compact: boolean, style: Style): string {
  if (isScalarValue(value)) {
    const inline = inlineScalar(value, 'block');
    if (inline !== null) return inline === '' ? '' : ` ${inline}`;
    const block = literalBlock(value as string, childCol);
    if (block) return ` ${block.replace(/\n$/, '')}`;
    return ` ${doubleQuoted(value as string)}`;
  }
  const empty = Array.isArray(value) ? value.length === 0 : Object.keys(value).length === 0;
  if (empty) return Array.isArray(value) ? ' []' : ' {}';
  const lines = stringifyBlock(value, style).split('\n');
  if (compact) return ` ${lines[0]}${lines.length > 1 ? `\n${indentLines(lines.slice(1), childCol)}` : ''}`;
  return `\n${indentLines(lines, childCol)}`;
}

// ============================================================================
// Equality and the meaning of each operation
// ============================================================================

/** Deep equality of JSON-shaped values; object key order counts. */
export function jsEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a === 'number' && typeof b === 'number') return Number.isNaN(a) && Number.isNaN(b);
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((x, i) => jsEqual(x, b[i]));
  }
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    const ka = Object.keys(a);
    const kb = Object.keys(b);
    if (ka.length !== kb.length || ka.some((k, i) => k !== kb[i])) return false;
    return ka.every((k) => jsEqual((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
  }
  return false;
}

function cloneValue<T extends FmValue>(v: T): T {
  return v === null || typeof v !== 'object' ? v : (JSON.parse(JSON.stringify(v)) as T);
}

/** `root` with `fn` applied to the container at `path` (a fresh copy along the path). */
function updateAt(root: FmValue, path: FmPath, fn: (container: FmValue) => FmValue): FmValue {
  if (path.length === 0) return fn(root);
  const [head, ...rest] = path;
  if (Array.isArray(root) && typeof head === 'number') {
    const copy = root.slice();
    copy[head] = updateAt(copy[head], rest, fn);
    return copy;
  }
  if (root && typeof root === 'object' && !Array.isArray(root) && typeof head === 'string') {
    return { ...root, [head]: updateAt(root[head] ?? null, rest, fn) };
  }
  throw new FrontmatterEditError('not-found', `no value at ${formatPath(path)}`);
}

function formatPath(path: FmPath): string {
  return path.length === 0 ? '(root)' : path.map((s) => (typeof s === 'number' ? `[${s}]` : s)).join('.');
}

// ============================================================================
// Applying an edit
// ============================================================================

type DocOp = (doc: Document.Parsed) => void;

interface Edit {
  readonly splice: TextEdit | null;
  readonly intended: FmValue;
  readonly fallback: DocOp;
  /** The model's {@link FrontmatterModel.forcedQuotes} after the edit (default: cleared, since paths may have moved). */
  readonly forcedQuotes?: ReadonlySet<string>;
}

function withSplice(text: string, e: TextEdit): string {
  return text.slice(0, e.start) + e.text + text.slice(e.end);
}

function rootValue(doc: Document.Parsed): FmValue {
  const read = readJs(doc);
  if ('error' in read) throw read.error;
  return read.value;
}

function ensureParsed(model: FrontmatterModel): void {
  if (model.errors.length > 0) {
    throw new FrontmatterEditError('parse-errors', 'the frontmatter has YAML errors; fix them in the YAML tab first');
  }
}

function commit(model: FrontmatterModel, edit: Edit): FrontmatterModel {
  if (edit.splice) {
    const text = withSplice(model.text, edit.splice);
    const doc = parse(text);
    if (errorsOf(doc, text).length === 0 && jsEqual(rootValue(doc), edit.intended)) {
      return { text, doc, errors: [], normalized: model.normalized, lastEdit: edit.splice, forcedQuotes: edit.forcedQuotes ?? NO_PATHS };
    }
  }
  // Fallback: the document API, the whole block re-serialized.
  const clone = model.doc.clone() as Document.Parsed;
  let text: string;
  try {
    edit.fallback(clone);
    text = clone.contents === null && !clone.commentBefore && !clone.comment ? '' : String(clone);
  } catch {
    // e.g. an alias left naming no anchor, which `yaml` refuses to write.
    throw new FrontmatterEditError('invalid', 'the edit could not be written as YAML');
  }
  if (!model.text.endsWith('\n')) text = text.replace(/\n$/, '');
  const doc = parse(text);
  if (errorsOf(doc, text).length > 0 || !jsEqual(rootValue(doc), edit.intended)) {
    throw new FrontmatterEditError('invalid', 'the edit could not be written as YAML');
  }
  return { text, doc, errors: [], normalized: true, lastEdit: null, forcedQuotes: NO_PATHS };
}

function requireLocated(model: FrontmatterModel, path: FmPath): Located {
  const loc = locate(model.doc, path);
  if (!loc) throw new FrontmatterEditError('not-found', `no value at ${formatPath(path)}`);
  return loc;
}

function setFallback(path: FmPath, value: FmValue): DocOp {
  return (doc) => {
    if (path.length === 0) doc.contents = doc.createNode(value) as never;
    else doc.setIn(path as unknown[], doc.createNode(value));
  };
}

/**
 * The splice that puts `valueText` (already formatted for its place) where
 * the value of the item at `loc` is, keeping the line break after it.
 */
function replaceValueSplice(model: FrontmatterModel, loc: Located, value: FmValue): TextEdit {
  const text = model.text;
  const { parent, index, node } = loc;
  const style = detectStyle(model);
  if (!parent) throw new FrontmatterEditError('invalid', 'the root is replaced through the document API');
  if (isFlow(parent)) {
    const start = node ? rangeOf(node)[0] : afterIndicator(parent, index);
    const end = node ? rangeOf(node)[1] : start;
    const inline = isScalarValue(value) ? (inlineScalar(value, 'flow') ?? doubleQuoted(String(value))) : stringifyFlow(value);
    const lead = node && rangeOf(node)[1] > rangeOf(node)[0] ? '' : onlySpaces(text.slice(start - 1, start)) ? '' : ' ';
    return { start, end, text: lead + inline };
  }
  const start = afterIndicator(parent, index);
  const itemCol = column(text, blockItemStart(parent, index));
  const isSeqItem = isSeq(parent);
  const childCol = isSeqItem ? itemCol + 2 : itemCol + (Array.isArray(value) && !style.indentSeq ? 0 : style.indent);
  let end = start;
  let trailing = '';
  if (node) {
    const valueEnd = rangeOf(node)[1];
    end = withoutFinalNewline(text, start, Math.max(nodeTextEnd(text, node), start));
    if (isScalar(node) && !node.type?.startsWith('BLOCK')) {
      // Keep a scalar's own trailing comment: it stays on the indicator line.
      // An empty value has no token: its range starts at the comment itself.
      const from = rangeOf(node)[0] === valueEnd ? start : valueEnd;
      const comment = text.slice(from, end).match(/^[ \t]+#[^\n]*$/);
      if (comment) trailing = comment[0];
    }
  }
  const after = afterBlockIndicator(value, childCol, isSeqItem, style);
  if (!trailing) return { start, end, text: after };
  // A comment must precede a nested block, or follow an inline value.
  const nl = after.indexOf('\n');
  if (nl === -1) return { start, end, text: `${after}${trailing}` };
  return { start, end, text: `${after.slice(0, nl)}${trailing}${after.slice(nl)}` };
}

/** The surgical splice for a scalar replacing a scalar, in `style` (the original's by default). */
function scalarSplice(model: FrontmatterModel, loc: Located, node: Scalar, value: FmScalar, style: Scalar.Type | undefined = node.type): TextEdit | null {
  const text = model.text;
  const [start, valueEnd] = rangeOf(node);
  const flow = loc.parent ? isFlow(loc.parent) : false;
  const ctx: Ctx = flow ? 'flow' : 'block';
  const block = node.type === 'BLOCK_LITERAL' || node.type === 'BLOCK_FOLDED';
  if (start === valueEnd && !block) return null; // an empty value: no token to restyle
  if (block) {
    if (typeof value === 'string' && value.includes('\n')) {
      const header = text.slice(start, text.indexOf('\n', start));
      const comment = header.match(/\s#.*$/)?.[0] ?? '';
      const firstContent = text.slice(text.indexOf('\n', start) + 1).match(/^( *)\S/m);
      const parentCol = loc.parent ? column(text, blockItemStart(loc.parent, loc.index)) : 0;
      const contentCol = firstContent && firstContent[1].length > parentCol ? firstContent[1].length : parentCol + 2;
      const literal = literalBlock(value, contentCol, comment);
      if (literal) return { start, end: valueEnd, text: literal };
      return null;
    }
    const inline = inlineScalar(value, ctx, 'PLAIN');
    if (inline === null) return null;
    return { start, end: valueEnd, text: `${inline}${text[valueEnd - 1] === '\n' ? '\n' : ''}` };
  }
  const inline = inlineScalar(value, ctx, style);
  if (inline === null) return null; // multi-line: the generic path makes a block
  if (inline === '') return null;
  return { start, end: valueEnd, text: inline };
}

// ============================================================================
// Operations
// ============================================================================

/**
 * Sets the value at `path` to a scalar. An existing scalar is rewritten in
 * place in its own quoting style where that still means `value` (a plain
 * scalar stays plain, quotes keep their kind, a block scalar stays a block
 * while the value has line breaks); its trailing comment, tag and anchor stay.
 * A plain (or empty) scalar the value forces into quotes is remembered in
 * {@link FrontmatterModel.forcedQuotes} and written plain again once it can
 * be, so typing `hello world` a keystroke at a time ends plain.
 * A missing last key or `index === length` is added.
 */
export function setScalar(model: FrontmatterModel, path: FmPath, value: FmScalar): FrontmatterModel {
  return setValue(model, path, value);
}

/**
 * Sets the value at `path` to any value (a collection is written in block
 * style inside block context, flow style inside a flow collection).
 */
export function setValue(model: FrontmatterModel, path: FmPath, value: FmValue): FrontmatterModel {
  ensureParsed(model);
  const loc = locate(model.doc, path);
  if (!loc) {
    const parentPath = path.slice(0, -1);
    const last = path[path.length - 1];
    if (typeof last === 'string') return addKey(model, parentPath, last, value);
    if (typeof last === 'number' && Array.isArray(getIn(model, parentPath)) && last === (getIn(model, parentPath) as FmValue[]).length) {
      return addItem(model, parentPath, value);
    }
    throw new FrontmatterEditError('not-found', `no value at ${formatPath(path)}`);
  }
  const intended = updateAt(rootValue(model.doc), path, () => cloneValue(value));
  const id = JSON.stringify(path);
  const node = loc.node;
  // Quotes an earlier keystroke forced on a plain scalar are not its style.
  const style = isScalar(node) && node.type === 'QUOTE_DOUBLE' && model.forcedQuotes.has(id) ? 'PLAIN' : isScalar(node) ? node.type : undefined;
  const plainBefore = !node || (isScalar(node) && (style === 'PLAIN' || rangeOf(node)[0] === rangeOf(node)[1]));
  let splice: TextEdit | null = null;
  if (isScalar(node) && isScalarValue(value)) splice = scalarSplice(model, loc, node, value, style);
  if (!splice && !isAlias(node) && loc.parent) splice = replaceValueSplice(model, loc, value);
  // Paths at and below this one are rewritten; a plain one quoted only because the value needs it is recorded.
  const forcedQuotes = new Set([...model.forcedQuotes].filter((k) => path.length > 0 && k !== id && !k.startsWith(`${id.slice(0, -1)},`)));
  if (splice && typeof value === 'string' && plainBefore && /^[ \t]*"/.test(splice.text)) forcedQuotes.add(id);
  return commit(model, { splice, intended, fallback: setFallback(path, value), forcedQuotes });
}

function containerAt(model: FrontmatterModel, path: FmPath): Located {
  return requireLocated(model, path);
}

/** Adds `key: value` at the end of the map at `path` (the root by default). */
export function addKey(model: FrontmatterModel, path: FmPath, key: string, value: FmValue): FrontmatterModel {
  ensureParsed(model);
  const loc = containerAt(model, path);
  const target = loc.node;
  const current = getIn(model, path);
  if (current && typeof current === 'object' && !Array.isArray(current) && Object.prototype.hasOwnProperty.call(current, key)) {
    throw new FrontmatterEditError('exists', `${formatPath([...path, key])} already exists`);
  }
  const isNullish = !target || (isScalar(target) && target.value === null);
  if (!isMap(target) && !isNullish) {
    throw new FrontmatterEditError('wrong-kind', `${formatPath(path)} is not a map`);
  }
  const intended = updateAt(rootValue(model.doc), path, (c) => ({ ...(c && typeof c === 'object' && !Array.isArray(c) ? c : {}), [key]: cloneValue(value) }));
  const fallback: DocOp = (doc) => {
    if (path.length === 0 && !isMap(doc.contents)) doc.contents = doc.createNode({}) as never;
    doc.setIn([...path, key] as unknown[], doc.createNode(value));
  };
  const text = model.text;
  const style = detectStyle(model);
  const keyText = inlineScalar(key, 'key', 'PLAIN') ?? doubleQuoted(key);
  let splice: TextEdit | null = null;

  if (isMap(target) && isFlow(target)) {
    const inline = isScalarValue(value) ? (inlineScalar(value, 'flow') ?? doubleQuoted(String(value))) : stringifyFlow(value);
    const entry = `${keyText}: ${inline}`;
    if (target.items.length === 0) splice = { start: rangeOf(target)[0] + 1, end: rangeOf(target)[0] + 1, text: entry };
    else {
      const at = flowItemEnd(target, target.items.length - 1);
      splice = { start: at, end: at, text: `, ${entry}` };
    }
  } else if (isMap(target) && target.items.length > 0) {
    const col = column(text, blockItemStart(target, 0));
    const at = blockItemEnd(text, target, target.items.length - 1);
    const lead = at > 0 && text[at - 1] !== '\n' ? '\n' : '';
    const line = `${' '.repeat(col)}${keyText}:${afterBlockIndicator(value, col + (Array.isArray(value) && !style.indentSeq ? 0 : style.indent), false, style)}`;
    const tail = at < text.length || text.endsWith('\n') ? '\n' : '';
    splice = { start: at, end: at, text: `${lead}${line}${tail}` };
  } else if (path.length === 0) {
    // An empty block, or one holding only comments (or an empty map).
    const lead = text.length > 0 && !text.endsWith('\n') ? '\n' : '';
    const line = `${keyText}:${afterBlockIndicator(value, style.indent, false, style)}`;
    splice = { start: text.length, end: text.length, text: `${lead}${line}${text.endsWith('\n') ? '\n' : ''}` };
  } else {
    // A null value or an empty block map: the value becomes `{key: value}`.
    return commit(model, { splice: replaceValueSplice(model, loc, { [key]: value }), intended, fallback, forcedQuotes: model.forcedQuotes });
  }
  return commit(model, { splice, intended, fallback, forcedQuotes: model.forcedQuotes });
}

/** Appends `value` to the sequence at `path` (a null value becomes a one-item sequence). */
export function addItem(model: FrontmatterModel, path: FmPath, value: FmValue): FrontmatterModel {
  ensureParsed(model);
  const loc = containerAt(model, path);
  const target = loc.node;
  const isNullish = !target || (isScalar(target) && target.value === null);
  if (!isSeq(target) && !isNullish) {
    throw new FrontmatterEditError('wrong-kind', `${formatPath(path)} is not a list`);
  }
  const intended = updateAt(rootValue(model.doc), path, (c) => [...(Array.isArray(c) ? c : []), cloneValue(value)]);
  const fallback: DocOp = (doc) => {
    if (isSeq(doc.getIn(path as unknown[]))) doc.addIn(path as unknown[], doc.createNode(value));
    else setFallback(path, [value])(doc);
  };
  const text = model.text;
  const style = detectStyle(model);
  let splice: TextEdit | null = null;
  if (isSeq(target) && isFlow(target)) {
    const inline = isScalarValue(value) ? (inlineScalar(value, 'flow') ?? doubleQuoted(String(value))) : stringifyFlow(value);
    if (target.items.length === 0) splice = { start: rangeOf(target)[0] + 1, end: rangeOf(target)[0] + 1, text: inline };
    else {
      const at = flowItemEnd(target, target.items.length - 1);
      splice = { start: at, end: at, text: `, ${inline}` };
    }
  } else if (isSeq(target) && target.items.length > 0) {
    const col = column(text, blockItemStart(target, 0));
    const at = blockItemEnd(text, target, target.items.length - 1);
    const lead = at > 0 && text[at - 1] !== '\n' ? '\n' : '';
    const tail = at < text.length || text.endsWith('\n') ? '\n' : '';
    splice = { start: at, end: at, text: `${lead}${' '.repeat(col)}-${afterBlockIndicator(value, col + 2, true, style)}${tail}` };
  } else if (loc.parent) {
    return commit(model, { splice: replaceValueSplice(model, loc, [value]), intended, fallback, forcedQuotes: model.forcedQuotes });
  }
  return commit(model, { splice, intended, fallback, forcedQuotes: model.forcedQuotes });
}

/** The splice removing item `index` of a collection (block or flow). */
function removeSplice(model: FrontmatterModel, coll: YAMLMap | YAMLSeq, index: number): TextEdit | null {
  const text = model.text;
  const n = coll.items.length;
  if (isFlow(coll)) {
    if (index < n - 1) return { start: flowItemStart(coll, index), end: flowItemStart(coll, index + 1), text: '' };
    if (index > 0) return { start: flowItemEnd(coll, index - 1), end: flowItemEnd(coll, index), text: '' };
    return { start: flowItemStart(coll, index), end: flowItemEnd(coll, index), text: '' };
  }
  const itemStart = blockItemStart(coll, index);
  const ls = lineStart(text, itemStart);
  if (onlySpaces(text.slice(ls, itemStart))) {
    let end = blockItemEnd(text, coll, index);
    let start = ls;
    // The last line of a block without a final newline: take the newline before it instead.
    if (end === text.length && !text.endsWith('\n') && start > 0) {
      start -= 1;
      end = text.length;
    }
    return { start, end, text: '' };
  }
  // The item shares its line with its parent's indicator (`- a: 1`): pull the next one up.
  if (index < n - 1) return { start: itemStart, end: blockItemStart(coll, index + 1), text: '' };
  return null;
}

function removeAt(model: FrontmatterModel, path: FmPath, kind: 'map' | 'seq'): FrontmatterModel {
  ensureParsed(model);
  if (path.length === 0) throw new FrontmatterEditError('not-found', 'cannot remove the root');
  const loc = requireLocated(model, path);
  const parent = loc.parent;
  if (!parent || (kind === 'map' ? !isMap(parent) : !isSeq(parent))) {
    throw new FrontmatterEditError('wrong-kind', `${formatPath(path)} is not ${kind === 'map' ? 'a key' : 'a list item'}`);
  }
  const parentPath = path.slice(0, -1);
  const last = path[path.length - 1];
  const intended = updateAt(rootValue(model.doc), parentPath, (c) => {
    if (Array.isArray(c)) return c.filter((_, i) => i !== last);
    const copy = { ...(c as Record<string, FmValue>) };
    delete copy[last as string];
    return copy;
  });
  const fallback: DocOp = (doc) => {
    doc.deleteIn(path as unknown[]);
  };
  const only = parent.items.length === 1;
  let splice: TextEdit | null;
  if (only && !isFlow(parent) && parentPath.length > 0) {
    // The last item of a nested block collection: leave an empty `[]`/`{}`.
    const ploc = requireLocated(model, parentPath);
    splice = replaceValueSplice(model, ploc, kind === 'map' ? {} : []);
  } else {
    splice = removeSplice(model, parent, loc.index);
  }
  return commit(model, { splice, intended, fallback });
}

/** Removes the key at `path` (its line or lines; comments on other lines stay). */
export function removeKey(model: FrontmatterModel, path: FmPath): FrontmatterModel {
  if (typeof path[path.length - 1] !== 'string') throw new FrontmatterEditError('wrong-kind', `${formatPath(path)} is not a key`);
  return removeAt(model, path, 'map');
}

/** Removes item `index` of the sequence at `path`. */
export function removeItem(model: FrontmatterModel, path: FmPath, index: number): FrontmatterModel {
  return removeAt(model, [...path, index], 'seq');
}

/** Renames the key at `path` to `newKey`, rewriting only the key's own token. */
export function renameKey(model: FrontmatterModel, path: FmPath, newKey: string): FrontmatterModel {
  ensureParsed(model);
  const oldKey = path[path.length - 1];
  if (typeof oldKey !== 'string') throw new FrontmatterEditError('wrong-kind', `${formatPath(path)} is not a key`);
  const loc = requireLocated(model, path);
  if (!loc.pair || !isMap(loc.parent)) throw new FrontmatterEditError('wrong-kind', `${formatPath(path)} is not a key`);
  if (newKey === oldKey) return model;
  const parentPath = path.slice(0, -1);
  const siblings = getIn(model, parentPath) as Record<string, FmValue>;
  if (Object.prototype.hasOwnProperty.call(siblings, newKey)) {
    throw new FrontmatterEditError('exists', `${formatPath([...parentPath, newKey])} already exists`);
  }
  const intended = updateAt(rootValue(model.doc), parentPath, (c) =>
    Object.fromEntries(Object.entries(c as Record<string, FmValue>).map(([k, v]) => [k === oldKey ? newKey : k, v])),
  );
  const pair = loc.pair;
  const fallback: DocOp = (doc) => {
    const target = doc.getIn(path.slice(0, -1) as unknown[], true) ?? doc.contents;
    if (!isMap(target)) return;
    const p = target.items.find((item) => keyString(item) === oldKey);
    if (p) p.key = doc.createNode(newKey);
  };
  let splice: TextEdit | null = null;
  if (isScalar(pair.key)) {
    const [start, end] = rangeOf(pair.key as YamlNode);
    const style = pair.key.type;
    const keyText = style === 'QUOTE_DOUBLE' ? doubleQuoted(newKey) : (inlineScalar(newKey, 'key', style) ?? doubleQuoted(newKey));
    splice = { start, end, text: keyText };
  }
  return commit(model, { splice, intended, fallback });
}

/**
 * Replaces the value at `path` with `yamlSnippet`, the user's own YAML for
 * it (what {@link sliceAt} handed out, edited): its bytes are kept and only
 * re-indented to the value's place. Throws `invalid` for a snippet that does
 * not parse.
 */
export function replaceSubtree(model: FrontmatterModel, path: FmPath, yamlSnippet: string): FrontmatterModel {
  ensureParsed(model);
  const snippetDoc = parseDocument(yamlSnippet);
  if (snippetDoc.errors.length > 0) {
    throw new FrontmatterEditError('invalid', snippetDoc.errors[0].message.split('\n')[0]);
  }
  // A snippet may name an anchor defined elsewhere in the note (`<<: *base`).
  const ownValue = unresolvedAlias(snippetDoc) ? undefined : ((snippetDoc.toJS() as FmValue | undefined) ?? null);
  if (ownValue === undefined && path.length === 0) {
    throw new FrontmatterEditError('invalid', `the alias *${unresolvedAlias(snippetDoc)!.source} names no anchor`);
  }
  if (path.length === 0) {
    const value = ownValue ?? null;
    const text = yamlSnippet;
    const intended = value ?? {};
    return commit(model, {
      splice: { start: 0, end: model.text.length, text },
      intended,
      fallback: (doc) => {
        doc.contents = snippetDoc.contents as never;
      },
    });
  }
  const loc = requireLocated(model, path);
  const fallback: DocOp = (doc) => {
    doc.setIn(path as unknown[], snippetDoc.contents ?? null);
  };
  // Aliases of an anchor the snippet (re)defines follow it, and the
  // snippet's own aliases name anchors elsewhere, so then the edit means
  // what the document API makes of it, not only a new value here.
  const viaDoc = hasAliases(model.doc) || ownValue === undefined ? meaningVia(model, fallback) : null;
  if (ownValue === undefined && viaDoc === null) {
    throw new FrontmatterEditError('invalid', `the alias *${unresolvedAlias(snippetDoc)!.source} names no anchor in the frontmatter`);
  }
  const intended = viaDoc ?? updateAt(rootValue(model.doc), path, () => ownValue ?? null);
  const value = ownValue !== undefined ? ownValue : valueAt(intended, path);
  const parent = loc.parent as YAMLMap | YAMLSeq;
  const text = model.text;
  const root = snippetDoc.contents;
  const lines = dedent(yamlSnippet.replace(/\n+$/, '')).split('\n');
  let splice: TextEdit;
  if (isFlow(parent)) {
    const node = loc.node;
    const start = node ? rangeOf(node)[0] : afterIndicator(parent, loc.index);
    const end = node ? rangeOf(node)[1] : start;
    const blockRoot = (isMap(root) || isSeq(root)) && !isFlow(root);
    // A flow collection cannot hold block YAML: a block snippet is rewritten in flow style.
    const out =
      lines.length === 1 && !blockRoot
        ? lines[0].trim()
        : !nodeInfoOf(root as YamlNode).hasComments
          ? stringifyFlow(value)
          : lines.map((l) => l.trim()).join(' ');
    splice = { start, end, text: out };
  } else {
    const start = afterIndicator(parent, loc.index);
    const node = loc.node;
    const end = node ? withoutFinalNewline(text, start, Math.max(nodeTextEnd(text, node), start)) : start;
    const itemCol = column(text, blockItemStart(parent, loc.index));
    const style = detectStyle(model);
    const blockCollection = (isMap(root) || isSeq(root)) && !isFlow(root);
    // A value already below its indicator stays on its own lines; lines below
    // the indicator keep the column they had, and an unchanged first line its spacing.
    const below = node !== null && text.slice(start, rangeOf(node)[0]).includes('\n');
    const span = text.slice(start, end);
    const nl = span.indexOf('\n');
    const origFirstRaw = nl === -1 ? span : span.slice(0, nl);
    const origRest = nl === -1 ? [] : span.slice(nl + 1).split('\n').filter((l) => l.trim() !== '');
    const origCol = origRest.length > 0 ? Math.min(...origRest.map((l) => l.match(/^ */)![0].length)) : undefined;
    // Only a sequence under a key may sit at the key's own column.
    const minCol = isMap(parent) && isSeq(root) ? itemCol : itemCol + 1;
    const belowCol = below && node ? column(text, rangeOf(node)[0]) : -1;
    const childCol =
      origCol !== undefined && origCol >= minCol
        ? origCol
        : belowCol >= minCol
          ? belowCol
          : isSeq(parent)
            ? itemCol + 2
            : itemCol + (isSeq(root) && !style.indentSeq ? 0 : style.indent);
    const firstSame = origFirstRaw.trim() !== '' && lines[0].trim() === origFirstRaw.trim();
    let out: string;
    if (lines.length === 1 && lines[0].trim() === '') out = '';
    else if (firstSame) out = `${origFirstRaw}${lines.length > 1 ? `\n${indentLines(lines.slice(1), childCol)}` : ''}`;
    else if (blockCollection && (below || !isSeq(parent))) out = `\n${indentLines(lines, childCol)}`;
    else out = ` ${lines[0]}${lines.length > 1 ? `\n${indentLines(lines.slice(1), childCol)}` : ''}`;
    splice = { start, end, text: out };
  }
  return commit(model, { splice, intended, fallback });
}

function valueAt(root: FmValue, path: FmPath): FmValue {
  let v: FmValue | undefined = root;
  for (const seg of path) v = Array.isArray(v) ? v[seg as number] : v && typeof v === 'object' ? v[seg as string] : undefined;
  return v ?? null;
}

function hasAliases(doc: Document.Parsed): boolean {
  let found = false;
  visit(doc, {
    Alias() {
      found = true;
      return visit.BREAK;
    },
  });
  return found;
}

/** The document's value after `op` (on a copy), or `null` when that does not read. */
function meaningVia(model: FrontmatterModel, op: DocOp): FmValue | null {
  try {
    const clone = model.doc.clone() as Document.Parsed;
    op(clone);
    return rootValue(clone);
  } catch {
    return null;
  }
}

/** `text` with the indentation common to its non-blank lines removed. */
function dedent(text: string): string {
  const lines = text.split('\n');
  const indents = lines.filter((l) => l.trim() !== '').map((l) => l.match(/^ */)![0].length);
  const common = indents.length > 0 ? Math.min(...indents) : 0;
  return lines.map((l) => l.slice(common)).join('\n');
}

/**
 * The value at `path` as standalone YAML, for "Edit as YAML": its source
 * (comments, anchors and styles included) with the indentation of its place
 * removed. What is on the indicator line comes first, then the lines below
 * it dedented. {@link replaceSubtree} with an unchanged slice leaves the text
 * as it was in the ordinary cases.
 */
export function sliceAt(model: FrontmatterModel, path: FmPath): string {
  const text = model.text;
  if (path.length === 0) return text;
  const loc = requireLocated(model, path);
  const parent = loc.parent as YAMLMap | YAMLSeq;
  const node = loc.node;
  if (isFlow(parent)) {
    return node ? text.slice(rangeOf(node)[0], rangeOf(node)[1]) : '';
  }
  const start = afterIndicator(parent, loc.index);
  const end = node ? withoutFinalNewline(text, start, Math.max(nodeTextEnd(text, node), start)) : start;
  const raw = text.slice(start, end);
  const nl = raw.indexOf('\n');
  const first = (nl === -1 ? raw : raw.slice(0, nl)).trim();
  const rest = nl === -1 ? '' : dedent(raw.slice(nl + 1));
  if (!first) return rest;
  return rest ? `${first}\n${rest}` : first;
}
