/**
 * Wikilinks — `[[target]]`, `[[target|alias]]`, `[[target#heading]]`,
 * `![[embed]]` — as a real inline construct in the editor (TODO "Editor:
 * cover mbr's markdown extensions", item 4). Replaces the post-save
 * `unescapeWikilinks` string patch.
 *
 * The renderer gets wikilinks from pulldown-cmark's `ENABLE_WIKILINKS`
 * (`[[Source:value]]` tag links are pre-rewritten by `wikilink.rs`, but are
 * the same `[[…]]` syntax). Its rules, mirrored by {@link parseWikilinkSource}
 * and the micromark tokenizer below:
 *
 * - `[[` opens and the first `]]` closes (a lone `]` inside is content:
 *   `[[a]b]]` links `a]b`); an optional `!` before `[[` makes it an embed
 *   (pulldown-cmark's image form).
 * - The first `|` splits target from alias; the part before it must not be
 *   empty (`[[|x]]` is text), the alias may be (`[[x|]]` links `x`).
 *   Everything is raw — a `\` does not escape a `|`, as in pulldown-cmark's
 *   `scan_wikilink_pipe`.
 * - `#` in the target names a heading (`[[Note#Heading]]`, `[[#Heading]]`);
 *   the target/heading split is display only, the source is kept whole.
 *
 * These are pinned on the Rust side by `editor_parity`'s
 * `wikilink_edges_the_editor_mirrors`. Narrower than pulldown-cmark on
 * purpose, in two shapes no one writes: a wikilink here never spans a line
 * break and never contains `[` (`wikilink.rs` refuses multi-line tag links for
 * the same reason). Such text stays text, as it did before.
 *
 * Each wikilink becomes an atom ProseMirror node (`mbr_wikilink`) shown as a
 * link chip, double-click to edit its source, and printed back byte for byte.
 * Typing a closing `]]` turns `[[…]]` into the node, and the `[[`
 * autocomplete (`editor-link-autocomplete.ts`) inserts the node directly.
 */
import type { MilkdownPlugin } from '@milkdown/kit/ctx';
import { InputRule } from '@milkdown/kit/prose/inputrules';
import type { Node as ProseNode, Schema } from '@milkdown/kit/prose/model';
import { NodeSelection } from '@milkdown/kit/prose/state';
import type { NodeView } from '@milkdown/kit/prose/view';
import { $inputRule, $nodeSchema, $remark, $view } from '@milkdown/kit/utils';
import { addRemarkExtensions, type MdNode } from './remark-support.js';
import { openSourcePopover } from './source-popover.js';

/** The mdast node type a wikilink parses into. */
export const WIKILINK_TYPE = 'wikiLink';

/** The ProseMirror node name. */
export const WIKILINK_NODE = 'mbr_wikilink';

/** A wikilink's parts; {@link wikilinkSource} reassembles the exact source. */
export interface Wikilink {
  /** `![[…]]`. */
  embed: boolean;
  /** Before the first `#` (may be empty: `[[#Heading]]`). */
  target: string;
  /** After the first `#` of the target part, or `null` without one. */
  heading: string | null;
  /** After the first `|`, or `null` without one. */
  alias: string | null;
}

