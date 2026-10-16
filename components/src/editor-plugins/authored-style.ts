/**
 * Cosmetic markdown choices kept as authored (TODO "Editor: cover mbr's
 * markdown extensions", item 6): the things CommonMark lets an author spell
 * several ways and remark-stringify spells one way.
 *
 * | Construct        | Authored                      | remark's default |
 * |------------------|-------------------------------|------------------|
 * | bullet list      | `-` / `*` / `+`               | `*`              |
 * | ordered list     | `1)` / `1.`, `1. 1. 1.`, `1.  x` | `1.`, counting, one space |
 * | list item gap    | `-   item`                    | `* item`         |
 * | thematic break   | `---`, `___`, `* * *`         | `***`            |
 * | heading          | setext (`===`/`---`), `# x #` | ATX, no closing  |
 * | code block       | indented, `~~~`, ```` ```` ````, info `meta` | ```` ``` ````, meta dropped |
 * | hard break       | two trailing spaces           | `\`              |
 * | blockquote       | `>>>` (marginalia)            | `> > >`          |
 * | table            | as typed (`\|-\|-\|`, unpadded) | re-padded, aligned |
 *
 * Each is recorded at parse time from the source ({@link recordAuthoredStyles},
 * a remark transform) as `mbrStyle` on the mdast node, carried through the
 * editor on the ProseMirror node's `mbrStyle` attr ({@link carryAuthoredStyle}
 * — copied along when ProseMirror splits or copies the node), and handed back
 * to mdast on serialize, where this module's to-markdown handlers print it.
 * A node made in the editor has no style and prints in the house style
 * ({@link HOUSE_STYLE}: `-` bullets, `---` rules), and a style that no longer
 * fits the node (a setext heading changed to level 3, an indented block given
 * a language) is dropped for the default rather than forced.
 *
 * Tables are the one construct kept row by row: a row whose cells still read
 * as the authored row's cells (escapes aside) is written as the authored line. When any row
 * changed, a padded and aligned table is re-aligned whole (remark's print);
 * any other keeps its rows and writes new ones in its own spacing
 * (`|a|b|` or `| a | b |`).
 */
import { gfmTableToMarkdown } from 'mdast-util-gfm-table';
import { defaultHandlers, type Info, type Options as ToMarkdownExtension, type State } from 'mdast-util-to-markdown';
import type { Blockquote, Break, Code, Heading, List, Parents, Table, TableRow, ThematicBreak } from 'mdast';
import type { Node as ProseNode } from '@milkdown/kit/prose/model';
import type { NodeSchema, ParserState, SerializerState } from '@milkdown/kit/transformer';
import type { $NodeSchema } from '@milkdown/kit/utils';
import { $remark } from '@milkdown/kit/utils';
import {
  bulletListSchema,
  codeBlockSchema,
  hardbreakSchema,
  headingSchema,
  hrSchema,
  orderedListSchema,
  paragraphSchema,
} from '@milkdown/kit/preset/commonmark';
import { tableSchema } from '@milkdown/kit/preset/gfm';
import { minimizeBlockEscapes, samePhrasing } from './minimal-escapes.js';
import { addRemarkExtensions, type MdNode } from './remark-support.js';

// ============================================================================
// Styles
// ============================================================================

export interface ListStyle {
  /** `-`, `*` or `+` for a bullet list; `.` or `)` for an ordered one. */
  marker: string;
  /** `tab`: content at the next multiple of four columns (`-   x`, `1.  x`). */
  indent?: 'tab';
  /** Every item repeats the first number (`1. 1. 1.`). */
  sameNumber?: true;
}

export interface HeadingStyle {
  /** The heading's source as loaded, for keeping its escapes (`minimal-escapes.ts`). */
  source?: string;
  /** The underline of a setext heading, as typed. */
  setext?: string;
  /** The underline was as long as the heading's last line (so it follows edits). */
  fit?: boolean;
  /** An ATX heading's closing sequence with the space before it (` ##`). */
  closing?: string;
}

export interface CodeStyle {
  /** An indented code block. */
  indented?: true;
  /** The opening fence as typed (`~~~`, ```` ```` ````). */
  fence?: string;
  /** The info string's words after the language (`title="x"`). */
  meta?: string;
}

