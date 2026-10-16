/** The minimizer (`minimize.ts`) and the report's line diffs (`diff.ts`), pure. */
import { describe, expect, it } from 'vitest';
import { changedLineCount, diffLines, linesOf, MAX_LCS_CELLS, unifiedDiff } from './diff.js';
import { BudgetClock, ddmin, minimizeDocument, renderCut, type DocumentCut } from './minimize.js';

const generous = () => new BudgetClock({ deadline: Number.POSITIVE_INFINITY, maxTests: 10_000 });

describe('ddmin', () => {
  it('finds the two items a failure needs', async () => {
    const items = Array.from({ length: 40 }, (_, i) => i);
    const result = await ddmin(items, async (subset) => subset.includes(7) && subset.includes(31), generous());
    expect(result).toEqual([7, 31]);
  });

  it('keeps everything when nothing smaller passes', async () => {
    expect(await ddmin(['a', 'b', 'c'], async (subset) => subset.length === 3, generous())).toEqual(['a', 'b', 'c']);
  });

  it('tests each subset at most once', async () => {
    const seen = new Set<string>();
    await ddmin(
      Array.from({ length: 16 }, (_, i) => i),
      async (subset) => {
        const key = subset.join(',');
        expect(seen.has(key), key).toBe(false);
        seen.add(key);
        return subset.includes(3);
      },
      generous(),
    );
  });

  it('stops at the test budget and returns what passed so far', async () => {
    const items = Array.from({ length: 64 }, (_, i) => i);
    let tests = 0;
    const result = await ddmin(
      items,
      async (subset) => {
        tests++;
        return subset.includes(5);
      },
      new BudgetClock({ deadline: Number.POSITIVE_INFINITY, maxTests: 3 }),
    );
    expect(tests).toBe(3);
    expect(result).toContain(5);
    expect(result.length).toBeLessThan(items.length);
  });

  it('stops at the deadline', async () => {
    let now = 0;
    const clock = new BudgetClock({ deadline: 2, maxTests: 1000, now: () => now++ });
    let tests = 0;
    await ddmin([1, 2, 3, 4, 5, 6, 7, 8], async () => (tests++, false), clock);
    expect(tests).toBe(2);
  });
});

const cut: DocumentCut = {
  prefix: '---\ntitle: t\n---\n',
  leading: '',
  blocks: ['# Heading', 'para one', 'Term\n:bad', '[ref]: /x', 'tail'],
  gaps: ['\n', '\n\n', '\n\n\n', '\n\n'],
  trailing: '\n',
};

describe('renderCut', () => {
  it('renders the whole note from every unit', () => {
    expect(renderCut(cut, ['frontmatter', 0, 1, 2, 3, 4])).toBe('---\ntitle: t\n---\n# Heading\npara one\n\nTerm\n:bad\n\n\n[ref]: /x\n\ntail\n');
  });

  it('keeps the text after each kept block', () => {
    expect(renderCut(cut, [0, 2])).toBe('# Heading\nTerm\n:bad\n');
    expect(renderCut(cut, [2, 4])).toBe('Term\n:bad\n\n\ntail\n');
  });
});

describe('minimizeDocument', () => {
  it('shrinks to the lines a problem needs', async () => {
    const interesting = async (markdown: string) => markdown.includes(':bad') && markdown.includes('Term');
    expect(await minimizeDocument(cut, [2], interesting, { deadline: Number.POSITIVE_INFINITY, maxTests: 1000 })).toBe('Term\n:bad\n');
  });

  it('starts from the whole note when the suspects alone do not reproduce', async () => {
    const interesting = async (markdown: string) => markdown.includes(':bad') && markdown.includes('[ref]');
    const result = await minimizeDocument(cut, [2], interesting, { deadline: Number.POSITIVE_INFINITY, maxTests: 1000 });
    expect(result).toBe(':bad\n[ref]: /x\n');
  });

  it('with no budget, returns the whole note', async () => {
    const result = await minimizeDocument(cut, [2], async () => true, { deadline: 0, maxTests: 1000 });
    expect(result).toBe(renderCut(cut, ['frontmatter', 0, 1, 2, 3, 4]));
  });
});

describe('diff', () => {
  it('splits lines keeping their terminators', () => {
    expect(linesOf('a\nb')).toEqual(['a\n', 'b']);
    expect(linesOf('a\r\n\n')).toEqual(['a\r\n', '\n']);
    expect(linesOf('')).toEqual([]);
  });

  it('writes a unified diff', () => {
    const a = 'one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n';
    const b = 'one\ntwo\n3\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\n';
    expect(unifiedDiff(a, b, 'a/x.md', 'b/x.md')).toBe(
      '--- a/x.md\n+++ b/x.md\n@@ -1,6 +1,6 @@\n one\n two\n-three\n+3\n four\n five\n six\n@@ -8,3 +8,4 @@\n eight\n nine\n ten\n+eleven\n',
    );
    // Changes 2 × context lines apart share a hunk, as `diff -u` does.
    expect(unifiedDiff('a\n1\n2\n3\n4\n5\n6\nb\n', 'A\n1\n2\n3\n4\n5\n6\nB\n', 'a', 'b')).toContain('@@ -1,8 +1,8 @@');
    expect(unifiedDiff(a, a, 'a', 'b')).toBe('');
  });

  it('marks a missing final newline', () => {
    expect(unifiedDiff('x\n', 'x', 'a', 'b')).toBe('--- a\n+++ b\n@@ -1,1 +1,1 @@\n-x\n+x\n\\ No newline at end of file\n');
  });

  it('numbers an insertion into an empty file', () => {
    expect(unifiedDiff('', 'x\n', 'a', 'b')).toBe('--- a\n+++ b\n@@ -0,0 +1,1 @@\n+x\n');
  });

  it('counts removed and added lines', () => {
    expect(changedLineCount('a\nb\nc\n', 'a\nB\nc\n\n')).toBe(3);
  });

  it('diffs a middle too large for the LCS table as one run', () => {
    const size = Math.ceil(Math.sqrt(MAX_LCS_CELLS)) + 1;
    const a = ['first\n', ...Array.from({ length: size }, (_, i) => `a${i}\n`), 'last\n'];
    const b = ['first\n', ...Array.from({ length: size }, (_, i) => `b${i}\n`), 'last\n'];
    const ops = diffLines(a, b).map((line) => line.op).join('');
    expect(ops).toBe(` ${'-'.repeat(size)}${'+'.repeat(size)} `);
  });
});
