/**
 * Every plugin's Enter and Backspace with **all** of them installed (the
 * editor's own configuration, `openCrepe`), where their nodes nest in one
 * another. Milkdown chains every keymap bound to a key by priority, so plugins
 * compose — but one `$useKeymap` call keeps a single handler per key *name*,
 * and a handler that claims a key in someone else's node wins silently. These
 * pin who owns what: definition lists (100) above task items (60) above the
 * list's own Enter (50); the alert's Backspace is a `handleKeyDown` prop.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { serializeBody } from '../editor-crepe.js';
import { destroyEditors, openCrepe, placeCursor, press, typeText, viewOf } from '../editor-test-support.js';

afterEach(destroyEditors);

/** Opens `source`, puts the cursor `offset` into `at`, presses `key`, types `text`; returns what a save writes. */
async function afterKey(source: string, at: string, offset: number, key: string, text = ''): Promise<string> {
  const crepe = await openCrepe(source);
  const view = viewOf(crepe);
  placeCursor(view, at, offset);
  expect(press(view, key), `${key} handled`).toBe(true);
  if (text) typeText(view, text);
  return serializeBody(crepe);
}

describe('Enter, every plugin installed', () => {
  it.each([
    ['* [-] cancelled\n', 'cancelled', 9, '* [-] cancelled\n* [ ] next\n'],
    ['* [>] deferred\n', 'deferred', 8, '* [>] deferred\n* [ ] next\n'],
    ['* [x] done\n', 'done', 4, '* [x] done\n* [ ] next\n'],
    ['* [X] done\n', 'done', 2, '* [X] do\n* [ ] nextne\n'],
  ])('in a task item starts an open task: %j', async (source, at, offset, expected) => {
    expect(await afterKey(source, at, offset, 'Enter', 'next')).toBe(expected);
  });

  it('at the start of a task item inserts the open task above and keeps the status', async () => {
    // `<br />` is how Milkdown prints any empty paragraph.
    expect(await afterKey('* [-] cancelled\n', 'cancelled', 0, 'Enter', 'new')).toBe(
      '* [ ] <br />\n* [-] newcancelled\n',
    );
  });

  it('in a plain list item is still the list\'s own Enter', async () => {
    expect(await afterKey('* one\n', 'one', 3, 'Enter', 'two')).toBe('* one\n* two\n');
  });

  it('in a definition inside a task item starts the next definition', async () => {
    expect(await afterKey('* [ ] Term\n  : def\n', 'def', 3, 'Enter', 'two')).toBe('* [ ] Term\n  : def\n  : two\n');
  });

  it('in a term inside a task item starts its first definition', async () => {
    expect(await afterKey('* [ ] Term\n  : def\n', 'Term', 4, 'Enter', 'x')).toBe('* [ ] Term\n  : x\n  : def\n');
  });

  it('in a definition inside an alert starts the next definition', async () => {
    expect(await afterKey('> [!NOTE]\n> Term\n> : def\n', 'def', 3, 'Enter', 'two')).toBe(
      '> [!NOTE]\n> Term\n> : def\n> : two\n',
    );
  });

  it('after a wikilink and math in a definition starts the next definition', async () => {
    expect(await afterKey('Term\n: see [[Note|n]] $x$\n', 'see', 0, 'Enter')).toBe('Term\n:\n: see [[Note|n]] $x$\n');
  });
});

describe('Backspace, every plugin installed', () => {
  it('at the start of an alert drops the alert', async () => {
    expect(await afterKey('> [!NOTE]\n> Body.\n', 'Body', 0, 'Backspace')).toBe('> Body.\n');
  });

  it('at the start of a term opening an alert drops the alert, keeping the list', async () => {
    expect(await afterKey('> [!NOTE]\n> Term\n> : def\n', 'Term', 0, 'Backspace')).toBe('> Term\n> : def\n');
  });

  it('at the start of a definition inside an alert merges it into the term', async () => {
    expect(await afterKey('> [!NOTE]\n> Term\n> : def\n', 'def', 0, 'Backspace')).toBe('> [!NOTE]\n> Termdef\n');
  });

  it('at the start of a definition inside a task item merges it into the term', async () => {
    expect(await afterKey('* [ ] Term\n  : def\n', 'def', 0, 'Backspace')).toBe('* [ ] Termdef\n');
  });

  it('at the start of a task item is still the list\'s own Backspace (lifts it out)', async () => {
    expect(await afterKey('* [>] later\n', 'later', 0, 'Backspace')).toBe('later\n');
  });
});
