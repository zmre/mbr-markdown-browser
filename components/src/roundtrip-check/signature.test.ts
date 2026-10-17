/** Causes (`signature.ts`): content-free line shapes, and the sampling they drive. */
import { describe, expect, it } from 'vitest';
import { CauseIndex, diffShapes, lineShape } from './signature.js';

describe('lineShape', () => {
  it('keeps indentation, prefix, notable text and trailing whitespace', () => {
    expect(lineShape('\t* Some words here \n')).toBe('⇥* ·␣');
    expect(lineShape('\t\t* Some words here \n')).toBe('⇥* ·␣');
    expect(lineShape('  * Some words here\n')).toBe('␣␣* ·');
    expect(lineShape('        * Some words here\n')).toBe('␣␣␣␣* ·');
    expect(lineShape('# Heading 12/3/24   \n')).toBe('# ·␣␣');
    expect(lineShape('- see <https://example.com/a>\n')).toBe('- <url>');
    expect(lineShape('- see https://example.com/a\n')).toBe('- url');
    expect(lineShape('* <br />\n')).toBe('* <br/>');
    expect(lineShape('12. [x] done\n')).toBe('0. [x] ·');
    expect(lineShape('[label]: https://x.y/z')).toBe('[·]: url (no final newline)');
    expect(lineShape('\n')).toBe('∅');
    expect(lineShape('foo\\_bar\n')).toBe('\\');
  });

  it('never keeps a note’s words or numbers', () => {
    for (const line of ['Secret Plan 2026 \n', '- [ ] Call Zanzibar https://zanzibar.example/2026\n', '[Zanzibar]: /x\n']) {
      for (const secret of ['Secret', 'Plan', '2026', 'Call', 'anzibar']) expect(lineShape(line)).not.toContain(secret);
    }
  });
});

describe('diffShapes', () => {
  it('is the same for the same rewrite of different text', () => {
    const a = diffShapes('* one\n\t* two\n', '* one\n  * two\n');
    expect(diffShapes('* alpha\n\t* beta gamma\n\t* delta\n', '* alpha\n  * beta gamma\n  * delta\n')).toEqual(a);
    expect(a).toEqual(['+␣␣* ·', '-⇥* ·']);
    expect(diffShapes('same\n', 'same\n')).toEqual([]);
  });
});

describe('CauseIndex', () => {
  it('ranks causes by notes and wants a note only while one of its causes lacks samples', () => {
    const causes = new CauseIndex();
    const first = causes.add('0001', ['a', 'b']);
    expect(CauseIndex.wanted(first, 1)).toBe(true);
    CauseIndex.sampled(first, '0001', 1);
    expect(CauseIndex.wanted(causes.add('0002', ['b']), 1)).toBe(false);
    const third = causes.add('0003', ['b', 'c']);
    expect(CauseIndex.wanted(third, 1)).toBe(true);
    CauseIndex.sampled(third, '0003', 1);
    expect(third.map((cause) => cause.sampled)).toEqual([['0001'], ['0003']]);
    expect(causes.size).toBe(3);
    expect(causes.ranked().map(({ label, cause }) => [label, cause.shape, cause.ids])).toEqual([
      ['S001', 'b', ['0001', '0002', '0003']],
      ['S002', 'a', ['0001']],
      ['S003', 'c', ['0003']],
    ]);
  });
});
