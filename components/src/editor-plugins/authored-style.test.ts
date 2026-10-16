/**
 * Authored spellings survive an *edit* of the block that holds them (the
 * untouched round trip of each is the parity harness's: `commonmark-variants`,
 * `lists-*`, `rules`, `marginalia`, `tables`), and a block made in the editor
 * gets the house style.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { serializeBody } from '../editor-crepe.js';
import { destroyEditors, openCrepe, placeCursor, press, typeText, viewOf } from '../editor-test-support.js';
import {
  authoredLines,
  contentKey,
  headingStyleFrom,
  keepAuthoredLines,
  listStyleFrom,
  rowCells,
  tableStyleFrom,
} from './authored-style.js';
import type { MdNode } from './remark-support.js';

afterEach(destroyEditors);

/** Opens `source`, types `text` `offset` characters into `at`, returns what a save writes. */
async function afterTyping(source: string, at: string, offset: number, text: string): Promise<string> {
  const crepe = await openCrepe(source);
  const view = viewOf(crepe);
  placeCursor(view, at, offset);
  typeText(view, text);
  return serializeBody(crepe);
}

/** Opens `source`, presses Enter at the end of `at`, types `text`. */
async function afterEnter(source: string, at: string, text: string): Promise<string> {
  const crepe = await openCrepe(source);
  const view = viewOf(crepe);
  placeCursor(view, at, at.length);
  press(view, 'Enter');
  typeText(view, text);
  return serializeBody(crepe);
}

describe('lists keep their marker, delimiter, numbering and gap when edited', () => {
  it.each([
    ['+ a\n+ b\n', 'b', '+ a\n+ b\n+ c\n'],
    ['- a\n- b\n', 'b', '- a\n- b\n- c\n'],
    ['1) a\n2) b\n', 'b', '1) a\n2) b\n3) c\n'],
    ['1. a\n1. b\n', 'b', '1. a\n1. b\n1. c\n'],
    ['-   a\n-   b\n', 'b', '-   a\n-   b\n-   c\n'],
    ['1.  a\n2.  b\n', 'b', '1.  a\n2.  b\n3.  c\n'],
  ])('%j + Enter', async (source, at, expected) => {
    expect(await afterEnter(source, at, 'c')).toBe(expected);
  });

  it('keeps two adjacent ordered lists apart with their own delimiters', async () => {
    expect(await afterTyping('1) a\n2) b\n\n5. c\n', 'c', 1, 'd')).toBe('1) a\n2) b\n\n5. cd\n');
  });

  it('gives a list made in the editor `*` bullets, whatever was typed', async () => {
    const crepe = await openCrepe('Para\n');
    const view = viewOf(crepe);
    placeCursor(view, 'Para', 0);
    typeText(view, '- ');
    expect(serializeBody(crepe)).toBe('* Para\n');
  });
});

describe('headings', () => {
  it('a setext heading stays setext; an underline as long as the text follows it', async () => {
    expect(await afterTyping('Title\n=====\n', 'Title', 5, 'X')).toBe('TitleX\n======\n');
  });

  it('an underline of its own length keeps it', async () => {
    expect(await afterTyping('Title\n---\n', 'Title', 5, 'X')).toBe('TitleX\n---\n');
  });

  it('an ATX closing sequence stays', async () => {
    expect(await afterTyping('## Title ##\n', 'Title', 5, 'X')).toBe('## TitleX ##\n');
  });

  it('a setext heading moved to level 3 becomes ATX', async () => {
    const crepe = await openCrepe('Title\n=====\n');
    const view = viewOf(crepe);
    view.dispatch(view.state.tr.setNodeMarkup(0, undefined, { ...view.state.doc.child(0).attrs, level: 3 }));
    expect(serializeBody(crepe)).toBe('### Title\n');
  });
});

describe('code blocks', () => {
  it('an indented block stays indented', async () => {
    expect(await afterTyping('Para\n\n    code\n', 'code', 4, 'X')).toBe('Para\n\n    codeX\n');
  });

  it('a `~~~` fence, a long fence and the info string\'s meta stay', async () => {
    expect(await afterTyping('~~~js title="a"\nx\n~~~\n', 'x', 1, 'y')).toBe('~~~js title="a"\nxy\n~~~\n');
    expect(await afterTyping('````\nx\n````\n', 'x', 1, 'y')).toBe('````\nxy\n````\n');
  });
});

