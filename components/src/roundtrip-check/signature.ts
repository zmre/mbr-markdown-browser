/**
 * Content-free "causes" for `roundtrip-check`: what kind of line a note's
 * round trip removes or adds, so the report can rank the problems across a
 * whole folder without anyone reading a note, and the tool can minimize only
 * a few notes per problem. Pure.
 *
 * A changed line is reduced to its **shape** ({@link lineShape}): its
 * indentation (tabs, or up to four spaces — 4, 8 and 12 spaces are one
 * shape), the block prefix (`#`, `>`, a list marker, a task box, a fence, a
 * table pipe, a reference definition), what its text holds that the editor
 * is known to respell (an autolink `<…>`, a bare URL, a `<br />`, a
 * backslash escape) and its trailing whitespace, prefixed `-` (removed) or
 * `+` (added). Letters and digits never survive, so shapes can be printed
 * anywhere. Each distinct shape is a cause; a note has every cause its diff
 * shows.
 */
import { diffLines, linesOf } from './diff.js';

/** The block prefixes a shape keeps, longest alternatives first. */
const PREFIX =
  /^(?:#{1,6}(?=[ \t]|$)|(?:>[ \t]?)+|(?:[*+-]|\d{1,9}[.)])(?=[ \t]|$)(?:[ \t]+\[[ xX>-]\])?|`{3,}|~{3,}|\||\[[^\]]+\]:|[=-]{2,}[ \t]*$|(?:_[ \t]*){3,}$)/;

/** One line (with its `\n`, if it has one) as a shape. */
export function lineShape(line: string): string {
  const text = line.replace(/\n$/, '');
  const noEol = line.endsWith('\n') ? '' : ' (no final newline)';
  const indent = /^[ \t]*/.exec(text)?.[0] ?? '';
  const rest = text.slice(indent.length);
  const trailing = /[ \t]*$/.exec(rest)?.[0] ?? '';
  const body = rest.slice(0, rest.length - trailing.length);
  if (body === '') return `${showIndent(indent)}∅${showTrailing(trailing)}${noEol}`;
  const prefix = PREFIX.exec(body)?.[0] ?? '';
  const content = body.slice(prefix.length);
  // The prefix's own letters and digits (`12.`, `[x]`, a definition's label) are reduced too.
  const shapedPrefix = prefix
    .replace(/^\[[^\]]+\]:$/, '[·]:')
    .replace(/\d+/g, '0')
    .replace(/[A-Za-z]/g, 'x')
    .replace(/[ \t]+/g, ' ');
  const notes: string[] = [];
  if (/<[a-z][a-z0-9+.-]*:[^>\s]*>|<[^>\s@]+@[^>\s]+>/i.test(content)) notes.push('<url>');
  else if (/\b(?:https?|ftp):\/\/|\bwww\./i.test(content)) notes.push('url');
  if (/<br\s*\/?>/i.test(content)) notes.push('<br/>');
  if (/\\[!-/:-@[-`{-~]/.test(content)) notes.push('\\');
  if (content.trim() === '') notes.push('∅');
  else if (notes.length === 0) notes.push('·');
  const head = shapedPrefix ? `${shapedPrefix.trimEnd()} ` : '';
  return `${showIndent(indent)}${head}${notes.join('')}${showTrailing(trailing)}${noEol}`;
}

/** `⇥` for tabs (any number), `␣`…`␣␣␣␣` for one to four or more spaces, `⇥␣` for a mix. */
function showIndent(ws: string): string {
  if (ws.includes('\t')) return ws.includes(' ') ? '⇥␣' : '⇥';
  return '␣'.repeat(Math.min(ws.length, 4));
}

/** Trailing whitespace: `␣`, `␣␣` (two or more spaces: a hard break), `⇥` for a tab in it. */
function showTrailing(ws: string): string {
  if (ws.includes('\t')) return '⇥';
  return '␣'.repeat(Math.min(ws.length, 2));
}

/** The distinct shapes of the lines `stored` → `saved` removes (`-…`) and adds (`+…`), sorted. */
export function diffShapes(stored: string, saved: string): string[] {
  const shapes = new Set<string>();
  for (const { op, text } of diffLines(linesOf(stored), linesOf(saved))) {
    if (op !== ' ') shapes.add(`${op}${lineShape(text)}`);
  }
  return [...shapes].sort();
}

/** One cause: a shape, the notes (report ids) that show it, and those minimized for it. */
export interface Cause {
  shape: string;
  ids: string[];
  sampled: string[];
}

/** The causes seen so far, and which notes are worth minimizing. */
export class CauseIndex {
  readonly #causes = new Map<string, Cause>();

  /** Records that note `id` shows `shapes`. */
  add(id: string, shapes: readonly string[]): Cause[] {
    return shapes.map((shape) => {
      let cause = this.#causes.get(shape);
      if (!cause) {
        cause = { shape, ids: [], sampled: [] };
        this.#causes.set(shape, cause);
      }
      cause.ids.push(id);
      return cause;
    });
  }

  /** Whether a note with these causes would add a sample to one with fewer than `samples`. */
  static wanted(causes: readonly Cause[], samples: number): boolean {
    return causes.some((cause) => cause.sampled.length < samples);
  }

  /** Records that note `id` was minimized: a sample of each of its causes with fewer than `samples`. */
  static sampled(causes: readonly Cause[], id: string, samples: number): void {
    for (const cause of causes) if (cause.sampled.length < samples) cause.sampled.push(id);
  }

  /** Every cause, most notes first (ties: first seen first), labelled `S001`, `S002`, … in that order. */
  ranked(): { label: string; cause: Cause }[] {
    const order = [...this.#causes.values()];
    const firstSeen = new Map(order.map((cause, i) => [cause, i]));
    order.sort((a, b) => b.ids.length - a.ids.length || (firstSeen.get(a) ?? 0) - (firstSeen.get(b) ?? 0));
    const width = Math.max(3, String(order.length).length);
    return order.map((cause, i) => ({ label: `S${String(i + 1).padStart(width, '0')}`, cause }));
  }

  get size(): number {
    return this.#causes.size;
  }
}
