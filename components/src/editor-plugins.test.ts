/**
 * The editor's mbr syntax plugins (`editor-plugins/`): section attributes,
 * heading attributes and wikilinks. The fixture-level guarantees (byte round
 * trip, renderer outline) live in `editor-parity.test.ts`; this file pins the
 * grammar edges against pulldown-cmark's behaviour and the editing paths
 * (typing, the autocomplete's node, the chip edits).
 */
import { afterEach, describe, expect, it } from 'vitest';
import type { Crepe } from '@milkdown/crepe';
import { editorViewCtx } from '@milkdown/kit/core';
import { NodeSelection, TextSelection } from '@milkdown/kit/prose/state';
import type { EditorView } from '@milkdown/kit/prose/view';
import { destroyEditors, editorOutline, openCrepe, savedFile } from './editor-test-support.js';
import { serializeBody } from './editor-crepe.js';
import {
  SECTION_BREAK_NODE,
  keepSectionAttrLines,
  parseSectionLine,
  validateSectionAttrs,
} from './editor-plugins/section-attrs.js';
import { headingContent, parseHeadingAttributes } from './editor-plugins/heading-attrs.js';
import {
  WIKILINK_NODE,
  parseWikilinkSource,
  validateWikilinkSource,
  wikilinkLabel,
  wikilinkNode,
  wikilinkSource,
} from './editor-plugins/wikilinks.js';

afterEach(destroyEditors);

function viewOf(crepe: Crepe): EditorView {
  return crepe.editor.action((ctx) => ctx.get(editorViewCtx));
}

/** Node type names of the document's top-level blocks. */
async function blocks(body: string): Promise<string[]> {
  const crepe = await openCrepe(body);
  const names: string[] = [];
  viewOf(crepe).state.doc.forEach((node) => names.push(node.type.name));
  return names;
}

/** Inline node type names of the first block. */
async function inlines(body: string): Promise<string[]> {
  const crepe = await openCrepe(body);
  const names: string[] = [];
  viewOf(crepe).state.doc.firstChild?.forEach((node) => names.push(node.type.name));
  return names;
}

/** Simulates typing `text` at the caret, so input rules run as they do for a user. */
function type(view: EditorView, text: string): void {
  for (const ch of text) {
    const { from, to } = view.state.selection;
    const handled = view.someProp('handleTextInput', (f) => f(view, from, to, ch, () => view.state.tr));
    if (!handled) view.dispatch(view.state.tr.insertText(ch, from, to));
  }
}