describe('breaks, rules, quotes', () => {
  it('a two-space hard break stays two spaces', async () => {
    expect(await afterTyping('one  \ntwo\n', 'two', 3, '!')).toBe('one  \ntwo!\n');
  });

  it('a rule keeps its spelling; one made in the editor is `---`', async () => {
    expect(await afterTyping('a\n\n* * *\n\nb\n', 'b', 1, 'c')).toBe('a\n\n* * *\n\nbc\n');
  });

  it('a rule under a paragraph in a tight list item is `***`, never a setext underline', async () => {
    const crepe = await openCrepe('- a\n- b\n');
    const view = viewOf(crepe);
    placeCursor(view, 'a', 1);
    const at = view.state.selection.$from.after();
    view.dispatch(view.state.tr.insert(at, view.state.schema.nodes.hr.create()));
    expect(serializeBody(crepe)).toBe('- a\n  ***\n- b\n');
  });

  it('marginalia stay compact', async () => {
    expect(await afterTyping('>>> note\n', 'note', 4, 's')).toBe('>>> notes\n');
  });
});

describe('tables', () => {
  it('an unaligned table keeps its rows; the edited one is printed in its spacing', async () => {
    const source = '|a|b|\n|-|-|\n|1|2|\n|3|4|\n';
    expect(await afterTyping(source, '3', 1, '0')).toBe('|a|b|\n|-|-|\n|1|2|\n|30|4|\n');
    const padded = '| a | b |\n|---|---|\n| long | 2 |\n| 3 | 4 |\n';
    expect(await afterTyping(padded, '3', 1, '0')).toBe('| a | b |\n|---|---|\n| long | 2 |\n| 30 | 4 |\n');
  });

  it('an aligned table is re-aligned whole', async () => {
    const source = '| a | b |\n| - | - |\n| 1 | 2 |\n';
    expect(await afterTyping(source, '1', 1, '00')).toBe('| a   | b |\n| --- | - |\n| 100 | 2 |\n');
  });
});

describe('paragraph lines and top-level gaps', () => {
  it('keeps an indented paragraph as typed until its text is edited', async () => {
    const crepe = await openCrepe('  one\n  two \n');
    expect(serializeBody(crepe)).toBe('  one\n  two \n');
    expect(await afterTyping('  one\n  two\n', 'two', 3, '!')).toBe('one\ntwo!\n');
  });

  it('drops a first-line indentation that would join the list above', async () => {
    const crepe = await openCrepe('* a\n\n# H\n\n  para\n');
    const view = viewOf(crepe);
    // Delete the heading: the indented paragraph now follows the list.
    placeCursor(view, 'H', 0);
    const $pos = view.state.selection.$from;
    view.dispatch(view.state.tr.delete($pos.before(), $pos.after()));
    expect(serializeBody(crepe)).toBe('* a\n\npara\n');
  });

  it('keeps more than one blank line between the same two blocks, not around a new one', async () => {
    const crepe = await openCrepe('One\n\n\nTwo\n');
    expect(serializeBody(crepe)).toBe('One\n\n\nTwo\n');
    expect(await afterEnter('One\n\n\nTwo\n', 'One', 'New')).toBe('One\n\nNew\n\nTwo\n');
  });
});

