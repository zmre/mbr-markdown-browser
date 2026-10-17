/**
 * Definition lists in the editor (TODO "Editor: cover mbr's markdown
 * extensions", item 1) — the format flashcard decks are written in:
 *
 * ```markdown
 * What does `move` do?
 * : It moves every captured variable into the closure.
 * : ___Review History___
 *   * 2026-10-01 09:15 - Hard
 * ```
 *
 * Parsing and printing are `definition-list-syntax.ts` (pulldown-cmark's
 * rules, held to the renderer by the parity harness); this module is the
 * Milkdown half: three ProseMirror nodes, the keys that edit them, a slash
 * menu entry, a `: ` input rule and the styles.
 *
 * | node                     | markdown                      | content      |
 * |--------------------------|-------------------------------|--------------|
 * | `definition_list`        | `defList` (`spread` = loose)  | terms + defs |
 * | `definition_term`        | `defListTerm` (`<dt>`)        | inline       |
 * | `definition_description` | `defListDescription` (`<dd>`) | blocks       |
 *
 * The list's content is any run of terms and definitions rather than strict
 * `(term definition+)+`: editing passes through states like "a term with no
 * definition yet", and ProseMirror refuses a step that leaves a node invalid.
 * Such a state still prints as markdown that parses (a term with no
 * definition is a paragraph).
 *
 * # Keys
 *
 * - **Enter in a term** starts its first definition (`: `), carrying any text
 *   after the cursor; on an empty last term it leaves the list.
 * - **Enter in a definition** starts the next definition, like a list item;
 *   on an empty definition it leaves the list (the last one) or turns into
 *   the next term (one in the middle).
 * - **Backspace at the start of a definition** undoes those: it merges the
 *   text into the definition or term above, or lifts the first block out of
 *   a list that has no term.
 */
import type { Ctx, MilkdownPlugin } from '@milkdown/kit/ctx';
import { commandsCtx } from '@milkdown/kit/core';
import { blockContainerTypes, clearTextInCurrentBlockCommand, paragraphSchema } from '@milkdown/kit/preset/commonmark';
import { InputRule } from '@milkdown/kit/prose/inputrules';
import { Fragment, type Node as ProseNode, type NodeType, type ResolvedPos } from '@milkdown/kit/prose/model';
import { TextSelection, type Command, type EditorState, type Transaction } from '@milkdown/kit/prose/state';
import type { JSONValue, SerializerState } from '@milkdown/kit/transformer';
import { $command, $inputRule, $nodeSchema, $remark, $useKeymap } from '@milkdown/kit/utils';
import { remarkDefinitionList } from './definition-list-syntax.js';
import { contentKey, type DefinitionStyle } from './authored-style.js';

// ---------------------------------------------------------------------------
// Schema

/** remark: parse and print `: ` definitions (see `definition-list-syntax.ts`). */
export const remarkDefinitionListPlugin = $remark('mbrDefinitionList', () => remarkDefinitionList);

export const definitionListSchema = $nodeSchema('definition_list', () => ({
  group: 'block',
  content: '(definition_term | definition_description)+',
  attrs: { spread: { default: false, validate: 'boolean' }, mbrStyle: { default: null } },
  parseDOM: [
    {
      tag: 'dl',
      // Milkdown draws footnote definitions as `<dl>` too.
      getAttrs: (dom: HTMLElement) =>
        dom.dataset.type === 'footnote_definition' ? false : { spread: dom.dataset.spread === 'true' },
    },
  ],
  toDOM: (node: ProseNode) => [
    'dl',
    { class: 'mbr-dl', 'data-type': 'definition_list', 'data-spread': String(node.attrs.spread) },
    0,
  ],
  parseMarkdown: {
    match: (node) => node.type === 'defList',
    runner: (state, node, type) => {
      state.openNode(type, { spread: Boolean(node.spread), mbrStyle: node.mbrStyle ?? null });
      state.next(node.children);
      state.closeNode();
    },
  },
  toMarkdown: {
    match: (node) => node.type.name === 'definition_list',
    runner: (state, node) => {
      dropListItemFiller(state);
      state.openNode('defList', undefined, { spread: Boolean(node.attrs.spread), ...styleProps(node) });
      state.next(node.content);
      state.closeNode();
    },
  },
}));

