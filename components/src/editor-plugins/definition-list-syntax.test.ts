/**
 * The remark half of definition lists, without Milkdown: what the tokenizer
 * and the term transform build, and what the printer writes. Structural
 * agreement with the renderer is the parity harness's job
 * (`editor-parity.test.ts`, `deflist-edge` and the flashcard fixtures); these
 * pin the mdast contract the Milkdown nodes and the outline read, and the
 * pieces a fixture cannot isolate.
 */
import { describe, expect, it } from 'vitest';
import { unified } from 'unified';
import remarkParse from 'remark-parse';
import remarkGfm from 'remark-gfm';
import remarkStringify from 'remark-stringify';
import type { Nodes, Root } from 'mdast';
import { escapeContinuationColons, remarkDefinitionList } from './definition-list-syntax.js';

const processor = unified().use(remarkParse).use(remarkGfm).use(remarkDefinitionList).use(remarkStringify, {
  bullet: '*',
});

function parse(markdown: string): Root {
  return processor.runSync(processor.parse(markdown)) as Root;
}

/** The tree as nested `type` names (`type[child, …]`), positions dropped. */
function shape(node: Nodes): string {
  const leaves = ['paragraph', 'heading', 'tableCell', 'defListTerm'];
  if (!('children' in node) || leaves.includes(node.type)) return node.type;
  return `${node.type}[${(node.children as Nodes[]).map(shape).join(', ')}]`;
}

const shapeOf = (markdown: string): string => shape(parse(markdown));

describe('definition list syntax: where a definition starts', () => {
  it('turns the paragraph before `:` into the term, with no blank line needed', () => {
    expect(shapeOf('Term\n: definition\n')).toBe('root[defList[defListTerm, defListDescription[paragraph]]]');
  });

  it('keeps a multi-line paragraph as one term, as the renderer does', () => {
    const tree = parse('A. one\nB. two\nC. three\n: answer\n');
    const list = tree.children[0];
    expect(list.type).toBe('defList');
    if (list.type !== 'defList') return;
    expect(list.children.map((c) => c.type)).toEqual(['defListTerm', 'defListDescription']);
    expect(list.children[0].position?.start.line).toBe(1);
    expect(list.children[0].position?.end.line).toBe(3);
  });

  it('accepts blank lines between the term and its definition, and makes the list loose', () => {
    const tree = parse('Term\n\n\n: definition\n');
    const list = tree.children[0];
    expect(list.type === 'defList' && list.spread).toBe(true);
    expect(list.type === 'defList' && list.children[1].type === 'defListDescription' && list.children[1].gapBefore).toBe(2);
  });

  it('is plain text after a heading, a list, or at the start of the document', () => {
    expect(shapeOf('# H\n: x\n')).toBe('root[heading, paragraph]');
    expect(shapeOf('* a\n\n: x\n')).toBe('root[list[listItem[paragraph]], paragraph]');
    expect(shapeOf(': x\n')).toBe('root[paragraph]');
  });

  it('is a lazy continuation, not a definition, when a container did not continue', () => {
    expect(shapeOf('* item\n: x\n')).toBe('root[list[listItem[paragraph]]]');
    expect(shapeOf('> quote\n: x\n')).toBe('root[blockquote[paragraph]]');
  });

  it('does not let a new container\'s first line be a definition', () => {
    expect(shapeOf('Term\n* : x\n')).toBe('root[paragraph, list[listItem[paragraph]]]');
  });

  it('needs no space after the colon', () => {
    expect(shapeOf('Term\n:x\n')).toBe('root[defList[defListTerm, defListDescription[paragraph]]]');
  });

  it('builds lists in items, quotes and definitions', () => {
    expect(shapeOf('* Term\n  : def\n')).toBe('root[list[listItem[defList[defListTerm, defListDescription[paragraph]]]]]');
    expect(shapeOf('> Term\n> : def\n')).toBe('root[blockquote[defList[defListTerm, defListDescription[paragraph]]]]');
    expect(shapeOf('T\n: outer\n  inner\n  : def\n')).toBe(
      'root[defList[defListTerm, defListDescription[defList[defListTerm, defListDescription[paragraph]]]]]',
    );
  });

  it('is a table row after a table', () => {
    expect(shapeOf('| a |\n| - |\n| 1 |\n: x\n')).toBe('root[table[tableRow[tableCell], tableRow[tableCell], tableRow[tableCell]]]');
  });
});