/** A paragraph's source as loaded, for keeping its escapes (`minimal-escapes.ts`). */
export interface ParagraphStyle {
  source: string;
}

/** The line of a thematic break as typed (`---`, `* * *`). */
export interface RuleStyle {
  source: string;
}

/** The whitespace before a hard break's line ending (`'  '`); a `\` break has none. */
export interface BreakStyle {
  spaces: string;
}

/** `>>`: no space between this quote's `>` and a nested one's. */
export interface BlockquoteStyle {
  compact: true;
}

/** Each source row of a table, the delimiter row second, without container prefixes. */
export interface TableStyle {
  rows: string[];
}

/** The styles a fresh node gets: remark's defaults, except `-` bullets and `---` rules. */
export const HOUSE_STYLE = { bullet: '-', rule: '-' } as const;

type Styled<T> = { mbrStyle?: T };

function styleOf<T>(node: unknown): T | undefined {
  const style = (node as Styled<T> | undefined)?.mbrStyle;
  return style ?? undefined;
}

// ============================================================================
// 1. Parse: record the authored style
// ============================================================================

type Positioned = MdNode & { position?: { start: { offset?: number; column?: number }; end: { offset?: number } } };

function startOf(node: Positioned): number | undefined {
  return node.position?.start.offset;
}

function sliceOf(node: Positioned, source: string): string | undefined {
  const start = node.position?.start.offset;
  const end = node.position?.end.offset;
  return start === undefined || end === undefined ? undefined : source.slice(start, end);
}

/** A list's marker, gap and numbering, from its first two items. */
export function listStyleFrom(source: string, list: Positioned): ListStyle | undefined {
  const [first, second] = (list.children ?? []) as Positioned[];
  const at = first && startOf(first);
  if (at === undefined) return undefined;
  const match = /^(?:([*+-])|(\d{1,9})([.)]))([ \t]*)/.exec(source.slice(at));
  if (!match) return undefined;
  const style: ListStyle = { marker: match[1] ?? match[3] };
  const markerLength = (match[1] ?? match[2] + match[3]).length;
  const gap = match[4];
  const contentFollows = !/^[ \t]*(?:\r?\n|$)/.test(source.slice(at + markerLength));
  if (contentFollows && /^ +$/.test(gap) && gap.length > 1 && gap.length <= 4) {
    const size = markerLength + gap.length;
    if (size === Math.ceil((markerLength + 1) / 4) * 4) style.indent = 'tab';
  }
  const secondAt = second && startOf(second);
  if (match[2] !== undefined && secondAt !== undefined) {
    const next = /^(\d{1,9})[.)]/.exec(source.slice(secondAt));
    if (next && Number(next[1]) === Number(match[2])) style.sameNumber = true;
  }
  return style;
}

