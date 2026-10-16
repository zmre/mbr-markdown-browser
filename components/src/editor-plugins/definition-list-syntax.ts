/**
 * Definition lists for the editor's markdown pipeline (remark): the micromark
 * syntax, the mdast it builds, and the markdown it prints. The Milkdown half
 * (ProseMirror nodes, keymaps, slash menu) is `definition-list.ts`.
 *
 * # Semantics: pulldown-cmark's, not Pandoc's
 *
 * The renderer parses with pulldown-cmark's `ENABLE_DEFINITION_LIST`, and the
 * flashcard writer (`flashcards.rs`) uses the same parser, so the editor has
 * to agree with it exactly — the parity harness (`editor-parity.test.ts`)
 * compares block outlines line by line. Its rules (pulldown-cmark 0.13
 * `firstpass.rs`), which this module reproduces:
 *
 * - A **definition** is a container opened by `:` at a block start, like a
 *   list item: up to three spaces before the colon, then up to four spaces
 *   (five or more: one counts, the rest is indented code), and **no space is
 *   required** (`:foo` is a definition too). Its content is every following
 *   line indented by at least the marker width, plus lazy paragraph
 *   continuations; a blank first line allows one more blank line only.
 * - It is a definition only when the block before it *in the same container*
 *   is a **paragraph** (which becomes the term — the whole paragraph, every
 *   line of it: `A. …\nB. …\nC. …\nD. …` is one multi-line term) or another
 *   definition. Blank lines in between do not matter. After anything else —
 *   a heading, a list, a fence, the document start — `: x` is paragraph text.
 * - `:` **interrupts** a paragraph, so `Term\n: def` needs no blank line.
 * - A paragraph straight after a definition list (blank lines allowed) that is
 *   itself followed by a definition is the next term **of the same list**.
 * - The list is **loose** when a blank line separates a term from its first
 *   definition, two definitions, or two blocks inside a definition. A blank
 *   line between a definition and the next term does not count (and is
 *   required, or the term would be a lazy continuation).
 *
 * micromark has no notion of "the previous block", so the term is attached
 * after parsing: the tokenizer only produces `defList` containers holding
 * definitions, and {@link groupDefinitionTerms} (an mdast transform) turns the
 * paragraph before each one into its `defListTerm` and merges lists a term
 * continues. The tokenizer's start condition ({@link mayStartDefinition}) is
 * the part that has to look backwards.
 *
 * Known divergences, both far from any deck: a "paragraph" that is only link
 * reference definitions counts as one here (pulldown-cmark consumes those, so
 * a `:` after them is text), and after a blank line inside a block quote
 * whose lines differ in the space after `>` (`> Term\n>\n>: x`) the column
 * check in {@link paragraphPrecedes} refuses a definition the renderer
 * accepts.
 *
 * # Why not `micromark-extension-definition-list`
 *
 * It implements a different dialect: each line of the paragraph before `: `
 * is its own term (T1-A09's four answer lines became four `<dt>`s where the
 * renderer has one), a space after `:` is required, at most one blank line
 * may separate term and definition, and it prints `:   ` with four-space
 * continuations. It also re-tokenizes the term paragraph in a resolver and
 * pulls in `debug`. Reproducing the renderer's rules is shorter written
 * directly.
 *
 * # Printing (mbr's house style)
 *
 * `: ` and a two-space continuation indent — what every flashcard deck uses
 * and what `flashcards.rs` writes — not the package's `:   `. Tight and loose
 * spacing round-trip per gap: each definition records how many blank lines
 * preceded it (`gapBefore`) and whether its blocks are blank-line separated
 * (`spread`), so an untouched list prints byte for byte. Not preserved
 * (structure is): the spacing after an authored `:` (`:x`, `:   x` print as
 * `: x`) and lazy continuation lines (printed indented).
 *
 * The extension also replaces mdast-util-to-markdown's `paragraph` handler,
 * with one that escapes a `:` starting any line but the first
 * ({@link escapeContinuationColons}); see there for why it cannot be an
 * `unsafe` pattern.
 */
