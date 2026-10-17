/// <reference types="node" />
/**
 * `roundtrip-check` end to end: `main()` (what `bun run roundtrip-check`
 * calls) over a temporary folder, running the real check in a child vitest.
 * The output must hold counts only — no note names, no note text — and the
 * report must have its files. Plus the argument parsing and the note walk.
 */
import { existsSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';
import { afterAll, describe, expect, it } from 'vitest';
import { fixture } from '../editor-test-support.js';
import { main, parseArgs, REPORT_MARKER } from './cli.js';
import { anonymize } from './anonymize.js';
import { ANON_REPRODUCES, findNotes } from './runner.js';

const temp = mkdtempSync(join(tmpdir(), 'mbr-roundtrip-test-'));
afterAll(() => rmSync(temp, { recursive: true, force: true }));

/** Words that must never reach the terminal. */
const SECRETS = ['Zanzibar', 'Quarterly', 'lossy', 'kitchen', 'tasks', 'hidden'];

/**
 * Rewritten by a full re-serialization: a lazy line in a footnote is
 * indented, `    secret_two here.` (the only
 * rewrite the test needs; if the editor learns to keep it, pick another). The
 * blank line ending the frontmatter is kept: an unedited frontmatter is saved
 * as loaded.
 */
const LOSSY =
  '---\ntitle: Quarterly Plan\n\n---\n# Quarterly Plan\nKickoff.\n\nMeet in Zanzibar on 2026-03-04.\n\n[^1]: See\nsecret_two here.\n';

function write(path: string, text: string): void {
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, text);
}

describe('parseArgs', () => {
  it('reads the folder, --out, --minimize-seconds and --samples-per-cause', () => {
    expect(parseArgs(['notes', '--out', 'r', '--minimize-seconds=2.5'])).toMatchObject({ minimizeSeconds: 2.5, samplesPerCause: 3 });
    expect(parseArgs(['notes', '--out', 'r', '--samples-per-cause', '1'])).toMatchObject({ samplesPerCause: 1, minimizeSeconds: 2 });
    expect(parseArgs(['notes', '--out', 'r', '--samples-per-cause', '1.5'])).toMatch(/whole number/);
    expect(parseArgs(['notes', '--out', 'r', '--minimize-total-seconds', '60'])).toMatchObject({ minimizeTotalSeconds: 60 });
    expect(parseArgs(['--help'])).toBe('help');
    expect(parseArgs(['notes'])).toMatch(/--out/);
    expect(parseArgs(['--out', 'r'])).toMatch(/missing/);
    expect(parseArgs(['a', 'b', '--out', 'r'])).toMatch(/one folder/);
    expect(parseArgs(['a', '--out', 'r', '--bogus'])).toMatch(/unknown option/);
  });
});

describe('findNotes', () => {
  it('finds markdown notes, skipping hidden folders and links that leave the folder', () => {
    const root = join(temp, 'walk');
    write(join(root, 'a.md'), 'a');
    write(join(root, 'sub', 'b.markdown'), 'b');
    write(join(root, 'sub', 'c.txt'), 'c');
    write(join(root, '.git', 'd.md'), 'd');
    write(join(root, '.obsidian', 'e.md'), 'e');
    write(join(root, '.hidden.md'), 'f');
    write(join(temp, 'outside', 'g.md'), 'g');
    symlinkSync(join(temp, 'outside', 'g.md'), join(root, 'g.md'));
    symlinkSync(join(temp, 'outside'), join(root, 'linked-dir'));
    symlinkSync(join(root, 'a.md'), join(root, 'sub', 'alias.md'));
    write(join(root, 'report', 'index.md'), 'report');
    const found = findNotes(root, join(root, 'report')).map((path) => relative(root, path));
    expect(found).toEqual(['a.md', 'sub/b.markdown']);
  });
});

