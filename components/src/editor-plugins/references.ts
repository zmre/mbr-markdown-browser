/**
 * Reference links, reference images and link reference definitions, kept as
 * authored: `[text][ref]`, `[text][]`, `[ref]`, `![alt][ref]` and
 * `[ref]: https://… "Title"`.
 *
 * Milkdown's commonmark preset runs `remark-inline-links`, which rewrites
 * every reference into an inline `[text](url "title")` and deletes the
 * definitions — so a save rewrote the author's references and dropped their
 * definition block. Here the parse renames the three mdast types
 * (`mbrLinkReference`, `mbrImageReference`, `mbrDefinition`) in a
 * from-markdown transform, which runs before any remark plugin, so
 * `remark-inline-links` finds nothing to inline whatever the plugin order.
 * Serializing produces the standard `linkReference` / `imageReference` mdast,
 * printed by remark-stringify's own handlers.
 *
 * In the editor:
 * - a reference link is the `mbr_link_ref` mark (attrs: `identifier`,
 *   `label`, `referenceType`) on its text, so the text stays editable;
 * - a reference image is the `mbr_image_ref` inline atom, shown as its
 *   source;
 * - a definition is the `mbr_definition` block holding its source line(s) as
 *   plain text (monospace, edited in place, printed verbatim). `tightNext`
 *   records that the next block followed it with no blank line — how a run
 *   of definitions is usually written — so it is printed that way again.
 */
import { $markSchema, $nodeSchema, $prose } from '@milkdown/kit/utils';
import { createVirtualCursor } from 'prosemirror-virtual-cursor';
import type { Definition, Literal } from 'mdast';
import type { CompileContext, Token } from 'mdast-util-from-markdown';
import type { Join } from 'mdast-util-to-markdown';
import { keepLinkEdgeSpaces, sourceAt } from './link-spaces.js';
import { $syntax } from './syntax-extension.js';

/**
 * What the editor serializes a definition as: its source text, printed
 * verbatim. `data.mbrTightNext`: no blank line before the next block.
 */
export interface MbrDefinition extends Literal {
  type: 'mbrDefinition';
  data?: Literal['data'] & { mbrTightNext?: boolean };
}

declare module 'mdast' {
  interface RootContentMap {
    mbrDefinition: MbrDefinition;
  }
  interface BlockContentMap {
    mbrDefinition: MbrDefinition;
  }
}

/** The renamed mdast types the editor parses (see the module docs). */
const RENAMED: Record<string, string> = {
  linkReference: 'mbrLinkReference',
  imageReference: 'mbrImageReference',
  definition: 'mbrDefinition',
};

interface PositionedNode {
  type: string;
  children?: PositionedNode[];
  position?: { start: { line: number }; end: { line: number } };
  data?: Record<string, unknown>;
}

/** Renames reference nodes and records which definitions the next block hugs. */
function renameReferences(tree: PositionedNode): void {
  const children = tree.children ?? [];
  children.forEach((child, index) => {
    if (child.type === 'definition') {
      const next = children[index + 1];
      const end = child.position?.end.line;
      const nextStart = next?.position?.start.line;
      const tightNext = end !== undefined && nextStart !== undefined && nextStart === end + 1;
      child.data = { ...child.data, mbrTightNext: tightNext };
    }
    renameReferences(child);
    const renamed = RENAMED[child.type];
    if (renamed) child.type = renamed;
  });
}

/** `[label]: url "title"` from a definition's fields, for one without source. */
export function definitionSource(node: Partial<Definition>): string {
  const url = node.url ?? '';
  const destination = url === '' || /[\s<>]/.test(url) ? `<${url}>` : url;
  const title = node.title ? ` "${node.title.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"` : '';
  return `[${node.label ?? node.identifier ?? ''}]: ${destination}${title}`;
}

/** A definition followed by a block it hugged in the source keeps hugging it. */
const joinTightDefinitions: Join = (left) =>
  left.type === 'mbrDefinition' && left.data?.mbrTightNext ? 0 : undefined;

