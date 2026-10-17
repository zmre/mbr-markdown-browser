/**
 * A link's text keeps the whitespace at its edges, and two adjacent links to
 * one target stay two (found by `roundtrip-check`; the untouched round trips
 * are the `rt-link-*` fixtures').
 */
import { afterEach, describe, expect, it } from 'vitest';
import { serializeBody } from '../editor-crepe.js';
import { destroyEditors, openCrepe, placeCursor, typeText, viewOf } from '../editor-test-support.js';
import { emptyLinksAsSource, guardEdgeSpaces } from './link-spaces.js';
import type { MdNode } from './remark-support.js';

afterEach(destroyEditors);

async function roundTrip(source: string): Promise<string> {
  return serializeBody(await openCrepe(source));
}

/** What a save writes after typing `text` right after `at` (a text node's end). */
async function afterTyping(source: string, at: string, text: string): Promise<string> {
  const crepe = await openCrepe(source);
  const view = viewOf(crepe);
  placeCursor(view, at, at.length);
  typeText(view, text);
  return serializeBody(crepe);
}

describe('link edges', () => {
  it.each([
    ['a trailing space', 'X [text ](https://a.b/) y\n'],
    ['a leading space', 'X [ text](https://a.b/) y\n'],
    ['only a space', 'X [ ](https://a.b/) y\n'],
    ['only a hard break', '[  \n](https://a.b/)\n'],
    ['a hard break at its end', 'X [a  \n](https://a.b/)\n'],
    ['adjacent links to one URL', '[a](https://a.b/)[b](https://a.b/)\n'],
    ['adjacent references to one definition', 'Xx[ ][r][r][r]: y\n\n[r]: https://a.b/\n'],
    ['three references with a spacer', '[a][r][ - ][r][b][r]\n\n[r]: https://a.b/\n'],
  ])('%s survives a round trip', async (_name, source) => {
    expect(await roundTrip(source)).toBe(source);
  });

  it('keeps a link with no text, as its source', async () => {
    expect(await roundTrip('A [](https://a.b/) b [][r] c\n\n[r]: https://a.b/\n')).toBe('A [](https://a.b/) b [][r] c\n\n[r]: https://a.b/\n');
    expect(await afterTyping('A [](https://a.b/) b\n', ' b', '!')).toBe('A [](https://a.b/) b!\n');
    const source = 'x [](u) [y](u)';
    const empty = { type: 'link', url: 'u', children: [], position: { start: { offset: 2 }, end: { offset: 7 } } };
    const full = { type: 'link', url: 'u', children: [{ type: 'text', value: 'y' }], position: { start: { offset: 8 }, end: { offset: 14 } } };
    const tree = { type: 'root', children: [{ type: 'paragraph', children: [empty, full] }] } as unknown as MdNode;
    emptyLinksAsSource(tree, source);
    const inline = (tree as unknown as { children: { children: { type: string; value?: string }[] }[] }).children[0].children;
    expect(inline.map((node) => node.type)).toEqual(['html', 'link']);
    expect(inline[0].value).toBe('[](u)');
  });

  it('survives an edit of its paragraph', async () => {
    expect(await afterTyping('X [ ](https://a.b/) y\n', ' y', '!')).toBe('X [ ](https://a.b/) y!\n');
    expect(await afterTyping('[a][r][b][r] end\n\n[r]: https://a.b/\n', ' end', '!')).toBe(
      '[a][r][b][r] end!\n\n[r]: https://a.b/\n',
    );
  });

  it('guards only an edge that is whitespace', () => {
    const children = [{ type: 'text', value: ' a' }, { type: 'emphasis' }, { type: 'text', value: 'b ' }];
    guardEdgeSpaces(children);
    expect(children.map((child) => child.value ?? child.type)).toEqual(['', ' a', 'emphasis', 'b ', '']);
    const plain = [{ type: 'text', value: 'a' }];
    guardEdgeSpaces(plain);
    expect(plain).toEqual([{ type: 'text', value: 'a' }]);
  });
});
