import { afterEach, describe, expect, it } from 'vitest';
import { serializeBody } from '../editor-crepe.js';
import { destroyEditors, openCrepe, placeCursor, typeText, viewOf } from '../editor-test-support.js';
import { unwrapNestedMarks } from './nested-marks.js';
import type { MdNode } from './remark-support.js';

afterEach(destroyEditors);

describe('nested marks of one kind', () => {
  it('unwraps a mark inside its own kind, at any depth', () => {
    const text = (value: string): MdNode => ({ type: 'text', value });
    const em = (...children: MdNode[]): MdNode => ({ type: 'emphasis', children });
    const strong = (...children: MdNode[]): MdNode => ({ type: 'strong', children });
    const tree: MdNode = { type: 'root', children: [{ type: 'paragraph', children: [em(text('a '), strong(em(em(text('b')))), text(' c'))] }] };
    unwrapNestedMarks(tree);
    expect(tree.children?.[0].children).toEqual([em(text('a '), strong(text('b')), text(' c'))]);
  });

  it('keeps the outer mark over the rest when the paragraph is edited', async () => {
    const crepe = await openCrepe('*a _b_ c* d\n');
    expect(serializeBody(crepe)).toBe('*a _b_ c* d\n');
    const view = viewOf(crepe);
    placeCursor(view, ' d', 2);
    typeText(view, '!');
    expect(serializeBody(crepe)).toBe('*a b c* d!\n');
  });
});
