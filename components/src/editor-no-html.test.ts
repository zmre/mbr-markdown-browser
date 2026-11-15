/**
 * The editor never puts HTML into a note.
 *
 * Milkdown prints an empty paragraph — what Enter on a blank line makes, the
 * usual way to leave a gap on a phone keyboard — as a `<br />` placeholder
 * (`remarkPreserveEmptyLine`), and folds a `<br />` an author wrote back into
 * an empty paragraph on load. And remark writes whitespace at the start of a
 * line as a character reference (`&#x20;`). These drive the editor as a user
 * does — the real keymaps, ProseMirror transactions — and save through the
 * real save path (`createBodySaver` → `documentToSave`), so what is asserted is
 * what lands on disk.
 *
 * HTML the author typed is the other half of the rule: it stays, byte for
 * byte, through edits of its own block and others.
 */
import { afterEach, describe, expect, it } from 'vitest';
import type { Crepe } from '@milkdown/crepe';
import type { EditorView } from '@milkdown/kit/prose/view';
import { createBodySaver, serializeBody } from './editor-crepe.js';
import { documentToSave, openDocument } from './editor-frontmatter.js';
import { openFrontmatter } from './frontmatter/model.js';
import { destroyEditors, fixture, openCrepe, placeCursor, press, typeText, viewOf } from './editor-test-support.js';

afterEach(destroyEditors);

/** Opens `source` as the editor does, runs `edit`, and returns what Save writes. */
async function save(source: string, edit: (view: EditorView, crepe: Crepe) => void): Promise<string> {
  const doc = openDocument(source);
  const crepe = await openCrepe(doc.body);
  const saver = createBodySaver(crepe, doc.body);
  edit(viewOf(crepe), crepe);
  return documentToSave(doc, openFrontmatter(doc.frontmatter ?? '').text, saver.body().body);
}

/** Puts the cursor at `offset` into `at`, then presses each key (`S-Enter` is Shift+Enter) and types `text`. */
function keys(at: string, offset: number, pressed: readonly string[], text = '') {
  return (view: EditorView): void => {
    placeCursor(view, at, offset);
    for (const key of pressed) {
      const shift = key.startsWith('S-');
      expect(press(view, shift ? key.slice(2) : key, { shiftKey: shift }), `${key} handled`).toBe(true);
    }
    if (text) typeText(view, text);
  };
}

/** Matches any HTML tag or character reference. */
const ADDED_HTML = /<[a-z/!]|&#?[a-z0-9]+;/i;

describe('typing never writes HTML', () => {
  it.each([
    // [what, source, cursor in, offset, keys, typed, saved]
    ['Enter twice between paragraphs', 'a\n\nb\n', 'a', 1, ['Enter', 'Enter'], '', 'a\n\nb\n'],
    ['Enter twice between paragraphs, then typing', 'a\n\nb\n', 'a', 1, ['Enter', 'Enter', 'Enter'], 'c', 'a\n\nc\n\nb\n'],
    ['Enter twice at the end, then typing', 'a\n\nb\n', 'b', 1, ['Enter', 'Enter'], 'c', 'a\n\nb\n\nc\n'],
    ['Enter at the end of a heading', '# abc\n\ntext\n', 'abc', 3, ['Enter'], '', '# abc\n\ntext\n'],
    ['Enter after a quote', '> abc\n', 'abc', 3, ['Enter', 'Enter'], '', '> abc\n'],
    ['Enter inside a quote', '> abc\n>\n> def\n', 'abc', 3, ['Enter'], '', '> abc\n>\n> def\n'],
    ['Enter twice at the end of a list', '* abc\n* def\n', 'def', 3, ['Enter', 'Enter'], '', '* abc\n* def\n'],
    ['Enter twice at the end of a list, then typing', '* abc\n* def\n', 'def', 3, ['Enter', 'Enter'], 'x', '* abc\n* def\n\nx\n'],
    ['Enter twice after a definition', 'Term\n: def\n', 'def', 3, ['Enter', 'Enter'], '', 'Term\n: def\n'],
    ['Shift+Enter in a paragraph', 'abc\n', 'abc', 1, ['S-Enter'], '', 'a\\\nbc\n'],
    ['Shift+Enter at the end of a paragraph', 'abc\n', 'abc', 3, ['S-Enter'], '', 'abc\n'],
    ['Shift+Enter twice at the end of a paragraph', 'abc\n\nz\n', 'abc', 3, ['S-Enter', 'S-Enter'], '', 'abc\n\nz\n'],
    ['Enter, then spaces on the new line', 'abc\n\nz\n', 'abc', 3, ['Enter'], '  ', 'abc\n\nz\n'],
    ['spaces at the start of a paragraph', 'abc\n\nz\n', 'abc', 0, [], '  ', 'abc\n\nz\n'],
    ['Shift+Enter, then spaces', 'abc\n', 'abc', 3, ['S-Enter'], '  x', 'abc\\\nx\n'],
  ] as const)('%s', async (_what, source, at, offset, pressed, typed, expected) => {
    const out = await save(source, keys(at, offset, pressed, typed));
    expect(out).not.toMatch(ADDED_HTML);
    expect(out).toBe(expected);
  });

  it('an empty paragraph made by a transaction anywhere prints as nothing', async () => {
    const out = await save('> quote\n\n* item\n\ntext\n', (view) => {
      const { schema } = view.state;
      const empty = () => schema.nodes.paragraph.create();
      // In the quote, in the list item, and between top-level blocks.
      let quoteEnd = -1;
      let itemEnd = -1;
      view.state.doc.descendants((node, pos) => {
        if (node.type.name === 'blockquote') quoteEnd = pos + node.nodeSize - 1;
        if (node.type.name === 'list_item') itemEnd = pos + node.nodeSize - 1;
        return true;
      });
      view.dispatch(view.state.tr.insert(itemEnd, empty()).insert(quoteEnd, empty()).insert(0, empty()));
    });
    expect(out).toBe('> quote\n\n* item\n\ntext\n');
  });

  it('an empty task item keeps its box, which loads back as a task', async () => {
    const out = await save('* [-] cancelled\n', keys('cancelled', 0, ['Enter']));
    expect(out).toBe('* [ ]\n* [-] cancelled\n');
    const items: unknown[] = [];
    viewOf(await openCrepe(out)).state.doc.descendants((node) => {
      if (node.type.name === 'list_item') items.push(node.attrs.checked);
      return true;
    });
    expect(items).toEqual([false, false]);
  });

  it('a whole-body save (the printer alone) writes no placeholder either', async () => {
    const crepe = await openCrepe('a\n\nb\n');
    keys('a', 1, ['Enter', 'Enter', 'Enter'], 'c')(viewOf(crepe));
    expect(serializeBody(crepe)).toBe('a\n\nc\n\nb\n');
  });

  it('a random mix of Enter, Shift+Enter, Backspace and typing never writes HTML', async () => {
    let seed = 7;
    const random = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
    const moves = ['Enter', 'Enter', 'S-Enter', 'Backspace', 'type', 'space'] as const;
    for (let run = 0; run < 25; run++) {
      destroyEditors();
      const out = await save('# Title\n\nfirst line\n\n> quoted\n\n* one\n* two\n', (view) => {
        placeCursor(view, ['Title', 'first', 'quoted', 'one', 'two'][run % 5], 2);
        for (let step = 0; step < 8; step++) {
          const move = moves[Math.floor(random() * moves.length)];
          if (move === 'type') typeText(view, 'x');
          else if (move === 'space') typeText(view, ' ');
          else press(view, move.replace('S-', ''), { shiftKey: move.startsWith('S-') });
        }
      });
      expect(out, `run ${run}`).not.toMatch(ADDED_HTML);
    }
  });
});

