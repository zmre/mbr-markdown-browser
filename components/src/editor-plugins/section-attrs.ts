/**
 * Section attributes — `--- {#id .class key=value}` — for the editor (TODO
 * "Editor: cover mbr's markdown extensions", item 3).
 *
 * `---` followed by anything is not a thematic break in CommonMark, so the
 * line is paragraph text to remark exactly as it is to pulldown-cmark. The
 * renderer's rule (`markdown::section_rule_attrs`, applied to the paragraph
 * pulldown-cmark makes of the line, after smart punctuation turned `---` into
 * an em dash) is: **a paragraph whose whole text is `— {…}`** — one em dash,
 * one space, then a run that starts with `{` and ends with `}` — is a section
 * rule, whatever is between the braces (`attrs.rs` parses it, and even an
 * unparseable block still makes the rule). This module states the same rule
 * over remark's tree (`sectionBreakOf`) and turns such a paragraph into a
 * `sectionBreak` mdast node, which becomes an atom ProseMirror node
 * (`mbr_section_break`) drawn as a rule with an attributes chip, edited
 * through a small popover, and printed back as its source line, verbatim.
 *
 * A plain `---` stays Milkdown's `hr`. A `--- {…}` line *directly under* a
 * paragraph (no blank line) is a continuation of that paragraph to both
 * parsers — not a setext underline, since `{` is not a `-` — so it stays
 * paragraph text; but remark-stringify escapes a `-` at the start of a line
 * (`\--- {…}`), which pulldown-cmark then reads as a different paragraph.
 * {@link keepSectionAttrLines} undoes exactly that escape.
 */
import { initTimerCtx, remarkStringifyOptionsCtx } from '@milkdown/kit/core';
import { createTimer, type MilkdownPlugin } from '@milkdown/kit/ctx';
import { NodeSelection } from '@milkdown/kit/prose/state';
import type { Node as ProseNode } from '@milkdown/kit/prose/model';
import type { EditorView, NodeView } from '@milkdown/kit/prose/view';
import { $nodeSchema, $remark, $view } from '@milkdown/kit/utils';
import { openSourcePopover } from './source-popover.js';
import { addRemarkExtensions, type MdNode } from './remark-support.js';

/** The mdast node type a section break parses into. */
export const SECTION_BREAK_TYPE = 'sectionBreak';

/** The ProseMirror node name. */
export const SECTION_BREAK_NODE = 'mbr_section_break';

/**
 * The source spellings of the em dash the renderer's rule starts with: `---`
 * (smart punctuation), the character itself, and its entity references
 * (pulldown-cmark merges a decoded entity into the text run around it).
 */