describe('section attributes: the renderer rule', () => {
  it('parses the marker and keeps the attribute block verbatim', () => {
    expect(parseSectionLine('--- {#intro .highlight data-transition="slide"}')).toEqual({
      marker: '---',
      attrs: '{#intro .highlight data-transition="slide"}',
    });
    expect(parseSectionLine('— {.dark}')).toEqual({ marker: '—', attrs: '{.dark}' });
    expect(parseSectionLine('&mdash; {.dark}')).toEqual({ marker: '&mdash;', attrs: '{.dark}' });
    // Even an empty or unparseable block makes a rule (`ParsedAttrs::parse` accepts it).
    expect(parseSectionLine('--- {}')).toEqual({ marker: '---', attrs: '{}' });
  });

  it('rejects what smart punctuation does not turn into `— {…}`', () => {
    for (const line of ['---{x}', '---  {x}', '---\t{x}', '---- {x}', '-- {x}', '--- {x', '--- x}', '\\--- {x}']) {
      expect(parseSectionLine(line), line).toBeNull();
    }
  });

  it('makes a section break only of a paragraph that is the whole line', async () => {
    expect(await blocks('a\n\n--- {#x}\n\nb\n')).toEqual(['paragraph', SECTION_BREAK_NODE, 'paragraph']);
    expect(await blocks('a\n\n---\n\nb\n')).toEqual(['paragraph', 'hr', 'paragraph']);
    // Directly under a paragraph: a continuation line, to both parsers.
    expect(await blocks('a\n--- {#x}\n\nb\n')).toEqual(['paragraph', 'paragraph']);
    // Inline markup splits the renderer's text run, so it is not a rule there either.
    expect(await blocks('--- {a *b*}\n')).toEqual(['paragraph']);
    expect(await blocks('\\--- {x}\n')).toEqual(['paragraph']);
  });

  it('counts a GFM autolink literal as text, as pulldown-cmark (no literal autolinks) does', async () => {
    const body = '--- {data-background-iframe=https://example.com/x}\n';
    expect(await blocks(body)).toEqual([SECTION_BREAK_NODE]);
    expect(await savedFile(body)).toBe(body);
  });

  it('round-trips every spelling and position byte for byte', async () => {
    for (const body of [
      '--- {.a}\n',
      '— {.a}\n',
      '&mdash; {.a}\n',
      '> quoted\n>\n> --- {.inside}\n',
      'para\n--- {.cont}\nmore\n',
      '* item\n  --- {.lazy}\n',
    ]) {
      expect(await savedFile(body), body).toBe(body);
    }
  });

  it('outlines a section break inside a blockquote, like the renderer', async () => {
    const outline = await editorOutline('> a\n>\n> --- {.x}\n');
    expect(outline.map((n) => n.kind)).toEqual(['blockquote', 'paragraph', 'section_break']);
  });

  it('only unescapes a line remark escaped that is exactly a section line', () => {
    const escape = (node: { value: string }): string => node.value.replace(/^-/gm, '\\-');
    const keep = keepSectionAttrLines(escape);
    expect(keep({ value: 'a\n--- {.x}' }, null, null, null)).toBe('a\n--- {.x}');
    expect(keep({ value: 'a\n- item' }, null, null, null)).toBe('a\n\\- item');
    expect(keep({ value: 'a\n--- x' }, null, null, null)).toBe('a\n\\--- x');
  });

  it('validates an edited block; empty means a plain rule', () => {
    expect(validateSectionAttrs('{#a .b}')).toBeNull();
    expect(validateSectionAttrs('  ')).toBeNull();
    expect(validateSectionAttrs('#a .b')).not.toBeNull();
  });

  it('saves edited attributes, and a plain rule when they are removed', async () => {
    const crepe = await openCrepe('a\n\n--- {#x}\n\nb\n');
    const view = viewOf(crepe);
    let pos = -1;
    view.state.doc.forEach((node, offset) => {
      if (node.type.name === SECTION_BREAK_NODE) pos = offset;
    });
    view.dispatch(view.state.tr.setNodeMarkup(pos, undefined, { marker: '---', attrs: '{#y .z}' }));
    expect(serializeBody(crepe)).toBe('a\n\n--- {#y .z}\n\nb\n');
    view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, pos)));
    view.dispatch(view.state.tr.replaceSelectionWith(view.state.schema.nodes.hr.create()));
    expect(serializeBody(crepe)).toBe('a\n\n***\n\nb\n');
  });
});

describe('heading attributes: pulldown-cmark rules', () => {
  it('reads the trailing block', () => {
    expect(parseHeadingAttributes('Title {#custom-id}')).toEqual({ id: 'custom-id', classes: [] });
    expect(parseHeadingAttributes('Both {#a .b .c #d}')).toEqual({ id: 'd', classes: ['b', 'c'] });
    expect(parseHeadingAttributes('Pairs {key=v # . .ok}  ')).toEqual({ classes: ['ok'] });
    expect(parseHeadingAttributes('Empty {}')).toEqual({ classes: [] });
    // The nearest `{` opens the block, as in pulldown-cmark.
    expect(parseHeadingAttributes('Nest {#a {.b}')).toEqual({ classes: ['b'] });
  });

  it('refuses a block that is not trailing or holds a forbidden byte', () => {
    for (const text of ['No block', '{#a} trailing', 'Esc {#a\\b}', 'Tag {#<i>}', 'Open {#a b', '}']) {
      expect(parseHeadingAttributes(text), text).toBeNull();
    }
  });

  it('looks in the ATX line or the setext content', () => {
    expect(headingContent('## Title {#x}')).toBe('Title {#x}');
    expect(headingContent('#')).toBe('');
    expect(headingContent('Title {#x}\n===')).toBe('Title {#x}');
    expect(headingContent('Two\nlines {#x}\n---')).toBe('Two\nlines {#x}');
  });

  it('puts them on the outline and leaves the text verbatim', async () => {
    const body = 'Setext {#s .t}\n===\n\n## ATX {.wide}\n';
    expect(await editorOutline(body)).toEqual([
      { depth: 0, kind: 'heading', lines: [1, 2], level: 1, id: 's', classes: ['t'] },
      { depth: 0, kind: 'heading', lines: [4, 4], level: 2, classes: ['wide'] },
    ]);
  });
});

