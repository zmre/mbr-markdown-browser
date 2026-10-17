/**
 * Authored spellings survive an *edit* of the block that holds them (the
 * untouched round trip of each is the parity harness's: `commonmark-variants`,
 * `lists-*`, `rules`, `marginalia`, `tables`), and a block made in the editor
 * gets the house style.
 */
import { remarkCtx } from '@milkdown/kit/core';
import { afterEach, describe, expect, it } from 'vitest';
import { serializeBody } from '../editor-crepe.js';
import { destroyEditors, openCrepe, placeCursor, press, typeText, viewOf } from '../editor-test-support.js';
import type { RootContent } from 'mdast';
import {
  adjacentIsSafe,
  authoredLines,
  contentKey,
  headingStyleFrom,
  keepAuthoredLines,
  listItemStyleFrom,
  listStyleFrom,
  rowCells,
  tableStyleFrom,
  visualWidth,
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
    // Two spaces after `-` is not a tab stop: kept as typed.
    expect(listStyleFrom('-  a\n', list(0))).toEqual({ marker: '-', spaces: 2 });
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

/** Opens `source`, deletes the `n`th list item (document order), returns what a save writes. */
async function afterDeletingItem(source: string, n: number): Promise<string> {
  const crepe = await openCrepe(source);
  const view = viewOf(crepe);
  let found: { from: number; to: number } | undefined;
  let seen = 0;
  view.state.doc.descendants((node, pos) => {
    if (found) return false;
    if (node.type.name === 'list_item' && seen++ === n) found = { from: pos, to: pos + node.nodeSize };
    return true;
  });
  if (!found) throw new Error(`no list item ${n}`);
  view.dispatch(view.state.tr.delete(found.from, found.to));
  return serializeBody(crepe);
}

describe('found by roundtrip-check (the rt-* fixtures hold the untouched round trips)', () => {
  it('a tab-indented nested list keeps its tabs when an item is edited', async () => {
    expect(await afterTyping('* a\n\t* b\n\t\t* c\n', 'b', 1, 'X')).toBe('* a\n\t* bX\n\t\t* c\n');
  });

  it('a four-space nested list keeps its indentation for a new item', async () => {
    expect(await afterEnter('* a\n    * b\n* c\n', 'b', 'n')).toBe('* a\n    * b\n    * n\n* c\n');
  });

  it('an empty item stays its marker alone', async () => {
    expect(await afterTyping('- one\n-\n- three\n', 'one', 3, 'X')).toBe('- oneX\n-\n- three\n');
  });

  it('an item that starts with a list gets no `<br />` line', async () => {
    expect(await afterTyping('- - x\n  - y\n', 'y', 1, 'Z')).toBe('- - x\n  - yZ\n');
  });

  it('ordered numbers stay as typed; a new item is counted on from the one before', async () => {
    expect(await afterTyping('1. a\n3. b\n7. c\n', 'b', 1, 'X')).toBe('1. a\n3. bX\n7. c\n');
    expect(await afterEnter('1. a\n3. b\n7. c\n', 'a', 'n')).toBe('1. a\n2. n\n3. b\n7. c\n');
  });

  it('a list right under a heading stays there; a block put between gets blank lines', async () => {
    expect(await afterTyping('# Title\n- a\n', 'Title', 5, 'X')).toBe('# TitleX\n- a\n');
    expect(await afterEnter('# Title\n- a\n', 'Title', 'p')).toBe('# Title\n\np\n\n- a\n');
  });

  it('heading trailing spaces stay until the heading is edited', async () => {
    expect(await afterTyping('# Title  \n\nText\n', 'Text', 4, '!')).toBe('# Title  \n\nText!\n');
    expect(await afterTyping('# Title  \n\nText\n', 'Title', 5, 'X')).toBe('# TitleX\n\nText\n');
  });

  it('a loose list keeps its own gaps between items', async () => {
    expect(await afterTyping('* a\n* b\n\n* c\n* d\n', 'd', 1, 'X')).toBe('* a\n* b\n\n* c\n* dX\n');
  });

  it('a loose list whose only gap is deleted is printed loose, as the editor still has it', async () => {
    // Deleting `c` leaves no authored blank line between items; the list is
    // still loose in the editor, so every gap is the printer's.
    expect(await afterDeletingItem('* a\n* b\n\n* c\n', 2)).toBe('* a\n\n* b\n');
  });

  it('a paragraph spelled differently but read the same keeps its spelling until edited', async () => {
    expect(await afterTyping('AT\\&T ~x~\n\nOther\n', 'Other', 5, '!')).toBe('AT\\&T ~x~\n\nOther!\n');
    expect(await afterTyping('AT\\&T ~x~\n\nOther\n', 'AT', 2, 'X')).toBe('ATX&T ~~x~~\n\nOther\n');
  });

  it('a lazy line stays lazy until its paragraph is edited', async () => {
    expect(await afterTyping('- item\nlazy line\n- next\n', 'next', 4, '!')).toBe('- item\nlazy line\n- next!\n');
    expect(await afterTyping('- item\nlazy line\n- next\n', 'item', 4, '!')).toBe('- item!\n  lazy line\n- next\n');
    expect(await afterTyping('> quote\nlazy\n\nafter\n', 'after', 5, '!')).toBe('> quote\nlazy\n\nafter!\n');
  });

  it('an item whose content starts on the next line keeps it there', async () => {
    expect(await afterTyping('*\n\t* a\n* b\n', 'b', 1, '!')).toBe('*\n\t* a\n* b!\n');
  });

  it('indented code written with tabs keeps them', async () => {
    expect(await afterTyping('Text.\n\n\tcode\n\t\tdeeper\n', 'Text', 4, '!')).toBe('Text!.\n\n\tcode\n\t\tdeeper\n');
  });

  it('a quote keeps `> ` on its blank lines', async () => {
    expect(await afterTyping('> a\n> \n> b\n', 'b', 1, 'X')).toBe('> a\n> \n> bX\n');
  });
});

/** Opens `source`, runs `edit` on the `n`th list item (document order), returns what a save writes. */
async function afterItemEdit(
  source: string,
  n: number,
  edit: (view: ReturnType<typeof viewOf>, pos: number, node: import('@milkdown/kit/prose/model').Node) => void,
): Promise<string> {
  const crepe = await openCrepe(source);
  const view = viewOf(crepe);
  let found: { pos: number; node: import('@milkdown/kit/prose/model').Node } | undefined;
  let seen = 0;
  view.state.doc.descendants((node, pos) => {
    if (found) return false;
    if (node.type.name === 'list_item' && seen++ === n) found = { pos, node };
    return true;
  });
  if (!found) throw new Error(`no list item ${n}`);
  edit(view, found.pos, found.node);
  return serializeBody(crepe);
}

describe('a list item is printed as typed while it holds what was loaded', () => {
  const MIXED = '* top\n\t* a\n    * b\n        * c\n';

  it.each([
    ['tabs and four spaces for one level', MIXED],
    ['a tab after the marker', '-\tx - a\n-\tx - b\n'],
    ['its own spacing after the marker', '*  0 x\n* b\n* --\n'],
    ['a nested list as its first block', '* a\n\t* 6. x\n'],
    ['an empty task\'s trailing space', '* [ ] \n* b\n'],
    ['a whitespace-only line inside it', '* u\n\t* x\n\t\n\t## h\n'],
    ['a lazy line under a four-space nested list', '* x\n    * a\n    * b\nlazy\n\n- y\n'],
    ['an indented top-level list', '  * item\n  * other\n'],
  ])('%s', async (_name, source) => {
    expect(serializeBody(await openCrepe(source))).toBe(source);
  });

  it('an edited item is printed again, its untouched nested items as typed', async () => {
    // `top` is printed with the house prefix; `b` keeps `c` four columns in.
    expect(await afterTyping(MIXED, 'a', 1, 'X')).toBe('* top\n  * aX\n  * b\n      * c\n');
    expect(await afterTyping(MIXED, 'c', 1, 'X')).toBe('* top\n  * a\n  * b\n    * cX\n');
  });

  it('an edited item holding indented code keeps the code where it was', async () => {
    // The code (`  * a`) starts four columns past `y`'s content column (6),
    // at 10: a tab prefix (column 8) would add two spaces to every line of it.
    const source = '* x\n\t* y\n\n\n\t\t\t* a\n\t\t\t* b\n';
    expect(serializeBody(await openCrepe(source))).toBe(source);
    expect(await afterTyping(source, 'y', 1, 'X')).toBe('* x\n\t* yX\n\n\t        * a\n\t        * b\n');
  });

  it('a toggled task, a new mark, a deleted nested item: printed again', async () => {
    const tasks = '* [ ] a\n* [ ]  b\n';
    const toggled = await afterItemEdit(tasks, 0, (view, pos, node) =>
      view.dispatch(view.state.tr.setNodeMarkup(pos, undefined, { ...node.attrs, checked: true })),
    );
    expect(toggled).toBe('* [x] a\n* [ ]  b\n');
    const bold = await afterItemEdit('* a word\n* b\n', 0, (view, pos) => {
      const from = pos + 2;
      view.dispatch(view.state.tr.addMark(from, from + 1, view.state.schema.marks.strong.create()));
    });
    expect(bold).toBe('* **a** word\n* b\n');
    const deleted = await afterItemEdit(MIXED, 2, (view, pos, node) => view.dispatch(view.state.tr.delete(pos, pos + node.nodeSize)));
    expect(deleted).toBe('* top\n  * a\n');
  });

  it('keys object fields too, so a table\'s alignment is content', () => {
    const table = (align: (string | null)[]) => [{ type: 'table', align, children: [] }];
    expect(contentKey(table(['left', null]))).not.toBe(contentKey(table(['right', null])));
    expect(contentKey(table(['left', null]))).toBe(contentKey(table(['left', null])));
  });
});

describe('recording list items', () => {
  const item = (start: number, end: number, children: number[] = []) => ({
    type: 'listItem',
    position: { start: { offset: start }, end: { offset: end } },
    children: children.map((offset, i) => ({
      type: i === 0 ? 'paragraph' : 'list',
      position: { start: { offset }, end: { offset } },
    })),
  });

  it('reads an empty item and its trailing space', () => {
    expect(listItemStyleFrom('- \n', item(0, 2), '').style).toEqual({ empty: ' ' });
    expect(listItemStyleFrom('3)\n', item(0, 2), '').style).toEqual({ empty: '', number: 3 });
  });

  it('reads the indentation of an item\'s lines after the first, relative to its parent', () => {
    const source = '* a\n\t* b\n';
    expect(listItemStyleFrom(source, item(0, 9, [2, 5]), '')).toEqual({ style: { indent: '\t' }, linePrefix: '\t' });
  });

  it('gives up on a lazy line, or an item indented past its parent', () => {
    const lazy = '* a\n    * b\nlazy\n';
    expect(listItemStyleFrom(lazy, item(0, 17, [2, 8]), '').style?.indent).toBeUndefined();
    const indented = ' * a\n     * b\n';
    expect(listItemStyleFrom(indented, item(1, 14, [3, 10]), '').style?.indent).toBeUndefined();
  });

  it('measures tabs to the next stop of four', () => {
    expect(visualWidth('\t')).toBe(4);
    expect(visualWidth('  \t')).toBe(4);
    expect(visualWidth('\t  \t')).toBe(8);
  });
});

describe('blocks with no blank line between them', () => {
  const node = (type: string, extra: object = {}) => ({ type, children: [], ...extra }) as unknown as RootContent;
  const item = (children: unknown[] = [{ type: 'paragraph', children: [{ type: 'text', value: 'x' }] }]) => ({
    type: 'listItem',
    children,
  });

  it('may follow a heading, a rule or a fence', () => {
    expect(adjacentIsSafe(node('heading', { depth: 1 }), node('paragraph'))).toBe(true);
    expect(adjacentIsSafe(node('thematicBreak'), node('list', { children: [item()] }))).toBe(true);
    expect(adjacentIsSafe(node('code', { value: 'x' }), node('paragraph'))).toBe(true);
  });

  it('may follow a paragraph only where a paragraph is interrupted', () => {
    const para = node('paragraph');
    expect(adjacentIsSafe(para, node('list', { ordered: false, children: [item()] }))).toBe(true);
    expect(adjacentIsSafe(para, node('list', { ordered: true, start: 2, children: [item()] }))).toBe(false);
    expect(adjacentIsSafe(para, node('list', { ordered: false, children: [item([])] }))).toBe(false);
    // A `---` rule (the house rule) would underline the paragraph as a heading.
    expect(adjacentIsSafe(para, node('thematicBreak'))).toBe(false);
    expect(adjacentIsSafe(para, node('paragraph'))).toBe(false);
  });

  it('may be a list of another kind right under a list', () => {
    const dash = node('list', { ordered: false, mbrStyle: { marker: '-' }, children: [item()] });
    const star = node('list', { ordered: false, mbrStyle: { marker: '*' }, children: [item()] });
    expect(adjacentIsSafe(dash, star)).toBe(true);
    expect(adjacentIsSafe(dash, dash)).toBe(false);
    expect(adjacentIsSafe(dash, node('list', { ordered: true, start: 1, children: [item()] }))).toBe(true);
    // `2.` cannot start a list under a paragraph: it would be the item's text.
    expect(adjacentIsSafe(dash, node('list', { ordered: true, start: 2, children: [item()] }))).toBe(false);
  });

  it('may follow a list whose last item is empty, unless it would join the list', () => {
    const list = node('list', { children: [item(), item([])] });
    expect(adjacentIsSafe(list, node('paragraph'))).toBe(true);
    expect(adjacentIsSafe(list, node('list', { children: [item()] }))).toBe(false);
    expect(adjacentIsSafe(node('list', { children: [item()] }), node('paragraph'))).toBe(false);
  });
});

describe('nodes Milkdown serializes without children', () => {
  // Milkdown's serializer leaves `children` off a node it pushed nothing
  // into; the two `roundtrip-check` errors were this TypeError (`length`).
  it('print without throwing', async () => {
    const crepe = await openCrepe('x\n');
    const remark = crepe.editor.action((ctx) => ctx.get(remarkCtx));
    const tree = {
      type: 'root',
      children: [
        { type: 'paragraph' },
        { type: 'list', ordered: false, spread: true, children: [{ type: 'listItem', spread: true }, { type: 'listItem' }] },
        { type: 'list', ordered: false, spread: false, children: [{ type: 'listItem', children: [{ type: 'paragraph' }] }] },
      ],
    };
    expect(() => remark.stringify(tree as never)).not.toThrow();
  });
});
