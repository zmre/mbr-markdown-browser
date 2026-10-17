/// <reference types="node" />
/**
 * `runRoundTripCheck` in process: notes share causes (shapes of changed
 * lines), a note is minimized only while one of its causes lacks samples,
 * and a note over the size cap or past the time budget is not.
 */
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterAll, expect, it } from 'vitest';
import { RoundTripChecker, type RoundTrip } from './checker.js';
import { ANON_OUT_OF_TIME, ANON_REPRODUCES, ANON_SEE, ANON_TOO_LARGE, runRoundTripCheck } from './runner.js';

/**
 * The real editor, plus a rewrite of its own that does not depend on what the
 * plugins keep today: tab-indented lines are saved with two spaces, and
 * `~~~` fences as backticks (rewrites the editor once made).
 */
class RewritingChecker extends RoundTripChecker {
  override async check(source: string): Promise<RoundTrip> {
    const trip = await super.check(source);
    const saved = trip.saved.replace(/^\t/gm, '  ').replace(/^~~~/gm, '```');
    return { ...trip, saved, changed: saved !== source };
  }
}

const temp = mkdtempSync(join(tmpdir(), 'mbr-roundtrip-runner-'));
afterAll(() => rmSync(temp, { recursive: true, force: true }));

function notesFolder(name: string): string {
  const notes = join(temp, name);
  mkdirSync(notes, { recursive: true });
  // Four notes with a tab-indented line (`-⇥…` → `+␣␣…`), d.md with a `~~~`
  // fence. 0-big.md comes first and is over the size cap below.
  writeFileSync(join(notes, '0-big.md'), `${'Filler paragraph.\n\n'.repeat(400)}* item\n\t* nested\n`);
  writeFileSync(join(notes, 'a.md'), '* one\n\t* two\n');
  writeFileSync(join(notes, 'b.md'), 'Some text.\n\n* other words\n\t* here\n');
  writeFileSync(join(notes, 'c.md'), '* x\n\t* y\n');
  writeFileSync(join(notes, 'd.md'), '~~~\nlet x = 1\n~~~\n');
  return notes;
}

it('minimizes a few notes per cause and points the rest at them', async () => {
  const out = join(temp, 'report');
  mkdirSync(out);
  const summary = await runRoundTripCheck({
    root: notesFolder('notes'),
    out,
    minimizeMs: 2000,
    samplesPerCause: 1,
    maxReproBytes: 4000,
    checker: new RewritingChecker(),
  });
  expect(summary).toMatchObject({ files: 5, wouldChange: 5, causes: 4 });
  const index = readFileSync(join(out, 'index.md'), 'utf8');
  expect(index).toMatch(/\| S00[12] \| 4 \| `-⇥\* ·` \| 0002 \|/);
  expect(index).toContain(`| 0001 | 0-big.md |`);
  expect(index).toContain(`| ${ANON_TOO_LARGE} | S001 S002 |`);
  expect(index).toMatch(new RegExp(`\\| 0002 \\| a\\.md \\| \\d+ \\| [^|]+ \\| ${ANON_REPRODUCES} \\| S001 S002 \\|`));
  expect(index).toContain(`| ${ANON_SEE} 0002 | S001 S002 |`);
  expect(index).toMatch(new RegExp(`\\| 0005 \\| d\\.md \\| \\d+ \\| [^|]+ \\| ${ANON_REPRODUCES} \\| S00[34] S00[34] \\|`));
  expect(existsSync(join(out, '0001', 'repro.md'))).toBe(false);
  expect(existsSync(join(out, '0002', 'repro.anon.md'))).toBe(true);
  expect(existsSync(join(out, '0003', 'repro.md'))).toBe(false);
  expect(existsSync(join(out, '0003', 'diff.patch'))).toBe(true);
}, 60_000);
it('stops minimizing when the total budget is spent', async () => {
  const out = join(temp, 'report-no-time');
  mkdirSync(out);
  await runRoundTripCheck({
    root: notesFolder('notes2'),
    out,
    minimizeMs: 2000,
    minimizeTotalMs: 0,
    checker: new RewritingChecker(),
  });
  const index = readFileSync(join(out, 'index.md'), 'utf8');
  expect(index).toContain(`| 0002 | a.md |`);
  expect(index.split(ANON_OUT_OF_TIME).length - 1).toBe(5);
  expect(existsSync(join(out, '0002', 'repro.md'))).toBe(false);
}, 60_000);

/** {@link RewritingChecker}, plus: a note holding `MARK` is one the safety net cannot keep. */
class UnkeepableChecker extends RewritingChecker {
  override async check(source: string): Promise<RoundTrip> {
    const trip = await super.check(source);
    if (!source.includes('MARK')) return trip;
    return { ...trip, notice: { ...trip.notice, onSave: [{ first: 1, last: 1 }] } };
  }
}

it('always minimizes a note any save rewrites, and marks it in the index', async () => {
  const notes = join(temp, 'notes-unkeepable');
  mkdirSync(notes, { recursive: true });
  writeFileSync(join(notes, 'a.md'), '* one\n\t* two\n');
  writeFileSync(join(notes, 'b.md'), '* MARK\n\t* two\n');
  const out = join(temp, 'report-unkeepable');
  mkdirSync(out);
  const summary = await runRoundTripCheck({ root: notes, out, minimizeMs: 2000, samplesPerCause: 1, checker: new UnkeepableChecker() });
  expect(summary).toMatchObject({ wouldChange: 2, rewrittenOnSave: 1 });
  const index = readFileSync(join(out, 'index.md'), 'utf8');
  // b.md's causes already have their sample (a.md), but it is minimized anyway.
  expect(existsSync(join(out, '0002', 'repro.anon.md'))).toBe(true);
  expect(index).toMatch(/\| 0002 \| b\.md \|.*\| rewritten \|\n/);
  expect(index).toMatch(/\| 0001 \| a\.md \|.*\|  \|\n/);
}, 60_000);
