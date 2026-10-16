/**
 * Only needed escapes: the pure algorithm with a stand-in parser, then the
 * editor's own (every plugin installed), where context decides.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { serializeBody } from '../editor-crepe.js';
import { destroyEditors, openCrepe, placeCursor, typeText, viewOf } from '../editor-test-support.js';
import { MAX_CANDIDATES, authoredEscapes, minimizeEscapes } from './minimal-escapes.js';

afterEach(destroyEditors);

/** A stand-in parse: `*x*` is emphasis, any other escape is just its character. */
const toyParse = (markdown: string): string =>
  markdown.replace(/(?<!\\)\*([^*\\]+)(?<!\\)\*/g, '<em>$1</em>').replace(/\\(.)/g, '$1');

describe('minimizeEscapes', () => {
  it('drops an escape only when the parse is unchanged', () => {
    expect(minimizeEscapes('2 \\* 3 and \\&', toyParse)).toBe('2 * 3 and &');
    // Dropping either would make `*a*` emphasis; dropping both too. One of
    // the pair is enough to keep it text, and right-to-left keeps the first.
    expect(minimizeEscapes('\\*a\\*', toyParse)).toBe('\\*a*');
  });

  it('keeps escapes the author wrote', () => {
    expect(minimizeEscapes('\\*a\\* and \\&', toyParse, undefined, '\\*a\\* and &')).toBe('\\*a\\* and &');
  });

  it('checks an escaped backslash like any other escape', () => {
    expect(minimizeEscapes('a\\\\b', (md) => md)).toBe('a\\\\b');
  });

  it('gives up above the candidate limit', () => {
    const many = '\\&'.repeat(MAX_CANDIDATES + 1);
    expect(minimizeEscapes(many, toyParse)).toBe(many);
  });
});

describe('authoredEscapes', () => {
  it('pairs printed backslashes with the authored ones, despite edits', () => {
    const printed = 'new \\[a] and \\[b]';
    const kept = authoredEscapes(printed, '\\[a] and [b]', [4, 13]);
    expect([...kept]).toEqual([4]);
  });
});

/** Opens `source`, types `text` at the end of `at`, returns what a save writes. */
async function edited(source: string, at: string, text: string): Promise<string> {
  const crepe = await openCrepe(source);
  const view = viewOf(crepe);
  placeCursor(view, at, at.length);
  typeText(view, text);
  return serializeBody(crepe);
}

describe('in the editor', () => {
  it.each([
    ['2 * 3 and AT&T, see [a] here.', 'here', '2 * 3 and AT&T, see [a] here!.'],
    ['\\[[not a wikilink]] and \\*not em\\*', 'em', '\\[[not a wikilink]] and \\*not emX\\*'],
    ['Price \\$5 and \\$6', '6', 'Price \\$5 and \\$6X'],
    ['\\# not a heading', 'heading', '\\# not a headingX'],
    ['2026\\. not a list', 'list', '2026\\. not a listX'],
  ])('an edited paragraph keeps needed escapes and adds no others: %j', async (source, at, expected) => {
    const text = at === 'here' ? '!' : 'X';
    expect(await edited(`${source}\n`, at, text)).toBe(`${expected}\n`);
  });

  it('keeps `[a]` escaped when the document defines `[a]`', async () => {
    expect(await edited('see \\[a] here\n\n[a]: /x\n', 'here', 'X')).toBe('see \\[a] hereX\n\n[a]: /x\n');
  });

  it('keeps `[^1]` escaped when the document defines the footnote', async () => {
    expect(await edited('not \\[^1] a note\n\n[^1]: n\n', 'note', 'X')).toBe('not \\[^1] a noteX\n\n[^1]: n\n');
  });

  it('keeps the box escaped first in a list item, and the tag first in a quote', async () => {
    expect(await edited('- \\[ ] not a task\n', 'task', 'X')).toBe('- \\[ ] not a taskX\n');
    expect(await edited('> \\[!NOTE]\n> not an alert\n', 'alert', 'X')).toBe('> \\[!NOTE]\n> not an alertX\n');
  });

  it('an empty extended task box is text and printed as typed', async () => {
    expect(await edited('- [-]\n- b\n', 'b', 'X')).toBe('- [-]\n- bX\n');
  });

  it('applies to headings', async () => {
    expect(await edited('# 2 * 3 & AT&T\n', 'AT&T', 'X')).toBe('# 2 * 3 & AT&TX\n');
  });
});
