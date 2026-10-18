/**
 * A list item opened inside a container opened on the same line
 * (`nested-list-start.ts`). The untouched round trip and the outline against
 * pulldown-cmark are the parity harness's `rt-nested-ordered-start`; these
 * pin the cases one by one and that an edit keeps the list.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { createBodySaver } from '../editor-crepe.js';
import { destroyEditors, editorOutline, openCrepe, placeCursor, typeText, viewOf } from '../editor-test-support.js';

afterEach(destroyEditors);

/** The ordered lists of `source` as the editor reads them: `start@line`. */
async function orderedLists(source: string): Promise<string[]> {
  return (await editorOutline(source))
    .filter((node) => node.kind === 'list' && node.ordered)
    .map((node) => `${String(node.start)}@${node.lines[0]}`);
}

describe('a list in a container opened on the same line', () => {
  it('starts at any number, as in the renderer', async () => {
    expect(await orderedLists('* a\n  * 6. x\n')).toEqual(['6@2']);
    expect(await orderedLists('* a\n\t* 2. x\n')).toEqual(['2@2']);
    expect(await orderedLists('a\n> 6. x\n')).toEqual(['6@2']);
    expect(await orderedLists('* a\n  > 6. x\n')).toEqual(['6@2']);
    expect(await orderedLists('a\n> * 6. x\n')).toEqual(['6@2']);
    // Empty, too.
    expect(await orderedLists('* a\n  * 6.\n')).toEqual(['6@2']);
  });

  it('still cannot interrupt the paragraph itself', async () => {
    expect(await orderedLists('* a\n  6. x\n')).toEqual([]);
    expect(await orderedLists('a\n6. x\n')).toEqual([]);
    expect(await orderedLists('* a\n  1. x\n')).toEqual(['1@2']);
  });

  it('is not escaped when the item is edited, so the list stays', async () => {
    for (const [source, at] of [
      ['* Parent\n  * 6. x\n  * 2. y\n', 'x'],
      ['* Parent\n  * 6. x\n', 'Parent'],
      ['para\n> 6. x\n', 'x'],
    ] as const) {
      const crepe = await openCrepe(source);
      const saver = createBodySaver(crepe, source);
      const view = viewOf(crepe);
      placeCursor(view, at, 1);
      typeText(view, '!');
      const { body } = saver.body();
      expect(body).toBe(source.replace(at, `${at.slice(0, 1)}!${at.slice(1)}`));
      expect(await orderedLists(body)).toEqual(await orderedLists(source));
    }
  });
});
