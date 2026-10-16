/**
 * Character references and `\$` survive an editor save as authored
 * (`escapes.ts`).
 */
import { editorViewCtx } from '@milkdown/kit/core';
import { afterEach, describe, expect, it } from 'vitest';
import { destroyEditors, openCrepe, savedFile } from '../editor-test-support.js';

afterEach(destroyEditors);

describe('character references round-trip', () => {
  const cases: [string, string][] = [
    ['named', 'Tom &amp; Jerry &copy; 2026.\n'],
    ['non-breaking space (invisible if decoded)', 'Ten&nbsp;km.\n'],
    ['numeric, decimal and hex', 'Copyright &#169; and &#xA9;.\n'],
    ['markup-looking (HTML if decoded)', 'Write &lt;b&gt; for bold.\n'],
    ['adjacent references', '&lt;&gt;&amp;\n'],
    ['in a heading', '# Q&amp;A\n'],
    ['in emphasis and strong', '*a &ndash; b* and **c &mdash; d**\n'],
    ['in link text', 'See [Tom &amp; Jerry](https://example.com/).\n'],
    ['in a blockquote', '> Quote &ldquo;this&rdquo;.\n'],
    ['at the start of a list item', '* &para; item\n'],
  ];
  for (const [name, source] of cases) {
    it(name, async () => {
      expect(await savedFile(source)).toBe(source);
    });
  }
});

describe('the `\\$` escape round-trips', () => {
  it('keeps the backslash, so `\\$x\\$` cannot become math', async () => {
    const source = 'A literal \\$x\\$ and \\$5.\n';
    expect(await savedFile(source)).toBe(source);
  });

  it('leaves other escapes to the serializer as before', async () => {
    const source = 'Literal \\*stars\\* and \\[brackets].\n';
    expect(await savedFile(source)).toBe(source);
  });
});

describe('where decoding is the right reading', () => {
  it('keeps the decoded value in link destinations (same link)', async () => {
    // The URL attribute holds `&`; the serializer escapes it as `\&`, which
    // reads back as the same `&`.
    expect(await savedFile('[l](u?a&amp;b)\n')).toBe('[l](u?a\\&b)\n');
  });
});

describe('a bare `&`', () => {
  it('stays text; before a letter remark escapes it (cosmetic, same meaning — item 6)', async () => {
    expect(await savedFile('AT&T and &unknown; and & alone.\n')).toBe('AT\\&T and \\&unknown; and & alone.\n');
  });
});

describe('editing around a reference', () => {
  it('prints the reference verbatim after an edit elsewhere in the paragraph', async () => {
    const saved = await savedFile('Fish &amp; chips.\n', (crepe) => {
      crepe.editor.action((ctx) => {
        const view = ctx.get(editorViewCtx);
        view.dispatch(view.state.tr.insertText('Hot ', 1));
      });
    });
    expect(saved).toBe('Hot Fish &amp; chips.\n');
  });

  it('shows the reference as its source in the editor', async () => {
    const crepe = await openCrepe('Ten&nbsp;km.\n');
    const text = crepe.editor.action((ctx) => ctx.get(editorViewCtx).dom.textContent);
    expect(text).toContain('&nbsp;');
  });
});