export const definitionTermSchema = $nodeSchema('definition_term', () => ({
  content: 'inline*',
  defining: true,
  attrs: { mbrStyle: { default: null } },
  parseDOM: [{ tag: 'dt' }],
  toDOM: () => ['dt', { class: 'mbr-dt' }, 0],
  parseMarkdown: {
    match: (node) => node.type === 'defListTerm',
    runner: (state, node, type) => {
      state.openNode(type, { mbrStyle: node.mbrStyle ?? null });
      state.next(node.children);
      state.closeNode();
    },
  },
  toMarkdown: {
    match: (node) => node.type.name === 'definition_term',
    runner: (state, node) => {
      state.openNode('defListTerm', undefined, { spread: false, ...styleProps(node) });
      serializeInline(state, node);
      state.closeNode();
    },
  },
}));

export const definitionDescriptionSchema = $nodeSchema('definition_description', () => ({
  content: 'block+',
  defining: true,
  attrs: {
    spread: { default: false, validate: 'boolean' },
    gapBefore: { default: 0, validate: 'number' },
    mbrStyle: { default: null },
  },
  parseDOM: [
    {
      tag: 'dd',
      getAttrs: (dom: HTMLElement) => ({
        spread: dom.dataset.spread === 'true',
        gapBefore: Number(dom.dataset.gapBefore) || 0,
      }),
    },
  ],
  toDOM: (node: ProseNode) => [
    'dd',
    { class: 'mbr-dd', 'data-spread': String(node.attrs.spread), 'data-gap-before': String(node.attrs.gapBefore) },
    0,
  ],
  parseMarkdown: {
    match: (node) => node.type === 'defListDescription',
    runner: (state, node, type) => {
      state.openNode(type, {
        spread: Boolean(node.spread),
        gapBefore: Number(node.gapBefore) || 0,
        mbrStyle: definitionStyle(node),
      });
      state.next(node.children);
      state.closeNode();
    },
  },
  toMarkdown: {
    match: (node) => node.type.name === 'definition_description',
    runner: (state, node) => {
      state.openNode('defListDescription', undefined, {
        spread: Boolean(node.attrs.spread),
        gapBefore: Number(node.attrs.gapBefore) || 0,
        ...styleProps(node),
      });
      // An empty `:` parses to a definition holding one empty paragraph
      // (the schema needs a block); print it as the bare `:` it came from
      // rather than the paragraph's `<br />` placeholder.
      if (!isEmptyDefinition(node)) state.next(node.content);
      state.closeNode();
    },
  },
}));

/**
 * A list item whose first block is a definition list (`- Term\n  : def`, a
 * card nested in a list) cannot exist in Milkdown: its `list_item` must start
 * with a paragraph, so parsing fills one in, empty — which would print as a
 * `<br />` line above the term. When the list is about to be written as the
 * item's second child and the first is that empty filler, drop the filler.
 * (A literal `<br />` line there folds into the same empty paragraph on load,
 * so the two are indistinguishable; the filler is by far the likelier.)
 */
function dropListItemFiller(state: SerializerState): void {
  const parent = state.top();
  if (parent?.type !== 'listItem' || parent.children?.length !== 1) return;
  const only = parent.children[0] as { type: string; children?: { type: string; value?: unknown }[] };
  const inline = only.children ?? [];
  const isFiller =
    only.type === 'paragraph' &&
    (inline.length === 0 ||
      (inline.length === 1 && inline[0].type === 'html' && /^<br\s*\/?>$/i.test(String(inline[0].value))));
  if (isFiller) parent.pop();
}

/** A paragraph's inline content, minus a trailing hard break (Milkdown's `serializeText`). */
function serializeInline(state: SerializerState, node: ProseNode): void {
  if (node.lastChild?.type.name !== 'hardbreak') {
    state.next(node.content);
    return;
  }
  const children: ProseNode[] = [];
  node.content.forEach((child, _offset, index) => {
    if (index < node.childCount - 1) children.push(child);
  });
  state.next(Fragment.fromArray(children));
}

