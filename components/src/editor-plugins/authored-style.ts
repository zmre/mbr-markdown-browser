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
 * | paragraph, term  | indented lines, trailing spaces | none        |
 * | definition       | as typed (see below)          | `: ` + two-space indent |
 * | top-level gap    | two or more blank lines       | one          |
 *
 * Each is recorded at parse time from the source ({@link recordAuthoredStyles},
 * a remark transform) as `mbrStyle` on the mdast node, carried through the
 * editor on the ProseMirror node's `mbrStyle` attr ({@link carryAuthoredStyle}
 * — copied along when ProseMirror splits or copies the node), and handed back
 * to mdast on serialize, where this module's to-markdown handlers print it.
 * A node made in the editor has no style and prints in the house style
 * ({@link HOUSE_STYLE}: `*` bullets, `---` rules), and a style that no longer
 * fits the node (a setext heading changed to level 3, an indented block given
 * a language) is dropped for the default rather than forced.
 *
 * Paragraphs and terms keep their lines while their text is the same
 * ({@link keepAuthoredLines}); a definition (`: …`) is written as typed while
 * its content is what was loaded ({@link DefinitionStyle}, compared by
 * {@link contentKey}), and in the house style once edited.
 *
 * Tables are the one construct kept row by row: a row whose cells still read
 * as the authored row's cells (escapes aside) is written as the authored line. When any row
 * changed, a padded and aligned table is re-aligned whole (remark's print);
 * any other keeps its rows and writes new ones in its own spacing
 * (`|a|b|` or `| a | b |`).
 */
import { gfmTableToMarkdown } from 'mdast-util-gfm-table';
import {
  defaultHandlers,
  type Info,
  type Join,
  type Options as ToMarkdownExtension,
  type State,
} from 'mdast-util-to-markdown';
import type { Blockquote, Break, Code, Heading, List, ListItem, Parents, RootContent, Table, TableRow, ThematicBreak } from 'mdast';
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
import { footnoteDefinitionSchema, tableSchema } from '@milkdown/kit/preset/gfm';
import { minimizeBlockEscapes, sameBlockMeaning, samePhrasing } from './minimal-escapes.js';
import { addRemarkExtensions, LAZY_LINE, withoutLazyMarks, type MdNode } from './remark-support.js';

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
  /** Spaces after the marker, two to four, where that is not `tab` (`*  x`). */
  spaces?: number;
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
  /** A fence never closed (the block runs to the end of the note). */
  unclosed?: true;
  /** Indented code whose every line is indented with a tab, not four spaces. */
  tab?: true;
}

/**
 * A paragraph's (or a definition term's) spelling: its source as loaded, for
 * keeping its escapes (`minimal-escapes.ts`), and its lines as typed
 * ({@link keepAuthoredText}).
 */
export interface ParagraphStyle {
  source?: string;
  /** The lines as typed, relative to the container ({@link authoredLines}); a task item's box left out. */
  lines?: string[];
}

/**
 * A definition's (`: …`) lines as typed, from its colon to the end of its
 * last line, relative to the container ({@link authoredLines}). Printed back
 * verbatim while its content is what was loaded — `key`, the
 * {@link contentKey} of its children, set when the editor parses it
 * (`definition-list.ts`). That keeps everything the house print normalizes:
 * the spacing after `:` (`:  x`, `: ` alone), lazy continuation lines, a
 * nested block indented further than the definition needs, the blank lines
 * between its blocks one by one, trailing whitespace.
 */
export interface DefinitionStyle {
  lines: string[];
  key?: string;
}

/**
 * Blank lines between two top-level blocks, when other than the one the
 * printer writes: several, or none where that is safe ({@link adjacentIsSafe});
 * and between two items of a list, when other than the printer writes for
 * the list ({@link recordItemGaps}). Kept on both blocks, tagged with the line the gap follows,
 * so it is written only between the same two blocks: a block inserted
 * between them, or a neighbour deleted, gets the printer's single blank line.
 */
export interface GapStyle {
  /** Blank lines after this block. */
  gapAfter?: number;
  /** The line this block ended on (with `gapAfter`) / the block before it ended on (with `gapBefore`). */
  gapAfterLine?: number;
  gapBeforeLine?: number;
}

/** The line of a thematic break as typed (`---`, `* * *`). */
export interface RuleStyle {
  source: string;
}

/** The whitespace before a hard break's line ending (`'  '`); a `\` break has none. */
export interface BreakStyle {
  spaces: string;
}

/**
 * `>>`: no space between this quote's `>` and a nested one's. `empty`: a
 * quote with nothing in it, and what followed its `>` (`''`, `' '`).
 */
export interface BlockquoteStyle {
  compact?: true;
  empty?: string;
  /** What follows `>` on the quote's blank lines, when every one has it (`'> '`). */
  blank?: string;
}

/**
 * A list item's spelling. `empty`: an item with nothing in it, and what
 * followed its marker on the line (`''`, `' '`) — printed as typed instead of
 * Milkdown's `<br />` placeholder. `indent`: the whitespace its lines after
 * the first begin with, relative to the item's own marker line (a tab, four
 * spaces where two would do), so nested lists keep their indentation.
 */
export interface ListItemStyle {
  empty?: string;
  indent?: string;
  /** The item's first block is not a paragraph of text (`- - x`, `- ## x`, a lone image): no paragraph in front of it. */
  leadingBlock?: true;
  /** Only this whitespace follows the marker on its line: the first block starts on the next line, after `indent`. */
  leadingBreak?: string;
  /** An ordered item's number as typed (`1.`, `3.`, `3.`): only the first one means anything. */
  number?: number;
  /** The whole item as typed, printed back while its content is what was loaded ({@link ItemSource}). */
  typed?: ItemSource;
}

/**
 * A list item as typed, for printing it back byte for byte while it holds
 * what was loaded — `key`, {@link itemKey} of the item, set when the editor
 * parses it ({@link carryAuthoredStyle}). Everything the item's printer
 * normalizes stays: a tab before one nested item and four spaces before the
 * next, a tab after the marker, the spacing after this item's marker when the
 * list's first item has other spacing, lazy lines, blank lines and the
 * whitespace on them, an empty task's trailing space, a nested list as the
 * item's first block. An edit anywhere in the item prints it as before; its
 * untouched nested items are still printed as typed.
 */