describe('roundtrip-check end to end', () => {
  it('prints counts only and writes the report', async () => {
    const notes = join(temp, 'notes');
    write(join(notes, 'kitchen-sink.md'), fixture('kitchen-sink'));
    write(join(notes, 'deep', 'tasks.md'), fixture('tasks'));
    write(join(notes, 'deep', 'lossy.md'), LOSSY);
    write(join(notes, '.obsidian', 'hidden.md'), LOSSY);
    const out = join(temp, 'report');

    let stdout = '';
    let stderr = '';
    const code = await main([notes, '--out', out, '--minimize-seconds', '5'], {
      stdout: (text) => (stdout += text),
      stderr: (text) => (stderr += text),
    });
    expect(code, `${stderr}\n${existsSync(join(out, 'run.log')) ? readFileSync(join(out, 'run.log'), 'utf8') : ''}`).toBe(0);

    expect(stdout).toMatch(/^roundtrip-check: 3 notes in [\d.]+ s\n/);
    expect(stdout).toMatch(/unchanged\s+2\n/);
    expect(stdout).toMatch(/would change\s+1\n/);
    expect(stdout).toMatch(/errors\s+0\n/);
    expect(stdout).toMatch(/causes\s+\d+\n/);
    expect(stdout).toContain(`report: ${join(out, 'index.md')}`);
    for (const secret of SECRETS) {
      expect(stdout).not.toContain(secret);
      expect(stderr).not.toContain(secret);
    }

    expect(readdirSync(out).sort()).toEqual(['.roundtrip-report', '0001', 'index.md', 'progress.json', 'run.log', 'summary.json']);
    const index = readFileSync(join(out, 'index.md'), 'utf8');
    expect(index).toMatch(/\| 0001 \| deep\/lossy\.md \| \d+ \| [^|]*footnote[^|]* \| reproduces \| S\d+/);
    expect(readdirSync(join(out, '0001')).sort()).toEqual([
      'diff.patch',
      'repro.anon.diff.patch',
      'repro.anon.md',
      'repro.diff.patch',
      'repro.md',
    ]);
    const patch = readFileSync(join(out, '0001', 'diff.patch'), 'utf8');
    // The footnote's lines are named: an edit to it would write them this way.
    expect(patch).toContain('Notice at open, saved as written unless edited: 10–11\n');
    expect(patch).toContain('Notice at open, if the frontmatter is edited: blank lines at its end are dropped');
    // The unedited frontmatter, its trailing blank line included, is not part of the diff.
    expect(patch).toContain('Any save writes the parts you did not edit as: the note unchanged');
    expect(patch).toContain('--- a/deep/lossy.md\n+++ b/deep/lossy.md\n');
    expect(patch).toContain('-secret_two here.\n+    secret_two here.\n');

    // The reproduction keeps the rewrite and drops what it does not need:
    // the heading and paragraphs, and the frontmatter (no longer rewritten).
    const repro = readFileSync(join(out, '0001', 'repro.md'), 'utf8');
    expect(repro).not.toContain('Zanzibar');
    expect(repro).not.toContain('title:');
    expect(repro).not.toContain('Kickoff');
    expect(repro).toBe('[^1]: See\nsecret_two here.\n');
    const anon = readFileSync(join(out, '0001', 'repro.anon.md'), 'utf8');
    expect(anon).toBe(anonymize(repro));
    expect(index).toContain(ANON_REPRODUCES);
    expect(readFileSync(join(out, '0001', 'repro.anon.diff.patch'), 'utf8')).toContain(
      '-xxxxxx_xxx xxxx.\n+    xxxxxx_xxx xxxx.\n',
    );

    // A rerun replaces its own report.
    const again = await main([notes, '--out', out, '--minimize-seconds', '0'], { stdout: () => {}, stderr: () => {} });
    expect(again).toBe(0);
    expect(existsSync(join(out, REPORT_MARKER))).toBe(true);
  }, 120_000);

  it('refuses a report folder holding something else', async () => {
    const out = join(temp, 'not-a-report');
    write(join(out, 'keep.txt'), 'mine');
    let stderr = '';
    const code = await main([join(temp, 'notes'), '--out', out], { stdout: () => {}, stderr: (text) => (stderr += text) });
    expect(code).toBe(2);
    expect(stderr).toContain('not empty');
    expect(readFileSync(join(out, 'keep.txt'), 'utf8')).toBe('mine');
  });

  it('refuses a report folder that contains the notes', async () => {
    let stderr = '';
    const code = await main([join(temp, 'notes'), '--out', temp], { stdout: () => {}, stderr: (text) => (stderr += text) });
    expect(code).toBe(2);
    expect(stderr).toContain('must not be or contain');
  });
});