/**
 * A definition's recorded lines (`authored-style.ts`) with the key of what
 * they parsed to, taken here — from the tree the editor's nodes are built
 * from, after every remark transform — so that the printer can tell an
 * untouched definition (same key) from an edited one.
 */
function definitionStyle(node: { mbrStyle?: unknown; children?: unknown[] }): DefinitionStyle | null {
  const style = node.mbrStyle as DefinitionStyle | undefined;
  return style?.lines ? { lines: style.lines, key: contentKey(node.children ?? []) } : null;
}

/** The recorded spelling a node carries, as mdast props (none when it has none). */
function styleProps(node: ProseNode): { mbrStyle?: JSONValue } {
  return node.attrs.mbrStyle == null ? {} : { mbrStyle: node.attrs.mbrStyle as JSONValue };
}

function isEmptyDefinition(node: ProseNode): boolean {
  return node.childCount === 1 && node.firstChild?.type.name === 'paragraph' && node.firstChild.content.size === 0;
}

/**
 * Lets Milkdown's HTML-block and empty-line transforms treat a definition as
 * the block container it is (an HTML block directly in a `: ` would
 * otherwise sit inline in block content and the parser would drop the whole
 * definition).
 */
const definitionIsBlockContainer: MilkdownPlugin = (ctx) => () => {
  if (!ctx.isInjected(blockContainerTypes.key)) return;
  ctx.update(blockContainerTypes.key, (types) =>
    types.includes('defListDescription') ? types : [...types, 'defListDescription'],
  );
};

// ---------------------------------------------------------------------------
// Editing

/** The node types the commands work with. */
export interface DefinitionListTypes {
  list: NodeType;
  term: NodeType;
  description: NodeType;
  paragraph: NodeType;
}

function typesOf(ctx: Ctx): DefinitionListTypes {
  return {
    list: definitionListSchema.type(ctx),
    term: definitionTermSchema.type(ctx),
    description: definitionDescriptionSchema.type(ctx),
    paragraph: paragraphSchema.type(ctx),
  };
}

/** A new definition: one empty paragraph, spaced like the list it joins. */
function newDefinition(types: DefinitionListTypes, list: ProseNode, content?: Fragment): ProseNode {
  return types.description.create({ gapBefore: list.attrs.spread ? 1 : 0 }, types.paragraph.create(null, content));
}

/** The cursor, when the selection is one. */
function cursorOf(state: EditorState): ResolvedPos | null {
  const { selection } = state;
  return selection instanceof TextSelection && selection.empty ? selection.$from : null;
}

/**
 * Replaces the list child at `depth` of `$pos` with nothing and puts an empty
 * paragraph after the list: Enter on an empty last entry leaves the list.
 */
function exitList(tr: Transaction, $pos: ResolvedPos, childDepth: number, types: DefinitionListTypes): Transaction {
  const list = $pos.node(childDepth - 1);
  const listStart = $pos.before(childDepth - 1);
  const paragraph = types.paragraph.create();
  if (list.childCount === 1) {
    tr.replaceWith(listStart, listStart + list.nodeSize, paragraph);
    return tr.setSelection(TextSelection.create(tr.doc, listStart + 1));
  }
  tr.delete($pos.before(childDepth), $pos.after(childDepth));
  const at = tr.mapping.map(listStart + list.nodeSize);
  tr.insert(at, paragraph);
  return tr.setSelection(TextSelection.create(tr.doc, at + 1));
}

/**
 * Enter in a term: start its first definition with the text after the
 * cursor (or step into an empty one already there); on an empty last term,
 * leave the list.
 */
