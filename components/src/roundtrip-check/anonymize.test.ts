/**
 * The anonymizer keeps markdown structure: unit cases for what it keeps, and
 * over every editor fixture, the anonymized note has the same block outline
 * (`editor-outline.ts`, the editor's parser) and the same round-trip verdict.
 */
import { afterEach, describe, expect, it } from 'vitest';
import type { OutlineNode } from '../editor-outline.js';
import { EDITOR_FIXTURES, destroyEditors, editorOutline, savedFile } from '../editor-test-support.js';
import { anonymize, maskChars } from './anonymize.js';

afterEach(destroyEditors);

describe('maskChars', () => {
  it('masks letters by case and digits, keeping everything else', () => {
    expect(maskChars('Hello, Wörld 42! [a](b) *é*\t\r')).toBe('Xxxxx, Xxxxx 00! [x](x) *x*\t\r');
  });

  it('masks other scripts and drops combining marks', () => {
    expect(maskChars('日本 é Ω')).toBe('xx x X');
  });
});

describe('anonymize', () => {
  it('keeps line structure, indentation and markdown punctuation', () => {
    const source = '# Title {#intro .big}\n\n  * item **bold** _em_ `code`\n    > quote\n\n| a | b |\n|:-:|---|\n';
    const anon = anonymize(source);
    expect(anon).toBe('# Xxxxx {#xxxxx .xxx}\n\n  * xxxx **xxxx** _xx_ `xxxx`\n    > xxxxx\n\n| x | x |\n|:-:|---|\n');
    expect(anon.length).toBe(source.length);
  });

  it('keeps CRLF line endings', () => {
    expect(anonymize('Ab\r\ncd\r\n')).toBe('Xx\r\nxx\r\n');
  });

  it("keeps mbr's keywords and markers", () => {
    expect(anonymize('TODO: call Bob, TK, FIXME and XXX\n')).toBe('TODO: xxxx Xxx, TK, FIXME xxx XXX\n');
    expect(anonymize('> [!NOTE]\n> Mind the gap\n')).toBe('> [!NOTE]\n> Xxxx xxx xxx\n');
    expect(anonymize('- [x] Pay rent @due(2026-10-01) @done(2026-10-02 09:30) #home !!\n')).toBe(
      '- [x] Xxx xxxx @due(2026-10-01) @done(2026-10-02 09:30) #xxxx !!\n',
    );
  });

  it('keeps flashcard histories', () => {
    const source = 'What is 2+2?\n: Four\n: ___Review History___\n  * 2026-01-02 10:00 - Good\n  * 2026-01-03 11:15 - Again\n';
    expect(anonymize(source)).toBe(
      'Xxxx xx 0+0?\n: Xxxx\n: ___Review History___\n  * 2026-01-02 10:00 - Good\n  * 2026-01-03 11:15 - Again\n',
    );
  });

  it('keeps the frontmatter keys and values mbr reads', () => {
    expect(anonymize('---\ntitle: My Deck\ntype: flashcard\nstyle: "slides"\ntags: [a]\n---\nBody\n')).toBe(
      '---\nxxxxx: Xx Xxxx\ntype: flashcard\nstyle: "slides"\nxxxx: [x]\n---\nXxxx\n',
    );
    expect(anonymize('---\ntype: Meeting Notes\n---\n')).toBe('---\ntype: Xxxxxxx Xxxxx\n---\n');
  });

  it('keeps ordered list numbers, fence info words, entities, schemes and HTML tag names', () => {
    expect(anonymize('3. third\n10) tenth\n')).toBe('3. xxxxx\n10) xxxxx\n');
    expect(anonymize('```chat\n{{Bob|hi}}\n```\n')).toBe('```chat\n{{Xxx|xx}}\n```\n');
    expect(anonymize('AT&amp;T &#38; &nbsp;\n')).toBe('XX&amp;X &#38; &nbsp;\n');
    expect(anonymize('<https://example.com/a1> mailto:me@x.org www.site.org\n')).toBe(
      '<https://xxxxxxx.xxx/x0> mailto:xx@x.xxx www.xxxx.xxx\n',
    );
    expect(anonymize('<div class="note">Secret</div> <custom-tag>\n')).toBe('<div xxxxx="xxxx">Xxxxxx</div> <xxxxxx-xxx>\n');
    expect(anonymize('{{ vid(path="Talks/Demo 1.mp4") }}\n')).toBe('{{ vid(xxxx="Xxxxx/Xxxx 0.xx0") }}\n');
  });

  it('renames reference and footnote labels uniquely and consistently', () => {
    const anon = anonymize('See [one][ab] and [cd], note[^1] and[^2].\n\n[ab]: /x\n[CD]: /y\n[^1]: First.\n[^2]: Second.\n');
    expect(anon).toBe('Xxx [xxx][xx] xxx [XY], xxxx[^0] xxx[^1].\n\n[xx]: /x\n[XY]: /x\n[^0]: Xxxxx.\n[^1]: Xxxxxx.\n');
  });
});

/** The outline with the fields anonymization renames reduced to their shape. */
function shapeOf(outline: OutlineNode[]): OutlineNode[] {
  return outline.map((node) => ({
    ...node,
    ...(node.id === undefined ? {} : { id: maskChars(node.id) }),
    ...(node.classes === undefined ? {} : { classes: node.classes.map(maskChars) }),
    ...(node.label === undefined ? {} : { label: '*' }),
  }));
}

describe('anonymize keeps every fixture parsing the same', () => {
  for (const [name, { source }] of Object.entries(EDITOR_FIXTURES)) {
    it(name, async () => {
      const anon = anonymize(source);
      expect(anon.split('\n').length).toBe(source.split('\n').length);
      expect(shapeOf(await editorOutline(anon))).toEqual(shapeOf(await editorOutline(source)));
      // Same verdict: a note the editor saves as is still is, and one it rewrites still is.
      expect((await savedFile(anon)) === anon).toBe((await savedFile(source)) === source);
    }, 60_000);
  }
});
