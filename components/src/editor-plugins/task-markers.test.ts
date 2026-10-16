/**
 * Extended task markers (`task-markers.ts`) through the real Crepe with the
 * editor's configuration. The fixture-level checks (the tasks fixture's
 * outline against pulldown-cmark) are in `editor-parity.test.ts`; these pin
 * the pieces: the marker model, byte identity of every marker and annotation,
 * and the checkbox's state cycle.
 */
import { afterEach, describe, expect, it } from 'vitest';
import type { Crepe } from '@milkdown/crepe';
import { editorViewCtx } from '@milkdown/kit/core';
import type { Node as ProseNode } from '@milkdown/kit/prose/model';
import { serializeBody } from '../editor-crepe.js';
import { destroyEditors, editorOutline, fixture, openCrepe, savedFile } from '../editor-test-support.js';
import { effectiveMarker, nextCancelMarker, nextToggleMarker, statusOf } from './task-markers.js';

afterEach(destroyEditors);

describe('marker model', () => {
  it('normalizes authored markers to the four statuses', () => {
    expect([' ', '\t', 'x', 'X', '-', '>', '?'].map(statusOf)).toEqual([' ', ' ', 'x', 'x', '-', '>', undefined]);
  });

  it('keeps the authored marker only while it agrees with `checked`', () => {
    expect(effectiveMarker(null, 'x')).toBeUndefined();
    expect(effectiveMarker(undefined, null)).toBeUndefined();
    expect(effectiveMarker(true, 'X')).toBe('X');
    expect(effectiveMarker(false, '-')).toBe('-');
    expect(effectiveMarker(false, '>')).toBe('>');
    // Something that only knows GFM flipped `checked`: its value wins.
    expect(effectiveMarker(false, 'X')).toBe(' ');
    expect(effectiveMarker(true, '-')).toBe('x');
    expect(effectiveMarker(true, null)).toBe('x');
    expect(effectiveMarker(false, 'q')).toBe(' ');
  });

  it('cycles like the document checkbox (task-toggle.ts) and writes what set_marker writes', () => {
    // Click: complete ↔ reopen; a cancelled or deferred task completes.
    expect([' ', 'x', 'X', '-', '>'].map(nextToggleMarker)).toEqual(['x', ' ', ' ', 'x', 'x']);
    // Right click: cancel ↔ reopen; deferred counts as cancelled.
    expect([' ', 'x', 'X', '-', '>'].map(nextCancelMarker)).toEqual(['-', '-', '-', ' ', ' ']);
  });
});

/** The body Crepe writes for `body`, untouched. */
async function roundTrip(body: string): Promise<string> {
  return serializeBody(await openCrepe(body));
}

describe('round trip', () => {
  it('saves the tasks fixture byte for byte (but for `-` bullets, item 6)', async () => {
    // `-` → `*` is the bullet-character setting (TODO item 6), not a marker;
    // with `*` bullets the whole fixture must survive.
    const source = fixture('tasks').replace(/^(\s*)- /gm, '$1* ');
    expect(await savedFile(source)).toBe(source);
  });

  it.each([
    ['open', '* [ ] a\n'],
    ['done', '* [x] a\n'],
    ['done, capital X', '* [X] a\n'],
    ['cancelled', '* [-] a\n'],
    ['deferred', '* [>] a\n'],
    ['ordered', '1. [-] a\n2. [>] b\n3. [X] c\n'],
    ['nested', '* [>] a\n  * [-] b\n  * [X] c\n'],
    ['loose', '* [-] a\n\n* [>] b\n'],
    ['annotations', '* [ ] urgent !! #work @due(2026-10-16 5:00 PM)\n* [x] done @done(2026-10-14 09:30)\n* [>] later > 2026-11-01 #errands\n'],
    ['a second paragraph', '* [-] a\n\n  more about a\n'],
  ])('%s', async (_name, body) => {
    expect(await roundTrip(body)).toBe(body);
  });

  it('leaves non-tasks alone', async () => {
    for (const body of [
      // Escaped: text, and must stay escaped or it would become a task.
      '* \\[-] not a task\n',
      // Not at the start of the item. (Remark escapes every bracket pair in
      // text, `[a]` too — `\[-]` would still be text, so it is safe either
      // way and not this plugin's to change.)
      '* see \\[-] here\n',
      // Not in a list.
      '\\[-] paragraph\n',
    ]) {
      expect(await roundTrip(body)).toBe(body);
    }
  });

  it('outlines status, not the authored character', async () => {
    const outline = await editorOutline('* [X] a\n* [>] b\n* [-] c\n* [ ] d\n* e\n');
    expect(outline.filter((node) => node.kind === 'item').map((node) => node.task)).toEqual([
      'x',
      '>',
      '-',
      ' ',
      undefined,
    ]);
  });

  it('leaves `[-]`/`[>]` to a reference link when the document defines that label, as the renderer does', async () => {
    const body = '* [-] a\n* [>] b\n* [x] c\n\n[-]: /dash\n[>]: /gt\n[x]: /x\n';
    expect(await savedFile(body)).toBe(body);
    const outline = await editorOutline(body);
    // GFM's check outranks a `[x]` reference; `[-]`/`[>]` are mbr's, read after links.
    expect(outline.filter((node) => node.kind === 'item').map((node) => node.task)).toEqual([undefined, undefined, 'x']);
  });

  it('needs content after the box, as GFM does', async () => {
    const outline = await editorOutline('* [-]\n* [>]   \n* [-] x\n');
    expect(outline.filter((node) => node.kind === 'item').map((node) => node.task)).toEqual([
      undefined,
      undefined,
      '-',
    ]);
  });
});