export function enterInTerm(types: DefinitionListTypes): Command {
  return (state, dispatch) => {
    const $pos = cursorOf(state);
    if (!$pos || $pos.parent.type !== types.term) return false;
    const depth = $pos.depth;
    const list = $pos.node(depth - 1);
    const isLast = $pos.index(depth - 1) === list.childCount - 1;
    if (!dispatch) return true;
    const tr = state.tr;
    if ($pos.parent.content.size === 0 && isLast) {
      dispatch(exitList(tr, $pos, depth, types).scrollIntoView());
      return true;
    }
    const termEnd = $pos.after(depth);
    const next = isLast ? null : list.child($pos.index(depth - 1) + 1);
    if ($pos.pos === $pos.end(depth) && next && isEmptyDefinition(next)) {
      // At the end of a term whose definition is still empty (a fresh list
      // from the slash menu): go there rather than add another.
      dispatch(tr.setSelection(TextSelection.create(tr.doc, termEnd + 2)).scrollIntoView());
      return true;
    }
    const rest = $pos.parent.content.cut($pos.parentOffset);
    tr.delete($pos.pos, $pos.end(depth));
    const at = tr.mapping.map(termEnd);
    tr.insert(at, newDefinition(types, list, rest));
    dispatch(tr.setSelection(TextSelection.create(tr.doc, at + 2)).scrollIntoView());
    return true;
  };
}

/** Enter in a definition's last paragraph: next definition, or leave the list. */
export function enterInDefinition(types: DefinitionListTypes): Command {
  return (state, dispatch) => {
    const $pos = cursorOf(state);
    if (!$pos || $pos.parent.type !== types.paragraph || $pos.depth < 2) return false;
    const ddDepth = $pos.depth - 1;
    const definition = $pos.node(ddDepth);
    if (definition.type !== types.description) return false;
    if ($pos.index(ddDepth) !== definition.childCount - 1) return false;
    const list = $pos.node(ddDepth - 1);
    const isLast = $pos.index(ddDepth - 1) === list.childCount - 1;
    if (!dispatch) return true;
    const tr = state.tr;
    if (isEmptyDefinition(definition)) {
      if (isLast) {
        dispatch(exitList(tr, $pos, ddDepth, types).scrollIntoView());
      } else {
        const start = $pos.before(ddDepth);
        tr.replaceWith(start, start + definition.nodeSize, types.term.create());
        dispatch(tr.setSelection(TextSelection.create(tr.doc, start + 1)).scrollIntoView());
      }
      return true;
    }
    const rest = $pos.parent.content.cut($pos.parentOffset);
    const definitionEnd = $pos.after(ddDepth);
    if ($pos.parent.content.size === 0) {
      // An empty trailing paragraph after other blocks: move it out as the next definition.
      tr.delete($pos.before(), $pos.after());
    } else {
      tr.delete($pos.pos, $pos.end());
    }
    const at = tr.mapping.map(definitionEnd);
    tr.insert(at, newDefinition(types, list, rest));
    dispatch(tr.setSelection(TextSelection.create(tr.doc, at + 2)).scrollIntoView());
    return true;
  };
}

/**
 * Backspace at the very start of a definition: merge its first paragraph's
 * text into the definition or term above (undoing Enter), or — in a list
 * that starts with a definition — lift that paragraph out before the list.
 */