const WIKILINK_SOURCE = /^(!?)\[\[([^[\n\r]+)\]\]$/;

/** `source` as a wikilink, or `null` when the renderer would not make one of it. */
export function parseWikilinkSource(source: string): Wikilink | null {
  const match = WIKILINK_SOURCE.exec(source);
  // The first `]]` closes, so the inner text holds none and does not end in `]`.
  if (!match || match[2].includes(']]') || match[2].endsWith(']')) return null;
  const inner = match[2];
  const pipe = inner.indexOf('|');
  const head = pipe === -1 ? inner : inner.slice(0, pipe);
  const alias = pipe === -1 ? null : inner.slice(pipe + 1);
  if (head === '') return null;
  const hash = head.indexOf('#');
  return {
    embed: match[1] === '!',
    target: hash === -1 ? head : head.slice(0, hash),
    heading: hash === -1 ? null : head.slice(hash + 1),
    alias,
  };
}

/** The markdown source of `link`, exactly as it was parsed. */
export function wikilinkSource(link: Wikilink): string {
  const heading = link.heading === null ? '' : `#${link.heading}`;
  const alias = link.alias === null ? '' : `|${link.alias}`;
  return `${link.embed ? '!' : ''}[[${link.target}${heading}${alias}]]`;
}

/** What a chip shows: the alias, else the target and heading. */
export function wikilinkLabel(link: Wikilink): string {
  if (link.alias) return link.alias;
  if (link.heading === null) return link.target;
  return link.target === '' ? `#${link.heading}` : `${link.target} › ${link.heading}`;
}

// ---------------------------------------------------------------------------
// micromark: the `[[…]]` tokenizer
// ---------------------------------------------------------------------------

/** micromark character codes (`null` is end of input; line endings are < -2). */
type Code = number | null;
type State = (code: Code) => State | undefined;
interface Effects {
  enter: (type: string) => unknown;
  exit: (type: string) => unknown;
  consume: (code: Code) => void;
}

const EXCLAMATION = 33;
const LEFT_BRACKET = 91;
const RIGHT_BRACKET = 93;
const PIPE = 124;
const TOKEN = 'mbrWikiLink';

function tokenizeWikilink(effects: Effects, ok: State, nok: State): State {
  // Length of the part before the first `|`, which must not be empty.
  let headSize = 0;
  let piped = false;

  const content = (code: Code): void => {
    if (code === PIPE && !piped) piped = true;
    else if (!piped) headSize++;
  };
  const afterBracket: State = (code) => {
    if (code === RIGHT_BRACKET) {
      if (headSize === 0) return nok(code);
      effects.consume(code);
      effects.exit(TOKEN);
      return ok;
    }
    // A lone `]` is content.
    content(RIGHT_BRACKET);
    return inside(code);
  };
  const inside: State = (code) => {
    if (code === null || code < -2 || code === LEFT_BRACKET) return nok(code);
    if (code === RIGHT_BRACKET) {
      effects.consume(code);
      return afterBracket;
    }
    content(code);
    effects.consume(code);
    return inside;
  };
  const secondBracket: State = (code) => {
    if (code !== LEFT_BRACKET) return nok(code);
    effects.consume(code);
    return inside;
  };
  const firstBracket: State = (code) => {
    if (code !== LEFT_BRACKET) return nok(code);
    effects.consume(code);
    return secondBracket;
  };
  return (code) => {
    effects.enter(TOKEN);
    if (code === EXCLAMATION) {
      effects.consume(code);
      return firstBracket;
    }
    return firstBracket(code);
  };
}

const wikilinkConstruct = { name: 'mbrWikiLink', tokenize: tokenizeWikilink };

/** The micromark syntax extension (text constructs at `[` and `!`). */
export const wikilinkSyntax = {
  text: { [LEFT_BRACKET]: wikilinkConstruct, [EXCLAMATION]: wikilinkConstruct },
};

interface FromMarkdownContext {
  enter: (node: MdNode, token: unknown) => void;
  exit: (token: unknown) => void;
  sliceSerialize: (token: unknown) => string;
  stack: MdNode[];
}

/** The mdast-util-from-markdown extension: a `wikiLink` node carrying its parts. */
export const wikilinkFromMarkdown = {
  enter: {
    [TOKEN](this: FromMarkdownContext, token: unknown) {
      this.enter({ type: WIKILINK_TYPE, value: '' }, token);
    },
  },
  exit: {
    [TOKEN](this: FromMarkdownContext, token: unknown) {
      const node = this.stack[this.stack.length - 1];
      const source = this.sliceSerialize(token);
      Object.assign(node, parseWikilinkSource(source), { value: source });
      this.exit(token);
    },
  },
};

/** The mdast-util-to-markdown extension: a `wikiLink` prints its source, unescaped. */
export const wikilinkToMarkdown = {
  handlers: { [WIKILINK_TYPE]: (node: MdNode) => String(node.value ?? '') },
};

const wikilinkRemark = $remark('mbrWikilinks', () => function wikilinks(this: unknown) {
  addRemarkExtensions(this, {
    micromark: wikilinkSyntax,
    fromMarkdown: wikilinkFromMarkdown,
    toMarkdown: wikilinkToMarkdown,
  });
});

// ---------------------------------------------------------------------------
// ProseMirror: the atom node, its chip, and the `]]` input rule
// ---------------------------------------------------------------------------

function attrsOf(node: ProseNode): Wikilink {
  const { embed, target, heading, alias } = node.attrs as Wikilink;
  return { embed: Boolean(embed), target: String(target), heading, alias };
}

/** The atom inline node. */
export const wikilinkSchema = $nodeSchema(WIKILINK_NODE, () => ({
  group: 'inline',
  inline: true,
  atom: true,
  selectable: true,
  draggable: true,
  attrs: {
    embed: { default: false },
    target: { default: '' },
    heading: { default: null },
    alias: { default: null },
  },
  leafText: (node) => wikilinkSource(attrsOf(node)),
  parseDOM: [
    {
      tag: 'span[data-mbr-wikilink]',
      priority: 60,
      getAttrs: (dom) => parseWikilinkSource((dom as HTMLElement).dataset.mbrWikilink ?? '') ?? false,
    },
  ],
  toDOM: (node) => [
    'span',
    { 'data-mbr-wikilink': wikilinkSource(attrsOf(node)), class: 'mbr-wikilink' },
    wikilinkLabel(attrsOf(node)),
  ],
  parseMarkdown: {
    match: (node) => node.type === WIKILINK_TYPE,
    runner: (state, node, type) => {
      const link = parseWikilinkSource(String(node.value ?? ''));
      if (link) state.addNode(type, { ...link });
      else state.addText(String(node.value ?? ''));
    },
  },
  toMarkdown: {
    match: (node) => node.type.name === WIKILINK_NODE,
    runner: (state, node) => {
      state.addNode(WIKILINK_TYPE, undefined, wikilinkSource(attrsOf(node)));
    },
  },
}));

/**
 * A wikilink node for `source` in `schema`, or `null` when `source` is not a
 * wikilink (or the schema has no wikilink node).
 */
export function wikilinkNode(schema: Schema, source: string): ProseNode | null {
  const type = schema.nodes[WIKILINK_NODE];
  const link = parseWikilinkSource(source);
  return type && link ? type.create({ ...link }) : null;
}

/** Typing the closing `]` of `[[…]]` (or `![[…]]`) makes the node. */
const wikilinkInputRule = $inputRule(
  () =>
    new InputRule(/!?\[\[[^[\n]+\]\]$/, (state, match, start, end) => {
      const node = wikilinkNode(state.schema, match[0]);
      return node ? state.tr.replaceWith(start, end, node) : null;
    }),
);

const VIEW_CSS_ID = 'mbr-wikilink-style';
const VIEW_CSS = `
.mbr-wikilink {
  display: inline-block; padding: 0 0.35em; border-radius: 0.35em; cursor: default;
  color: var(--pico-primary, #1095c1); background: var(--pico-primary-focus, rgba(16,149,193,0.12));
  text-decoration: underline; text-decoration-style: dotted; text-underline-offset: 0.15em;
  white-space: pre-wrap;
}
.mbr-wikilink.embed::before { content: '↪ '; text-decoration: none; opacity: 0.7; }
.mbr-wikilink.ProseMirror-selectednode { outline: 2px solid var(--pico-primary, #1095c1); }
`;

function ensureViewStyle(): void {
  if (document.getElementById(VIEW_CSS_ID)) return;
  const style = document.createElement('style');
  style.id = VIEW_CSS_ID;
  style.textContent = VIEW_CSS;
  document.head.appendChild(style);
}

/** `null` when `value` may replace a wikilink's source (empty removes it). */
export function validateWikilinkSource(value: string): string | null {
  const trimmed = value.trim();
  if (trimmed === '' || parseWikilinkSource(trimmed)) return null;
  return 'Write [[Note]], [[Note|text]], [[Note#Heading]] or ![[file]]';
}

/** The link-chip node view; double-click edits the source. */
const wikilinkView = $view(wikilinkSchema.node, () => (initial, view, getPos): NodeView => {
  ensureViewStyle();
  let node = initial;
  const dom = document.createElement('span');
  const render = (): void => {
    const link = attrsOf(node);
    dom.className = link.embed ? 'mbr-wikilink embed' : 'mbr-wikilink';
    dom.textContent = wikilinkLabel(link);
    dom.title = `${wikilinkSource(link)} (double-click to edit)`;
  };
  render();

  dom.addEventListener('dblclick', (event) => {
    event.preventDefault();
    const pos = getPos();
    if (pos === undefined) return;
    view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, pos)));
    openSourcePopover({
      anchor: dom,
      label: 'Wikilink',
      value: wikilinkSource(attrsOf(node)),
      hint: '[[Note]] · [[Note|text]] · [[Note#Heading]] · ![[file]] — empty removes it',
      validate: validateWikilinkSource,
      onCommit: (value) => {
        const at = getPos();
        if (at === undefined) return;
        const trimmed = value.trim();
        const { state } = view;
        const replacement = trimmed === '' ? null : wikilinkNode(state.schema, trimmed);
        const tr = replacement
          ? state.tr.replaceWith(at, at + node.nodeSize, replacement)
          : state.tr.delete(at, at + node.nodeSize);
        view.dispatch(tr);
        view.focus();
      },
      onCancel: () => view.focus(),
    });
  });

  return {
    dom,
    update: (next) => {
      if (next.type !== node.type) return false;
      node = next;
      render();
      return true;
    },
    selectNode: () => dom.classList.add('ProseMirror-selectednode'),
    deselectNode: () => dom.classList.remove('ProseMirror-selectednode'),
    ignoreMutation: () => true,
  };
});

/** Everything item 4 registers: `MARKDOWN_EXTENSION_PLUGINS` lists this. */
export const wikilinksPlugin: MilkdownPlugin[] = [
  ...wikilinkRemark,
  ...wikilinkSchema,
  wikilinkInputRule,
  wikilinkView,
];
