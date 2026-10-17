/**
 * Bare URLs stay bare.
 *
 * The editor parses with remark-gfm, whose autolink literals turn
 * `https://example.com`, `www.example.com` and `someone@example.com` in
 * running text into links. remark-stringify prints a link whose text is its
 * URL as an autolink, `<https://example.com>` — and pulldown-cmark, which has
 * no autolink literals, renders that as a link where the note had plain text.
 * So every bare URL in a note was rewritten (the most common change
 * `roundtrip-check` found).
 *
 * Here a link is marked `literal` at parse when its source starts with
 * neither `<` nor `[` ({@link recordLiteralLinks}), the mark carries that
 * (`mbrStyle` on Milkdown's link mark), and a literal link whose text still
 * reads as its URL prints as that text ({@link literalText}). One the reader
 * changed (other text, another URL) prints as remark prints any link.
 */
import type { Link, Parents } from 'mdast';
import { defaultHandlers, type Info, type State } from 'mdast-util-to-markdown';
import type { Mark, Node as ProseNode } from '@milkdown/kit/prose/model';
import { linkSchema } from '@milkdown/kit/preset/commonmark';
import type { MarkSchema, ParserState, SerializerState } from '@milkdown/kit/transformer';
import { $remark } from '@milkdown/kit/utils';
import { keepLinkEdgeSpaces } from './link-spaces.js';
import { addRemarkExtensions, type MdNode } from './remark-support.js';

/** A link typed as a bare URL (a GFM autolink literal); `at`, where it was parsed (see `link-spaces.ts`). */
export interface LinkStyle {
  literal?: true;
  at?: number;
}

type Positioned = MdNode & { position?: { start: { offset?: number } }; mbrStyle?: unknown };

/** Marks every link in `tree` whose source is a bare URL (not `<…>`, not `[…](…)`). */
export function recordLiteralLinks(tree: MdNode, source: string): void {
  const visit = (node: Positioned): void => {
    if (node.type === 'link') {
      const at = node.position?.start.offset;
      if (at !== undefined) {
        const literal = source[at] !== '<' && source[at] !== '[';
        node.mbrStyle = (literal ? { literal: true, at } : { at }) satisfies LinkStyle;
      }
    }
    for (const child of node.children ?? []) visit(child as Positioned);
  };
  visit(tree as Positioned);
}

/**
 * The text a literal link prints as: its one text child, when the URL is what
 * GFM makes of that text (itself, `http://` + a `www.` text, `mailto:` + an
 * address); `undefined` otherwise.
 */
export function literalText(node: Link): string | undefined {
  const [only, ...rest] = node.children;
  if (!only || rest.length > 0 || only.type !== 'text' || node.title) return undefined;
  const text = only.value;
  return node.url === text || node.url === `http://${text}` || node.url === `mailto:${text}` ? text : undefined;
}

function printLink(node: Link, parent: Parents | undefined, state: State, info: Info): string {
  const literal = (node as Link & { mbrStyle?: LinkStyle }).mbrStyle?.literal ? literalText(node) : undefined;
  return literal ?? defaultHandlers.link(node, parent, state, info);
}

/** Records literals at parse; prints them back bare. */
export const remarkAutolinkLiterals = $remark('mbrAutolinkLiterals', () => function autolinkLiterals(this: unknown) {
  addRemarkExtensions(this, { toMarkdown: { handlers: { link: printLink as never } } });
  return (tree: unknown, file: { value?: unknown }) => recordLiteralLinks(tree as MdNode, String(file.value ?? ''));
});

/** Runs `run` with `extra` merged into the attrs (`argIndex`) of the first call of `method` on `target`. */
function withFirstCall(target: object, method: string, argIndex: number, extra: Record<string, unknown>, run: () => void): void {
  const host = target as Record<string, (...args: unknown[]) => unknown>;
  const own = host[method];
  host[method] = function patched(this: unknown, ...args: unknown[]) {
    host[method] = own;
    args[argIndex] = { ...(args[argIndex] as object | undefined), ...extra };
    return own.apply(target, args);
  };
  try {
    run();
  } finally {
    host[method] = own;
  }
}

/** Milkdown's link mark with an `mbrStyle` attr, filled from and handed back to the mdast link. */
export const literalLinkSchema = linkSchema.extendSchema((prev) => (ctx) => {
  const base: MarkSchema = prev(ctx);
  return {
    ...base,
    attrs: { ...base.attrs, mbrStyle: { default: null } },
    parseMarkdown: {
      match: base.parseMarkdown.match,
      runner: (state: ParserState, node, markType) => {
        const style = (node as Positioned).mbrStyle;
        const run = () => base.parseMarkdown.runner(state, node, markType);
        if (style === undefined) run();
        else withFirstCall(state, 'openMark', 1, { mbrStyle: style }, run);
      },
    },
    toMarkdown: {
      match: base.toMarkdown.match,
      runner: (state: SerializerState, mark: Mark, node: ProseNode) => {
        keepLinkEdgeSpaces(state);
        const style: unknown = mark.attrs.mbrStyle;
        let result: void | boolean = undefined;
        const run = () => {
          result = base.toMarkdown.runner(state, mark, node);
        };
        // `withMark(mark, type, value, props)`.
        if (style == null) run();
        else withFirstCall(state, 'withMark', 3, { mbrStyle: style }, run);
        return result;
      },
    },
  };
});

export const autolinkLiteralsPlugin = [remarkAutolinkLiterals, literalLinkSchema].flat();