describe('recording', () => {
  const at = (offset: number, column = 1) => ({ start: { offset, column }, end: { offset } });

  it('reads list markers', () => {
    const list = (second: number): MdNode => ({
      type: 'list',
      children: [
        { type: 'listItem', position: at(0) },
        { type: 'listItem', position: at(second) },
      ],
    });
    expect(listStyleFrom('* a\n* b\n', list(4))).toEqual({ marker: '*' });
    expect(listStyleFrom('3) a\n3) b\n', list(5))).toEqual({ marker: ')', sameNumber: true });
    expect(listStyleFrom('-   a\n', list(0))).toEqual({ marker: '-', indent: 'tab' });
    // Three spaces after `-` is not a tab stop: printed with one.
    expect(listStyleFrom('-  a\n', list(0))).toEqual({ marker: '-' });
  });

  it('reads headings', () => {
    const heading = (source: string) => headingStyleFrom(source, { type: 'heading', position: { start: { offset: 0 }, end: { offset: source.length } } });
    expect(heading('Title\n===')).toEqual({ source: 'Title\n===', setext: '===', fit: false });
    expect(heading('# A #')).toEqual({ source: '# A #', closing: ' #' });
    expect(heading('# C# #')).toEqual({ source: '# C# #', closing: ' #' });
    expect(heading('# #')).toEqual({ source: '# #' });
  });

  it('splits table rows on unescaped pipes', () => {
    expect(rowCells('| a | b\\|c |')).toEqual(['a', 'b\\|c']);
    expect(rowCells('a|b')).toEqual(['a', 'b']);
    expect(rowCells('|a|')).toEqual(['a']);
  });

  it('reads a block\'s lines relative to its container', () => {
    const node = (start: number, column: number, end: number) => ({
      type: 'paragraph',
      position: { start: { offset: start, column }, end: { offset: end } },
    });
    // Top level: the first line's indentation and the last line's trailing whitespace are kept.
    expect(authoredLines('  a\n    b  \nnext', node(2, 3, 11), 0)).toEqual(['  a', '    b  ']);
    // In a container: its prefix comes off every later line.
    expect(authoredLines('> a\n>   b\n', node(2, 3, 9), 2)).toEqual(['a', '  b']);
    // A lazy line in a container cannot be placed.
    expect(authoredLines('> a\nb\n', node(2, 3, 5), 2)).toBeUndefined();
  });

  it('keeps authored lines only for the same text', () => {
    expect(keepAuthoredLines('a\nb', ['  a', 'b '], true)).toBe('  a\nb ');
    expect(keepAuthoredLines('a\nb', ['  a', 'b '], false)).toBe('a\nb ');
    expect(keepAuthoredLines('a\nc', ['  a', 'b'], true)).toBe('a\nc');
    expect(keepAuthoredLines('a\nb', ['a', 'b', 'c'], true)).toBe('a\nb');
    // Trailing spaces inside the block are a hard break or not: they must match.
    expect(keepAuthoredLines('a\nb', ['a  ', 'b'], true)).toBe('a\nb');
    // Four columns would be indented code.
    expect(keepAuthoredLines('a', ['    a'], true)).toBe('a');
  });

  it('keys content the same on both sides of the editor\'s round trip', () => {
    const parsed = [
      {
        type: 'paragraph',
        position: { start: { offset: 0 } },
        children: [
          { type: 'text', value: 'a' },
          { type: 'break', data: { isInline: true } },
          { type: 'text', value: 'b' },
          { type: 'inlineMath', value: '$x$', display: false },
        ],
      },
      { type: 'list', ordered: true, start: 1, children: [{ type: 'listItem', label: 1, spread: false, children: [] }] },
    ];
    const serialized = [
      {
        type: 'paragraph',
        children: [
          { type: 'text', value: 'a' },
          { type: 'text', value: '\n' },
          { type: 'text', value: 'b' },
          { type: 'inlineMath', value: '$x$', isMark: true },
        ],
      },
      { type: 'list', ordered: true, start: 1, children: [{ type: 'listItem', spread: false, children: [] }] },
    ];
    expect(contentKey(serialized)).toBe(contentKey(parsed));
    // A hard break is content.
    const hard = [{ type: 'paragraph', children: [{ type: 'text', value: 'a' }, { type: 'break' }, { type: 'text', value: 'b' }] }];
    expect(contentKey(hard)).not.toBe(contentKey(parsed.slice(0, 1)));
  });

  it('strips container prefixes from table rows', () => {
    const source = '> | a |\n> | - |\n> | 1 |';
    const table = { type: 'table', position: { start: { offset: 2, column: 3 }, end: { offset: source.length } } };
    expect(tableStyleFrom(source, table)).toEqual({ rows: ['| a |', '| - |', '| 1 |'] });
  });
});
