/**
 * Reference links, reference images and definitions survive an editor save
 * as authored (`references.ts`) instead of being inlined and deleted.
 */
import { editorViewCtx } from '@milkdown/kit/core';
import type { EditorView } from '@milkdown/kit/prose/view';
import { afterEach, describe, expect, it } from 'vitest';
import { serializeBody } from '../editor-crepe.js';
import { destroyEditors, openCrepe, placeCursor, press, savedFile, typeText, viewOf } from '../editor-test-support.js';

afterEach(destroyEditors);

describe('references round-trip', () => {
  const cases: [string, string][] = [
    ['full', 'See [the docs][docs].\n\n[docs]: https://example.com/docs\n'],
    ['collapsed', 'See [docs][].\n\n[docs]: https://example.com/docs\n'],
    ['shortcut', 'See [docs].\n\n[docs]: https://example.com/docs\n'],
    ['label case kept', 'See [the docs][Docs Page].\n\n[docs page]: /docs/\n'],
    ['image reference', 'A ![logo][logo] here.\n\n[logo]: images/logo.png "Logo"\n'],
    ['collapsed image reference', 'A ![logo][] here.\n\n[logo]: images/logo.png\n'],
    [
      'definition styles: <url>, single quotes, parentheses, title on the next line',
      '[a]: <https://example.com/a b> \'Single\'\n[b]: /b (Parens)\n[c]: /c\n  "Next line"\n',
    ],
    ['definitions separated by blank lines stay separated', '[a]: /a\n\n[b]: /b\n'],
    ['a definition hugging the paragraph after it', '[a]: /a\nThe [a] paragraph.\n'],
    ['an unused definition', 'Text.\n\n[unused]: https://example.com/\n'],
    ['in a blockquote', '> See [x][a].\n>\n> [a]: /a\n> [b]: /b\n'],
    ['reference with emphasis inside', 'See [*the* docs][docs].\n\n[docs]: /docs/\n'],
  ];
  for (const [name, source] of cases) {
    it(name, async () => {
      expect(await savedFile(source)).toBe(source);
    });
  }

  it('leaves inline links inline', async () => {
    const source = 'An [inline](https://example.com/ "T") link.\n';
    expect(await savedFile(source)).toBe(source);
  });
});

/** Applies `edit` to the editor and returns the saved file. */
const editSaved = (source: string, edit: (view: EditorView) => void) =>
  savedFile(source, (crepe) => crepe.editor.action((ctx) => edit(ctx.get(editorViewCtx))));

/** Document position of the first occurrence of `text`. */
function positionOf(view: EditorView, text: string): number {
  let found = -1;
  view.state.doc.descendants((node, pos) => {
    if (found !== -1) return false;
    const index = node.isText ? (node.text ?? '').indexOf(text) : -1;
    if (index !== -1) found = pos + index;
    return true;
  });
  if (found === -1) throw new Error(`"${text}" not in the document`);
  return found;
}

describe('editing references', () => {
  it('keeps the reference when its text is edited', async () => {
    const saved = await editSaved('See [the docs][docs].\n\n[docs]: /docs/\n', (view) => {
      const at = positionOf(view, 'the docs') + 'the'.length;
      view.dispatch(view.state.tr.insertText(' fine', at));
    });
    expect(saved).toBe('See [the fine docs][docs].\n\n[docs]: /docs/\n');
  });

  it('does not extend the reference over text typed at its end (like a link)', async () => {
    const saved = await editSaved('See [the docs][docs].\n\n[docs]: /docs/\n', (view) => {
      const at = positionOf(view, 'the docs') + 'the docs'.length;
      view.dispatch(view.state.tr.insertText(' page', at));
    });
    expect(saved).toBe('See [the docs][docs] page.\n\n[docs]: /docs/\n');
  });

  it('edits a definition as text', async () => {
    const saved = await editSaved('See [x][a].\n\n[a]: /old\n', (view) => {
      const at = positionOf(view, '/old');
      view.dispatch(view.state.tr.insertText('/new', at, at + '/old'.length));
    });
    expect(saved).toBe('See [x][a].\n\n[a]: /new\n');
  });
});

describe('the caret at a reference link\'s edges', () => {
  const SOURCE = 'See [docs][d] now.\n\n[d]: /d\n';

  it('typing right after the link text goes outside the link', async () => {
    const crepe = await openCrepe(SOURCE);
    const view = viewOf(crepe);
    placeCursor(view, 'docs', 4);
    typeText(view, 'X');
    expect(serializeBody(crepe)).toBe('See [docs][d]X now.\n\n[d]: /d\n');
  });

  it('← at the edge moves the caret inside first (the virtual cursor), so typing extends the text', async () => {
    const crepe = await openCrepe(SOURCE);
    const view = viewOf(crepe);
    placeCursor(view, 'docs', 4);
    expect(press(view, 'ArrowLeft')).toBe(true);
    typeText(view, 'X');
    expect(serializeBody(crepe)).toBe('See [docsX][d] now.\n\n[d]: /d\n');
  });

  it('typing right before the link text stays outside it', async () => {
    const crepe = await openCrepe(SOURCE);
    const view = viewOf(crepe);
    placeCursor(view, 'docs', 0);
    typeText(view, 'X');
    expect(serializeBody(crepe)).toBe('See X[docs][d] now.\n\n[d]: /d\n');
  });
});