export interface ItemSource {
  /** What precedes the marker on its line: container prefixes (`> `) and the item's own indentation. */
  lead: string;
  /** The marker as typed (`*`, `12.`). */
  marker: string;
  /** The item's lines: the first from just after its marker; the others whole, container prefixes included. */
  lines: string[];
  /** Every container around the item leaves a lazy line unprefixed ({@link LAZY_LINE}). */
  lazyOk: boolean;
  key?: string;
}

/** Each source row of a table, the delimiter row second, without container prefixes. */
export interface TableStyle {
  rows: string[];
}

/** The styles a fresh node gets: remark's defaults (`*` bullets), except `---` rules. */
export const HOUSE_STYLE = { bullet: '*', rule: '-' } as const;

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
    else style.spaces = gap.length;
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

/**
 * The lines of `node` as typed, trailing whitespace on its last line
 * included, with `prefix` columns of container prefix (`> `, indentation)
 * removed from every line after the first; the first starts at the node, or
 * — `prefix` 0, a block not inside any container — at its line's start, so
 * its indentation is kept. `undefined` when a later line does not have the
 * prefix (a lazy continuation inside a container), or a position is missing —
 * unless `lazy`: then such a line is kept whole, marked {@link LAZY_LINE}.
 */
export function authoredLines(
  source: string,
  node: Positioned,
  prefix: number | string,
  lazy = false,
): string[] | undefined {
  let start = node.position?.start.offset;
  let end = node.position?.end.offset;
  if (start === undefined || end === undefined) return undefined;
  if (prefix === 0) {
    const lineStart = source.lastIndexOf('\n', start - 1) + 1;
    if (/^[ \t]*$/.test(source.slice(lineStart, start))) start = lineStart;
  }
  const trailing = /^[ \t]*(?=\r?\n|$)/.exec(source.slice(end))?.[0];
  if (trailing) end += trailing.length;
  const lines: string[] = [];
  for (const [index, raw] of source.slice(start, end).split('\n').entries()) {
    const line = raw.replace(/\r$/, '');
    if (index === 0 || prefix === 0) {
      lines.push(line);
      continue;
    }
    // The container's prefix as typed (a list item's `indent`, tabs included), or its width.
    const prefixed = typeof prefix === 'string' ? line.startsWith(prefix) : /^[ >]*$/.test(line.slice(0, prefix));
    if (!prefixed) {
      if (!lazy) return undefined;
      lines.push(LAZY_LINE + line);
      continue;
    }
    lines.push(line.slice(typeof prefix === 'string' ? prefix.length : prefix));
  }
  return lines;
}

/** Container nodes: their children's lines carry a prefix (the container's column). */
const CONTAINERS = new Set(['listItem', 'blockquote', 'defListDescription', 'footnoteDefinition']);

/** The prefix {@link authoredLines} strips for `node`: none outside containers, else up to its column. */
function prefixOf(node: Positioned, inContainer: boolean): number {
  return inContainer ? (node.position?.start.column ?? 1) - 1 : 0;
}

/**
 * `linePrefix`: the literal prefix of the container's lines after its first,
 * when the container is a list item that records it ({@link ListItemStyle}):
 * the paragraph's lines are kept relative to it, since that is what the item
 * prints them under. `lazyLines`: every container around it is a list item
 * or quote, whose printers leave a lazy line ({@link LAZY_LINE}) as typed.
 */
export function paragraphStyleFrom(
  source: string,
  paragraph: Positioned,
  inContainer = false,
  linePrefix?: string,
  lazyLines = false,
): ParagraphStyle | undefined {
  const text = sliceOf(paragraph, source);
  if (text === undefined) return undefined;
  const style: ParagraphStyle = {};
  // Each only worth carrying when there is something to keep: an escape, or
  // whitespace the printer drops.
  if (text.includes('\\')) style.source = text;
  const prefix = inContainer && linePrefix !== undefined ? linePrefix : prefixOf(paragraph, inContainer);
  const lines = authoredLines(source, paragraph, prefix, inContainer && lazyLines);
  // A task item's box is in its paragraph's source but printed by the item.
  if (lines && /^\[[ \txX>-]\][ \t]/.test(lines[0])) lines[0] = lines[0].replace(/^\[.\][ \t]+/, '');
  if (lines) style.lines = lines;
  return style.source === undefined && style.lines === undefined ? undefined : style;
}

/** A definition's lines (see {@link DefinitionStyle}); its `key` is added by the editor's parse. */
export function definitionStyleFrom(source: string, definition: Positioned): DefinitionStyle | undefined {
  // Relative to its colon wherever it sits: the printer writes `:` at the
  // start of the line, inside whatever container prefix.
  const lines = authoredLines(source, definition, (definition.position?.start.column ?? 1) - 1);
  return lines ? { lines } : undefined;
}

/** Trailing spaces and tabs of a line. */
function trailingSpace(line: string): string {
  return /[ \t]*$/.exec(line)?.[0] ?? '';
}

/**
 * `printed` (one paragraph or term, as the printer wrote it) as the author
 * typed it, when it is the same text: the same number of lines, each equal
 * but for leading whitespace (which a paragraph's lines do not mean) and
 * trailing whitespace on the last line (which nothing means). Trailing
 * whitespace on any other line must match exactly — two spaces there are a
 * hard break. Anything else returns `printed`, so an edited paragraph prints
 * as the printer prints it.
 *
 * `indentFirst`: whether the first line may keep its indentation. Not after a
 * list or definition list, where an indented paragraph would be read as a
 * continuation of the item or definition above (an edit can put it there).
 */
export function keepAuthoredLines(printed: string, lines: readonly string[] | undefined, indentFirst: boolean): string {
  if (!lines) return printed;
  const out = printed.split('\n');
  if (out.length !== lines.length) return printed;
  for (let i = 0; i < out.length; i++) {
    const line = withoutLazyMarks(lines[i]);
    if (out[i].trim() !== line.trim()) return printed;
    if (i < out.length - 1 && trailingSpace(out[i]) !== trailingSpace(line)) return printed;
  }
  return authoredText(lines, indentFirst);
}

/** `lines` as one block, the first line's indentation kept only where allowed (see {@link keepAuthoredLines}). */
function authoredText(lines: readonly string[], indentFirst: boolean): string {
  const first = lines[0];
  // Four columns of indentation would make the first line indented code.
  const keepIndent = indentFirst && /^ {0,3}(?! )/.test(first) && !first.startsWith('\t');
  return [keepIndent ? first : first.trimStart(), ...lines.slice(1)].join('\n');
}

