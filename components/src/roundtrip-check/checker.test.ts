/**
 * `RoundTripChecker` reuses one editor for every note; that must give exactly
 * what a freshly opened editor gives — the editor's own `savedFile` and
 * `createBodySaver` — on every fixture, whatever was loaded before.
 */
import { afterAll, describe, expect, it } from 'vitest';
import { createBodySaver } from '../editor-crepe.js';
import { frontmatterRewrites, openDocument } from '../editor-frontmatter.js';
import { EDITOR_FIXTURES, destroyEditors, openCrepe, savedFile } from '../editor-test-support.js';
import { constructGuess, RoundTripChecker, touchedLines } from './checker.js';

/** A note in the editor's own spelling: unchanged by an untouched save. */
const CLEAN = '# Title\n\nSome *text*.\n\n* one\n* two\n';
/**
 * Rewritten by design, not for want of a plugin: a tab after a quote's `>`
 * is part of a nested item's indentation, printed as spaces (`>   * other`). The blank
 * line ending the frontmatter is kept (an unedited frontmatter is saved as
 * loaded); editing the frontmatter would drop it.
 */
const LOSSY = '---\ntitle: t\n\n---\n# Heading\n\n> * item\n>\t* other\n';

describe('RoundTripChecker', () => {
  const checker = new RoundTripChecker();
  afterAll(() => checker.dispose());

  it('reports a clean note unchanged and a lossy one changed, with the notice in file lines', async () => {
    const clean = await checker.check(CLEAN);
    expect(clean.changed).toBe(false);
    expect(clean.notice).toEqual({ ifEdited: [], onSave: [], frontmatter: [] });

    const lossy = await checker.check(LOSSY);
    expect(lossy.changed).toBe(true);
    expect(lossy.saved).toBe('---\ntitle: t\n\n---\n# Heading\n\n> * item\n>   * other\n');
    // The body's unedited blocks are kept, and so is the unedited frontmatter.
    expect(lossy.untouchedSave).toBe(LOSSY);
    expect(lossy.notice).toEqual({
      ifEdited: [{ first: 7, last: 8 }],
      onSave: [],
      frontmatter: ['blank lines at its end are dropped'],
    });
    expect(constructGuess(LOSSY, lossy)).toBe('blockquote (whitespace only)');
  });

  it('keeps CRLF and names a lost byte-order mark', async () => {
    const crlf = await checker.check(CLEAN.replace(/\n/g, '\r\n'));
    expect(crlf.changed).toBe(false);
    const bom = `\uFEFF${CLEAN}`;
    const trip = await checker.check(CLEAN);
    expect(constructGuess(bom, trip)).toContain('byte-order mark');
  });

  it('gives what a fresh editor gives, on every fixture', async () => {
    // Reverse order, so each note is loaded over a different one.
    for (const [name, { source }] of Object.entries(EDITOR_FIXTURES).reverse()) {
      const trip = await checker.check(source);
      const doc = openDocument(source);
      const fresh = await openCrepe(doc.body);
      const saver = createBodySaver(fresh, doc.body);
      const shift = (ranges: { first: number; last: number }[]) =>
        ranges.map((r) => ({ first: r.first + doc.bodyLineOffset, last: r.last + doc.bodyLineOffset }));
      expect(trip.saved, name).toBe(await savedFile(source));
      expect(trip.notice, name).toEqual({
        ifEdited: shift(saver.rewritten.ifEdited),
        onSave: shift(saver.rewritten.onSave),
        frontmatter: frontmatterRewrites(doc),
      });
      // `destroyEditors` would take the checker's editor too; drop only the fresh ones.
      await fresh.destroy();
    }
  }, 300_000);
});

describe('touchedLines', () => {
  it('names removed lines and both neighbours of an insertion', () => {
    expect(touchedLines('a\nb\nc\n', 'a\nB\nc\n')).toEqual([2]);
    expect(touchedLines('a\nb\n', 'a\n\nb\n')).toEqual([1, 2]);
    expect(touchedLines('a\nb\n', 'a\nB\nC\n')).toEqual([2]);
  });
});

afterAll(destroyEditors);