const SECTION_LINE = /^(---|—|&mdash;|&#8212;|&#x2014;) (\{.*\})$/i;

/** A section break's source line, split into its marker and its `{…}` block. */
export interface SectionBreak {
  /** How the em dash is spelled: `---`, `—` or an entity reference. */
  marker: string;
  /** The attribute block, braces included, exactly as written. */
  attrs: string;
}

/** The source line of a section break. */
export function sectionBreakSource(br: SectionBreak): string {
  return `${br.marker} ${br.attrs}`;
}

/** `line` as a section break, or `null`: the renderer's rule on one source line. */
export function parseSectionLine(line: string): SectionBreak | null {
  const match = SECTION_LINE.exec(line);
  return match ? { marker: match[1], attrs: match[2] } : null;
}

/**
 * The section break a paragraph is, or `null`.
 *
 * The renderer requires the paragraph to be one merged text run, so here every
 * inline child must be text — or a GFM autolink literal, which remark makes a
 * link of but pulldown-cmark (no literal autolinks) leaves as text, so a
 * `data-background-iframe=https://…` attribute still counts. Emphasis, code,
 * HTML, a real link, a line break: not a section break, in either parser.
 * The source itself must then match: that rejects an escaped `\---`, which
 * remark's text would hide.
 */
export function sectionBreakOf(paragraph: MdNode, source: string): SectionBreak | null {
  const start = paragraph.position?.start.offset;
  const end = paragraph.position?.end.offset;
  if (start === undefined || end === undefined) return null;
  const plain = (paragraph.children ?? []).every((child) => {
    if (child.type === 'text') return true;
    const at = child.position?.start.offset;
    return child.type === 'link' && at !== undefined && source[at] !== '[' && source[at] !== '<';
  });
  if (!plain) return null;
  const raw = source.slice(start, end).trim();
  return raw.includes('\n') ? null : parseSectionLine(raw);
}

/** Replaces every section-break paragraph in `tree`, at any depth. */
export function transformSectionBreaks(tree: MdNode, source: string): void {
  const children = tree.children;
  if (!children) return;
  for (let i = 0; i < children.length; i++) {
    const child = children[i];
    const br = child.type === 'paragraph' ? sectionBreakOf(child, source) : null;
    if (br) {
      children[i] = { type: SECTION_BREAK_TYPE, position: child.position, marker: br.marker, attrs: br.attrs };
    } else {
      transformSectionBreaks(child, source);
    }
  }
}

/** Text handler signature (mdast-util-to-markdown's `Handle`, narrowed). */
type TextHandle = (node: { value: string }, parent: unknown, state: unknown, info: unknown) => string;

/**
 * Wraps the text handler so a line of a paragraph that is a section-attribute
 * line keeps its `-`: remark-stringify escapes a line-initial `-` followed by
 * a space or a `-` (it could start a list or a rule), but `--- {…}` can be
 * neither, nor a setext underline. Only a line remark escaped and that is,
 * unescaped, exactly the source text is restored, so nothing else changes.
 */
export function keepSectionAttrLines(inner: TextHandle): TextHandle {
  return (node, parent, state, info) => {
    const out = inner(node, parent, state, info);
    if (!node.value.includes('---') && !node.value.includes('—')) return out;
    const src = node.value.split('\n');
    const dst = out.split('\n');
    if (src.length !== dst.length) return out;
    return dst
      .map((line, i) => (line === `\\${src[i]}` && SECTION_LINE.test(src[i]) ? src[i] : line))
      .join('\n');
  };
}

const TextHandlerReady = createTimer('MbrSectionAttrsTextHandler');

/**
 * Installs {@link keepSectionAttrLines} around the stringify options' text
 * handler. Milkdown passes its own `text` handler as a top-level stringify
 * option, which overrides any handler a remark plugin's `toMarkdownExtensions`
 * could add, so the option itself is wrapped — before `init` builds the
 * processor from it (hence the init timer).
 */
const sectionTextHandler: MilkdownPlugin = (ctx) => {
  ctx.record(TextHandlerReady);
  ctx.update(initTimerCtx, (timers) => [...timers, TextHandlerReady]);
  return async () => {
    ctx.update(remarkStringifyOptionsCtx, (options) => {
      const text = options.handlers?.text as TextHandle | undefined;
      if (!text) return options;
      return { ...options, handlers: { ...options.handlers, text: keepSectionAttrLines(text) } };
    });
    ctx.done(TextHandlerReady);
    return () => {
      ctx.clearTimer(TextHandlerReady);
    };
  };
};

/** Parse (`sectionBreak` nodes) and print (the source line) for remark. */
const sectionAttrsRemark = $remark('mbrSectionAttrs', () => function sectionAttrs(this: unknown) {
  addRemarkExtensions(this, {
    toMarkdown: {
      handlers: {
        [SECTION_BREAK_TYPE]: (node: MdNode) =>
          sectionBreakSource({ marker: String(node.marker), attrs: String(node.attrs) }),
      },
    },
  });
  return (tree: unknown, file: { value?: unknown }) => {
    transformSectionBreaks(tree as MdNode, String(file.value ?? ''));
  };
});

/** The atom node: a rule carrying its source marker and attribute block. */
export const sectionBreakSchema = $nodeSchema(SECTION_BREAK_NODE, () => ({
  group: 'block',
  atom: true,
  selectable: true,
  draggable: true,
  attrs: { marker: { default: '---' }, attrs: { default: '{}' } },
  parseDOM: [
    {
      tag: 'div[data-mbr-section-break]',
      priority: 60,
      getAttrs: (dom) => ({
        marker: (dom as HTMLElement).dataset.marker ?? '---',
        attrs: (dom as HTMLElement).dataset.attrs ?? '{}',
      }),
    },
  ],
  toDOM: (node) => [
    'div',
    {
      'data-mbr-section-break': '',
      'data-marker': node.attrs.marker,
      'data-attrs': node.attrs.attrs,
      class: 'mbr-section-break',
    },
    ['hr'],
  ],
  parseMarkdown: {
    match: (node) => node.type === SECTION_BREAK_TYPE,
    runner: (state, node, type) => {
      state.addNode(type, { marker: String(node.marker ?? '---'), attrs: String(node.attrs ?? '{}') });
    },
  },
  toMarkdown: {
    match: (node) => node.type.name === SECTION_BREAK_NODE,
    runner: (state, node) => {
      state.addNode(SECTION_BREAK_TYPE, undefined, undefined, {
        marker: node.attrs.marker,
        attrs: node.attrs.attrs,
      });
    },
  },
}));

const VIEW_CSS_ID = 'mbr-section-break-style';
const VIEW_CSS = `
.mbr-section-break { position: relative; margin: 1.25rem 0; }
.mbr-section-break hr { margin: 0; }
.mbr-section-break-chip {
  position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%);
  max-width: 80%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  margin: 0; padding: 0.1rem 0.5rem; width: auto; height: auto; line-height: 1.4;
  font: 0.75rem var(--pico-font-family-monospace, monospace);
  color: var(--pico-muted-color, #555); background: var(--pico-background-color, #fff);
  border: 1px solid var(--pico-muted-border-color, #ccc); border-radius: 999px; cursor: pointer;
}
.mbr-section-break.ProseMirror-selectednode .mbr-section-break-chip {
  border-color: var(--pico-primary, #1095c1); color: var(--pico-primary, #1095c1);
}
`;

function ensureViewStyle(): void {
  if (document.getElementById(VIEW_CSS_ID)) return;
  const style = document.createElement('style');
  style.id = VIEW_CSS_ID;
  style.textContent = VIEW_CSS;
  document.head.appendChild(style);
}

/** `null` when `value` may replace a section break's attribute block. */
export function validateSectionAttrs(value: string): string | null {
  const trimmed = value.trim();
  if (trimmed === '') return null; // removes the attributes: a plain rule
  return /^\{[^\n]*\}$/.test(trimmed) ? null : 'Write the attributes inside { }, e.g. {#intro .dark}';
}

/** Applies an edited attribute block: a new block, or a plain rule when empty. */
function commitAttrs(view: EditorView, pos: number, node: ProseNode, value: string): void {
  const trimmed = value.trim();
  const { state } = view;
  const tr =
    trimmed === ''
      ? state.tr.replaceWith(pos, pos + node.nodeSize, state.schema.nodes.hr.create())
      : state.tr.setNodeMarkup(pos, undefined, { ...node.attrs, attrs: trimmed });
  view.dispatch(tr);
  view.focus();
}

/** The rule-with-a-chip node view. */
const sectionBreakView = $view(sectionBreakSchema.node, () => (initial, view, getPos): NodeView => {
  ensureViewStyle();
  let node = initial;
  const dom = document.createElement('div');
  dom.className = 'mbr-section-break';
  dom.contentEditable = 'false';
  const rule = document.createElement('hr');
  const chip = document.createElement('button');
  chip.type = 'button';
  chip.className = 'mbr-section-break-chip';
  dom.append(rule, chip);
  const render = (): void => {
    chip.textContent = String(node.attrs.attrs);
    chip.title = `Section attributes: ${sectionBreakSource(node.attrs as SectionBreak)} (click to edit)`;
  };
  render();

  const edit = (): void => {
    const pos = getPos();
    if (pos === undefined) return;
    view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, pos)));
    openSourcePopover({
      anchor: chip,
      label: 'Section attributes',
      value: String(node.attrs.attrs),
      hint: '{#id .class key=value} — empty for a plain rule',
      validate: validateSectionAttrs,
      onCommit: (value) => {
        const at = getPos();
        if (at !== undefined) commitAttrs(view, at, node, value);
      },
      onCancel: () => view.focus(),
    });
  };
  chip.addEventListener('mousedown', (event) => event.preventDefault());
  chip.addEventListener('click', (event) => {
    event.preventDefault();
    edit();
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
    stopEvent: (event) => event.target === chip,
    ignoreMutation: () => true,
  };
});

/** Everything item 3 registers: `MARKDOWN_EXTENSION_PLUGINS` lists this. */
export const sectionAttrsPlugin: MilkdownPlugin[] = [
  sectionTextHandler,
  ...sectionAttrsRemark,
  ...sectionBreakSchema,
  sectionBreakView,
];