import type {
  Code,
  Construct,
  ContainerState,
  Event,
  Effects,
  Extension as SyntaxExtension,
  State,
  TokenizeContext,
} from 'micromark-util-types';
import type { CompileContext, Extension as FromMarkdownExtension, Token } from 'mdast-util-from-markdown';
import type { Handle, Join, Options as ToMarkdownExtension, State as ToMarkdownState, Info } from 'mdast-util-to-markdown';
import type { Nodes, Parent, Paragraph, PhrasingContent, Root, BlockContent, DefinitionContent } from 'mdast';
import { blankLine, content } from 'micromark-core-commonmark';
import { factorySpace } from 'micromark-factory-space';
import { markdownSpace } from 'micromark-util-character';

// ---------------------------------------------------------------------------
// mdast

/** A definition list: alternating terms and definitions, as the renderer has them. */
export interface DefList extends Parent {
  type: 'defList';
  /** Loose (pulldown-cmark's `!is_tight`): any authored blank line between a term and a definition, two definitions, or two blocks of one definition. */
  spread: boolean;
  children: (DefListTerm | DefListDescription)[];
}

/** A term (`<dt>`): the paragraph before the first `:`, inline content only. */
export interface DefListTerm extends Parent {
  type: 'defListTerm';
  /** Always false: a term is one paragraph. Set so list-looseness checks that read `spread` see a tight child. */
  spread: false;
  children: PhrasingContent[];
}

/** A definition (`<dd>`), opened by `:`. */
export interface DefListDescription extends Parent {
  type: 'defListDescription';
  /** Its blocks are separated by blank lines. */
  spread: boolean;
  /** Blank lines between it and the term or definition before it (0: none). */
  gapBefore: number;
  children: (BlockContent | DefinitionContent)[];
}

declare module 'mdast' {
  interface RootContentMap {
    defList: DefList;
    defListTerm: DefListTerm;
    defListDescription: DefListDescription;
  }
  interface BlockContentMap {
    defList: DefList;
  }
}

declare module 'micromark-util-types' {
  interface TokenTypeMap {
    defList: 'defList';
    defListDescription: 'defListDescription';
    defListDescriptionPrefix: 'defListDescriptionPrefix';
    defListDescriptionMarker: 'defListDescriptionMarker';
    defListDescriptionPrefixWhitespace: 'defListDescriptionPrefixWhitespace';
  }
  interface ParseContext {
    /** {@link mayStartDefinition}'s answer for one offset (see there). */
    mbrDefListGate?: { offset: number; ok: boolean };
  }
}

declare module 'mdast-util-to-markdown' {
  interface ConstructNameMap {
    defList: 'defList';
    defListTerm: 'defListTerm';
    defListDescription: 'defListDescription';
  }
}

// ---------------------------------------------------------------------------
// micromark syntax

const COLON = 58;
/** micromark's `constants.tabSize`: a line prefix of four spaces is indented code. */
const TAB_SIZE = 4;

/**
 * Token types a container's *continuation* leaves on a line before a new
 * container may start: whitespace and block quote markers. Anything else
 * between the last flow chunk and the colon means a container opened on this
 * line (`- : x`, `> : x`, a new item `- a\n- : x`), so the colon starts that
 * container's first block, where there is no paragraph before it.
 */
const CONTINUATION_PREFIXES = new Set<string>([
  'linePrefix',
  'listItemIndent',
  'blockQuotePrefix',
  'blockQuoteMarker',
  'blockQuotePrefixWhitespace',
  'gfmFootnoteDefinitionIndent',
]);

/**
 * The token for a definition's continuation indent. Deliberately the list
 * item's type, not one of our own: `mdast-util-from-markdown`'s list
 * compiler skips exactly the prefix types it knows when it looks for where a
 * list item ends, and a list nested in a definition would otherwise have each
 * item run on to the next one's line. (Not `linePrefix` either: a list item
 * starting after it would count it into its own content indent.)
 */
const DEFINITION_INDENT = 'listItemIndent';

/** Flow tokens that are only spacing between blocks. */
const FLOW_SPACING = new Set<string>(['lineEnding', 'lineEndingBlank', 'linePrefix']);