describe('wikilinks: the renderer grammar', () => {
  it('splits target, heading and alias losslessly', () => {
    const cases: [string, ReturnType<typeof parseWikilinkSource>][] = [
      ['[[Note]]', { embed: false, target: 'Note', heading: null, alias: null }],
      ['[[Note|the alias]]', { embed: false, target: 'Note', heading: null, alias: 'the alias' }],
      ['[[Note#Some Heading]]', { embed: false, target: 'Note', heading: 'Some Heading', alias: null }],
      ['[[#Local]]', { embed: false, target: '', heading: 'Local', alias: null }],
      ['![[diagram.png]]', { embed: true, target: 'diagram.png', heading: null, alias: null }],
      ['[[a#b|c|d]]', { embed: false, target: 'a', heading: 'b', alias: 'c|d' }],
      ['[[Tags:rust]]', { embed: false, target: 'Tags:rust', heading: null, alias: null }],
      ['[[a\\|b]]', { embed: false, target: 'a\\', heading: null, alias: 'b' }],
      ['[[x|]]', { embed: false, target: 'x', heading: null, alias: '' }],
      ['[[a]b]]', { embed: false, target: 'a]b', heading: null, alias: null }],
    ];
    for (const [source, parsed] of cases) {
      expect(parseWikilinkSource(source), source).toEqual(parsed);
      expect(wikilinkSource(parsed!), source).toBe(source);
    }
  });

  it('refuses what pulldown-cmark does not link (or what we keep as text)', () => {
    for (const source of ['[[]]', '[[|x]]', '[[a\nb]]', '[[a[b]]', '[[a]]]', '[x]', '[[x]', '!![[x]]']) {
      expect(parseWikilinkSource(source), source).toBeNull();
    }
  });

  it('labels a chip with the alias, else target and heading', () => {
    expect(wikilinkLabel(parseWikilinkSource('[[N|A]]')!)).toBe('A');
    expect(wikilinkLabel(parseWikilinkSource('[[N#H]]')!)).toBe('N › H');
    expect(wikilinkLabel(parseWikilinkSource('[[#H]]')!)).toBe('#H');
    expect(wikilinkLabel(parseWikilinkSource('[[N|]]')!)).toBe('N');
  });

  it('parses each form into one atom node', async () => {
    expect(await inlines('See [[A]], [[B|b]], [[C#h]] and ![[d.png]].')).toEqual([
      'text',
      WIKILINK_NODE,
      'text',
      WIKILINK_NODE,
      'text',
      WIKILINK_NODE,
      'text',
      WIKILINK_NODE,
      'text',
    ]);
  });

  it('round-trips byte for byte, without any post-save string patch', async () => {
    for (const body of [
      'See [[John Doe]] and [[Jane]].\n',
      'Alias [[Name|the *alias*]] here.\n',
      'Hash [[Name#Sec]] and [[#Local]].\n',
      'Odd [[x|]], [[a]b]] and [[a]]] stay as written.\n',
      '![[img.png]]\n',
      'Wow\\![[x]] is not an embed.\n',
      '`[[code]]` stays code.\n',
      '# Heading with [[Link]]\n',
      '| a        |\n| -------- |\n| [[Cell]] |\n',
    ]) {
      expect(await savedFile(body), body).toBe(body);
    }
  });

  it('leaves code alone', async () => {
    expect(await inlines('`[[code]]`')).toEqual(['text']);
    expect(await blocks('```\n[[x]]\n```\n')).toEqual(['code_block']);
  });

  it('typing the closing `]]` makes the node', async () => {
    const crepe = await openCrepe('Start\n');
    const view = viewOf(crepe);
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 6)));
    type(view, ' [[New Note|shown]]');
    const names: string[] = [];
    view.state.doc.firstChild?.forEach((node) => names.push(node.type.name));
    expect(names).toEqual(['text', WIKILINK_NODE]);
    expect(serializeBody(crepe)).toBe('Start [[New Note|shown]]\n');
  });

  it('builds the node the autocomplete inserts, and saves it verbatim', async () => {
    const crepe = await openCrepe('Link to here\n');
    const view = viewOf(crepe);
    const node = wikilinkNode(view.state.schema, '[[C# Notes]]');
    expect(node?.type.name).toBe(WIKILINK_NODE);
    expect(wikilinkNode(view.state.schema, '[[a[b]]')).toBeNull();
    view.dispatch(view.state.tr.insert(1 + 'Link to '.length, node!));
    expect(serializeBody(crepe)).toBe('Link to [[C# Notes]]here\n');
  });

  it('validates an edited source; empty removes the link', () => {
    expect(validateWikilinkSource('[[x]]')).toBeNull();
    expect(validateWikilinkSource('')).toBeNull();
    expect(validateWikilinkSource('x')).not.toBeNull();
  });
});
