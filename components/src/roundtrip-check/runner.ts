/// <reference types="node" />
/**
 * The `roundtrip-check` run: finds the notes under a folder, runs each through
 * the editor's round trip ({@link RoundTripChecker}), and writes the report.
 * Runs inside vitest (it needs the editor and a DOM); `cli.ts` starts it and
 * prints the summary. See `scripts/roundtrip-check.ts` for usage.
 *
 * Nothing here writes to stdout: note names and contents go to the report
 * only.
 */
import { lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, statSync, writeFileSync } from 'node:fs';
import { extname, join, relative, sep } from 'node:path';
import { formatLineRanges, type LineRange } from '../editor-minimal-diff.js';
import { toLf, withLineEnding } from '../editor-frontmatter.js';
import { anonymize } from './anonymize.js';
import { constructGuess, RoundTripChecker, type RoundTrip } from './checker.js';
import { changedLineCount, lineChanges, unifiedDiff } from './diff.js';
import { minimizeDocument, type DocumentCut } from './minimize.js';

/** Extensions the tool checks (the editor opens any markdown page). */
const MARKDOWN_EXTENSIONS = new Set(['.md', '.markdown']);

/** What the CLI hands the run. */
export interface RunOptions {
  /** A folder (searched recursively) or a single note. */
  root: string;
  /** The report folder (created by the CLI). */
  out: string;
  /** Time budget for shrinking one note's reproduction, in ms. */
  minimizeMs: number;
  /** Called after each note with counts only. */
  onProgress?: (done: number, total: number) => void;
}

/** Aggregate counts: the only thing the CLI prints. */
export interface RunSummary {
  files: number;
  unchanged: number;
  wouldChange: number;
  /** Of `wouldChange`: notes the safety net cannot keep, so any save rewrites lines. */
  rewrittenOnSave: number;
  errors: number;
  elapsedMs: number;
}

/** The anonymized reproduction's status, as written in the index. */
export const ANON_REPRODUCES = 'reproduces';
export const ANON_DIFFERS = 'differs (review repro.md manually)';

/**
 * The notes under `root`, sorted by relative path: `.md`/`.markdown` files,
 * skipping anything hidden (`.git`, `.obsidian`, `.mbr`, dotfiles) and the
 * `exclude` folder (the report). Symlinked folders are not entered; a
 * symlinked note is checked only when it resolves inside `root`, once.
 * A `root` that is a file is checked whatever its extension.
 */
export function findNotes(root: string, exclude?: string): string[] {
  if (statSync(root).isFile()) return [root];
  const realRoot = realpathSync(root);
  const realExclude = exclude ? safeRealpath(exclude) : undefined;
  const found = new Map<string, string>();
  const walk = (dir: string) => {
    if (realExclude && safeRealpath(dir) === realExclude) return;
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.name.startsWith('.')) continue;
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(path);
      } else if (MARKDOWN_EXTENSIONS.has(extname(entry.name).toLowerCase())) {
        const real = entry.isSymbolicLink() ? safeRealpath(path) : path;
        if (!real) continue;
        if (entry.isSymbolicLink() && (!isInside(real, realRoot) || !lstatSync(real).isFile())) continue;
        const key = entry.isSymbolicLink() ? real : safeRealpath(path) ?? path;
        if (!found.has(key)) found.set(key, path);
      }
    }
  };
  walk(root);
  return [...found.values()].sort((a, b) => relative(root, a).localeCompare(relative(root, b), 'en', { numeric: true }));
}

function safeRealpath(path: string): string | undefined {
  try {
    return realpathSync(path);
  } catch {
    return undefined;
  }
}

function isInside(path: string, dir: string): boolean {
  return path === dir || path.startsWith(dir.endsWith(sep) ? dir : dir + sep);
}

