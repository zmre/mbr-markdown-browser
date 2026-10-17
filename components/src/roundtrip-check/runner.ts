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
import { CauseIndex, diffShapes, type Cause } from './signature.js';

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
  /**
   * Reproductions written per cause (a shape of changed line, `signature.ts`):
   * a note is minimized only when one of its causes has fewer samples than
   * this; the others get `diff.patch` only. Default {@link DEFAULT_SAMPLES_PER_CAUSE}.
   */
  samplesPerCause?: number;
  /**
   * Time for all minimizations together, in ms; once spent, no more notes
   * are minimized. Default {@link DEFAULT_MINIMIZE_TOTAL_MS}.
   */
  minimizeTotalMs?: number;
  /** Notes larger than this (bytes) get no reproduction. Default {@link MAX_REPRO_BYTES}. */
  maxReproBytes?: number;
  /** The checker to use (tests); one is made, and disposed, by default. */
  checker?: RoundTripChecker;
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
  /** Distinct causes (shapes of changed lines) among `wouldChange`. */
  causes: number;
  elapsedMs: number;
}

/** Default {@link RunOptions.samplesPerCause}. */
export const DEFAULT_SAMPLES_PER_CAUSE = 3;

/** Default {@link RunOptions.minimizeTotalMs}: five minutes. */
export const DEFAULT_MINIMIZE_TOTAL_MS = 300_000;

/**
 * Default {@link RunOptions.maxReproBytes}: past this a note is not shrunk. A
 * check costs ~2 ms per kB, so minimizing a larger note would spend its whole
 * budget on a handful of tests, and its anonymized copy is too big to read.
 */
export const MAX_REPRO_BYTES = 200_000;

/** The anonymized reproduction's status, as written in the index. */
export const ANON_REPRODUCES = 'reproduces';
export const ANON_DIFFERS = 'differs (review repro.md manually)';
/** The index's anonymized column for a note too large to shrink ({@link MAX_REPRO_BYTES}). */
export const ANON_TOO_LARGE = 'not minimized (too large)';
/** The index's anonymized column once the total minimization time is spent. */
export const ANON_OUT_OF_TIME = 'not minimized (time limit)';
/** The index's anonymized column for a note whose causes all have their samples: `see <id>`. */
export const ANON_SEE = 'see';

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

/** One note that would change, for the index. */
interface Row {
  id: string;
  rel: string;
  changedLines: number;
  construct: string;
  causes: Cause[];
  anon: string;
  /** Whether any save rewrites lines of it (the notice's `onSave`). */
  onSave: boolean;
}

/** Runs the check over `options.root` and writes the report to `options.out`. */
export async function runRoundTripCheck(options: RunOptions): Promise<RunSummary> {
  const started = Date.now();
  const files = findNotes(options.root, options.out);
  const base = statSync(options.root).isFile() ? join(options.root, '..') : options.root;
  const samples = options.samplesPerCause ?? DEFAULT_SAMPLES_PER_CAUSE;
  const maxBytes = options.maxReproBytes ?? MAX_REPRO_BYTES;
  let minimizeLeft = options.minimizeTotalMs ?? DEFAULT_MINIMIZE_TOTAL_MS;
  const checker = options.checker ?? new RoundTripChecker();
  const causes = new CauseIndex();
  const rows: Row[] = [];
  const errorRows: string[] = [];
  const summary: RunSummary = {
    files: files.length,
    unchanged: 0,
    wouldChange: 0,
    rewrittenOnSave: 0,
    errors: 0,
    causes: 0,
    elapsedMs: 0,
  };
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
          const construct = constructGuess(stored, trip);
          const noteCauses = causes.add(id, diffShapes(stored, trip.saved));
          const dir = join(options.out, id);
          mkdirSync(dir, { recursive: true });
          writeFileSync(join(dir, 'diff.patch'), noticeHeader(trip, stored) + unifiedDiff(stored, trip.saved, `a/${rel}`, `b/${rel}`));
          // Only the first few notes of a cause are worth shrinking: the rest
          // show the same rewrite, and minimizing is what costs.
          // A note any save rewrites is always worth a reproduction: those are the dangerous ones.
          const onSave = trip.notice.onSave.length > 0;
          let anon: string;
          if (!onSave && !CauseIndex.wanted(noteCauses, samples)) {
            const sample = noteCauses.find((cause) => cause.sampled.length > 0)?.sampled[0];
            anon = sample ? `${ANON_SEE} ${sample}` : 'not minimized';
          } else if (bytes.length > maxBytes) {
            anon = ANON_TOO_LARGE;
          } else if (minimizeLeft <= 0) {
            anon = ANON_OUT_OF_TIME;
          } else {
            CauseIndex.sampled(noteCauses, id, samples);
            const begun = Date.now();
            anon = await writeReproduction(checker, dir, stored, trip, Math.min(options.minimizeMs, minimizeLeft));
            minimizeLeft -= Date.now() - begun;
          }
          rows.push({ id, rel, changedLines: changedLineCount(stored, trip.saved), construct, causes: noteCauses, anon, onSave });
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
  summary.causes = causes.size;
  summary.elapsedMs = Date.now() - started;
  writeFileSync(join(options.out, 'index.md'), indexMarkdown(summary, rows, errorRows, causes.ranked()));
  writeFileSync(join(options.out, 'summary.json'), `${JSON.stringify(summary, null, 2)}\n`);
  return summary;
}