/**
 * {@link keepAuthoredLines}, and where that prints `printed` (the text was
 * spelled differently: an escape the printer does not write, `~x~`, an
 * address written as a link), the lines as typed when they parse to the same
 * tree in the block's context (`sameBlockMeaning`). Only blocks that print
 * differently are parsed, and only at a save or open.
 */
export function keepAuthoredText(
  printed: string,
  lines: readonly string[] | undefined,
  indentFirst: boolean,
  state: State,
): string {
  const kept = keepAuthoredLines(printed, lines, indentFirst);
  if (kept !== printed || !lines) return kept;
  const authored = authoredText(lines, indentFirst);
  // Compared with its lazy lines lazy: the wrap leaves marked lines unprefixed.
  return withoutLazyMarks(authored) !== printed && sameBlockMeaning(state, authored, printed) ? authored : printed;
}

/** Whether an indented paragraph right after `node` (blank lines between) would continue it instead. */
export function absorbsIndentedParagraph(node: { type: string } | undefined): boolean {
  return node?.type === 'list' || node?.type === 'defList';
}

/**
 * A comparable key for flow content — `nodes` and everything under them,
 * every field with a primitive value but positions and recorded styles. Equal
 * keys: the same content, however it was spelled. Used to tell whether a
 * definition still holds what was loaded ({@link DefinitionStyle}).
 */
/**
 * Fields that are not content: the tree walk, positions, recorded spellings,
 * and what Milkdown adds on one side only — `isMark` on the mdast its
 * serializer builds from a mark; on the parsed tree, `label` (an ordered
 * item's number, derived from the list's `start`) and inline math's `display`
 * (derived from its `value`, which keeps the `$`/`$$` delimiters).
 */
const KEY_IGNORED = new Set(['type', 'children', 'position', 'mbrStyle', 'isMark', 'label', 'display']);

/**
 * `children` as one side of the round trip sees them: a soft line break is
 * a `break` marked `isInline` in the parsed tree (Milkdown's line-break
 * transform) and a `"\n"` text node in the serialized one, and the text
 * around it is one node or three; both become merged text.
 */
function normalizedChildren(children: readonly Record<string, unknown>[]): Record<string, unknown>[] {
  const out: Record<string, unknown>[] = [];
  for (const child of children) {
    const softBreak = child.type === 'break' && (child.data as { isInline?: unknown } | undefined)?.isInline === true;
    const node = softBreak ? { type: 'text', value: '\n' } : child;
    const previous = out[out.length - 1];
    if (node.type === 'text' && previous?.type === 'text' && previous.children === undefined) {
      out[out.length - 1] = { type: 'text', value: String(previous.value ?? '') + String(node.value ?? '') };
    } else {
      out.push(node);
    }
  }
  return out;
}

/**
 * A comparable key for flow content — `nodes` and everything under them,
 * every field with a primitive value but those in {@link KEY_IGNORED}. Equal
 * keys: the same content, however it was spelled — so a definition printed
 * from the editor's nodes can be matched to the tree it was loaded from
 * ({@link DefinitionStyle}). The two sides differ in small ways (see the
 * helpers); a difference not normalized here only costs the authored
 * spelling, never content.
 */
export function contentKey(nodes: readonly unknown[]): string {
  // Object fields count too (a table's `align`, an alert's `data`): a key
  // that missed an edit would print the content as loaded.
  const stableJson = (value: unknown): string => {
    if (value === null || typeof value !== 'object') return JSON.stringify(value) ?? '';
    if (Array.isArray(value)) return `[${value.map(stableJson).join(',')}]`;
    const record = value as Record<string, unknown>;
    const fields = Object.keys(record)
      .filter((field) => !KEY_IGNORED.has(field) && record[field] !== undefined && record[field] !== null)
      .sort();
    return `{${fields.map((field) => `${field}:${stableJson(record[field])}`).join(',')}}`;
  };
  const parts: string[] = [];
  const walk = (node: Record<string, unknown>): void => {
    parts.push(`(${String(node.type)}`);
    for (const field of Object.keys(node).sort()) {
      if (KEY_IGNORED.has(field)) continue;
      const value = node[field];
      // `''` is no value too (a code block's `lang`, a link's `title`), but for text.
      if (value === null || value === undefined || (value === '' && field !== 'value')) continue;
      parts.push(`${field}=${typeof value === 'object' ? stableJson(value) : JSON.stringify(value)}`);
    }
    const children = node.children;
    if (Array.isArray(children)) for (const child of normalizedChildren(children)) walk(child);
    parts.push(')');
  };
  for (const node of normalizedChildren(nodes as Record<string, unknown>[])) walk(node);
  return parts.join('');
}