export const referencesSyntax = {
  fromMarkdown: {
    transforms: [(tree: unknown) => renameReferences(tree as PositionedNode)],
    exit: {
      // The default is a plain close; also keep the definition's source.
      // Sliced from micromark's chunks, so it is without container prefixes.
      definition(this: CompileContext, token: Token) {
        const node = this.stack[this.stack.length - 1] as Definition;
        node.data = { ...node.data, mbrSource: this.sliceSerialize(token) } as Definition['data'];
        this.exit(token);
      },
    },
  },
  toMarkdown: {
    handlers: {
      // `?? ''`: Milkdown drops an empty `value` (never emitted: empty ones are skipped).
      mbrDefinition: (node: MbrDefinition) => node.value ?? '',
    },
    join: [joinTightDefinitions],
  },
};

/** Remark plugin: references survive parsing as themselves. */
export const remarkMbrReferences = $syntax('remarkMbrReferences', referencesSyntax);

type ReferenceType = 'full' | 'collapsed' | 'shortcut';

function referenceType(value: unknown): ReferenceType {
  return value === 'collapsed' || value === 'shortcut' ? value : 'full';
}

/** The mark on a reference link's text. */
export const linkReferenceSchema = $markSchema('mbr_link_ref', () => ({
  attrs: {
    identifier: { default: '' },
    label: { default: '' },
    referenceType: { default: 'full' },
    // `{ at }`, where the reference was parsed: keeps two adjacent references
    // to one definition (`[a][r][b][r]`) two marks, not one (see `link-spaces.ts`).
    mbrStyle: { default: null },
  },
  inclusive: false,
  // Opened before other marks when a run starts (Milkdown's serializer opens
  // lower priorities first), so `[*the* docs][ref]` keeps its emphasis
  // inside the brackets instead of splitting the reference in two.
  priority: 10,
  parseDOM: [
    {
      tag: 'span[data-type="mbr-link-ref"]',
      getAttrs: (dom: HTMLElement) => ({
        identifier: dom.dataset.identifier ?? '',
        label: dom.dataset.label ?? '',
        referenceType: referenceType(dom.dataset.referenceType),
      }),
    },
  ],
  toDOM: (mark) => [
    'span',
    {
      'data-type': 'mbr-link-ref',
      'data-identifier': mark.attrs.identifier,
      'data-label': mark.attrs.label,
      'data-reference-type': mark.attrs.referenceType,
      title: `[${mark.attrs.label}]`,
      class: 'mbr-editor-link-ref',
      style: 'color: var(--crepe-color-primary, inherit); text-decoration: underline dotted;',
    },
  ],
  parseMarkdown: {
    match: (node) => node.type === 'mbrLinkReference',
    runner: (state, node, markType) => {
      state.openMark(markType, {
        identifier: String(node.identifier ?? ''),
        label: String(node.label ?? node.identifier ?? ''),
        referenceType: referenceType(node.referenceType),
        mbrStyle: sourceAt(node),
      });
      state.next(node.children as never);
      state.closeMark(markType);
    },
  },
  toMarkdown: {
    match: (mark) => mark.type.name === 'mbr_link_ref',
    runner: (state, mark) => {
      keepLinkEdgeSpaces(state);
      state.withMark(mark, 'linkReference', undefined, {
        identifier: mark.attrs.identifier,
        label: mark.attrs.label,
        referenceType: mark.attrs.referenceType,
        ...(mark.attrs.mbrStyle ? { mbrStyle: mark.attrs.mbrStyle } : {}),
      });
    },
  },
}));