const defListConstruct: Construct = {
  name: 'defList',
  tokenize: tokenizeDefinitionStart,
  continuation: { tokenize: tokenizeDefinitionContinuation },
  exit: tokenizeDefListExit,
};

const prefixWhitespaceConstruct: Construct = { tokenize: tokenizePrefixWhitespace, partial: true };
const indentConstruct: Construct = { tokenize: tokenizeIndent, partial: true };

/** The micromark extension: `:` definitions as a document (container) construct. */
export const defListSyntax: SyntaxExtension = {
  document: { [COLON]: defListConstruct },
};

interface DefListState extends ContainerState {
  size?: number;
  initialBlankLine?: boolean;
  furtherBlankLines?: boolean;
}

/**
 * Whether a `:` at the current position may open a definition: the block
 * before it in the same container is a paragraph (possibly still open, which
 * the colon then interrupts) — see the module docs.
 *
 * micromark tries a container start twice at one position (an `effects.check`
 * and then the real `attempt`), and between the two it closes the open
 * paragraph, which changes what this function would see. The first answer is
 * therefore kept for the offset, so both runs agree.
 */
function mayStartDefinition(self: TokenizeContext): boolean {
  const offset = self.now().offset;
  const cached = self.parser.mbrDefListGate;
  if (cached && cached.offset === offset) return cached.ok;
  const ok = paragraphPrecedes(self);
  self.parser.mbrDefListGate = { offset, ok };
  return ok;
}

/**
 * Whether the flow construct still open across the line break is a
 * paragraph: micromark's `content`, or GFM's table construct while it is
 * still looking at a one-line paragraph for the delimiter row that would make
 * it a table head (it holds every paragraph's first line that way).
 */
function isOpenParagraph(construct: Construct, flowEvents: readonly Event[]): boolean {
  if (construct === content) return true;
  if (construct.name !== 'table') return false;
  let index = flowEvents.length;
  while (index--) {
    // GFM's token types; not in the core `TokenTypeMap`.
    const type: string = flowEvents[index][1].type;
    if (type === 'tableDelimiterRow') return false;
    if (type === 'table' || type === 'tableHead') return true;
  }
  return true;
}

function paragraphPrecedes(self: TokenizeContext): boolean {
  const events = self.events;
  let index = events.length;
  while (index--) {
    const type = events[index][1].type;
    if (type === 'chunkFlow') break;
    if (!CONTINUATION_PREFIXES.has(type)) return false;
  }
  if (index < 0) return false;
  const flow = events[index][1]._tokenizer;
  if (!flow) return false;

  if (flow.currentConstruct) {
    // A block is still open. `interrupt` is set only when every open
    // container continued on this line; otherwise this line would be a lazy
    // continuation of that paragraph, not a definition.
    return Boolean(self.interrupt) && isOpenParagraph(flow.currentConstruct, flow.events);
  }

  // Nothing open: the last block of this flow must be a paragraph…
  const flowEvents = flow.events;
  let last = flowEvents.length;
  while (last--) {
    if (!FLOW_SPACING.has(flowEvents[last][1].type)) break;
  }
  if (last < 0 || flowEvents[last][1].type !== 'content') return false;

  // …and every container holding it must have continued on this line. A
  // container that did not (`- a\n\n: x`) leaves this line's content left of
  // where the paragraph's first line had its content (the start of that
  // line's flow chunk, i.e. after its container prefixes).
  const paragraphStart = flowEvents[last][1].start.offset;
  let chunk = index;
  while (chunk >= 0 && (events[chunk][1].type !== 'chunkFlow' || events[chunk][1].start.offset > paragraphStart)) {
    chunk--;
  }
  if (chunk < 0) return false;
  const now = self.now();
  const tail = events[events.length - 1];
  const lineStart =
    tail[1].type === 'linePrefix' && tail[1].start.line === now.line ? tail[1].start.column : now.column;
  return lineStart === events[chunk][1].start.column;
}