/** Every list item in document order, with its position. */
function listItems(crepe: Crepe): { node: ProseNode; pos: number }[] {
  return crepe.editor.action((ctx) => {
    const items: { node: ProseNode; pos: number }[] = [];
    ctx.get(editorViewCtx).state.doc.descendants((node, pos) => {
      if (node.type.name === 'list_item') items.push({ node, pos });
    });
    return items;
  });
}

/** The label wrappers of the rendered list items, in document order. */
function labels(crepe: Crepe): HTMLElement[] {
  return crepe.editor.action((ctx) => [
    ...ctx.get(editorViewCtx).dom.querySelectorAll<HTMLElement>('.milkdown-list-item-block .label-wrapper'),
  ]);
}

function click(target: HTMLElement): void {
  target.dispatchEvent(new PointerEvent('pointerdown', { button: 0, bubbles: true, cancelable: true }));
}

function rightClick(target: HTMLElement): void {
  target.dispatchEvent(new MouseEvent('contextmenu', { button: 2, bubbles: true, cancelable: true }));
}

describe('the checkbox', () => {
  it('renders each status', async () => {
    const crepe = await openCrepe('* [ ] a\n* [X] b\n* [-] c\n* [>] d\n* e\n');
    const blocks = crepe.editor.action((ctx) => [
      ...ctx.get(editorViewCtx).dom.querySelectorAll<HTMLElement>('.milkdown-list-item-block'),
    ]);
    expect(blocks.map((block) => block.dataset.taskMarker)).toEqual([' ', 'X', '-', '>', undefined]);
    const classes = blocks.map((block) => block.querySelector('.label')?.className ?? '');
    expect(classes[0]).toContain('unchecked');
    expect(classes[1]).toContain('checked');
    expect(classes[2]).toContain('mbr-task-canceled');
    expect(classes[3]).toContain('mbr-task-deferred');
    expect(classes[4]).toContain('bullet');
  });

  it('click completes and reopens; right click cancels and reopens', async () => {
    const crepe = await openCrepe('* [ ] a\n* [-] b\n* [>] c\n* [X] d\n');
    const [a, b, c, d] = labels(crepe);
    click(a);
    click(b);
    rightClick(c);
    click(d);
    expect(serializeBody(crepe)).toBe('* [x] a\n* [x] b\n* [ ] c\n* [ ] d\n');
    rightClick(labels(crepe)[0]);
    rightClick(labels(crepe)[1]);
    expect(serializeBody(crepe)).toBe('* [-] a\n* [-] b\n* [ ] c\n* [ ] d\n');
    expect(listItems(crepe).map(({ node }) => node.attrs.checked)).toEqual([false, false, false, false]);
  });

  it('does nothing on a plain item', async () => {
    const crepe = await openCrepe('* a\n');
    click(labels(crepe)[0]);
    rightClick(labels(crepe)[0]);
    expect(serializeBody(crepe)).toBe('* a\n');
  });

  it('a task made by setting only `checked` (slash menu, GFM input rule) prints GFM boxes', async () => {
    const crepe = await openCrepe('* a\n* [-] b\n');
    crepe.editor.action((ctx) => {
      const view = ctx.get(editorViewCtx);
      const [first, second] = listItems(crepe);
      // As Crepe's own label handler and slash menu do: `checked` only.
      view.dispatch(
        view.state.tr
          .setNodeAttribute(first.pos, 'checked', false)
          .setNodeAttribute(second.pos, 'checked', true),
      );
    });
    expect(serializeBody(crepe)).toBe('* [ ] a\n* [x] b\n');
  });
});