export function codeStyleFrom(source: string, code: Positioned & { meta?: string | null }): CodeStyle | undefined {
  const at = startOf(code);
  if (at === undefined) return undefined;
  const fence = /^(`{3,}|~{3,})/.exec(source.slice(at))?.[1];
  const style: CodeStyle = {};
  if (!fence) {
    style.indented = true;
    const column = (code.position?.start.column ?? 1) - 1;
    const lines = (sliceOf(code, source) ?? '').split('\n').map((line, i) => (i === 0 ? line : line.slice(column)));
    if (column === 0 && lines.every((line) => line.trim() === '' || line.startsWith('\t'))) style.tab = true;
  }
  else if (fence !== '```') style.fence = fence;
  if (fence) {
    const lines = (sliceOf(code, source) ?? '').replace(/\r?\n$/, '').split('\n');
    const closing = new RegExp(`^[ \\t]*${fence[0] === '`' ? '`' : '~'}{${fence.length},}[ \\t]*\\r?$`);
    if (lines.length < 2 || !closing.test(lines[lines.length - 1])) style.unclosed = true;
  }
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
  if (at === undefined) return undefined;
  if ((quote.children ?? []).length === 0) {
    const rest = /^>([ \t]*)$/.exec(sliceOf(quote, source) ?? '')?.[1];
    return rest === undefined ? undefined : { empty: rest };
  }
  const style: BlockquoteStyle = {};
  if (source.startsWith('>>', at)) style.compact = true;
  const column = (quote.position?.start.column ?? 1) - 1;
  const blanks = new Set<string>();
  for (const line of (sliceOf(quote, source) ?? '').split('\n').slice(1)) {
    const rest = /^>([ \t]*)\r?$/.exec(line.slice(column))?.[1];
    if (rest !== undefined) blanks.add(rest);
  }
  const [blank] = blanks;
  if (blanks.size === 1 && blank !== '') style.blank = blank;
  return Object.keys(style).length ? style : undefined;
}

/** The start of the line `offset` is on. */
function lineStartOf(source: string, offset: number): number {
  return source.lastIndexOf('\n', offset - 1) + 1;
}

/**
 * A list item's {@link ListItemStyle}, and the literal prefix of its lines
 * after the first (`linePrefix`), for what is nested in it. `parentPrefix`:
 * the same for the item this one's list is in (`''` for a list at the top
 * level), `undefined` when not known — inside a block quote, or under an
 * item whose lines do not share one prefix.
 *
 * `indent` is recorded only when it is exact: the item starts right at its
 * parent's prefix (no indentation of its own, which the printer would drop),
 * its second block starts on a line of its own after a whitespace prefix, and
 * every other non-blank line of the item starts with that prefix too (a lazy
 * line does not).
 */
export function listItemStyleFrom(
  source: string,
  item: Positioned,
  parentPrefix: string | undefined,
): { style?: ListItemStyle; linePrefix?: string } {
  const start = item.position?.start.offset;
  const end = item.position?.end.offset;
  if (start === undefined || end === undefined) return {};
  const children = (item.children ?? []) as Positioned[];
  if (children.length === 0) {
    const empty = /^(?:[*+-]|(\d{1,9})[.)])([ \t]*)$/.exec(source.slice(start, end));
    if (!empty) return {};
    return { style: { empty: empty[2], ...(empty[1] !== undefined ? { number: Number(empty[1]) } : {}) } };
  }
  const first = children[0] as Positioned & { children?: { type: string }[] };
  const imageOnly = first.type === 'paragraph' && first.children?.length === 1 && first.children[0].type === 'image';
  const number = /^(\d{1,9})[.)]/.exec(source.slice(start, start + 10))?.[1];
  const leading: ListItemStyle | undefined =
    first.type !== 'paragraph' || imageOnly || number !== undefined
      ? {
          ...(first.type !== 'paragraph' || imageOnly ? { leadingBlock: true as const } : {}),
          ...(number !== undefined ? { number: Number(number) } : {}),
        }
      : undefined;
  const lead = source.slice(lineStartOf(source, start), start);
  // `*` then the item's first block on the next line: the prefix is that block's.
  const firstAt = first.position?.start.offset;
  const breaks = firstAt !== undefined && lineStartOf(source, firstAt) > start;
  const second = breaks ? firstAt : children[1]?.position?.start.offset;
  if (parentPrefix === undefined || second === undefined) return { style: leading };
  const secondLine = lineStartOf(source, second);
  const linePrefix = source.slice(secondLine, second);
  if (lead !== parentPrefix || secondLine <= start || !/^[ \t]+$/.test(linePrefix) || !linePrefix.startsWith(lead)) {
    return { style: leading };
  }
  const lines = source.slice(start, end).split('\n').slice(1);
  if (lines.some((line) => /\S/.test(line) && !line.startsWith(linePrefix))) return { style: leading };
  return {
    style: {
      ...leading,
      indent: linePrefix.slice(lead.length),
      ...(breaks ? { leadingBreak: /^(?:[*+-]|\d{1,9}[.)])([ \t]*)/.exec(source.slice(start))?.[1] ?? '' } : {}),
    },
    linePrefix,
  };
}

/**
 * A list item's {@link ItemSource}: from its marker to the end of its last
 * line (trailing whitespace included). `undefined` when something other than
 * container prefixes precedes the marker (`- - x`: the inner item is printed
 * with the outer one) or the item holds a bare `\r` (a line ending the
 * editor normalizes).
 */
export function itemSourceFrom(source: string, item: Positioned, lazyOk: boolean): ItemSource | undefined {
  const start = item.position?.start.offset;
  let end = item.position?.end.offset;
  if (start === undefined || end === undefined) return undefined;
  const lead = source.slice(lineStartOf(source, start), start);
  const marker = /^(?:[*+-]|\d{1,9}[.)])/.exec(source.slice(start, start + 10))?.[0];
  if (!/^[ \t>]*$/.test(lead) || marker === undefined) return undefined;
  end += /^[ \t]*(?=\r?\n|$)/.exec(source.slice(end))?.[0].length ?? 0;
  const lines = source
    .slice(start + marker.length, end)
    .split('\n')
    .map((line) => line.replace(/\r$/, ''));
  if (lines.some((line) => line.includes('\r'))) return undefined;
  return { lead, marker, lines, lazyOk };
}

/**
 * What an item's content compares by ({@link ItemSource}): its task state and
 * the {@link contentKey} of its blocks. The serialized side drops the empty
 * paragraph the editor puts first in an item that starts with another block,
 * and an empty item's placeholder, as {@link printListItem} does.
 */
export function itemKey(item: ListItem & { status?: unknown }, leadingBlock = false): string {
  let children: readonly RootContent[] = item.children ?? [];
  if (isEmptyFlow(children)) children = [];
  else if (leadingBlock && children.length > 1 && isEmptyFlow(children.slice(0, 1))) children = children.slice(1);
  const checked = typeof item.checked === 'boolean' ? String(item.checked) : '';
  const status = typeof item.status === 'string' ? item.status : '';
  return `${checked}|${status}|${contentKey(children)}`;
}

/**
 * `line`, a line of an item after its first, relative to the item's marker
 * line: without `lead`, or `''` for a blank line, `lazy` for a line short of
 * the item's indentation (a lazy continuation), `undefined` for one whose
 * prefix cannot be cut at `lead` (a tab spanning it, `>` spaced differently).
 */
function relativeItemLine(line: string, lead: string): { text: string; lazy?: true } | undefined {
  if (/^[ \t>]*$/.test(line) && countOf(line, '>') === countOf(lead, '>')) return { text: '' };
  if (line.startsWith(lead)) return { text: line.slice(lead.length) };
  if (lead.includes('>')) return countOf(line, '>') === 0 || !/^[ \t>]*>/.test(line) ? { text: line, lazy: true } : undefined;
  const width = visualWidth(lead);
  let column = 0;
  let index = 0;
  while (index < line.length && column < width && (line[index] === ' ' || line[index] === '\t')) {
    column = line[index] === '\t' ? column + 4 - (column % 4) : column + 1;
    index++;
  }
  if (column < width) return { text: line, lazy: true };
  return column === width ? { text: line.slice(index) } : undefined;
}

