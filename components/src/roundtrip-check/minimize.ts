/**
 * Shrinks a note to a minimal reproduction for the `roundtrip-check` report:
 * delta debugging (Zeller's ddmin) over top-level blocks, then over lines,
 * within a time and test budget. Pure apart from the `interesting` callback,
 * which the tool backs with the real editor.
 */
import { linesOf } from './diff.js';

/** How long a minimization may run. `now` is injectable for tests. */
export interface Budget {
  /** Stop starting new tests at this time (`now()` units, ms). */
  deadline: number;
  /** Stop after this many tests. */
  maxTests: number;
  now?: () => number;
}

/** Tracks a {@link Budget} across phases. */
export class BudgetClock {
  #tests = 0;
  constructor(private readonly budget: Budget) {}
  /** Whether another test may start (and counts it). */
  take(): boolean {
    const now = this.budget.now ?? Date.now;
    if (this.#tests >= this.budget.maxTests || now() >= this.budget.deadline) return false;
    this.#tests++;
    return true;
  }
  get tests(): number {
    return this.#tests;
  }
}

/**
 * A 1-minimal subset of `items` (as far as the budget allows) for which `test`
 * holds, assuming it holds for `items` itself. Returns `items` unchanged when
 * nothing smaller passes. Each distinct subset is tested at most once.
 */
export async function ddmin<T>(
  items: readonly T[],
  test: (subset: readonly T[]) => Promise<boolean>,
  clock: BudgetClock,
): Promise<T[]> {
  let current = items.map((_, i) => i);
  const seen = new Map<string, boolean>();
  // `undefined`: the budget ran out before this subset could be tested.
  const passes = async (subset: number[]): Promise<boolean | undefined> => {
    const key = subset.join(',');
    const known = seen.get(key);
    if (known !== undefined) return known;
    if (!clock.take()) return undefined;
    const result = await test(subset.map((i) => items[i]));
    seen.set(key, result);
    return result;
  };
  let n = 2;
  search: while (current.length >= 2) {
    const chunks = split(current, n);
    // Removing one chunk first: it shrinks fastest when most of a note is irrelevant.
    for (let c = 0; c < chunks.length; c++) {
      const complement = chunks.flatMap((chunk, k) => (k === c ? [] : chunk));
      const result = await passes(complement);
      if (result === undefined) break search;
      if (result) {
        current = complement;
        n = Math.max(n - 1, 2);
        continue search;
      }
    }
    for (const chunk of chunks) {
      if (chunk.length === current.length) continue;
      const result = await passes(chunk);
      if (result === undefined) break search;
      if (result) {
        current = chunk;
        n = 2;
        continue search;
      }
    }
    if (n >= current.length) break;
    n = Math.min(current.length, n * 2);
  }
  return current.map((i) => items[i]);
}

/** `items` in `n` contiguous, nearly equal chunks. */
function split<T>(items: readonly T[], n: number): T[][] {
  const chunks: T[][] = [];
  let at = 0;
  for (let k = 0; k < n; k++) {
    const size = Math.floor((items.length - at) / (n - k));
    chunks.push(items.slice(at, at + size));
    at += size;
  }
  return chunks.filter((chunk) => chunk.length > 0);
}

/**
 * A note cut into what a reproduction can keep or drop: the frontmatter (with
 * its fences, `''` without one) and the body's top-level blocks with the text
 * between them (`editor-minimal-diff.ts`'s `BlockSplit`).
 */
export interface DocumentCut {
  prefix: string;
  leading: string;
  blocks: readonly string[];
  /** `gaps[i]` follows `blocks[i]`. */
  gaps: readonly string[];
  trailing: string;
}

/** A unit of a cut: the frontmatter, or block `i`. */
export type CutUnit = 'frontmatter' | number;

/**
 * The note made of `units` only, in order. A kept block is followed by the
 * text that followed it in the note, so blank lines between blocks — and
 * their absence — survive.
 */
export function renderCut(cut: DocumentCut, units: readonly CutUnit[]): string {
  const blocks = units.filter((unit): unit is number => unit !== 'frontmatter');
  let body = cut.leading;
  blocks.forEach((b, k) => {
    body += cut.blocks[b];
    if (k < blocks.length - 1) body += cut.gaps[b] ?? '\n\n';
  });
  body += cut.trailing;
  return (units.includes('frontmatter') ? cut.prefix : '') + body;
}

/**
 * The smallest note (within `budget`) that is still `interesting`: first the
 * frontmatter plus the `suspects` (the changed blocks and any definitions they
 * may need) — or the whole note when those alone do not reproduce — reduced
 * block by block, then line by line.
 */
export async function minimizeDocument(
  cut: DocumentCut,
  suspects: readonly number[],
  interesting: (markdown: string) => Promise<boolean>,
  budget: Budget,
): Promise<string> {
  const clock = new BudgetClock(budget);
  const all: CutUnit[] = [...(cut.prefix ? ['frontmatter' as const] : []), ...cut.blocks.map((_, i) => i)];
  const narrowed: CutUnit[] = [...(cut.prefix ? ['frontmatter' as const] : []), ...[...suspects].sort((a, b) => a - b)];
  let start = all;
  if (narrowed.length < all.length && clock.take() && (await interesting(renderCut(cut, narrowed)))) start = narrowed;
  const units = await ddmin(start, (subset) => interesting(renderCut(cut, subset)), clock);
  const lines = linesOf(renderCut(cut, units));
  const kept = await ddmin(lines, (subset) => interesting(subset.join('')), clock);
  return kept.join('');
}
