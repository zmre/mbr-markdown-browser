/**
 * Bare URLs stay bare (`autolink-literals.ts`); the untouched round trip is
 * the parity harness's `rt-bare-urls`.
 */
import type { Link } from 'mdast';
import { afterEach, describe, expect, it } from 'vitest';
import { serializeBody } from '../editor-crepe.js';
import { destroyEditors, openCrepe, placeCursor, typeText, viewOf } from '../editor-test-support.js';
import { literalText, recordLiteralLinks } from './autolink-literals.js';
import type { MdNode } from './remark-support.js';

afterEach(destroyEditors);

function link(url: string, text: string, title: string | null = null): Link {
  return { type: 'link', url, title, children: [{ type: 'text', value: text }] };
}

describe('literalText', () => {
  it('is the text when the URL is what GFM makes of it', () => {
    expect(literalText(link('https://x.com/a', 'https://x.com/a'))).toBe('https://x.com/a');
    expect(literalText(link('http://www.x.com', 'www.x.com'))).toBe('www.x.com');
    expect(literalText(link('mailto:a@b.co', 'a@b.co'))).toBe('a@b.co');
  });

  it('is undefined once the text, the URL or a title says otherwise', () => {
    expect(literalText(link('https://x.com/b', 'https://x.com/a'))).toBeUndefined();
    expect(literalText(link('https://x.com/a', 'https://x.com/a', 'T'))).toBeUndefined();
    const twoChildren: Link = {
      type: 'link',
      url: 'https://x.com',
      title: null,
      children: [
        { type: 'text', value: 'https://' },
        { type: 'emphasis', children: [{ type: 'text', value: 'x.com' }] },
      ],
    };
    expect(literalText(twoChildren)).toBeUndefined();
  });
});

describe('recordLiteralLinks', () => {
  it('marks only links typed bare, and where each link was parsed', () => {
    const source = 'a https://x.com <https://y.com> [z](https://z.com)';
    const at = (offset: number) => ({ start: { offset } });
    const links = [4, 16, 32].map((offset) => ({ ...link('u', 'u'), position: at(offset) }));
    const tree = { type: 'root', children: [{ type: 'paragraph', children: links }] } as unknown as MdNode;
    recordLiteralLinks(tree, source);
    expect(links.map((l) => (l as { mbrStyle?: unknown }).mbrStyle)).toEqual([{ literal: true, at: 4 }, { at: 16 }, { at: 32 }]);
  });
});

describe('in the editor', () => {
  it('keeps a bare URL bare when the paragraph around it is edited', async () => {
    const crepe = await openCrepe('See https://example.com/a_b and more\n');
    const view = viewOf(crepe);
    placeCursor(view, 'more', 4);
    typeText(view, '!');
    expect(serializeBody(crepe)).toBe('See https://example.com/a_b and more!\n');
  });

  it('keeps an autolink typed with brackets as typed', async () => {
    const crepe = await openCrepe('See <https://example.com> and more\n');
    const view = viewOf(crepe);
    placeCursor(view, 'more', 4);
    typeText(view, '!');
    expect(serializeBody(crepe)).toBe('See <https://example.com> and more!\n');
  });

  it('keeps a `<` that ends a bare URL unescaped when the paragraph is edited', async () => {
    // Escaped, the backslash would be the URL's last character to GFM.
    for (const source of [
      'See https://example.com/?q=<a_b> here.\n',
      'See https://example.com<x here.\n',
      'See www.example.com/<!x here.\n',
      '* https://example.com/<a and here\n',
    ]) {
      const crepe = await openCrepe(source);
      const view = viewOf(crepe);
      placeCursor(view, 'here', 4);
      typeText(view, '!');
      expect(serializeBody(crepe)).toBe(source.replace('here', 'here!'));
    }
  });

  it('prints the link in brackets where the `<` after it starts a tag', async () => {
    // Typed against the URL: escaping the tag would put the backslash in the URL.
    const crepe = await openCrepe('See https://example.com here\n');
    const view = viewOf(crepe);
    placeCursor(view, 'https://example.com', 19);
    const at = view.state.selection.from;
    view.dispatch(view.state.tr.insert(at, view.state.schema.text('<b>')));
    const saved = serializeBody(crepe);
    expect(saved).toBe('See <https://example.com>\\<b> here\n');
    // Re-read, the link is whole and `<b>` is text.
    const again = await openCrepe(saved);
    const para = viewOf(again).state.doc.firstChild;
    expect(para?.textContent).toBe('See https://example.com<b> here');
    expect(para?.child(1).marks[0]?.attrs.href).toBe('https://example.com');
  });
});