/** Runs the check over `options.root` and writes the report to `options.out`. */
export async function runRoundTripCheck(options: RunOptions): Promise<RunSummary> {
  const started = Date.now();
  const files = findNotes(options.root, options.out);
  const base = statSync(options.root).isFile() ? join(options.root, '..') : options.root;
  const checker = new RoundTripChecker();
  const rows: string[] = [];
  const errorRows: string[] = [];
  const summary: RunSummary = { files: files.length, unchanged: 0, wouldChange: 0, rewrittenOnSave: 0, errors: 0, elapsedMs: 0 };
  try {
    for (const [index, file] of files.entries()) {
      const rel = relative(base, file).split(sep).join('/');
      try {
        // As `fetch(...).text()` hands it to the editor: UTF-8, a BOM removed.
        const bytes = readFileSync(file);
        const stored = new TextDecoder('utf-8', { ignoreBOM: true }).decode(bytes);
        const trip = await checker.check(new TextDecoder('utf-8').decode(bytes));
        if (trip.saved === stored) {
          summary.unchanged++;
        } else {
          summary.wouldChange++;
          if (trip.notice.onSave.length > 0) summary.rewrittenOnSave++;
          const id = String(summary.wouldChange).padStart(4, '0');
          const row = await reportNote(checker, join(options.out, id), rel, stored, trip, options.minimizeMs);
          rows.push(`| ${id} | ${cell(rel)} | ${row.changedLines} | ${cell(row.construct)} | ${row.anon} |`);
        }
      } catch (err) {
        summary.errors++;
        const message = (err instanceof Error ? err.message : String(err)).split('\n')[0].slice(0, 200);
        errorRows.push(`| E${String(summary.errors).padStart(3, '0')} | ${cell(rel)} | ${cell(message)} |`);
      }
      options.onProgress?.(index + 1, files.length);
    }
  } finally {
    checker.dispose();
  }
  summary.elapsedMs = Date.now() - started;
  writeFileSync(join(options.out, 'index.md'), indexMarkdown(summary, rows, errorRows));
  writeFileSync(join(options.out, 'summary.json'), `${JSON.stringify(summary, null, 2)}\n`);
  return summary;
}

/** A markdown table cell: no pipes or line breaks. */
function cell(text: string): string {
  return text.replace(/\|/g, '\\|').replace(/[\r\n]+/g, ' ');
}

function indexMarkdown(summary: RunSummary, rows: readonly string[], errorRows: readonly string[]): string {
  let out =
    '# Editor round-trip report\n\n' +
    `${summary.files} notes checked: ${summary.unchanged} unchanged, ${summary.wouldChange} would change ` +
    `(${summary.rewrittenOnSave} rewritten by any save), ${summary.errors} errors.\n\n` +
    'Each id is a folder: `diff.patch` (the note → what a full re-serialize writes, and the lines the editor ' +
    'notice lists), `repro.md` (a minimal reproduction), `repro.anon.md` (the same, anonymized), and their ' +
    '`*.diff.patch`. **Read `repro.anon.md` before sharing it**: anonymization is automatic and keeps markdown ' +
    "syntax, not meaning. See docs/modes/editing.md, \"Checking your own notes\".\n\n";
  if (rows.length > 0) {
    out += '| id | source | changed lines | construct | anonymized |\n|---|---|---|---|---|\n' + rows.join('\n') + '\n';
  }
  if (errorRows.length > 0) out += '\n## Errors\n\n| id | source | error |\n|---|---|---|\n' + errorRows.join('\n') + '\n';
  return out;
}

