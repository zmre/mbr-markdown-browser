/**
 * Editing definition lists in the real Crepe (the editor's own configuration,
 * `editor-test-support.ts`): the keys, the `: ` input rule, the slash-menu
 * entry, and what each saves. Round-trip and structure of untouched decks are
 * the parity harness's (`editor-parity.test.ts`).
 */
import { afterEach, describe, expect, it } from 'vitest';
import { serializeBody } from '../editor-crepe.js';
import { destroyEditors, openCrepe, placeCursor, placeInEmpty, press, typeText, viewOf } from '../editor-test-support.js';
import { addDefinitionListMenuItem } from './definition-list.js';

afterEach(destroyEditors);

describe('definition list editing: Enter', () => {
  it('in a term starts its first definition, carrying the text after the cursor', async () => {
    const fresh = await openCrepe('Question and answer\n: first\n');
    const v = viewOf(fresh);
    placeCursor(v, 'and answer');
    expect(press(v, 'Enter')).toBe(true);
    expect(serializeBody(fresh)).toBe('Question \n: and answer\n: first\n');
  });

  it('in a definition starts the next definition', async () => {
    const crepe = await openCrepe('Term\n: one\n');
    const view = viewOf(crepe);
    placeCursor(view, 'one', 3);
    expect(press(view, 'Enter')).toBe(true);
    typeText(view, 'two');
    expect(serializeBody(crepe)).toBe('Term\n: one\n: two\n');
  });

  it('copies the list\'s looseness to a new definition', async () => {
    const crepe = await openCrepe('Term\n\n: one\n');
    const view = viewOf(crepe);
    placeCursor(view, 'one', 3);
    press(view, 'Enter');
    typeText(view, 'two');
    expect(serializeBody(crepe)).toBe('Term\n\n: one\n\n: two\n');
  });

  it('on an empty last definition leaves the list', async () => {
    const crepe = await openCrepe('Term\n: one\n');
    const view = viewOf(crepe);
    placeCursor(view, 'one', 3);
    press(view, 'Enter');
    press(view, 'Enter');
    typeText(view, 'After');
    expect(serializeBody(crepe)).toBe('Term\n: one\n\nAfter\n');
  });

  it('on an empty definition in the middle turns it into the next term', async () => {
    const crepe = await openCrepe('Q1\n: a\n:\n: b\n');
    const view = viewOf(crepe);
    placeInEmpty(view, 'paragraph');
    press(view, 'Enter');
    typeText(view, 'Q2');
    expect(serializeBody(crepe)).toBe('Q1\n: a\n\nQ2\n: b\n');
  });

  it('on an empty last term leaves the list', async () => {
    const crepe = await openCrepe('Q1\n: a\n');
    const view = viewOf(crepe);
    const list = view.state.doc.firstChild;
    if (!list) throw new Error('no list');
    view.dispatch(view.state.tr.insert(list.nodeSize - 1, view.state.schema.nodes.definition_term.create()));
    placeInEmpty(view, 'definition_term');
    expect(press(view, 'Enter')).toBe(true);
    typeText(view, 'After');
    expect(serializeBody(crepe)).toBe('Q1\n: a\n\nAfter\n');
  });

  it('leaves Enter in a list inside a definition to the list', async () => {
    const crepe = await openCrepe('Q\n: aids:\n  * one\n');
    const view = viewOf(crepe);
    placeCursor(view, 'one', 3);
    press(view, 'Enter');
    typeText(view, 'two');
    expect(serializeBody(crepe)).toBe('Q\n: aids:\n  * one\n  * two\n');
  });
});

describe('definition list printing in the editor', () => {
  it('leaves `:` lines inside an HTML block alone', async () => {
    const source = '<div>\n:not a definition\n</div>\n';
    const crepe = await openCrepe(source);
    expect(serializeBody(crepe)).toBe(source);
  });

  it('escapes a literal `:` line inside a paragraph so it stays text', async () => {
    const crepe = await openCrepe('para\n\\: literal\n');
    expect(serializeBody(crepe)).toBe('para\n\\: literal\n');
  });
});

describe('definition list editing: Backspace at the start of a definition', () => {
  it('merges it into the definition above', async () => {
    const crepe = await openCrepe('Term\n: one\n: two\n');
    const view = viewOf(crepe);
    placeCursor(view, 'two');
    expect(press(view, 'Backspace')).toBe(true);
    expect(serializeBody(crepe)).toBe('Term\n: onetwo\n');
    expect(view.state.selection.from).toBe(view.state.selection.to);
  });

  it('merges the first definition into the term, undoing Enter', async () => {
    const crepe = await openCrepe('Question\n: and answer\n: first\n');
    const view = viewOf(crepe);
    placeCursor(view, 'and answer');
    press(view, 'Backspace');
    expect(serializeBody(crepe)).toBe('Questionand answer\n: first\n');
  });

  it('keeps the rest of a definition whose first paragraph merged away', async () => {
    const crepe = await openCrepe('Term\n: one\n: two\n  * x\n');
    const view = viewOf(crepe);
    placeCursor(view, 'two');
    press(view, 'Backspace');
    expect(serializeBody(crepe)).toBe('Term\n: onetwo\n: * x\n');
  });

  it('leaves Backspace away from a definition\'s start to the browser', async () => {
    const crepe = await openCrepe('Term\n: one\n');
    const view = viewOf(crepe);
    placeCursor(view, 'one', 1);
    expect(press(view, 'Backspace')).toBe(false);
    expect(serializeBody(crepe)).toBe('Term\n: one\n');
  });
});

describe('definition list editing: creating one', () => {
  it('typing `: ` after a paragraph makes it the term', async () => {
    const crepe = await openCrepe('Question\n\nAnswer\n');
    const view = viewOf(crepe);
    placeCursor(view, 'Answer');
    typeText(view, ': ');
    expect(serializeBody(crepe)).toBe('Question\n: Answer\n');
  });

  it('typing `: ` after a definition list adds a definition', async () => {
    const crepe = await openCrepe('Q\n: a\n\nmore\n');
    const view = viewOf(crepe);
    placeCursor(view, 'more');
    typeText(view, ': ');
    expect(serializeBody(crepe)).toBe('Q\n: a\n: more\n');
  });

  it('typing `: ` after a heading is just text', async () => {
    const crepe = await openCrepe('# H\n\nx\n');
    const view = viewOf(crepe);
    placeCursor(view, 'x');
    typeText(view, ': ');
    expect(serializeBody(crepe)).toBe('# H\n\n: x\n');
  });

  it('the slash menu has a Definition list entry that inserts an empty term and definition', async () => {
    const items: { key: string; label: string; onRun?: (ctx: never) => void }[] = [];
    addDefinitionListMenuItem({
      getGroup: (key) => {
        expect(key).toBe('list');
        return { addItem: (itemKey, item) => items.push({ key: itemKey, ...item }) };
      },
    });
    expect(items.map((item) => item.label)).toEqual(['Definition list']);

    const crepe = await openCrepe('Intro\n\n/def\n');
    const view = viewOf(crepe);
    placeCursor(view, '/def', 4);
    crepe.editor.action((ctx) => items[0].onRun?.(ctx as never));
    typeText(view, 'Term');
    press(view, 'Enter');
    typeText(view, 'Meaning');
    expect(serializeBody(crepe)).toBe('Intro\n\nTerm\n: Meaning\n');
  });
});
