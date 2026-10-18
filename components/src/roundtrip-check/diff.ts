/**
 * Line diffs for the `roundtrip-check` report: which lines a save changes and
 * a unified diff a reader (or `patch`) can apply. Pure.
 *
 * The longest common subsequence is the editor's own `matchSequences`
 * (`editor-minimal-diff.ts`), on the lines left after a shared prefix and
 * suffix. That table is quadratic, so a middle larger than
 * {@link MAX_LCS_CELLS} is reported as one replaced run instead: still a
 * correct diff, only a coarser one, and a 20,000-line note cannot exhaust
 * memory.
 */
import { matchSequences } from '../editor-minimal-diff.js';

/** Cells of the LCS table above which the middle is diffed as one run. */
export const MAX_LCS_CELLS = 16_000_000;

/** One line of a diff: kept (`' '`), removed (`'-'`) or added (`'+'`). */
export interface DiffLine {
  op: ' ' | '-' | '+';
  text: string;
}

/**
 * `text` as lines, each with its `\n` (the last one without, when the text
 * does not end in one), so a missing final newline is a difference too.
 */
export function linesOf(text: string): string[] {
  return text.match(/[^\n]*\n|[^\n]+$/g) ?? [];
}

/** The line edits turning `a` into `b`, in order. */
export function diffLines(a: readonly string[], b: readonly string[]): DiffLine[] {
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head++;
  let tail = 0;
  while (tail < a.length - head && tail < b.length - head && a[a.length - 1 - tail] === b[b.length - 1 - tail]) tail++;
  const midA = a.slice(head, a.length - tail);
  const midB = b.slice(head, b.length - tail);
  const pairs = midA.length * midB.length > MAX_LCS_CELLS ? [] : matchSequences(midA, midB);
  const out: DiffLine[] = a.slice(0, head).map((text) => ({ op: ' ', text }));
  let i = 0;
  let j = 0;
  for (const [pi, pj] of [...pairs, [midA.length, midB.length] as [number, number]]) {
    for (; i < pi; i++) out.push({ op: '-', text: midA[i] });
    for (; j < pj; j++) out.push({ op: '+', text: midB[j] });
    if (i < midA.length && j < midB.length) out.push({ op: ' ', text: midA[i] });
    i++;
    j++;
  }
  for (const text of a.slice(a.length - tail)) out.push({ op: ' ', text });
  return out;
}

/** Lines a rewrite of `a` to `b` removes plus lines it adds (the report's "changed lines"). */
export function changedLineCount(a: string, b: string): number {
  return diffLines(linesOf(a), linesOf(b)).filter((line) => line.op !== ' ').length;
}

/** The removed and added lines of `a` → `b`, as sets of line text. */
export function lineChanges(a: string, b: string): { removed: Set<string>; added: Set<string> } {
  const removed = new Set<string>();
  const added = new Set<string>();
  for (const { op, text } of diffLines(linesOf(a), linesOf(b))) {
    if (op === '-') removed.add(text);
    else if (op === '+') added.add(text);
  }
  return { removed, added };
}

/**
 * A unified diff of `a` → `b` (`diff -u` format, `context` lines around each
 * hunk), or `''` when they are equal. Lines are compared exactly, `\r`
 * included, and written as they are.
 */
export function unifiedDiff(a: string, b: string, labelA: string, labelB: string, context = 3): string {
  if (a === b) return '';
  const lines = diffLines(linesOf(a), linesOf(b));
  // Positions (in `lines`) of the changes, grouped into hunks whose context overlaps.
  const changes = lines.flatMap((line, k) => (line.op === ' ' ? [] : [k]));
  const hunks: [number, number][] = [];
  for (const k of changes) {
    const start = Math.max(0, k - context);
    const end = Math.min(lines.length, k + context + 1);
    const last = hunks[hunks.length - 1];
    if (last && start <= last[1]) last[1] = Math.max(last[1], end);
    else hunks.push([start, end]);
  }
  let out = `--- ${labelA}\n+++ ${labelB}\n`;
  // Line numbers in `a` and `b` at the start of `lines[k]`.
  let atA = 1;
  let atB = 1;
  let k = 0;
  for (const [start, end] of hunks) {
    for (; k < start; k++) {
      if (lines[k].op !== '+') atA++;
      if (lines[k].op !== '-') atB++;
    }
    const body = lines.slice(start, end);
    const countA = body.filter((line) => line.op !== '+').length;
    const countB = body.filter((line) => line.op !== '-').length;
    out += `@@ -${hunkStart(atA, countA)},${countA} +${hunkStart(atB, countB)},${countB} @@\n`;
    for (const line of body) {
      out += `${line.op}${line.text}`;
      if (!line.text.endsWith('\n')) out += '\n\\ No newline at end of file\n';
    }
  }
  return out;
}

/** `diff -u` numbers an empty range by the line before it. */
function hunkStart(at: number, count: number): number {
  return count === 0 ? at - 1 : at;
}
