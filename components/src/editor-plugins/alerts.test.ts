/**
 * GitHub alerts (`alerts.ts`) through the real Crepe with the editor's
 * configuration. The `alerts` fixture's byte round trip and outline against
 * pulldown-cmark are in `editor-parity.test.ts`; these cover the recognition
 * rule's edges, the serializer and the editing affordances.
 */
import { afterEach, describe, expect, it } from 'vitest';
import type { Crepe } from '@milkdown/crepe';
import { editorViewCtx } from '@milkdown/kit/core';
import { TextSelection } from '@milkdown/kit/prose/state';
import { serializeBody } from '../editor-crepe.js';
import { destroyEditors, editorOutline, openCrepe } from '../editor-test-support.js';
import { alertLineFor, parseAlertLine } from './alerts.js';

afterEach(destroyEditors);

describe('the tag line rule (pulldown-cmark scan_blockquote_tag)', () => {
  it.each([
    ['> [!NOTE]\n> x', 'note', ' [!NOTE]'],
    ['>[!tip]', 'tip', '[!tip]'],
    ['> [!Important]  \n', 'important', ' [!Important]  '],
    ['  > [!WARNING]\r\n> x', 'warning', ' [!WARNING]'],
    ['> [!caution]\t', 'caution', ' [!caution]\t'],
  ])('%j is an alert', (source, kind, line) => {
    expect(parseAlertLine(source)).toEqual({ kind, line });
  });

  it.each(['>  [!NOTE]', '> [!NOTE] text', '> [!NOTES]', '> [!FOO]', '> \\[!NOTE]', '> [NOTE]', '> x\n> [!NOTE]'])(
    '%j is not',
    (source) => {
      expect(parseAlertLine(source)).toBeUndefined();
    },
  );

  it('prints the authored line while it names the kind, else the canonical one', () => {
    expect(alertLineFor('note', '[!note]')).toBe('[!note]');
    expect(alertLineFor('tip', ' [!NOTE]')).toBe(' [!TIP]');
    expect(alertLineFor('tip', null)).toBe(' [!TIP]');
    expect(alertLineFor('tip', ' [!TIP] trailing')).toBe(' [!TIP]');
  });
});

async function roundTrip(body: string): Promise<string> {
  return serializeBody(await openCrepe(body));
}

describe('round trip', () => {
  it.each([
    ['every kind', ['NOTE', 'TIP', 'IMPORTANT', 'WARNING', 'CAUTION'].map((k) => `> [!${k}]\n> Body of ${k}.\n`).join('\n')],
    ['authored case and spacing', '>[!note]\n> lower case, no space\n'],
    ['tag alone', '> [!TIP]\n'],
    ['several blocks', '> [!WARNING]\n> First.\n>\n> * a\n> * b\n'],
    ['a heading right after the tag', '> [!NOTE]\n> # Heading\n'],
    ['nested in a blockquote', '> Outer.\n>\n> > [!TIP]\n> > Inner.\n'],
    ['in a list item', '* item\n\n  > [!CAUTION]\n  > Careful.\n'],
    ['inline markup in the body', '> [!NOTE]\n> **Bold** and [a link](https://example.com).\n'],
    ['an ordinary quote', '> Just a quote.\n'],
  ])('%s', async (_name, body) => {
    expect(await roundTrip(body)).toBe(body);
  });

  it('keeps a hard break after the tag (trailing spaces) as an alert', async () => {
    const body = '> [!NOTE]  \n> Body.\n';
    expect(await roundTrip(body)).toBe(body);
  });

  it('never turns a non-alert into one', async () => {
    for (const body of ['> \\[!NOTE]\n> x\n', '> x\n> [!NOTE]\n', '> [!NOTE] same line\n', '> [!NOPE]\n> x\n']) {
      const saved = await roundTrip(body);
      const outline = await editorOutline(saved);
      expect(outline.filter((node) => node.alert), saved).toEqual([]);
    }
  });
});

describe('outline', () => {
  it('starts the first paragraph after the tag line, as the renderer does', async () => {
    expect(await editorOutline('> [!NOTE]\n> Body\n> more.\n\n> [!TIP]\n')).toEqual([
      { depth: 0, kind: 'blockquote', lines: [1, 3], alert: 'note' },
      { depth: 1, kind: 'paragraph', lines: [2, 3] },
      { depth: 0, kind: 'blockquote', lines: [5, 5], alert: 'tip' },
    ]);
  });
});

function editorDom(crepe: Crepe): HTMLElement {
  return crepe.editor.action((ctx) => ctx.get(editorViewCtx).dom);
}

/** Puts the cursor at the start of the first text block inside the first blockquote. */
function cursorInQuote(crepe: Crepe): void {
  crepe.editor.action((ctx) => {
    const view = ctx.get(editorViewCtx);
    let target = -1;
    view.state.doc.descendants((node, pos) => {
      if (target === -1 && node.type.name === 'blockquote') target = pos + 2;
      return target === -1;
    });
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, target)));
  });
}

describe('editing', () => {
  it('renders an alert as a labelled callout', async () => {
    const crepe = await openCrepe('> [!WARNING]\n> Careful.\n\n> Plain.\n');
    const quotes = [...editorDom(crepe).querySelectorAll('blockquote')];
    expect(quotes.map((q) => q.dataset.mbrAlert)).toEqual(['warning', undefined]);
    expect(quotes[0].dataset.mbrAlertLabel).toBe('Warning');
  });

  it('typing `[!tip] ` at the start of a blockquote makes it an alert', async () => {
    const crepe = await openCrepe('> \n');
    cursorInQuote(crepe);
    crepe.editor.action((ctx) => {
      const view = ctx.get(editorViewCtx);
      const text = '[!tip] ';
      for (const char of text) {
        const { from, to } = view.state.selection;
        const handled = view.someProp('handleTextInput', (f) => f(view, from, to, char, () => view.state.tr.insertText(char, from, to)));
        if (!handled) view.dispatch(view.state.tr.insertText(char, from, to));
      }
      view.dispatch(view.state.tr.insertText('Hint.'));
    });
    expect(serializeBody(crepe)).toBe('> [!TIP]\n> Hint.\n');
  });

  it('Backspace at the start of an alert makes it a plain blockquote', async () => {
    const crepe = await openCrepe('> [!NOTE]\n> Body.\n');
    cursorInQuote(crepe);
    crepe.editor.action((ctx) => {
      const view = ctx.get(editorViewCtx);
      const event = new KeyboardEvent('keydown', { key: 'Backspace' });
      view.someProp('handleKeyDown', (f) => f(view, event));
    });
    expect(serializeBody(crepe)).toBe('> Body.\n');
  });
});
