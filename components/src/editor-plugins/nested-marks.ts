/**
 * Emphasis inside emphasis (`*a _b_ c*`), strong inside strong
 * (`**__x__**`), strike inside strike, held as one mark.
 *
 * ProseMirror marks do not nest, and Milkdown's parser closes a mark when the
 * inner one of the same kind closes, so ` c` lost its emphasis: an edit to
 * the paragraph printed `*a* _b_ c` (found by `roundtrip-check`). The inner
 * mark adds nothing the reader sees, so it is unwrapped at parse; the
 * untouched paragraph still prints as typed (`minimal-escapes.ts` reads the
 * two spellings as one).
 */
import { $remark } from '@milkdown/kit/utils';
import type { MdNode } from './remark-support.js';

/** Marks that can be nested in their own kind in markdown. */
const SELF_NESTING = new Set(['emphasis', 'strong', 'delete']);

/** Replaces every mark inside a mark of its own kind by its children. */
export function unwrapNestedMarks(tree: MdNode): void {
  const visit = (node: MdNode, open: ReadonlySet<string>): void => {
    if (!node.children) return;
    const inner = SELF_NESTING.has(node.type) ? new Set([...open, node.type]) : open;
    // Until none is left: a child pulled up may itself be a nested mark.
    while (node.children.some((child) => inner.has(child.type))) {
      node.children = node.children.flatMap((child) => (inner.has(child.type) ? (child.children ?? []) : [child]));
    }
    for (const child of node.children) visit(child, inner);
  };
  visit(tree, new Set());
}

/** {@link unwrapNestedMarks} at parse. */
export const remarkNestedMarks = $remark('mbrNestedMarks', () => function nestedMarks() {
  return (tree: unknown) => unwrapNestedMarks(tree as MdNode);
});

export const nestedMarksPlugin = [remarkNestedMarks].flat();