describe('hard breaks follow the note', () => {
  it('a note that breaks lines with two spaces gets two spaces', async () => {
    const out = await save('first  \nsecond\n\nabc\n', keys('abc', 1, ['S-Enter']));
    expect(out).toBe('first  \nsecond\n\na  \nbc\n');
  });

  it('a note that breaks lines with a backslash gets a backslash', async () => {
    const out = await save('first\\\nsecond\n\nabc\n', keys('abc', 1, ['S-Enter']));
    expect(out).toBe('first\\\nsecond\n\na\\\nbc\n');
  });

  it('a note with both, or neither, gets a backslash', async () => {
    expect(await save('a  \nb\\\nc\n\nabc\n', keys('abc', 1, ['S-Enter']))).toBe('a  \nb\\\nc\n\na\\\nbc\n');
    expect(await save('abc\n', keys('abc', 1, ['S-Enter']))).toBe('a\\\nbc\n');
  });
});

describe('HTML the author wrote stays', () => {
  const AUTHORED = fixture('raw-html-breaks');

  it('loads and saves untouched, byte for byte', async () => {
    expect(await save(AUTHORED, () => {})).toBe(AUTHORED);
  });

  it('stays through an edit of every block around it', async () => {
    const out = await save(AUTHORED, (view) => {
      for (const word of ['Before', 'between', 'cell', 'item', 'quoted', 'After']) {
        placeCursor(view, word, 0);
        typeText(view, 'X');
      }
    });
    expect(out).toBe(AUTHORED.replace(/\b(Before|between|cell|item|quoted|After)/g, 'X$1'));
  });

  it('a character reference the author wrote at the start of a line stays when its paragraph is edited', async () => {
    const out = await save('&#x20; indented\n\n&#9;tabbed\n', keys('tabbed', 6, [], ' more'));
    expect(out).toBe('&#x20; indented\n\n&#9;tabbed more\n');
  });

  it('the space after a task box stays as typed', async () => {
    const out = await save('* [ ] a\n* [ ]  b\n', keys('a', 1, [], 'x'));
    expect(out).toBe('* [ ] ax\n* [ ]  b\n');
  });

  it('stays when the printer writes the whole body', async () => {
    const { body } = openDocument(AUTHORED);
    const crepe = await openCrepe(body);
    placeCursor(viewOf(crepe), 'After', 0);
    typeText(viewOf(crepe), 'X');
    expect(serializeBody(crepe).match(/<br\s*\/?>/gi)).toEqual(body.match(/<br\s*\/?>/gi));
  });
});