/** Setext underline or ATX closing sequence, and the source. */
export function headingStyleFrom(source: string, heading: Positioned): HeadingStyle | undefined {
  const text = sliceOf(heading, source);
  if (text === undefined) return undefined;
  if (!/^#/.test(text)) {
    const lines = text.split('\n');
    const underline = /([=-]+)[ \t]*$/.exec(lines[lines.length - 1] ?? '')?.[1];
    if (!underline || lines.length < 2) return { source: text };
    const last = lines[lines.length - 2].replace(/^[ \t>]*/, '').replace(/[ \t]+$/, '');
    return { source: text, setext: underline, fit: underline.length === last.length };
  }
  const closing = /[ \t]+#+[ \t]*$/.exec(text)?.[0];
  // `#` alone, or `# #`: the "closing" is the whole heading.
  if (!closing || /^#+[ \t]+#+[ \t]*$/.test(text) || /^#+[ \t]*$/.test(text)) return { source: text };
  return { source: text, closing: closing.replace(/[ \t]+$/, '') };
}

export function paragraphStyleFrom(source: string, paragraph: Positioned): ParagraphStyle | undefined {
  const text = sliceOf(paragraph, source);
  // Only worth carrying when there is an escape to keep.
  return text?.includes('\\') ? { source: text } : undefined;
}

export function codeStyleFrom(source: string, code: Positioned & { meta?: string | null }): CodeStyle | undefined {
  const at = startOf(code);
  if (at === undefined) return undefined;
  const fence = /^(`{3,}|~{3,})/.exec(source.slice(at))?.[1];
  const style: CodeStyle = {};
  if (!fence) style.indented = true;
  else if (fence !== '```') style.fence = fence;
  if (code.meta) style.meta = code.meta;
  return Object.keys(style).length ? style : undefined;
}

export function ruleStyleFrom(source: string, rule: Positioned): RuleStyle | undefined {
  const text = sliceOf(rule, source)?.replace(/[ \t]+$/, '');
  return text ? { source: text } : undefined;
}

export function breakStyleFrom(source: string, hardBreak: Positioned): BreakStyle | undefined {
  const spaces = /^([ \t]+)\r?\n/.exec(sliceOf(hardBreak, source) ?? '')?.[1];
  return spaces ? { spaces } : undefined;
}

export function blockquoteStyleFrom(source: string, quote: Positioned): BlockquoteStyle | undefined {
  const at = startOf(quote);
  return at !== undefined && source.startsWith('>>', at) ? { compact: true } : undefined;
}

/**
 * A table's rows as typed. Lines after the first lose the container prefix
 * the first line's column implies (`> `, list indentation); a line whose
 * prefix is anything else gives up (no style, the default print).
 */
export function tableStyleFrom(source: string, table: Positioned): TableStyle | undefined {
  const text = sliceOf(table, source);
  const column = table.position?.start.column;
  if (text === undefined || column === undefined) return undefined;
  const prefix = column - 1;
  const rows: string[] = [];
  for (const [index, line] of text.split('\n').entries()) {
    if (index === 0) {
      rows.push(line);
      continue;
    }
    if (!/^[ \t>]*$/.test(line.slice(0, prefix))) return undefined;
    rows.push(line.slice(prefix));
  }
  return { rows: rows.map((row) => row.replace(/\r$/, '')) };
}

const RECORDERS: Record<string, (source: string, node: Positioned) => unknown> = {
  paragraph: paragraphStyleFrom,
  list: listStyleFrom,
  heading: headingStyleFrom,
  code: codeStyleFrom,
  thematicBreak: ruleStyleFrom,
  break: breakStyleFrom,
  blockquote: blockquoteStyleFrom,
  table: tableStyleFrom,
};

/** Sets `mbrStyle` on every node in `tree` whose spelling is recorded (see the module docs). */
export function recordAuthoredStyles(tree: MdNode, source: string): void {
  const visit = (node: MdNode): void => {
    const record = RECORDERS[node.type];
    if (record) {
      const style = record(source, node as Positioned);
      if (style) (node as Styled<unknown>).mbrStyle = style;
    }
    for (const child of node.children ?? []) visit(child);
  };
  visit(tree);
}

// ============================================================================
// 2. ProseMirror: carry the style on the node
// ============================================================================

/**
 * Runs `run`, adding `extra` to whatever the first `openNode`/`addNode` call
 * on `state` passes as `key` (the node's attrs or the mdast node's props) —
 * that call is the node's own in every schema's runner. Leaves wrapping the
 * schema's runner to the caller, so no runner is copied.
 */
function withFirstNode<S extends ParserState | SerializerState>(
  state: S,
  argIndex: number,
  extra: Record<string, unknown>,
  run: () => void,
): void {
  const target = state as unknown as Record<'openNode' | 'addNode', (...args: unknown[]) => unknown>;
  const own = { openNode: target.openNode, addNode: target.addNode };
  let done = false;
  const patch = (name: 'openNode' | 'addNode') =>
    function patched(...args: unknown[]): unknown {
      if (!done) {
        done = true;
        target.openNode = own.openNode;
        target.addNode = own.addNode;
        args[argIndex] = { ...(args[argIndex] as object | undefined), ...extra };
      }
      return own[name].apply(state, args);
    };
  target.openNode = patch('openNode');
  target.addNode = patch('addNode');
  try {
    run();
  } finally {
    target.openNode = own.openNode;
    target.addNode = own.addNode;
  }
}

/**
 * `schema` with an `mbrStyle` attr, filled from the mdast node's `mbrStyle`
 * on parse and handed back on serialize. The schema's own runners do the
 * rest, so this composes with any other extension of the same node (alerts'
 * blockquote) as long as it is applied last.
 */
export function carryAuthoredStyle<T extends string>(schema: $NodeSchema<T>): $NodeSchema<T> {
  return schema.extendSchema((prev) => (ctx) => {
    const base: NodeSchema = prev(ctx);
    return {
      ...base,
      attrs: { ...base.attrs, mbrStyle: { default: null } },
      parseMarkdown: {
        match: base.parseMarkdown.match,
        runner: (state, node, type) => {
          const style = styleOf(node);
          const run = () => base.parseMarkdown.runner(state, node, type);
          if (style === undefined) run();
          else withFirstNode(state, 1, { mbrStyle: style }, run);
        },
      },
      toMarkdown: {
        match: base.toMarkdown.match,
        runner: (state, node: ProseNode) => {
          const style: unknown = node.attrs.mbrStyle;
          const run = () => base.toMarkdown.runner(state, node);
          // `openNode(type, value, props)` / `addNode(type, children, value, props)`.
          if (style == null) run();
          else withFirstNodeProps(state, { mbrStyle: style }, run);
        },
      },
    };
  });
}

/** {@link withFirstNode} for the serializer, whose two calls put `props` at different positions. */
function withFirstNodeProps(state: SerializerState, extra: Record<string, unknown>, run: () => void): void {
  const target = state as unknown as Record<'openNode' | 'addNode', (...args: unknown[]) => unknown>;
  const own = { openNode: target.openNode, addNode: target.addNode };
  let done = false;
  const restore = () => {
    target.openNode = own.openNode;
    target.addNode = own.addNode;
  };
  target.openNode = (...args: unknown[]) => {
    if (!done) {
      done = true;
      restore();
      args[2] = { ...(args[2] as object | undefined), ...extra };
    }
    return own.openNode.apply(state, args);
  };
  target.addNode = (...args: unknown[]) => {
    if (!done) {
      done = true;
      restore();
      args[3] = { ...(args[3] as object | undefined), ...extra };
    }
    return own.addNode.apply(state, args);
  };
  try {
    run();
  } finally {
    restore();
  }
}

// ============================================================================
// 3. Markdown out
// ============================================================================

type Handle<N> = (node: N, parent: Parents | undefined, state: State, info: Info) => string;

/** Runs `run` with `options` set on the state, restoring the previous values after. */
function withOptions<R>(state: State, options: Partial<State['options']>, run: () => R): R {
  const saved: Record<string, unknown> = {};
  const target = state.options as Record<string, unknown>;
  for (const key of Object.keys(options)) saved[key] = target[key];
  Object.assign(target, options);
  try {
    return run();
  } finally {
    Object.assign(target, saved);
  }
}

const BULLETS = new Set(['-', '*', '+']);

const printList: Handle<List> = (node, parent, state, info) => {
  const style = styleOf<ListStyle>(node);
  const options: Partial<State['options']> = {};
  if (style && node.ordered && (style.marker === '.' || style.marker === ')')) options.bulletOrdered = style.marker;
  if (style && !node.ordered && BULLETS.has(style.marker)) {
    options.bullet = style.marker as '-' | '*' | '+';
    // The other bullet (two lists in a row must differ) must differ from this one.
    if (style.marker === state.options.bulletOther) options.bulletOther = style.marker === '-' ? '*' : '-';
  }
  options.listItemIndent = style?.indent ?? 'one';
  options.incrementListMarker = !style?.sameNumber;
  return withOptions(state, options, () => defaultHandlers.list(node, parent, state, info));
};

/**
 * A rule as typed, else the house `---` — except straight under a paragraph
 * in a tight list item, where nothing separates them and a `-` rule would be
 * that paragraph's setext underline: there it is `***`.
 */
const printRule: Handle<ThematicBreak> = (node, parent, state) => {
  const value = styleOf<RuleStyle>(node)?.source ?? defaultHandlers.thematicBreak(node, parent, state);
  if (!value.startsWith('-') || !parent || !('spread' in parent) || parent.spread !== false) return value;
  const previous = parent.children[parent.children.indexOf(node as never) - 1];
  return previous?.type === 'paragraph' ? '***' : value;
};

const printHeading: Handle<Heading> = (node, parent, state, info) => {
  const style = styleOf<HeadingStyle>(node);
  const setextChar = node.depth === 1 ? '=' : '-';
  const setext = style?.setext && node.depth <= 2 && style.setext[0] === setextChar;
  const value = minimizeBlockEscapes(
    withOptions(state, { setext: Boolean(setext) }, () => defaultHandlers.heading(node, parent, state, info)),
    state,
    style?.source,
  );
  if (setext && style?.setext && !style.fit && !value.startsWith('#')) {
    const lastBreak = value.lastIndexOf('\n');
    if (lastBreak > 0) return value.slice(0, lastBreak + 1) + style.setext;
  }
  if (style?.closing && value.startsWith('#') && !value.includes('\n')) return value + style.closing;
  return value;
};

const printCode: Handle<Code> = (node, parent, state, info) => {
  const style = styleOf<CodeStyle>(node);
  const meta = node.meta ?? style?.meta;
  const withMeta = meta && node.lang ? { ...node, meta } : node;
  const fenceChar = style?.fence?.[0] === '~' ? '~' : '`';
  const value = withOptions(state, { fences: !style?.indented, fence: fenceChar }, () =>
    defaultHandlers.code(withMeta, parent, state, info),
  );
  const fence = style?.fence;
  if (!fence || !value.startsWith(fenceChar)) return value;
  // remark uses the shortest fence that works; the authored one, if longer.
  const open = /^[`~]+/.exec(value)?.[0] ?? '';
  if (open.length >= fence.length) return value;
  return fence + value.slice(open.length, value.length - open.length) + fence;
};

const printBreak: Handle<Break> = (node, parent, state, info) => {
  const value = defaultHandlers.break(node, parent, state, info);
  const style = styleOf<BreakStyle>(node);
  return style && value === '\\\n' ? `${style.spaces}\n` : value;
};

/**
 * A blockquote: `> ` before each line, or `>` before a line that is itself a
 * quote's when the authored quote was compact (`>>>`). The one blockquote
 * printer — `alerts.ts` prints an alert's body through it.
 */
export const printBlockquote: Handle<Blockquote> = (node, _parent, state, info) => {
  const compact = Boolean(styleOf<BlockquoteStyle>(node)?.compact);
  const exit = state.enter('blockquote');
  const tracker = state.createTracker(info);
  tracker.move('> ');
  tracker.shift(2);
  const value = state.indentLines(state.containerFlow(node, tracker.current()), (line, _index, blank) => {
    if (blank) return `>${line}`;
    return compact && line.startsWith('>') ? `>${line}` : `> ${line}`;
  });
  exit();
  return value;
};

const gfmTable = gfmTableToMarkdown().handlers?.table as Handle<Table>;

/** A source row's cells: split on unescaped pipes, outer pipes and padding dropped. */
export function rowCells(row: string): string[] {
  const cells: string[] = [];
  let current = '';
  for (let i = 0; i < row.length; i++) {
    const char = row[i];
    if (char === '\\' && i + 1 < row.length) {
      current += char + row[i + 1];
      i++;
    } else if (char === '|') {
      cells.push(current);
      current = '';
    } else {
      current += char;
    }
  }
  cells.push(current);
  const trimmed = row.trim();
  if (trimmed.startsWith('|')) cells.shift();
  if (trimmed.endsWith('|') && !trimmed.endsWith('\\|')) cells.pop();
  return cells.map((cell) => cell.trim());
}

type Align = Table['align'];

function alignOf(delimiter: string): Align {
  return rowCells(delimiter).map((cell) => {
    const left = cell.startsWith(':');
    const right = cell.endsWith(':');
    if (left && right) return 'center';
    if (left) return 'left';
    if (right) return 'right';
    return null;
  });
}

function sameAlign(a: Align, b: Align, columns: number): boolean {
  for (let i = 0; i < columns; i++) if ((a?.[i] ?? null) !== (b?.[i] ?? null)) return false;
  return true;
}

/** What each row's cells print as, the way remark-gfm's handler prints them. */
function printedCells(node: Table, state: State, info: Info): string[][] {
  const exitTable = state.enter('table');
  const rows = node.children.map((row: TableRow) => {
    const exitRow = state.enter('tableRow');
    const cells = row.children.map((cell) => state.handle(cell, row, state, info));
    exitRow();
    return cells;
  });
  exitTable();
  return rows;
}

const printTable: Handle<Table> = (node, parent, state, info) => {
  const fresh = () => gfmTable(node, parent, state, info);
  const style = styleOf<TableStyle>(node);
  if (!style || style.rows.length < 2) return fresh();
  const [header, delimiter, ...body] = style.rows;
  const authored = [header, ...body];
  const columns = Math.max(...node.children.map((row) => row.children.length));
  const cells = printedCells(node, state, info);
  // Each printed row takes the next authored row (in order) with the same cells.
  let next = 0;
  const lines = cells.map((row) => {
    for (let i = next; i < authored.length; i++) {
      const source = rowCells(authored[i]);
      if (source.length === row.length && source.every((cell, c) => samePhrasing(state, cell, row[c]))) {
        next = i + 1;
        return authored[i];
      }
    }
    return undefined;
  });
  const keptDelimiter = rowCells(delimiter).length === columns && sameAlign(alignOf(delimiter), node.align, columns);
  if (lines.every((line) => line !== undefined) && keptDelimiter) {
    return [lines[0], delimiter, ...lines.slice(1)].join('\n');
  }
  // Something changed. A padded, aligned table (every row as wide) is
  // re-aligned whole; any other keeps its rows, new ones in its spacing.
  const padded = /\|[ \t]/.test(header);
  const aligned = padded && style.rows.every((row) => row.length === style.rows[0].length);
  if (aligned) return fresh();
  const outer = header.trim().startsWith('|');
  const join = (row: string[]) => {
    const inner = row.join(padded ? ' | ' : '|');
    if (!outer) return inner;
    return padded ? `| ${inner} |` : `|${inner}|`;
  };
  const delimiterLine = keptDelimiter
    ? delimiter
    : join(
        Array.from({ length: columns }, (_, i) => {
          const align = node.align?.[i];
          return align === 'center' ? ':-:' : align === 'left' ? ':--' : align === 'right' ? '--:' : '---';
        }),
      );
  const rows = lines.map((line, i) => line ?? join(cells[i]));
  return [rows[0], delimiterLine, ...rows.slice(1)].join('\n');
};

export const authoredStyleToMarkdown: ToMarkdownExtension = {
  bullet: HOUSE_STYLE.bullet,
  rule: HOUSE_STYLE.rule,
  handlers: {
    list: printList as never,
    thematicBreak: printRule as never,
    heading: printHeading as never,
    code: printCode as never,
    break: printBreak as never,
    table: printTable as never,
  },
};

// ============================================================================
// 4. The plugin
// ============================================================================

/** Records styles on parse; prints them on serialize. */
export const remarkAuthoredStyle = $remark('mbrAuthoredStyle', () => function authoredStyle(this: unknown) {
  addRemarkExtensions(this, { toMarkdown: authoredStyleToMarkdown });
  return (tree: unknown, file: { value?: unknown }) => {
    recordAuthoredStyles(tree as MdNode, String(file.value ?? ''));
  };
});

/**
 * Every schema this module carries a style on. `blockquote` is not here:
 * alerts extends it too, and two extensions of one node replace each other,
 * so `alerts.ts` applies {@link carryAuthoredStyle} to its own and prints
 * through {@link printBlockquote} (it owns the `blockquote` handler).
 */
export const authoredStyleSchemas = [
  carryAuthoredStyle(paragraphSchema),
  carryAuthoredStyle(bulletListSchema),
  carryAuthoredStyle(orderedListSchema),
  carryAuthoredStyle(hrSchema),
  carryAuthoredStyle(headingSchema),
  carryAuthoredStyle(codeBlockSchema),
  carryAuthoredStyle(hardbreakSchema),
  carryAuthoredStyle(tableSchema),
];

/** Everything item 6 registers, for `MARKDOWN_EXTENSION_PLUGINS`. */
export const authoredStylePlugin = [remarkAuthoredStyle, ...authoredStyleSchemas].flat();
