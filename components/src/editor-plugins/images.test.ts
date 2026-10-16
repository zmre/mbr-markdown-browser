/**
 * Images survive an editor save as authored (`images.ts`): an untitled image
 * is not dropped, and a standalone image (Crepe's `image-block`) keeps its alt
 * text unless it was resized in the editor.
 */
import { editorViewCtx } from '@milkdown/kit/core';
import { afterEach, describe, expect, it } from 'vitest';
import { destroyEditors, savedFile } from '../editor-test-support.js';
import { ratioFromAlt } from './images.js';

afterEach(destroyEditors);

describe('images round-trip', () => {
  const cases: [string, string][] = [
    ['inline, untitled (was deleted)', 'An ![untitled](a.png) image.\n'],
    ['inline, titled', 'A ![titled](a.png "T") image.\n'],
    ['standalone, untitled (was deleted)', '![A diagram of the flow](d.png)\n'],
    ['standalone, titled (alt was saved as 1.00)', '![A diagram](d.png "Caption")\n'],
    ['standalone, empty alt', '![](d.png)\n'],
    ['standalone, numeric alt (a ratio) kept as written', '![0.5](d.png)\n'],
    ['in a list item', '* ![icon](i.png) item\n'],
  ];
  for (const [name, source] of cases) {
    it(name, async () => {
      expect(await savedFile(source)).toBe(source);
    });
  }
});

describe('a resized standalone image', () => {
  it('saves the new ratio as before (Crepe stores it in the alt)', async () => {
    const saved = await savedFile('![A diagram](d.png)\n', (crepe) => {
      crepe.editor.action((ctx) => {
        const view = ctx.get(editorViewCtx);
        let target = -1;
        view.state.doc.descendants((node, pos) => {
          if (node.type.name === 'image-block') target = pos;
          return target === -1;
        });
        expect(target).not.toBe(-1);
        view.dispatch(view.state.tr.setNodeAttribute(target, 'ratio', 0.5));
      });
    });
    expect(saved).toBe('![0.50](d.png)\n');
  });
});

describe('ratioFromAlt', () => {
  it("reads Crepe's ratio, 1 for text or nothing", () => {
    expect(ratioFromAlt('0.75')).toBe(0.75);
    expect(ratioFromAlt('A diagram')).toBe(1);
    expect(ratioFromAlt('')).toBe(1);
    expect(ratioFromAlt('0')).toBe(1);
  });
});