describe('definition list syntax: where a definition ends', () => {
  it('continues one list across terms separated by blank lines', () => {
    const tree = parse('Q1\n: a\n\nQ2\n: b\n');
    expect(shape(tree)).toBe('root[defList[defListTerm, defListDescription[paragraph], defListTerm, defListDescription[paragraph]]]');
    // A blank line before a term does not loosen the list.
    expect(tree.children[0].type === 'defList' && tree.children[0].spread).toBe(false);
  });

  it('keeps an indented fence inside its definition instead of swallowing the document', () => {
    const tree = parse('Q\n: ```rust\n  let x = 1;\n  ```\n\nAfter\n');
    expect(shape(tree)).toBe('root[defList[defListTerm, defListDescription[code]], paragraph]');
    const list = tree.children[0];
    if (list.type !== 'defList') throw new Error('expected a list');
    const code = list.children[1].type === 'defListDescription' ? list.children[1].children[0] : undefined;
    expect(code?.type === 'code' && code.value).toBe('let x = 1;');
  });

  it('ends nested list items on their own lines', () => {
    const tree = parse('Q\n: aids:\n  * one\n  * two\n: next\n');
    const list = tree.children[0];
    if (list.type !== 'defList') throw new Error('expected a list');
    const first = list.children[1];
    if (first.type !== 'defListDescription') throw new Error('expected a definition');
    const nested = first.children[1];
    if (nested.type !== 'list') throw new Error('expected a nested list');
    expect(nested.children.map((item) => [item.position?.start.line, item.position?.end.line])).toEqual([
      [3, 3],
      [4, 4],
    ]);
    expect(list.children).toHaveLength(3);
  });

  it('takes a lazy line into the definition and the next `:` as another definition of the same term', () => {
    expect(shapeOf('T\n: a\nlazy\n: b\n')).toBe('root[defList[defListTerm, defListDescription[paragraph], defListDescription[paragraph]]]');
  });

  it('records blank lines between blocks of one definition as `spread`', () => {
    const tree = parse('T\n: a\n\n  b\n');
    const list = tree.children[0];
    if (list.type !== 'defList') throw new Error('expected a list');
    expect(list.spread).toBe(true);
    expect(list.children[1].type === 'defListDescription' && list.children[1].spread).toBe(true);
  });
});

describe('definition list syntax: printing', () => {
  const roundTrip = (markdown: string): string => processor.stringify(parse(markdown));

  it.each([
    ['tight', 'Term\n: one\n: two\n'],
    ['loose', 'Term\n\n: one\n\n: two\n'],
    ['mixed gaps', 'Term\n: one\n\n\n: two\n'],
    ['two terms', 'Q1\n: a\n\nQ2\n: b\n'],
    // (`___…___` is Milkdown's marker preservation, not this printer's: see the parity fixtures.)
    ['nested list', 'Q\n: aids:\n  * one\n  * two\n: Review History\n  * 2026-10-07 14:53 - Good\n'],
    ['loose definition', 'Q\n: para\n\n  second\n'],
    ['fence', 'Q\n: ```rust\n  let x = 1;\n  ```\n'],
    ['empty definition', 'Q\n:\n'],
  ])('prints %s in house style, unchanged', (_name, markdown) => {
    expect(roundTrip(markdown)).toBe(markdown);
  });

  it('writes `: ` and a two-space continuation indent', () => {
    expect(roundTrip('Term\n:    wide\n     continued\n')).toBe('Term\n: wide\n  continued\n');
  });

  it('escapes a `:` that would open a definition, but not at a paragraph start', () => {
    expect(roundTrip('para\n\\: not a definition\n')).toBe('para\n\\: not a definition\n');
    expect(roundTrip('# H\n\n:::note\n')).toBe('# H\n\n:::note\n');
  });
});

describe('escapeContinuationColons', () => {
  it('escapes `:` at the start of every line but the first', () => {
    expect(escapeContinuationColons(':a\n:b\n  :c')).toBe(':a\n\\:b\n  \\:c');
    expect(escapeContinuationColons('a\r\n: b')).toBe('a\r\n\\: b');
    expect(escapeContinuationColons('a: b\nc :d')).toBe('a: b\nc :d');
  });
});