function tokenizeDefinitionStart(
  this: TokenizeContext,
  effects: Effects,
  ok: State,
  nok: State,
): State {
  const self = this;
  const tail = self.events[self.events.length - 1];
  const initialSize = tail && tail[1].type === 'linePrefix' ? tail[2].sliceSerialize(tail[1], true).length : 0;
  return start;

  function start(code: Code): State | undefined {
    const state = self.containerState as DefListState;
    if (code !== COLON) return nok(code);
    if (!state.type) {
      if (!mayStartDefinition(self)) return nok(code);
      state.type = 'defList';
      effects.enter('defList', { _container: true });
    }
    effects.enter('defListDescriptionPrefix');
    effects.enter('defListDescriptionMarker');
    effects.consume(code);
    effects.exit('defListDescriptionMarker');
    // Unlike a list item, an empty definition may interrupt a paragraph, and
    // a blank first line does not widen the indent (pulldown-cmark: `:` +
    // end of line is indent 1).
    return effects.check(
      blankLine,
      onBlank,
      effects.attempt(prefixWhitespaceConstruct, endOfPrefix, otherPrefix),
    );
  }

  function onBlank(code: Code): State | undefined {
    (self.containerState as DefListState).initialBlankLine = true;
    return endOfPrefix(code);
  }

  /** Five or more spaces (one belongs to the marker, the rest is indented code) or none at all. */
  function otherPrefix(code: Code): State | undefined {
    if (markdownSpace(code)) {
      effects.enter('defListDescriptionPrefixWhitespace');
      effects.consume(code);
      effects.exit('defListDescriptionPrefixWhitespace');
      return endOfPrefix;
    }
    return endOfPrefix(code);
  }

  function endOfPrefix(code: Code): State | undefined {
    const prefix = effects.exit('defListDescriptionPrefix');
    (self.containerState as DefListState).size = initialSize + self.sliceSerialize(prefix, true).length;
    return ok(code);
  }
}

function tokenizeDefinitionContinuation(
  this: TokenizeContext,
  effects: Effects,
  ok: State,
  nok: State,
): State {
  const self = this;
  const state = self.containerState as DefListState;
  state._closeFlow = undefined;
  return effects.check(blankLine, onBlank, notBlank);

  function onBlank(code: Code): State | undefined {
    state.furtherBlankLines = state.furtherBlankLines || state.initialBlankLine;
    return factorySpace(effects, ok, DEFINITION_INDENT, (state.size ?? 0) + 1)(code);
  }

  function notBlank(code: Code): State | undefined {
    if (state.furtherBlankLines || !markdownSpace(code)) {
      state.furtherBlankLines = undefined;
      state.initialBlankLine = undefined;
      return notInCurrentDefinition(code);
    }
    state.furtherBlankLines = undefined;
    state.initialBlankLine = undefined;
    return effects.attempt(indentConstruct, ok, notInCurrentDefinition)(code);
  }

  /** Not this definition's content: maybe the next `:` of the same list. */
  function notInCurrentDefinition(code: Code): State | undefined {
    state._closeFlow = true;
    self.interrupt = undefined;
    return factorySpace(effects, effects.attempt(defListConstruct, ok, nok), 'linePrefix', TAB_SIZE)(code);
  }
}

function tokenizeIndent(this: TokenizeContext, effects: Effects, ok: State, nok: State): State {
  const self = this;
  const size = (self.containerState as DefListState).size ?? 0;
  return factorySpace(effects, afterPrefix, DEFINITION_INDENT, size + 1);

  function afterPrefix(code: Code): State | undefined {
    const tail = self.events[self.events.length - 1];
    return tail &&
      tail[1].type === DEFINITION_INDENT &&
      tail[2].sliceSerialize(tail[1], true).length === size
      ? ok(code)
      : nok(code);
  }
}

/** One to four spaces after the colon, then content (micromark's list-item rule). */
function tokenizePrefixWhitespace(this: TokenizeContext, effects: Effects, ok: State, nok: State): State {
  const self = this;
  return factorySpace(effects, afterPrefix, 'defListDescriptionPrefixWhitespace', TAB_SIZE + 1);

  function afterPrefix(code: Code): State | undefined {
    const tail = self.events[self.events.length - 1];
    return !markdownSpace(code) && tail && tail[1].type === 'defListDescriptionPrefixWhitespace'
      ? ok(code)
      : nok(code);
  }
}

