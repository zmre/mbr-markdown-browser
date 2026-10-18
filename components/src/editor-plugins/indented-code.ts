/**
 * One indented code block where micromark makes two.
 *
 * After an empty list item, micromark (the editor's parser) splits an
 * indented code block in two at its first line:
 *
 * ```markdown
 * -
 *
 *     - one        ← code
 *         - two    ← a second code block
 * ```
 *
 * CommonMark — and pulldown-cmark, the renderer — read one block: indented
 * chunks with only blank lines between are one code block, so two can never
 * be neighbours. The editor printed the two with a blank line between, which
 * is not what the note says and does not pair with it, so any save rewrote
 * the lines around them (`roundtrip-check`'s "rewritten by any save").
 *
 * {@link mergeSplitIndentedCode} joins top-level neighbours back into one,
 * before anything reads their positions (`authored-style.ts`).
 */
import type { Code } from 'mdast';
import { $remark } from '@milkdown/kit/utils';
import type { MdNode } from './remark-support.js';

type Positioned = MdNode & {
  value?: string;
  lang?: string | null;
  position?: { start: { offset?: number; line?: number }; end: { offset?: number; line?: number } };
};

/** Whether `node` is an indented (not fenced) code block of `source`. */
function isIndentedCode(node: Positioned, source: string): boolean {
  const at = node.position?.start.offset;
  if (node.type !== 'code' || at === undefined) return false;
  return !/^[ \t]*(?:`{3,}|~{3,})/.test(source.slice(at));
}

/** `line` without up to four columns of indentation (tab stops every four), as a code block strips it. */
function stripIndent(line: string): string {
  let column = 0;
  let i = 0;
  while (i < line.length && column < 4) {
    if (line[i] === ' ') column++;
    else if (line[i] === '\t') column += 4 - (column % 4);
    else break;
    i++;
  }
  // A tab that overshoots the fourth column leaves its remainder as spaces.
  return ' '.repeat(Math.max(0, column - 4)) + line.slice(i);
}

/** Joins neighbouring top-level indented code blocks of `tree` (see the module docs). */
export function mergeSplitIndentedCode(tree: MdNode, source: string): void {
  const children = (tree.children ?? []) as Positioned[];
  for (let i = 0; i < children.length - 1; ) {
    const first = children[i];
    const second = children[i + 1];
    const end = first.position?.end.offset;
    const start = second.position?.start.offset;
    if (end === undefined || start === undefined || !isIndentedCode(first, source) || !isIndentedCode(second, source)) {
      i++;
      continue;
    }
    const between = source.slice(end, start);
    if (!/^(?:\r?\n[ \t]*)+$/.test(between)) {
      i++;
      continue;
    }
    // The lines between (blank) are part of the block, less their indentation.
    const blanks = between.split('\n').slice(1, -1).map((line) => stripIndent(line.replace(/\r$/, '')));
    const merged = first as unknown as Code;
    merged.value = [String(first.value ?? ''), ...blanks, String(second.value ?? '')].join('\n');
    const secondEnd = (second as unknown as Code).position?.end;
    if (merged.position && secondEnd) merged.position = { ...merged.position, end: secondEnd };
    children.splice(i + 1, 1);
  }
}

/** The transform, on the editor's processor. */
export const indentedCodePlugin = $remark('mbrIndentedCode', () => () => (tree: unknown, file: { value?: unknown }) => {
  mergeSplitIndentedCode(tree as MdNode, String(file.value ?? ''));
});