/** A markdown table cell: no pipes or line breaks. */
function cell(text: string): string {
  return text.replace(/\|/g, '\\|').replace(/[\r\n]+/g, ' ');
}

/** Causes named per note in the index. */
const CAUSES_SHOWN = 4;

function indexMarkdown(
  summary: RunSummary,
  rows: readonly Row[],
  errorRows: readonly string[],
  ranked: readonly { label: string; cause: Cause }[],
): string {
  const labels = new Map(ranked.map(({ label, cause }) => [cause, label]));
  const rank = new Map(ranked.map(({ cause }, i) => [cause, i]));
  let out =
    '# Editor round-trip report\n\n' +
    `${summary.files} notes checked: ${summary.unchanged} unchanged, ${summary.wouldChange} would change ` +
    `(${summary.rewrittenOnSave} rewritten by any save), ${summary.errors} errors; ${summary.causes} causes.\n\n` +
    'Each id is a folder with `diff.patch` (the note → what a full re-serialize writes, and the lines the editor ' +
    'notice lists). Notes that bring a cause its first samples also get `repro.md` (a minimal reproduction), ' +
    '`repro.anon.md` (the same, anonymized) and their `*.diff.patch`; the others say `see <id>`. A note ' +
    'marked `rewritten` under "any save" is one the safety net cannot keep: any save rewrites lines of it; ' +
    'those are always minimized. ' +
    '**Read `repro.anon.md` before sharing it**: anonymization is automatic and keeps markdown ' +
    'syntax, not meaning. See docs/modes/editing.md, "Checking your own notes".\n\n';
  if (ranked.length > 0) {
    out +=
      '## Causes\n\n' +
      'Each cause is the shape of a line the round trip removes (`-`) or adds (`+`): its indentation (`⇥` tabs, ' +
      '`␣` one space … `␣␣␣␣` four or more), block prefix, what its text holds (`·` text, `url` a bare URL, ' +
      '`<url>` an autolink, `<br/>`, `\\` an escape, `∅` nothing) and trailing whitespace. No note text. ' +
      'Most notes first; a note usually shows a removed and an added shape for each rewrite.\n\n' +
      '| cause | notes | shape | minimized in |\n|---|---|---|---|\n' +
      ranked
        .map(({ label, cause }) => {
          const shape = `\`${cause.shape.replace(/`/g, 'ˋ')}\``;
          return `| ${label} | ${cause.ids.length} | ${cell(shape)} | ${cause.sampled.join(', ')} |`;
        })
        .join('\n') +
      '\n\n## Notes\n\n';
  }
  if (rows.length > 0) {
    out +=
      '| id | source | changed lines | construct | anonymized | causes | any save |\n|---|---|---|---|---|---|---|\n' +
      rows
        .map((row) => {
          const named = [...row.causes].sort((a, b) => (rank.get(a) ?? 0) - (rank.get(b) ?? 0)).map((cause) => labels.get(cause));
          const shown = named.slice(0, CAUSES_SHOWN).join(' ') + (named.length > CAUSES_SHOWN ? ` +${named.length - CAUSES_SHOWN}` : '');
          return `| ${row.id} | ${cell(row.rel)} | ${row.changedLines} | ${cell(row.construct)} | ${row.anon} | ${shown} | ${row.onSave ? 'rewritten' : ''} |`;
        })
        .join('\n') +
      '\n';
  }
  if (errorRows.length > 0) out += '\n## Errors\n\n| id | source | error |\n|---|---|---|\n' + errorRows.join('\n') + '\n';
  return out;
}

/** Writes a note's reproduction files into `dir`; returns the index's anonymized column. */
async function writeReproduction(
  checker: RoundTripChecker,
  dir: string,
  stored: string,
  trip: RoundTrip,
  minimizeMs: number,
): Promise<string> {
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
  return reproduces ? ANON_REPRODUCES : ANON_DIFFERS;
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
    `Notice at open, if the frontmatter is edited: ${trip.notice.frontmatter.join('; ') || 'none'}\n` +
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