function tokenizeDefListExit(this: TokenizeContext, effects: Effects): undefined {
  effects.exit('defList');
}

// ---------------------------------------------------------------------------
// mdast-util-from-markdown

/**
 * A definition has no closing token of its own (the next `:` or the end of
 * the container ends it), so it is entered on its prefix with a token made
 * here, and exited with that token once its end is known.
 */
function closeOpenDefinition(context: CompileContext): void {
  const node = context.stack[context.stack.length - 1];
  if (node?.type !== 'defListDescription') return;
  const open = context.tokenStack[context.tokenStack.length - 1];
  if (!open) return;
  const token = open[0];
  const lastChild = node.children[node.children.length - 1];
  const end = lastChild?.position?.end;
  if (end) token.end = { ...token.end, line: end.line, column: end.column, offset: end.offset ?? token.end.offset };
  context.exit(token);
}

export const defListFromMarkdown: FromMarkdownExtension = {
  enter: {
    defList(token) {
      this.enter({ type: 'defList', spread: false, children: [] }, token);
    },
    defListDescriptionPrefix(token) {
      closeOpenDefinition(this);
      const definition: Token = { type: 'defListDescription', start: { ...token.start }, end: { ...token.end } };
      this.enter({ type: 'defListDescription', spread: false, gapBefore: 0, children: [] }, definition);
    },
  },
  exit: {
    defList(token) {
      closeOpenDefinition(this);
      this.exit(token);
    },
  },
  transforms: [groupDefinitionTerms],
};

/** Lines between `a`'s end and `b`'s start — blank lines, as both are blocks of one container. */
function blankLinesBetween(a: Nodes, b: Nodes): number {
  if (!a.position || !b.position) return 0;
  return Math.max(0, b.position.start.line - a.position.end.line - 1);
}

function toTerm(paragraph: Paragraph): DefListTerm {
  const term: DefListTerm = { type: 'defListTerm', spread: false, children: paragraph.children };
  if (paragraph.position) term.position = paragraph.position;
  return term;
}

/**
 * Attaches each definition list to its term — the paragraph right before it —
 * merging a list into the one before that paragraph (a term following a
 * definition list continues it), then computes the spacing fields. Runs
 * depth-first, so lists nested in definitions and items are handled too.
 */
export function groupDefinitionTerms(tree: Root): undefined {
  const visit = (node: Nodes): void => {
    if (!('children' in node)) return;
    const out: Nodes[] = [];
    for (const child of node.children as Nodes[]) {
      visit(child);
      if (child.type !== 'defList' || child.children[0]?.type === 'defListTerm') {
        out.push(child);
        continue;
      }
      const previous = out[out.length - 1];
      if (previous?.type !== 'paragraph') {
        // Only reachable when the "paragraph" was link reference definitions
        // alone; keep the definitions rather than drop them.
        out.push(child);
        continue;
      }
      out.pop();
      const term = toTerm(previous);
      const before = out[out.length - 1];
      if (before?.type === 'defList') {
        before.children.push(term, ...child.children);
        if (before.position && child.position) before.position.end = child.position.end;
      } else {
        child.children.unshift(term);
        if (child.position && term.position) child.position.start = term.position.start;
        out.push(child);
      }
    }
    (node as Parent).children = out as Parent['children'];
    for (const child of out) if (child.type === 'defList') measureSpacing(child);
  };
  visit(tree);
}

function measureSpacing(list: DefList): void {
  let loose = false;
  list.children.forEach((child, index) => {
    if (child.type !== 'defListDescription') return;
    const previous = list.children[index - 1];
    child.gapBefore = previous ? blankLinesBetween(previous, child) : 0;
    child.spread = child.children.some((block, i) => i > 0 && blankLinesBetween(child.children[i - 1], block) > 0);
    loose ||= child.gapBefore > 0 || child.spread;
  });
  list.spread = loose;
}

// ---------------------------------------------------------------------------
// mdast-util-to-markdown

