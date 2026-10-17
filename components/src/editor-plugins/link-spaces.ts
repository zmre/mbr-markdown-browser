/**
 * Whitespace at the edges of a link's text stays inside the link.
 *
 * Milkdown's serializer moves spaces at the edges of every mark out of it
 * when the mark closes (`SerializerState#moveSpaces`), which `**x **` needs —
 * emphasis cannot end on a space — but a link can: `[text ](url)` became
 * `[text](url) `, and a link whose text is only spaces (`[ ](url)`, a spacer
 * some exports put between two links) lost its text, then its anchor.
 * `roundtrip-check` found both. The move is private and only looks at a
 * `text` child with a space at the edge, so an empty text node at that edge
 * (which prints as nothing) keeps the spaces where they were.
 *
 * Two links next to each other with the same target (`[a][r][ - ][r]`) are
 * one mark to ProseMirror, and two equal adjacent nodes to Milkdown's
 * serializer, so they came back as one link. Each parsed link carries where
 * it was parsed ({@link sourceAt}, in `mbrStyle`, which nothing compares or
 * prints), which keeps them apart.
 *
 * A link with no text at all has nothing for the mark to sit on; it is held
 * as its source ({@link emptyLinksAsSource}).
 */
import type { SerializerState } from '@milkdown/kit/transformer';
import { $remark } from '@milkdown/kit/utils';
import type { MdNode } from './remark-support.js';

/** The mdast types Milkdown's link marks serialize to. */
const LINK_TYPES = new Set(['link', 'linkReference']);

type Element = { type: string; children?: { type: string; value?: unknown }[] };

const guarded = new WeakSet<object>();

/** Children with an empty text node on each edge that is a text with whitespace there. */
export function guardEdgeSpaces(children: { type: string; value?: unknown }[]): void {
  const first = children[0];
  const last = children[children.length - 1];
  if (last?.type === 'text' && /\s$/.test(String(last.value ?? ''))) children.push({ type: 'text', value: '' });
  if (first?.type === 'text' && /^\s/.test(String(first.value ?? ''))) children.unshift({ type: 'text', value: '' });
}

/**
 * Makes `state` keep a link's edge whitespace inside it ({@link guardEdgeSpaces}
 * on each link it closes). Called by the link marks' runners; once per state.
 */
export function keepLinkEdgeSpaces(state: SerializerState): void {
  if (guarded.has(state)) return;
  guarded.add(state);
  // `close` is an instance property the private move calls through `this`.
  const host = state as unknown as { close: () => Element };
  const close = host.close;
  host.close = () => {
    const element = close();
    if (LINK_TYPES.has(element.type) && element.children?.length) guardEdgeSpaces(element.children);
    return element;
  };
}

/** `{ at }`: where `node` starts in the source, for telling adjacent links apart; `null` without a position. */
export function sourceAt(node: { position?: { start?: { offset?: number } } }): { at: number } | null {
  const at = node.position?.start?.offset;
  return at === undefined ? null : { at };
}

/** Link types whose text is their children (remark's, and `references.ts`'s rename). */
const EMPTY_LINK_TYPES = new Set(['link', 'linkReference', 'mbrLinkReference']);

type SourceNode = MdNode & { position?: { start?: { offset?: number }; end?: { offset?: number } } };

/**
 * Replaces each link with no text (`[](url)`, `[][ref]`) by an inline HTML
 * node holding its source: ProseMirror keeps a link as a mark on text, so a
 * link with none had nothing to hold it and was dropped from the note. As
 * HTML it prints back as typed (and shows its source in the editor).
 */
export function emptyLinksAsSource(tree: MdNode, source: string): void {
  const visit = (node: SourceNode): void => {
    const children = node.children as SourceNode[] | undefined;
    if (!children) return;
    for (let i = 0; i < children.length; i++) {
      const child = children[i];
      const start = child.position?.start?.offset;
      const end = child.position?.end?.offset;
      if (EMPTY_LINK_TYPES.has(child.type) && (child.children?.length ?? 0) === 0 && start !== undefined && end !== undefined) {
        children[i] = { type: 'html', value: source.slice(start, end), position: child.position } as SourceNode;
      } else {
        visit(child);
      }
    }
  };
  visit(tree as SourceNode);
}

/** {@link emptyLinksAsSource} at parse. */
export const remarkEmptyLinks = $remark('mbrEmptyLinks', () => function emptyLinks() {
  return (tree: unknown, file: { value?: unknown }) => emptyLinksAsSource(tree as MdNode, String(file.value ?? ''));
});

export const linkEdgesPlugin = [remarkEmptyLinks].flat();
