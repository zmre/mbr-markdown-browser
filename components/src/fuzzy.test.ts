import { describe, expect, it } from 'vitest';
import { escapeRegex, fuzzyScore, fuzzyFilter } from './fuzzy.js';

describe('escapeRegex', () => {
  it('escapes regex metacharacters', () => {
    expect(escapeRegex('a.b*c+')).toBe('a\\.b\\*c\\+');
    expect(escapeRegex('(x)[y]{z}')).toBe('\\(x\\)\\[y\\]\\{z\\}');
  });

  it('leaves plain text untouched', () => {
    expect(escapeRegex('hello world')).toBe('hello world');
  });
});

describe('fuzzyScore', () => {
  it('returns 0 for an empty query', () => {
    expect(fuzzyScore('anything', '')).toBe(0);
  });

  it('returns 0 when characters are not present in order', () => {
    expect(fuzzyScore('abc', 'zzz')).toBe(0);
    expect(fuzzyScore('abc', 'cba')).toBe(0);
  });

  it('scores an exact prefix highest', () => {
    expect(fuzzyScore('guide', 'gui')).toBe(1500);
  });

  it('scores a word-start substring above a mid-word substring', () => {
    const wordStart = fuzzyScore('the quick fox', 'quick');
    const midWord = fuzzyScore('aquickb', 'quick');
    expect(wordStart).toBe(1200);
    expect(midWord).toBe(1000);
    expect(wordStart).toBeGreaterThan(midWord);
  });

  it('is case-insensitive', () => {
    expect(fuzzyScore('GUIDE', 'guide')).toBe(1500);
    expect(fuzzyScore('guide', 'GUIDE')).toBe(1500);
  });

  it('gives a positive subsequence score when no substring matches', () => {
    const score = fuzzyScore('gopher under den', 'gud');
    expect(score).toBeGreaterThan(0);
    expect(score).toBeLessThan(1000);
  });

  it('ranks a closer subsequence match higher', () => {
    // "abc" matches consecutively at the start of "abcdef" (best),
    // but is spread out in "axbxcx".
    expect(fuzzyScore('abcdef', 'abc')).toBeGreaterThan(fuzzyScore('axbxcx', 'abc'));
  });
});

describe('fuzzyFilter', () => {
  interface Row {
    path: string;
    title: string;
  }
  const rows: Row[] = [
    { path: 'docs/guide.md', title: 'The Guide' },
    { path: 'docs/intro.md', title: 'Introduction' },
    { path: 'notes/todo.md', title: 'Todo List' },
  ];
  const candidates = rows.map((r) => ({ item: r, haystacks: [r.path, r.title] }));

  it('returns every item (original order) for an empty query', () => {
    expect(fuzzyFilter(candidates, '')).toEqual(rows);
    expect(fuzzyFilter(candidates, '   ')).toEqual(rows);
  });

  it('filters out non-matching items', () => {
    const result = fuzzyFilter(candidates, 'guide');
    expect(result).toHaveLength(1);
    expect(result[0].path).toBe('docs/guide.md');
  });

  it('matches on either haystack (path or title)', () => {
    // "Introduction" only matches via the title, not the path stem.
    const result = fuzzyFilter(candidates, 'introduction');
    expect(result).toHaveLength(1);
    expect(result[0].path).toBe('docs/intro.md');
  });

  it('ranks better matches first', () => {
    const mixed = fuzzyFilter(candidates, 'to');
    // "Todo List"/"notes/todo.md" should outrank an incidental subsequence.
    expect(mixed[0].path).toBe('notes/todo.md');
  });
});
