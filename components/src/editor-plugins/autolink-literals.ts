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
 *
 * A literal ends at `<`, so `https://example.com/?q=<a_b>` is a link and the
 * text `<a_b>`. remark escapes that `<` (`<a` could start a tag), and after a
 * bare URL the backslash is not an escape: GFM reads it as the URL's last
 * character, `https://example.com/?q=\`, and the escape check
 * (`minimal-escapes.ts`) could not drop it, since it compares against the
 * printed text. {@link keepLessThanAfterLiteral} prints that `<` bare whenever
 * it starts nothing (no tag, no autolink) — always, for text the editor
 * parsed, since a literal cannot end before a `<` that does. Where it would
 * (`<b>` typed against a URL), the link is printed as remark prints any link,
 * `<url>\<b>`, which keeps the URL whole.
 */
import type { Link, Parents, Text } from 'mdast';
import { defaultHandlers, type Info, type State } from 'mdast-util-to-markdown';
import type { Mark, Node as ProseNode } from '@milkdown/kit/prose/model';
import { linkSchema } from '@milkdown/kit/preset/commonmark';
import type { MarkSchema, ParserState, SerializerState } from '@milkdown/kit/transformer';
import { initTimerCtx, remarkStringifyOptionsCtx } from '@milkdown/kit/core';
import { createTimer, type MilkdownPlugin } from '@milkdown/kit/ctx';
import { $remark } from '@milkdown/kit/utils';
import { keepLinkEdgeSpaces } from './link-spaces.js';
import { editorCanonical } from './minimal-escapes.js';
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

/** A `<` remark escapes in text: one that could start a tag, a declaration or an autolink. */
const ESCAPED_LESS_THAN = /^<[!/?A-Za-z]/;

/**
 * Whether the `<` that opens `value`, printed right after a bare URL, needs
 * its escape (unescaped, it would start a tag or an autolink); `undefined`
 * without the editor's parse to ask (`minimal-escapes.ts` not installed).
 */
function lessThanNeedsEscape(state: State, value: string): boolean | undefined {
  if (!ESCAPED_LESS_THAN.test(value)) return false;
  const canonical = editorCanonical(state);
  // After a word character, so a `<div>` cannot read as an HTML block.
  return canonical === undefined ? undefined : canonical(`x\\${value}`) !== canonical(`x${value}`);
}

/** The bare text `node` prints as, when it is a literal and still reads as its URL. */
function bareText(node: MdNode | undefined): string | undefined {
  if (node?.type !== 'link' || !(node as MdNode & { mbrStyle?: LinkStyle }).mbrStyle?.literal) return undefined;
  return literalText(node as unknown as Link);
}

function printLink(node: Link, parent: Parents | undefined, state: State, info: Info): string {
  const literal = bareText(node as unknown as MdNode);
  if (literal === undefined) return defaultHandlers.link(node, parent, state, info);
  const siblings = (parent as MdNode | undefined)?.children ?? [];
  const next = siblings[siblings.indexOf(node as unknown as MdNode) + 1];
  // Its escape would join the URL: print the link as any link instead.
  if (next?.type === 'text' && lessThanNeedsEscape(state, String(next.value ?? '')) === true) {
    return defaultHandlers.link(node, parent, state, info);
  }
  return literal;
}

/** Text handler signature (mdast-util-to-markdown's `Handle`, narrowed). */
type TextHandle = (node: Text, parent: Parents | undefined, state: State, info: Info) => string;

/**
 * Wraps the text handler so a `<` right after a bare URL is not escaped when
 * it starts nothing (see the module docs).
 */
export function keepLessThanAfterLiteral(inner: TextHandle): TextHandle {
  return (node, parent, state, info) => {
    const out = inner(node, parent, state, info);
    if (!out.startsWith('\\<')) return out;
    const siblings = (parent?.children ?? []) as unknown as MdNode[];
    const previous = siblings[siblings.indexOf(node as unknown as MdNode) - 1];
    if (bareText(previous) === undefined) return out;
    return lessThanNeedsEscape(state, node.value) === false ? out.slice(1) : out;
  };
}

const TextHandlerReady = createTimer('MbrAutolinkLiteralTextHandler');

/**
 * Installs {@link keepLessThanAfterLiteral} around the stringify options'
 * text handler, which overrides any a remark plugin could add (as
 * `section-attrs.ts` does) — before `init` builds the processor from it.
 */
const literalTextHandler: MilkdownPlugin = (ctx) => {
  ctx.record(TextHandlerReady);
  ctx.update(initTimerCtx, (timers) => [...timers, TextHandlerReady]);
  return async () => {
    ctx.update(remarkStringifyOptionsCtx, (options) => {
      const text = options.handlers?.text as TextHandle | undefined;
      if (!text) return options;
      return { ...options, handlers: { ...options.handlers, text: keepLessThanAfterLiteral(text) } };
    });
    ctx.done(TextHandlerReady);
    return () => {
      ctx.clearTimer(TextHandlerReady);
    };
  };
};

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

export const autolinkLiteralsPlugin = [remarkAutolinkLiterals, literalLinkSchema, literalTextHandler].flat();
