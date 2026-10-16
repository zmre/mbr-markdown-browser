/// <reference types="node" />
/**
 * `bun run roundtrip-check <folder-or-note> --out <report-folder>`: the
 * command-line half of the tool (usage in `scripts/roundtrip-check.ts`).
 *
 * Validates the arguments, prepares the report folder, runs the check
 * (`run.check.ts`) in a child vitest with `vitest.roundtrip.config.ts`, and
 * prints **only aggregate counts** — never a note's name or a line of it, so
 * the output can be pasted anywhere. The child's own output goes to
 * `<report>/run.log`. Progress (counts) goes to stderr.
 *
 * Imports nothing from the editor, so it runs under plain bun.
 */
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, openSync, closeSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Where the CLI writes. */
export interface CliIo {
  stdout: (text: string) => void;
  stderr: (text: string) => void;
  /** Whether stderr is a terminal (progress is redrawn in place). */
  isTTY?: boolean;
}

/** Marks a folder as a report this tool wrote, so a rerun may replace it. */
export const REPORT_MARKER = '.roundtrip-report';

export const USAGE = `Usage: bun run roundtrip-check <folder-or-note> --out <report-folder> [--minimize-seconds N]

Runs every .md/.markdown note under the folder through the editor's round trip
(open, then save untouched) and writes a report of the notes it would change.
Prints only counts. Hidden folders (.git, .obsidian, .mbr) are skipped.

  --out <dir>             report folder (created; replaced if it holds a
                          previous report; refused if it holds anything else)
  --minimize-seconds N    time spent shrinking each reproduction (default 10)
`;

interface Args {
  input: string;
  out: string;
  minimizeSeconds: number;
}

/** Parses `argv`; a string is an error message. */
export function parseArgs(argv: readonly string[]): Args | string | 'help' {
  let input: string | undefined;
  let out: string | undefined;
  let minimizeSeconds = 10;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const [flag, inline] = arg.startsWith('--') && arg.includes('=') ? [arg.slice(0, arg.indexOf('=')), arg.slice(arg.indexOf('=') + 1)] : [arg, undefined];
    const value = () => inline ?? argv[++i];
    if (flag === '-h' || flag === '--help') return 'help';
    else if (flag === '--out') out = value();
    else if (flag === '--minimize-seconds') {
      minimizeSeconds = Number(value());
      if (!Number.isFinite(minimizeSeconds) || minimizeSeconds < 0) return '--minimize-seconds needs a number of seconds';
    } else if (flag.startsWith('-')) return `unknown option ${flag}`;
    else if (input === undefined) input = arg;
    else return 'only one folder or note can be checked at a time';
  }
  if (!input) return 'missing the folder or note to check';
  if (!out) return 'missing --out <report-folder>';
  return { input: resolve(input), out: resolve(out), minimizeSeconds };
}

/** Empties or creates `out`; a string is an error message. */
function prepareReport(out: string, input: string): string | null {
  const rel = relative(out, input);
  if (rel === '' || (!rel.startsWith('..') && !isAbsolute(rel))) return '--out must not be or contain the folder being checked';
  if (existsSync(out)) {
    if (!statSync(out).isDirectory()) return '--out exists and is not a folder';
    const entries = readdirSync(out);
    if (entries.length > 0 && !entries.includes(REPORT_MARKER)) {
      return '--out is not empty and is not a previous report; choose another folder';
    }
    rmSync(out, { recursive: true, force: true });
  }
  mkdirSync(out, { recursive: true });
  writeFileSync(join(out, REPORT_MARKER), 'Written by `bun run roundtrip-check`; a rerun replaces this folder.\n');
  return null;
}

/** The components folder (this file is `components/src/roundtrip-check/cli.ts`). */
const COMPONENTS = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

/** Runs the tool; resolves to the exit code. */
export async function main(argv: readonly string[], io: CliIo): Promise<number> {
  const args = parseArgs(argv);
  if (args === 'help') {
    io.stdout(USAGE);
    return 0;
  }
  if (typeof args === 'string') {
    io.stderr(`roundtrip-check: ${args}\n\n${USAGE}`);
    return 2;
  }
  if (!existsSync(args.input)) {
    io.stderr('roundtrip-check: the folder or note to check does not exist\n');
    return 2;
  }
  const problem = prepareReport(args.out, args.input);
  if (problem) {
    io.stderr(`roundtrip-check: ${problem}\n`);
    return 2;
  }

  const log = openSync(join(args.out, 'run.log'), 'w');
  const env: NodeJS.ProcessEnv = {};
  for (const [key, value] of Object.entries(process.env)) if (!key.startsWith('VITEST')) env[key] = value;
  Object.assign(env, {
    MBR_ROUNDTRIP_DIR: args.input,
    MBR_ROUNDTRIP_OUT: args.out,
    MBR_ROUNDTRIP_MINIMIZE_MS: String(Math.round(args.minimizeSeconds * 1000)),
  });
  // Vitest runs on node (`bun run test` does the same); `bun run` puts a `node` on PATH.
  const node = process.versions.bun ? 'node' : process.execPath;
  const child = spawn(node, [join(COMPONENTS, 'node_modules/vitest/vitest.mjs'), 'run', '--config', 'vitest.roundtrip.config.ts'], {
    cwd: COMPONENTS,
    env,
    stdio: ['ignore', log, log],
  });

  let shown = '';
  let tenth = -1;
  const showProgress = () => {
    try {
      const { done, total } = JSON.parse(readFileSync(join(args.out, 'progress.json'), 'utf8')) as { done: number; total: number };
      const line = `checked ${done}/${total}`;
      if (line === shown) return;
      // Off a terminal, one line per tenth, not one per poll.
      if (!io.isTTY && Math.floor((done * 10) / total) === tenth) return;
      tenth = Math.floor((done * 10) / total);
      shown = line;
      io.stderr(io.isTTY ? `\r${line}` : `${line}\n`);
    } catch {
      // Not written yet.
    }
  };
  const timer = setInterval(showProgress, 500);
  const code = await new Promise<number>((resolveExit) => {
    child.on('error', () => resolveExit(-1));
    child.on('close', (status) => resolveExit(status ?? -1));
  });
  clearInterval(timer);
  closeSync(log);
  showProgress();
  if (io.isTTY && shown) io.stderr('\n');

  let summary: { files: number; unchanged: number; wouldChange: number; rewrittenOnSave: number; errors: number; elapsedMs: number };
  try {
    summary = JSON.parse(readFileSync(join(args.out, 'summary.json'), 'utf8'));
  } catch {
    io.stderr(`roundtrip-check: the check did not finish (exit ${code}); its log is in ${join(args.out, 'run.log')}\n`);
    return 2;
  }
  const width = String(summary.files).length;
  const row = (label: string, n: number) => `  ${label.padEnd(16)}${String(n).padStart(width)}\n`;
  io.stdout(
    `roundtrip-check: ${summary.files} notes in ${(summary.elapsedMs / 1000).toFixed(1)} s\n` +
      row('unchanged', summary.unchanged) +
      row('would change', summary.wouldChange) +
      row('  on any save', summary.rewrittenOnSave) +
      row('errors', summary.errors) +
      `report: ${join(args.out, 'index.md')}\n`,
  );
  return 0;
}