/** Writes one note's folder; returns its index row's facts. */
async function reportNote(
  checker: RoundTripChecker,
  dir: string,
  rel: string,
  stored: string,
  trip: RoundTrip,
  minimizeMs: number,
): Promise<{ changedLines: number; construct: string; anon: string }> {
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'diff.patch'), noticeHeader(trip, stored) + unifiedDiff(stored, trip.saved, `a/${rel}`, `b/${rel}`));

  const repro = await reproduction(checker, stored, trip, minimizeMs);
  const reproTrip = await checker.check(repro);
  writeFileSync(join(dir, 'repro.md'), repro);
  writeFileSync(join(dir, 'repro.diff.patch'), unifiedDiff(repro, reproTrip.saved, 'a/repro.md', 'b/repro.md'));

  const anon = anonymize(repro);
  const anonTrip = await checker.check(anon);
  writeFileSync(join(dir, 'repro.anon.md'), anon);
  writeFileSync(join(dir, 'repro.anon.diff.patch'), unifiedDiff(anon, anonTrip.saved, 'a/repro.anon.md', 'b/repro.anon.md'));
  const reproduces =
    anonTrip.changed && changedLineCount(anon, anonTrip.saved) === changedLineCount(repro, reproTrip.saved);
  return {
    changedLines: changedLineCount(stored, trip.saved),
    construct: constructGuess(stored, trip),
    anon: reproduces ? ANON_REPRODUCES : ANON_DIFFERS,
  };
}

/** `diff.patch`'s preamble (text before `---` is ignored by `patch` and `git apply`). */
function noticeHeader(trip: RoundTrip, stored: string): string {
  const ranges = (list: readonly LineRange[]) => (list.length > 0 ? formatLineRanges(list) : 'none');
  const untouched =
    trip.untouchedSave === stored
      ? 'the note unchanged'
      : trip.untouchedSave === trip.saved
        ? 'the whole re-serialization below'
        : 'part of the re-serialization below';
  return (
    'Editor round trip (file line numbers).\n' +
    `Notice at open, saved as written unless edited: ${ranges(trip.notice.ifEdited)}\n` +
    `Notice at open, rewritten by any save: ${ranges(trip.notice.onSave)}\n` +
    `Any save writes the parts you did not edit as: ${untouched}\n` +
    'Below: the note -> what a full re-serialize writes (any edit to a listed block writes that block this way).\n\n'
  );
}

/**
 * A small note that still shows the same rewrite: the editor removes and adds
 * exactly the lines (as text) the full note's round trip does. "Exactly" keeps
 * every distinct problem in the note (a subset would let shrinking settle on
 * one of them) and stops shrinking from wandering off to a different one (an
 * unclosed fence, say). Falls back to the whole note when the shrunk one does
 * not reproduce.
 */
async function reproduction(checker: RoundTripChecker, stored: string, trip: RoundTrip, minimizeMs: number): Promise<string> {
  const source = stored.charCodeAt(0) === 0xfeff ? stored.slice(1) : stored;
  if (trip.saved === source) return stored; // only the byte-order mark is lost
  const target = lineChanges(source, trip.saved);
  const { lineEnding, body } = trip.doc;
  const interesting = async (lf: string): Promise<boolean> => {
    const markdown = withLineEnding(lf, lineEnding);
    const candidate = await checker.check(markdown);
    if (!candidate.changed) return false;
    const { removed, added } = lineChanges(markdown, candidate.saved);
    return sameLines(removed, target.removed) && sameLines(added, target.added);
  };
  const lf = toLf(source);
  const split = trip.blocks;
  const cut: DocumentCut = {
    prefix: lf.slice(0, lf.length - body.length),
    leading: split ? split.leading : '',
    blocks: split ? split.blocks : [body],
    gaps: split ? split.gaps : [],
    trailing: split ? split.trailing : '',
  };
  const definitions = trip.blockNodes.flatMap((node, i) =>
    node.type === 'definition' || node.type === 'footnoteDefinition' ? [i] : [],
  );
  const suspects = [...new Set([...trip.changedBlocks, ...definitions])];
  const now = Date.now();
  const shrunk = await minimizeDocument(cut, suspects, interesting, { deadline: now + minimizeMs, maxTests: 5000 });
  return (await interesting(shrunk)) ? withLineEnding(shrunk, lineEnding) : source;
}

function sameLines(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
  return a.size === b.size && [...a].every((line) => b.has(line));
}