export function backspaceAtDefinitionStart(types: DefinitionListTypes): Command {
  return (state, dispatch) => {
    const $pos = cursorOf(state);
    if (!$pos || $pos.parentOffset !== 0 || $pos.parent.type !== types.paragraph || $pos.depth < 2) return false;
    const ddDepth = $pos.depth - 1;
    const definition = $pos.node(ddDepth);
    if (definition.type !== types.description || $pos.index(ddDepth) !== 0) return false;
    const listDepth = ddDepth - 1;
    const index = $pos.index(listDepth);
    const list = $pos.node(listDepth);
    const previous = index > 0 ? list.child(index - 1) : null;
    const paragraph = $pos.parent;
    const ddStart = $pos.before(ddDepth);
    const ddEnd = $pos.after(ddDepth);
    // What stays a definition once the first paragraph has gone.
    const remaining = definition.content.cut(paragraph.nodeSize);

    if (!previous) {
      if (!dispatch) return true;
      const listStart = $pos.before(listDepth);
      const rest: ProseNode[] = remaining.size > 0 ? [definition.copy(remaining)] : [];
      for (let i = 1; i < list.childCount; i++) rest.push(list.child(i));
      const replacement = rest.length > 0 ? [paragraph, list.copy(Fragment.fromArray(rest))] : [paragraph];
      const tr = state.tr.replaceWith(listStart, $pos.after(listDepth), replacement);
      dispatch(tr.setSelection(TextSelection.create(tr.doc, listStart + 1)).scrollIntoView());
      return true;
    }

    // The textblock the text joins: the term, or the definition's last block.
    const target = previous.type === types.term ? previous : previous.lastChild;
    if (!target?.isTextblock) return false;
    if (!dispatch) return true;
    const tr = state.tr;
    const joinAt = ddStart - 1 - (previous.type === types.term ? 0 : 1);
    if (remaining.size > 0) tr.replaceWith(ddStart, ddEnd, definition.copy(remaining));
    else tr.delete(ddStart, ddEnd);
    tr.insert(joinAt, paragraph.content);
    dispatch(tr.setSelection(TextSelection.create(tr.doc, joinAt)).scrollIntoView());
    return true;
  };
}

/** Replaces the cursor's empty paragraph with a new list: an empty term and its first definition. */
export function insertDefinitionList(types: DefinitionListTypes): Command {
  return (state, dispatch) => {
    const $pos = cursorOf(state);
    if (!$pos || $pos.parent.type !== types.paragraph) return false;
    const list = types.list.create(null, [
      types.term.create(),
      types.description.create(null, types.paragraph.create()),
    ]);
    const start = $pos.before();
    const end = $pos.after();
    const parent = $pos.node(-1);
    if (!parent.canReplaceWith($pos.index(-1), $pos.index(-1) + 1, types.list)) return false;
    if (!dispatch) return true;
    const tr = state.tr.replaceWith(start, end, list);
    dispatch(tr.setSelection(TextSelection.create(tr.doc, start + 2)).scrollIntoView());
    return true;
  };
}

/**
 * Typing `: ` at the start of a paragraph right after a paragraph or a
 * definition list does what it does in markdown: the paragraph above becomes
 * the term (or the list grows a definition).
 */
export function definitionInputRule(types: DefinitionListTypes): InputRule {
  return new InputRule(/^:\s$/, (state, _match, start, end) => {
    const $start = state.doc.resolve(start);
    if ($start.parent.type !== types.paragraph || $start.parentOffset !== 0) return null;
    const container = $start.node(-1);
    const index = $start.index(-1);
    const previous = index > 0 ? container.child(index - 1) : null;
    if (!previous || (previous.type !== types.paragraph && previous.type !== types.list)) return null;
    const paragraphStart = $start.before();
    const paragraphEnd = $start.after();
    const content = $start.parent.content.cut(end - start);
    const tr = state.tr;
    if (previous.type === types.list) {
      const definition = newDefinition(types, previous, content);
      // Into the list, just before its closing token.
      tr.delete(paragraphStart, paragraphEnd);
      const at = paragraphStart - 1;
      tr.insert(at, definition);
      return tr.setSelection(TextSelection.create(tr.doc, at + 2));
    }
    const term = types.term.create(null, previous.content);
    const list = types.list.create(null, [term, newDefinition(types, types.list.create(), content)]);
    const listStart = paragraphStart - previous.nodeSize;
    tr.replaceWith(listStart, paragraphEnd, list);
    return tr.setSelection(TextSelection.create(tr.doc, listStart + 1 + term.nodeSize + 2));
  });
}

const enterInTermCommand = $command('DefinitionListEnterInTerm', (ctx) => () => enterInTerm(typesOf(ctx)));
const enterInDefinitionCommand = $command(
  'DefinitionListEnterInDefinition',
  (ctx) => () => enterInDefinition(typesOf(ctx)),
);
const backspaceCommand = $command(
  'DefinitionListBackspace',
  (ctx) => () => backspaceAtDefinitionStart(typesOf(ctx)),
);
const insertCommand = $command('InsertDefinitionList', (ctx) => () => insertDefinitionList(typesOf(ctx)));