/** `![alt][ref]`: an atom showing its source. */
export const imageReferenceSchema = $nodeSchema('mbr_image_ref', () => ({
  group: 'inline',
  inline: true,
  atom: true,
  attrs: {
    alt: { default: '' },
    identifier: { default: '' },
    label: { default: '' },
    referenceType: { default: 'full' },
  },
  parseDOM: [
    {
      tag: 'span[data-type="mbr-image-ref"]',
      getAttrs: (dom: HTMLElement) => ({
        alt: dom.dataset.alt ?? '',
        identifier: dom.dataset.identifier ?? '',
        label: dom.dataset.label ?? '',
        referenceType: referenceType(dom.dataset.referenceType),
      }),
    },
  ],
  toDOM: (node) => {
    const { alt, label } = node.attrs as { alt: string; label: string };
    const type = referenceType(node.attrs.referenceType);
    const source = type === 'full' ? `![${alt}][${label}]` : type === 'collapsed' ? `![${alt}][]` : `![${alt}]`;
    return [
      'span',
      {
        'data-type': 'mbr-image-ref',
        'data-alt': alt,
        'data-identifier': node.attrs.identifier,
        'data-label': label,
        'data-reference-type': type,
        class: 'mbr-editor-image-ref',
        style: 'font-family: var(--crepe-font-code, monospace); font-size: 0.9em; opacity: 0.8;',
      },
      source,
    ];
  },
  parseMarkdown: {
    match: (node) => node.type === 'mbrImageReference',
    runner: (state, node, type) => {
      state.addNode(type, {
        alt: String(node.alt ?? ''),
        identifier: String(node.identifier ?? ''),
        label: String(node.label ?? node.identifier ?? ''),
        referenceType: referenceType(node.referenceType),
      });
    },
  },
  toMarkdown: {
    match: (node) => node.type.name === 'mbr_image_ref',
    runner: (state, node) => {
      state.addNode('imageReference', undefined, undefined, {
        alt: node.attrs.alt,
        identifier: node.attrs.identifier,
        label: node.attrs.label,
        referenceType: node.attrs.referenceType,
      });
    },
  },
}));

/** `[ref]: url "title"`: its source as editable plain text. */
export const definitionSchema = $nodeSchema('mbr_definition', () => ({
  group: 'block',
  content: 'text*',
  marks: '',
  code: true,
  defining: true,
  // `mbrStyle`: what `authored-style.ts` recorded (blank lines around it).
  attrs: { tightNext: { default: false }, mbrStyle: { default: null } },
  parseDOM: [
    {
      tag: 'div[data-type="mbr-definition"]',
      preserveWhitespace: 'full' as const,
      getAttrs: (dom: HTMLElement) => ({ tightNext: dom.dataset.tightNext === 'true' }),
    },
  ],
  toDOM: (node) => [
    'div',
    {
      'data-type': 'mbr-definition',
      'data-tight-next': node.attrs.tightNext ? 'true' : 'false',
      class: 'mbr-editor-definition',
      style:
        'font-family: var(--crepe-font-code, monospace); font-size: 0.9em; opacity: 0.8; white-space: pre-wrap;',
    },
    0,
  ],
  parseMarkdown: {
    match: (node) => node.type === 'mbrDefinition',
    runner: (state, node, type) => {
      const data = node.data as { mbrSource?: string; mbrTightNext?: boolean } | undefined;
      const source = data?.mbrSource ?? definitionSource(node as Partial<Definition>);
      const mbrStyle = (node as { mbrStyle?: unknown }).mbrStyle ?? null;
      state.addNode(type, { tightNext: data?.mbrTightNext === true, mbrStyle }, source ? [type.schema.text(source)] : []);
    },
  },
  toMarkdown: {
    match: (node) => node.type.name === 'mbr_definition',
    runner: (state, node) => {
      const source = node.textContent;
      if (!source.trim()) return;
      const style: unknown = node.attrs.mbrStyle;
      const props: Record<string, unknown> = { data: { mbrTightNext: node.attrs.tightNext === true } };
      if (style != null) props.mbrStyle = style;
      state.addNode('mbrDefinition', undefined, source, props as never);
    },
  },
}));

/** Everything the editor needs for references. */
/**
 * Crepe's virtual cursor (the caret that shows, at a mark's edge, whether
 * typing goes inside or outside it; ←/→ choose), configured for
 * `mbr_link_ref`. The plugin warns about every non-inclusive mark it was not
 * told about, and `mbr_link_ref` must be non-inclusive — typing after
 * `[text][ref]` must not grow the link — exactly like Milkdown's own
 * `inlineCode`, which Crepe lists. So this is Crepe's plugin with our mark
 * added; Crepe's own is switched off (`CREPE_FEATURE_CONFIGS` in
 * `editor-crepe.ts`), or there would be two carets.
 */
export const virtualCursorPlugin = $prose(() => createVirtualCursor({ skipWarning: ['inlineCode', 'mbr_link_ref'] }));

export const referencesPlugin = [
  remarkMbrReferences,
  linkReferenceSchema,
  imageReferenceSchema,
  definitionSchema,
  virtualCursorPlugin,
].flat();