function countOf(text: string, char: string): number {
  let count = 0;
  for (const c of text) if (c === char) count++;
  return count;
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

const RECORDERS: Record<
  string,
  (source: string, node: Positioned, inContainer: boolean, linePrefix: string | undefined, lazyLines: boolean) => unknown
> = {
  paragraph: paragraphStyleFrom,
  defListTerm: paragraphStyleFrom,
  defListDescription: definitionStyleFrom,
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
  // `prefix`: the literal prefix of the lines `node` is printed under, when
  // known — `''` at the top level, a list item's for its paragraphs and lists.
  // `lazyLines`: every container so far prints lazy lines as typed (list items, quotes).
  const visit = (node: MdNode, inContainer: boolean, prefix: string | undefined, lazyLines: boolean): void => {
    let childPrefix: string | undefined;
    if (node.type === 'listItem') {
      const { style, linePrefix } = listItemStyleFrom(source, node as Positioned, prefix);
      if (style) mergeStyle(node, style);
      const typed = itemSourceFrom(source, node as Positioned, lazyLines);
      if (typed) mergeStyle(node, { typed });
      childPrefix = linePrefix;
    } else {
      const record = RECORDERS[node.type];
      if (record) {
        const style = record(source, node as Positioned, inContainer, prefix, lazyLines);
        if (style) mergeStyle(node, style);
      }
      childPrefix = node.type === 'root' || node.type === 'list' ? prefix : undefined;
    }
    if (node.type === 'list') recordItemGaps(node as MdNode & { spread?: boolean | null });
    const nested = inContainer || CONTAINERS.has(node.type);
    const lazy = lazyLines && (!CONTAINERS.has(node.type) || node.type === 'listItem' || node.type === 'blockquote');
    for (const child of node.children ?? []) visit(child, nested, childPrefix, lazy);
  };
  visit(tree, false, '', true);
  recordGaps(tree);
}

/** Adds `style` to what is recorded on `node` (a list's item gaps are recorded before its items). */
function mergeStyle(node: MdNode, style: object): void {
  const styled = node as Styled<object>;
  styled.mbrStyle = { ...styled.mbrStyle, ...style };
}

/** Tags `left`/`right` with the `blank` lines between them, which end and start at `line` (see {@link GapStyle}). */
function tagGap(left: Positioned, right: Positioned, blank: number, line: number): void {
  const l = left as Styled<GapStyle>;
  const r = right as Styled<GapStyle>;
  l.mbrStyle = { ...l.mbrStyle, gapAfter: blank, gapAfterLine: line };
  r.mbrStyle = { ...r.mbrStyle, gapBeforeLine: line };
}

/**
 * Records {@link GapStyle} between the items of a list where it is not what
 * the printer writes for the whole list — one blank line in a loose list,
 * none in a tight one. A list with a blank line between two of its items is
 * loose, and the printer would put one between all of them.
 */
function recordItemGaps(list: MdNode & { spread?: boolean | null }): void {
  const items = (list.children ?? []) as Positioned[];
  const printed = list.spread ? 1 : 0;
  for (let i = 0; i < items.length - 1; i++) {
    const end = (items[i].position?.end as { line?: number } | undefined)?.line;
    const next = (items[i + 1].position?.start as { line?: number } | undefined)?.line;
    if (end === undefined || next === undefined || next <= end) continue;
    const blank = next - end - 1;
    if (blank !== printed) tagGap(items[i], items[i + 1], blank, end);
  }
}

/**
 * Records {@link GapStyle} on each pair of top-level blocks with other than
 * one blank line between: none (a list right under a heading) or several.
 */
function recordGaps(root: MdNode): void {
  const children = (root.children ?? []) as Positioned[];
  for (let i = 0; i < children.length - 1; i++) {
    const end = (children[i].position?.end as { line?: number } | undefined)?.line;
    const next = (children[i + 1].position?.start as { line?: number } | undefined)?.line;
    if (end === undefined || next === undefined || next - end - 1 === 1 || next <= end) continue;
    const left = children[i];
    const right = children[i + 1];
    tagGap(left, right, next - end - 1, end);
  }
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
          const style = keyedStyle(node);
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

/** A node's `mbrStyle`, with the key of an item recorded as typed ({@link ItemSource}). */
function keyedStyle(node: unknown): unknown {
  const style = styleOf<ListItemStyle>(node);
  if (!style?.typed || (node as { type?: unknown }).type !== 'listItem') return style;
  return { ...style, typed: { ...style.typed, key: itemKey(node as ListItem) } };
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
  if (!authoredItemGapsHold(node)) unheldItemGaps.add(node);
  return withOptions(state, options, () => defaultHandlers.list(node, parent, state, info));
};

/** Lists whose authored item gaps would change whether they are loose (see {@link authoredItemGapsHold}). */
const unheldItemGaps = new WeakSet<List>();

/** The authored blank lines between `left` and `right` ({@link GapStyle}), if they are still neighbours. */
function authoredGap(left: unknown, right: unknown): number | undefined {
  const after = styleOf<GapStyle>(left);
  const line = after?.gapAfterLine;
  if (after?.gapAfter === undefined || line === undefined || styleOf<GapStyle>(right)?.gapBeforeLine !== line) {
    return undefined;
  }
  return after.gapAfter;
}

/**
 * Whether printing `list`'s items with their authored gaps keeps it as loose
 * (or tight) as it is: a loose list needs a blank line between two items or
 * inside one. An edit can remove the item gaps that made it loose (delete the
 * one item set apart); then every gap is printed as the printer would.
 */
function authoredItemGapsHold(list: List): boolean {
  const items = list.children;
  let loose = items.some((item) => item.spread && (item.children?.length ?? 0) > 1);
  for (let i = 0; i < items.length - 1; i++) {
    const gap = authoredGap(items[i], items[i + 1]) ?? (list.spread ? 1 : 0);
    if (gap > 0) loose = true;
  }
  return loose === Boolean(list.spread);
}

/** Milkdown's stand-in for an empty paragraph (`remarkPreserveEmptyLine`). */
const EMPTY_LINE_PLACEHOLDER = /^<br\s*\/?\s*>$/i;

/**
 * Whether flow content is empty as far as the source goes: no blocks, or one
 * paragraph that is empty or only Milkdown's `<br />` placeholder (the editor
 * puts an empty paragraph in every container it loads empty).
 */
function isEmptyFlow(children: readonly RootContent[] | undefined): boolean {
  if (!children || children.length === 0) return true;
  if (children.length !== 1 || children[0].type !== 'paragraph') return false;
  const inline = children[0].children ?? [];
  return (
    inline.length === 0 ||
    (inline.length === 1 && inline[0].type === 'html' && EMPTY_LINE_PLACEHOLDER.test(inline[0].value.trim()))
  );
}

/**
 * The last of `children`, which may be missing: Milkdown's serializer leaves
 * `children` off a node it pushed nothing into.
 */
function lastOf<T>(children: readonly T[] | undefined): T | undefined {
  return children && children.length > 0 ? children[children.length - 1] : undefined;
}

/** Whether a list item prints as its marker alone. */
function isEmptyItem(item: ListItem | undefined): boolean {
  return item !== undefined && item.checked == null && isEmptyFlow(item.children);
}

/** Columns `text` takes from the start of a line (tab stops every four). */
export function visualWidth(text: string): number {
  let column = 0;
  for (const char of text) column = char === '\t' ? column + 4 - (column % 4) : column + 1;
  return column;
}

/**
 * A list item being printed: `depth`, `state.indexStack.length` when it
 * started; `lines`, the literal prefix of its lines after the first, when the
 * whole of it is known (every container around it is a list item, none on its
 * parent's first line).
 */
interface ItemFrame {
  depth: number;
  lines: string | undefined;
}

const itemFrames = new WeakMap<State, ItemFrame[]>();

/** The number each ordered item was printed with, for the next item's. */
const itemNumbers = new WeakMap<ListItem, number>();

/**
 * What a list item's lines after the first begin with: `indent` as typed
 * ({@link ListItemStyle}) when it keeps every block of the item where the
 * default (`size` spaces) puts it — at least the content column, less than
 * four columns past it — else the default. Spaces measure the same anywhere;
 * a tab's width depends on its column, so it is used only where the item's
 * column is known.
 */
function continuationPrefix(indent: string | undefined, size: number, marker: string | undefined): string {
  const spaces = ' '.repeat(size);
  if (indent === undefined || indent === spaces || !/^[ \t]+$/.test(indent)) return spaces;
  let width: number;
  if (!indent.includes('\t')) width = indent.length;
  else if (marker !== undefined) width = visualWidth(marker + indent) - visualWidth(marker);
  else return spaces;
  return width >= size && width < size + 4 ? indent : spaces;
}

/**
 * remark's `listItem` handler (mdast-util-to-markdown), with the item's
 * authored spelling ({@link ListItemStyle}): an empty item prints as its
 * marker, and the item's lines after the first start with its `indent`.
 * `task-markers.ts` (which owns the `listItem` handler) prints through it.
 */
export const printListItem: Handle<ListItem> = (node, parent, state, info) => {
  const style = styleOf<ListItemStyle>(node);
  let bullet = state.bulletCurrent || state.options.bullet || '*';
  if (parent && parent.type === 'list' && parent.ordered) {
    const index = parent.children.indexOf(node);
    const start = typeof parent.start === 'number' && parent.start > -1 ? parent.start : 1;
    const previous = index > 0 ? itemNumbers.get(parent.children[index - 1]) : undefined;
    let number: number;
    if (previous === undefined) number = start + (state.options.incrementListMarker === false ? 0 : index);
    else if (style?.number !== undefined && style.number !== previous) {
      // After the first, a number is only a spelling (CommonMark counts from
      // the first): as typed. One equal to the previous item's is a copy
      // (Enter splits an item, attrs and all), so it is counted on.
      number = style.number;
    } else number = state.options.incrementListMarker === false ? previous : previous + 1;
    itemNumbers.set(node, number);
    bullet = number + bullet;
  }
  const frames = itemFrames.get(state) ?? [];
  itemFrames.set(state, frames);
  const depth = state.indexStack.length;
  const marker = markerLinePrefix(frames, depth, state);
  const typed = asTyped(node, style, bullet, marker);
  if (typed !== undefined) {
    printedAsTyped.add(node);
    return typed;
  }
  if (isEmptyItem(node)) return bullet + (style?.empty ?? '');
  // `- - x`: the editor's empty first paragraph is not in the source.
  if (style?.leadingBlock && (node.children?.length ?? 0) > 1 && isEmptyFlow(node.children.slice(0, 1))) {
    node = { ...node, children: node.children.slice(1) };
  }
  // `*  x`: the list's gap after the marker as typed (two to four spaces).
  const spaces = parent?.type === 'list' ? (styleOf<ListStyle>(parent)?.spaces ?? 1) : 1;
  let size = bullet.length + spaces;
  const listItemIndent = state.options.listItemIndent ?? 'one';
  if (
    listItemIndent === 'tab' ||
    (listItemIndent === 'mixed' && ((parent && parent.type === 'list' && parent.spread) || node.spread))
  ) {
    size = Math.ceil(size / 4) * 4;
  }

  // Indented code is four columns past the content column exactly: a wider
  // authored prefix would move its text, so such an item takes the default.
  const holdsIndentedCode = (node.children ?? []).some((child) => isIndentedCode(child));
  const prefix = holdsIndentedCode ? ' '.repeat(size) : continuationPrefix(style?.indent, size, marker);

  const tracker = state.createTracker(info);
  tracker.move(bullet + ' '.repeat(size - bullet.length));
  tracker.shift(prefix.length);
  const exit = state.enter('listItem');
  frames.push({ depth, lines: marker === undefined ? undefined : marker + prefix });
  try {
    // `*` alone on its line, the first block under it — only with the authored prefix, which keeps it in the item.
    const leadingBreak = style?.leadingBreak !== undefined && prefix === style.indent ? style.leadingBreak : undefined;
    return state.indentLines(state.containerFlow(node, tracker.current()), (line, index, blank) => {
      if (index && line.startsWith(LAZY_LINE)) return line;
      if (index) return (blank ? '' : prefix) + line;
      if (leadingBreak !== undefined && !blank) return `${bullet}${leadingBreak}\n${prefix}${line}`;
      return (blank ? bullet : bullet + ' '.repeat(size - bullet.length)) + line;
    });
  } finally {
    frames.pop();
    exit();
  }
};

/** Where an item's marker line starts, as a literal prefix, when known. */
function markerLinePrefix(frames: readonly ItemFrame[], depth: number, state: State): string | undefined {
  const outer = frames[frames.length - 1];
  if (!outer) return depth === 2 ? '' : undefined; // root → list → this item
  if (depth !== outer.depth + 2) return undefined;
  // outer item → list → this item; not on the outer item's first line.
  const onFirstLine = state.indexStack[outer.depth] === 0 && state.indexStack[outer.depth + 1] === 0;
  return onFirstLine ? undefined : outer.lines;
}

/** Items {@link printListItem} printed as typed, so `task-markers.ts` does not add their box again. */
const printedAsTyped = new WeakSet<ListItem>();

/** Whether {@link printListItem} printed `item` as typed (its task box included). */
export function wasPrintedAsTyped(item: ListItem): boolean {
  return printedAsTyped.has(item);
}

/**
 * `node` as typed ({@link ItemSource}) when it holds what was loaded, under
 * `bullet` (the printer's marker, the same width as the typed one). Its lines
 * after the first are written whole — marked lazy, so no container prefixes
 * them again — when the printer puts the marker line under the prefix it was
 * typed under (`marker`); else relative to the marker line, under whatever
 * prefix the containers print, which keeps every block where it was as long
 * as no tab in the item's indentation lands on another column.
 */
function asTyped(node: ListItem, style: ListItemStyle | undefined, bullet: string, marker: string | undefined): string | undefined {
  const typed = style?.typed;
  if (!typed?.key || bullet.length !== typed.marker.length) return undefined;
  if (typed.key !== itemKey(node, Boolean(style?.leadingBlock))) return undefined;
  const [first, ...rest] = typed.lines;
  // A top-level item's own indentation (one to three spaces) is printed by the item.
  const ownIndent = marker === '' && /^ {1,3}$/.test(typed.lead) ? typed.lead : '';
  if (marker !== undefined && marker + ownIndent === typed.lead) {
    return [ownIndent + bullet + first, ...rest.map((line) => LAZY_LINE + line)].join('\n');
  }
  const lines = [bullet + first];
  let tabs = /^[ \t]*\t/.test(first);
  for (const line of rest) {
    const relative = relativeItemLine(line, typed.lead);
    if (!relative || (relative.lazy && !typed.lazyOk)) return undefined;
    if (relative.lazy) {
      lines.push(LAZY_LINE + relative.text);
      continue;
    }
    tabs ||= /^[ \t]*\t/.test(relative.text);
    lines.push(/^[ \t]*$/.test(relative.text) ? '' : relative.text);
  }
  // A tab's width depends on its column: only where the marker sits on a column of the same tab stop phase.
  const typedColumn = visualWidth(typed.lead.replace(/>/g, ' '));
  if (tabs && (marker === undefined || visualWidth(marker) % 4 !== typedColumn % 4)) return undefined;
  return lines.join('\n');
}

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
  const value = printHeadingAs(style, node, parent, state, info);
  // Trailing whitespace on the heading's line, which nothing means.
  const source = style?.source;
  if (source === undefined) return value;
  // Trailing whitespace on the heading's lines (a setext underline's too), which nothing means here.
  if (/[ \t]$/m.test(source) && value === source.replace(/[ \t]+$/gm, '')) return source;
  if (source.includes('\n') || value.includes('\n')) return value;
  // Spelled differently, read the same (`###  x`, an escape the printer does not write).
  return source !== value && sameBlockMeaning(state, source, value) ? source : value;
};

function printHeadingAs(
  style: HeadingStyle | undefined,
  node: Heading,
  parent: Parents | undefined,
  state: State,
  info: Info,
): string {
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
}

const printCode: Handle<Code> = (node, parent, state, info) => {
  const style = styleOf<CodeStyle>(node);
  const meta = node.meta ?? style?.meta;
  const withMeta = meta && node.lang ? { ...node, meta } : node;
  const fenceChar = style?.fence?.[0] === '~' ? '~' : '`';
  const value = withOptions(state, { fences: !style?.indented, fence: fenceChar }, () =>
    defaultHandlers.code(withMeta, parent, state, info),
  );
  if (style?.indented && style.tab && !value.startsWith(fenceChar)) {
    // The printer indents with four spaces; the author used a tab (the same column).
    return value.replace(/^ {4}/gm, '\t');
  }
  const fenced = withAuthoredFence(value, style?.fence, fenceChar);
  // A fence never closed, still ending the note: no closing line.
  const last = parent?.type === 'root' && parent.children[parent.children.length - 1] === node;
  if (style?.unclosed && last && fenced.startsWith(fenceChar)) return fenced.replace(/\n[`~]{3,}$/, '');
  return fenced;
};

/** remark uses the shortest fence that works; the authored one, if longer. */
function withAuthoredFence(value: string, fence: string | undefined, fenceChar: string): string {
  if (!fence || !value.startsWith(fenceChar)) return value;
  const open = /^[`~]+/.exec(value)?.[0] ?? '';
  if (open.length >= fence.length) return value;
  return fence + value.slice(open.length, value.length - open.length) + fence;
}

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
  const style = styleOf<BlockquoteStyle>(node);
  // An empty quote: as typed, not `> <br />` (Milkdown's empty paragraph).
  if (isEmptyFlow(node.children)) return `>${style?.empty ?? ''}`;
  const compact = Boolean(style?.compact);
  const exit = state.enter('blockquote');
  const tracker = state.createTracker(info);
  tracker.move('> ');
  tracker.shift(2);
  const value = state.indentLines(state.containerFlow(node, tracker.current()), (line, _index, blank) => {
    if (blank) return `>${style?.blank ?? ''}${line}`;
    if (line.startsWith(LAZY_LINE)) return line;
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
  if (!style?.rows || style.rows.length < 2) return fresh();
  const [header, delimiter, ...body] = style.rows;
  const authored = [header, ...body];
  const columns = Math.max(...node.children.map((row) => row.children?.length ?? 0));
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

/** Whether `node` prints as an ATX heading (one line), not setext. */
function isAtxHeading(node: RootContent): boolean {
  if (node.type !== 'heading') return false;
  const setext = styleOf<HeadingStyle>(node)?.setext;
  return !(setext && node.depth <= 2 && setext[0] === (node.depth === 1 ? '=' : '-'));
}

function isFencedCode(node: RootContent): boolean {
  return node.type === 'code' && !styleOf<CodeStyle>(node)?.indented;
}

/** Whether `node` prints as indented code (its style, and no language to fence). */
function isIndentedCode(node: RootContent): boolean {
  return node.type === 'code' && Boolean(styleOf<CodeStyle>(node)?.indented) && !node.lang;
}

/** A list that may start right under a paragraph (CommonMark: not empty, an ordered one at 1). */
function interruptsParagraph(list: List): boolean {
  if (isEmptyItem(list.children?.[0])) return false;
  return !list.ordered || list.start === 1 || list.start == null;
}

/** Whether `right` is a list of another kind than `left` (bullet vs number, another marker), so not its continuation. */
function startsAnotherList(left: List, right: List): boolean {
  if (Boolean(left.ordered) !== Boolean(right.ordered)) return true;
  const a = styleOf<ListStyle>(left)?.marker;
  const b = styleOf<ListStyle>(right)?.marker;
  return a !== undefined && b !== undefined && a !== b;
}

/** A rule that may follow a paragraph line: not `-`, which would underline it as a heading. */
function ruleAfterParagraph(rule: ThematicBreak): boolean {
  const source = styleOf<RuleStyle>(rule)?.source ?? HOUSE_STYLE.rule;
  return !source.startsWith('-');
}

/**
 * Whether `right` printed on the line after `left` (no blank line between)
 * still starts a block of its own: nothing continues a heading, a rule or a
 * fenced code block, and only indented code continues indented code; a paragraph, a list's last paragraph or a quote's lazy
 * paragraph takes in the next line unless that line interrupts a paragraph.
 * Only the pairs authored that way get here ({@link GapStyle}); this keeps an
 * edit from making them merge.
 */
export function adjacentIsSafe(left: RootContent, right: RootContent): boolean {
  const type = left.type as string;
  if (type === 'heading' || type === 'thematicBreak' || type === 'sectionBreak' || isFencedCode(left)) return true;
  // Indented code ends at a line indented less; only more indented code would join it.
  if (left.type === 'code') return !isIndentedCode(right);
  // A footnote's `[^x]:` starts the next one.
  if (type === 'footnoteDefinition') return (right.type as string) === 'footnoteDefinition';
  // An empty last item has no paragraph to continue; a list like it would continue the list.
  if (left.type === 'list' && isEmptyItem(lastOf(left.children))) {
    if (right.type === 'list') return startsAnotherList(left, right);
    return !isIndentedCode(right);
  }
  const interrupts =
    isAtxHeading(right) ||
    isFencedCode(right) ||
    (right.type === 'thematicBreak' && ruleAfterParagraph(right));
  // A list of another kind (`-` then `*`, `1.` then `1)`) may start right under one.
  if (left.type === 'list') {
    return interrupts || (right.type === 'list' && startsAnotherList(left, right) && interruptsParagraph(right));
  }
  if (left.type === 'paragraph' || left.type === 'blockquote') {
    return interrupts || right.type === 'blockquote' || (right.type === 'list' && interruptsParagraph(right));
  }
  return false;
}

/**
 * Blank lines between two top-level blocks as authored ({@link GapStyle}).
 * Never before indented code after a list, which would be the list's (the
 * default join separates the two with a comment) — unless the list's last
 * item is empty, which nothing after a blank line continues — or after
 * indented code, which it would join.
 */
const joinAuthoredGaps: Join = (left, right, parent) => {
  if (parent.type === 'list') return unheldItemGaps.has(parent) ? undefined : authoredGap(left, right);
  if (parent.type !== 'root') return undefined;
  const gap = authoredGap(left, right);
  if (isIndentedCode(right as RootContent)) {
    if (left.type === 'list') return isEmptyItem(lastOf(left.children)) ? Math.max(gap ?? 1, 1) : undefined;
    // Two indented blocks are one, however many blank lines are between.
    if (left.type === 'code') return undefined;
  }
  if (gap === 0) return adjacentIsSafe(left as RootContent, right as RootContent) ? 0 : undefined;
  return gap;
};

export const authoredStyleToMarkdown: ToMarkdownExtension = {
  bullet: HOUSE_STYLE.bullet,
  rule: HOUSE_STYLE.rule,
  join: [joinAuthoredGaps],
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
 * Milkdown's paragraph drops a trailing hard break (the editor's Shift+Enter
 * at the end of a line), but one inside a link is the link's text —
 * `[a  ⏎](url)`, or a link that is only a break, which went with it.
 */
const paragraphKeepingLinkedBreak = paragraphSchema.extendSchema((prev) => (ctx) => {
  const base: NodeSchema = prev(ctx);
  return {
    ...base,
    toMarkdown: {
      match: base.toMarkdown.match,
      runner: (state, node: ProseNode) => {
        const last = node.lastChild;
        if (last?.type.name !== 'hardbreak' || !last.marks.some((mark) => mark.type.name === 'link')) {
          base.toMarkdown.runner(state, node);
          return;
        }
        state.openNode('paragraph');
        state.next(node.content);
        state.closeNode();
      },
    },
  };
});

/**
 * Every schema this module carries a style on. `blockquote` is not here:
 * alerts extends it too, and two extensions of one node replace each other,
 * so `alerts.ts` applies {@link carryAuthoredStyle} to its own and prints
 * through {@link printBlockquote} (it owns the `blockquote` handler).
 */
export const authoredStyleSchemas = [
  carryAuthoredStyle(paragraphKeepingLinkedBreak),
  carryAuthoredStyle(bulletListSchema),
  carryAuthoredStyle(orderedListSchema),
  carryAuthoredStyle(hrSchema),
  carryAuthoredStyle(headingSchema),
  carryAuthoredStyle(codeBlockSchema),
  carryAuthoredStyle(hardbreakSchema),
  carryAuthoredStyle(tableSchema),
  // For the blank lines between footnotes (`GapStyle`).
  carryAuthoredStyle(footnoteDefinitionSchema),
];

/** Everything item 6 registers, for `MARKDOWN_EXTENSION_PLUGINS`. */
export const authoredStylePlugin = [remarkAuthoredStyle, ...authoredStyleSchemas].flat();