/** Above the paragraph/list defaults (50), which would otherwise split the term like any textblock. */
const KEY_PRIORITY = 100;

// One entry per key: `$useKeymap` keys its shortcuts by key name, so a
// second `Enter` entry would silently replace the first.
const definitionListKeymap = $useKeymap('definitionListKeymap', {
  Enter: {
    shortcuts: 'Enter',
    priority: KEY_PRIORITY,
    command: (ctx) => () => {
      const commands = ctx.get(commandsCtx);
      return commands.call(enterInTermCommand.key) || commands.call(enterInDefinitionCommand.key);
    },
  },
  Backspace: {
    shortcuts: 'Backspace',
    priority: KEY_PRIORITY,
    command: (ctx) => () => ctx.get(commandsCtx).call(backspaceCommand.key),
  },
});

const definitionListInputRule = $inputRule((ctx) => definitionInputRule(typesOf(ctx)));

/** Everything the editor needs for definition lists, for `MARKDOWN_EXTENSION_PLUGINS`. */
export const definitionListPlugins: MilkdownPlugin[] = [
  remarkDefinitionListPlugin,
  definitionIsBlockContainer,
  definitionListSchema,
  definitionTermSchema,
  definitionDescriptionSchema,
  enterInTermCommand,
  enterInDefinitionCommand,
  backspaceCommand,
  insertCommand,
  definitionListKeymap,
  definitionListInputRule,
].flat();

/** Crepe's slash-menu builder (the `BlockEdit` feature's `buildMenu`). */
interface SlashMenuBuilder {
  getGroup(key: string): {
    addItem(key: string, item: { label: string; icon: string; onRun?: (ctx: Ctx) => void }): unknown;
  };
}

/** A small `<dl>` glyph in Crepe's 24px icon style. */
const DEFINITION_LIST_ICON = `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path fill="currentColor" d="M4 5h11v2H4zm4 4h12v2H8zm-4 4h11v2H4zm4 4h12v2H8z"/></svg>`;

/** Adds "Definition list" to the slash menu's List group. */
export function addDefinitionListMenuItem(builder: SlashMenuBuilder): void {
  builder.getGroup('list').addItem('definition-list', {
    label: 'Definition list',
    icon: DEFINITION_LIST_ICON,
    onRun: (ctx) => {
      const commands = ctx.get(commandsCtx);
      commands.call(clearTextInCurrentBlockCommand.key);
      commands.call(insertCommand.key);
    },
  });
}

/**
 * The editor's definition-list look: theme.css's FAQ list (question weight,
 * separators, indented answers) without the collapsing — every answer is
 * always open, since it is being edited.
 */
export const DEFINITION_LIST_CSS = `
.milkdown .ProseMirror dl.mbr-dl {
  margin: 0.75rem 0;
  border-bottom: 1px solid var(--pico-muted-border-color, #e1e2e8);
}
.milkdown .ProseMirror dl.mbr-dl > dt {
  font-weight: 600;
  border-top: 1px solid var(--pico-muted-border-color, #e1e2e8);
  padding-top: 0.5rem;
  margin: 0;
}
.milkdown .ProseMirror dl.mbr-dl > dd {
  margin: 0 0 0 1.5rem;
  padding-bottom: 0.25rem;
  color: var(--pico-muted-color, #43474e);
}
.milkdown .ProseMirror dl.mbr-dl > dt + dd {
  padding-top: 0.15rem;
}
.milkdown .ProseMirror dl.mbr-dl > dd > p {
  margin: 0.15rem 0;
}
.milkdown .ProseMirror dl.mbr-dl > dt:empty::before,
.milkdown .ProseMirror dl.mbr-dl > dt:has(> br:only-child)::before {
  content: 'Term';
  color: var(--pico-muted-color, #73777f);
  font-weight: 400;
  pointer-events: none;
}
`;