const defListHandler: Handle = (node: DefList, _parent, state: ToMarkdownState, info: Info) => {
  const exit = state.enter('defList');
  const value = state.containerFlow(node, info);
  exit();
  return value;
};

/**
 * A line of a multi-line paragraph or term that starts with `:` would open a
 * definition (`:` interrupts a paragraph), so a literal one there is escaped.
 * The first line is left alone: a paragraph starting with `:` after a
 * heading, a list or the document start is plain text to the renderer too,
 * and escaping it would rewrite e.g. the `:::note` fences other tools use.
 *
 * Done on the printed text rather than with an `unsafe` pattern because a
 * pattern cannot tell the two apart: `atBreak` matches a first line too, and
 * Milkdown prints a soft break as a text node of its own and its text handler
 * passes no `beforeNode`, so a later line looks exactly like a first one.
 */
export function escapeContinuationColons(value: string): string {
  return value.replace(/(\r?\n|\r)([\t ]*):/g, '$1$2\\:');
}

/**
 * mdast-util-to-markdown's `paragraph` handler plus
 * {@link escapeContinuationColons} — except for a paragraph that is one HTML
 * node, which is how Milkdown holds an HTML *block*: its lines are raw HTML,
 * where a backslash would be content, and the renderer does not read
 * definitions inside an HTML block anyway.
 */
const paragraphHandler: Handle = (node: Paragraph, _parent, state: ToMarkdownState, info: Info) => {
  const exit = state.enter('paragraph');
  const phrasing = state.enter('phrasing');
  const value = state.containerPhrasing(node, info);
  phrasing();
  exit();
  const htmlBlock = node.children.length === 1 && node.children[0].type === 'html';
  return htmlBlock ? value : escapeContinuationColons(value);
};

const defListTermHandler: Handle = (node: DefListTerm, _parent, state: ToMarkdownState, info: Info) => {
  const exit = state.enter('defListTerm');
  const phrasing = state.enter('phrasing');
  const value = state.containerPhrasing(node, { ...info, before: '\n', after: '\n' });
  phrasing();
  exit();
  return escapeContinuationColons(value);
};

/** `: ` then the content, continuation lines indented two spaces, blank lines empty. */
const defListDescriptionHandler: Handle = (
  node: DefListDescription,
  _parent,
  state: ToMarkdownState,
  info: Info,
) => {
  const exit = state.enter('defListDescription');
  const tracker = state.createTracker(info);
  tracker.move(': ');
  tracker.shift(2);
  const value = state.indentLines(state.containerFlow(node, tracker.current()), (line, index, blank) => {
    if (index === 0) return blank ? ':' : `: ${line}`;
    return blank ? '' : `  ${line}`;
  });
  exit();
  return value;
};

/**
 * Blank lines inside a list: before a definition as many as were authored
 * (`gapBefore`; one made in the editor copies the list's looseness); before a
 * term always one — without it the term would be a lazy continuation of the
 * definition above. Inside a definition, `spread` decides
 * (mdast-util-to-markdown's own rule for any parent with `spread`).
 */
const joinDefinitionList: Join = (_left, right, parent) => {
  if (parent.type !== 'defList') return undefined;
  if (right.type === 'defListDescription') return right.gapBefore;
  return 1;
};

export const defListToMarkdown: ToMarkdownExtension = {
  handlers: {
    paragraph: paragraphHandler,
    defList: defListHandler,
    defListTerm: defListTermHandler,
    defListDescription: defListDescriptionHandler,
  },
  join: [joinDefinitionList],
};

// ---------------------------------------------------------------------------
// remark

interface RemarkData {
  micromarkExtensions?: SyntaxExtension[];
  fromMarkdownExtensions?: FromMarkdownExtension[];
  toMarkdownExtensions?: ToMarkdownExtension[];
}

/** The remark plugin: parse and print definition lists (no options). */
export function remarkDefinitionList(this: { data(): unknown }): undefined {
  const data = this.data() as RemarkData;
  (data.micromarkExtensions ??= []).push(defListSyntax);
  (data.fromMarkdownExtensions ??= []).push(defListFromMarkdown);
  (data.toMarkdownExtensions ??= []).push(defListToMarkdown);
}
