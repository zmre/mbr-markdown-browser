/**
 * The editor's round trip of one note, for `roundtrip-check`: what opening
 * and saving it writes, and what the editor's open-time notice lists. Runs
 * the real editor (Crepe with `CREPE_FEATURES`, `CREPE_FEATURE_CONFIGS` and
 * `MARKDOWN_EXTENSION_PLUGINS`, via `editor-test-support.ts`'s `openCrepe`)
 * and the editor's own code for everything around it: `openDocument` /
 * `documentToSave` (frontmatter split, CRLF), `serializeBody`, and
 * `createBodySaver`, whose `rewritten` is the notice's line list.
 *
 * Needs a DOM (happy-dom under vitest).
 *
 * # One editor for every note
 *
 * Creating a Crepe per note costs ~30 ms, and V8 keeps every WeakRef target
 * created in a task alive until that task ends (happy-dom's `DOMParser` makes
 * one document per Crepe icon), so a loop that never yields grows by ~8 MB a
 * note. The checker therefore opens one Crepe and loads each note into it with
 * Milkdown's `replaceAll(body, true)` — the same parser, and a fresh
 * `EditorState` over the same plugins, which is what `create()` builds — and
 * yields a macrotask before each note. `checker.test.ts` pins that a reused
 * editor gives exactly what a fresh one does on every fixture.
 */
import type { Crepe } from '@milkdown/crepe';
import { remarkCtx } from '@milkdown/kit/core';
import { replaceAll } from '@milkdown/kit/utils';
import { createBodySaver, serializeBody } from '../editor-crepe.js';
import { documentToSave, frontmatterRewrites, openDocument, toLf, type OpenedDocument } from '../editor-frontmatter.js';
import { splitBlocks, type BlockSplit, type LineRange, type ParsedRoot } from '../editor-minimal-diff.js';
import { destroyEditors, openCrepe, textareaValue } from '../editor-test-support.js';
import { diffLines, linesOf } from './diff.js';

/** The parts of an mdast node the construct guess reads. */
interface MdastBlock {
  type: string;
  data?: { alert?: unknown };
  checked?: boolean | null;
  status?: string;
  children?: MdastBlock[];
}

/** One note's round trip. Line numbers are file lines (1-based). */
export interface RoundTrip {
  /** The note as the editor opened it. */
  doc: OpenedDocument;
  /** What opening and saving it writes if the whole body is re-serialized. */
  saved: string;
  /** Whether {@link saved} differs from the note. */
  changed: boolean;
  /**
   * What a save writes for the parts not edited: the body through the safety
   * net (`BodySaver.body()`), the frontmatter through the textarea. (A save
   * with no edit at all writes nothing: the editor is not dirty.)
   */
  untouchedSave: string;
  /**
   * The open-time notice: lines kept unless edited, lines any save rewrites,
   * and what editing the frontmatter would normalize (`frontmatterRewrites`).
   */
  notice: { ifEdited: LineRange[]; onSave: LineRange[]; frontmatter: string[] };
  /** The body's top-level blocks (`null` if the parser gave no positions). */
  blocks: BlockSplit | null;
  /** Indices into `blocks` of the blocks the notice names or the round trip touches. */
  changedBlocks: number[];
  /** The mdast node of every block, in order. */
  blockNodes: MdastBlock[];
}

/** Yields a macrotask: lets V8 drop the previous note's WeakRef targets. */
function nextTask(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

/** Runs notes through the editor's round trip, one editor for all of them. */
export class RoundTripChecker {
  #crepe: Crepe | null = null;

  /** The round trip of `source` (a whole file, frontmatter and all). */
  async check(source: string): Promise<RoundTrip> {
    await nextTask();
    const doc = openDocument(source);
    const crepe = await this.#load(doc.body);
    const saver = createBodySaver(crepe, doc.body);
    const yaml = textareaValue(doc.frontmatter);
    const saved = documentToSave(doc, yaml, serializeBody(crepe));
    const untouchedSave = documentToSave(doc, yaml, saver.body().body);
    const shift = (ranges: readonly LineRange[]) =>
      ranges.map(({ first, last }) => ({ first: first + doc.bodyLineOffset, last: last + doc.bodyLineOffset }));
    const notice = {
      ifEdited: shift(saver.rewritten.ifEdited),
      onSave: shift(saver.rewritten.onSave),
      frontmatter: frontmatterRewrites(doc),
    };
    const blockNodes = (this.parse(doc.body) as unknown as { children: MdastBlock[] }).children;
    const blocks = splitBlocks(doc.body, (markdown) => this.parse(markdown));
    // Body lines the round trip touches: the notice's blocks, and every line
    // the diff removes or inserts next to (a blank line added between two
    // blocks belongs to both).
    const touched = touchedLines(toLf(source), toLf(saved)).map((line) => line - doc.bodyLineOffset);
    const bodyRanges = [...saver.rewritten.ifEdited, ...saver.rewritten.onSave, ...touched.map((line) => ({ first: line, last: line }))];
    const changedBlocks = blocks
      ? blocks.lines.flatMap((first, i) => {
          const last = first + blocks.blocks[i].split('\n').length - 1;
          return bodyRanges.some((r) => r.first <= last && r.last >= first) ? [i] : [];
        })
      : [];
    return { doc, saved, changed: saved !== source, untouchedSave, notice, blocks, changedBlocks, blockNodes };
  }

  /** The editor's parser (remark with every plugin Milkdown registers). */
  parse(markdown: string): ParsedRoot {
    if (!this.#crepe) throw new Error('RoundTripChecker: no editor yet');
    const remark = this.#crepe.editor.action((ctx) => ctx.get(remarkCtx));
    return remark.parse(markdown) as unknown as ParsedRoot;
  }

  /** Destroys the editor. */
  dispose(): void {
    destroyEditors();
    this.#crepe = null;
  }

  async #load(body: string): Promise<Crepe> {
    if (!this.#crepe) {
      this.#crepe = await openCrepe(body);
      return this.#crepe;
    }
    this.#crepe.editor.action(replaceAll(body, true));
    return this.#crepe;
  }
}

/** 1-based lines of `a` that `a` → `b` removes or replaces, or inserts lines between. */
export function touchedLines(a: string, b: string): number[] {
  const touched = new Set<number>();
  let line = 1;
  let previous = ' ';
  for (const { op } of diffLines(linesOf(a), linesOf(b))) {
    if (op === '+') {
      // A pure insertion touches both neighbours; a replacement, only what it replaced.
      if (previous !== '-') {
        touched.add(line - 1);
        touched.add(line);
      }
    } else {
      if (op === '-') touched.add(line);
      line++;
    }
    if (op !== '+') previous = op;
  }
  return [...touched].filter((n) => n >= 1).sort((x, y) => x - y);
}

/** Plain names for the mdast block types a note is made of. */
const BLOCK_NAMES: Record<string, string> = {
  defList: 'definition list',
  list: 'list',
  heading: 'heading',
  paragraph: 'paragraph',
  blockquote: 'blockquote',
  code: 'code block',
  table: 'table',
  html: 'html',
  thematicBreak: 'rule',
  sectionBreak: 'section attributes',
  footnoteDefinition: 'footnote',
  definition: 'link reference',
  math: 'math block',
  image: 'image',
  'image-block': 'image',
};

/** What kind of block `node` is, in words (`task list`, `alert`, …). */
function blockName(node: MdastBlock): string {
  if (node.type === 'blockquote' && node.data?.alert) return 'alert';
  if (node.type === 'list' && node.children?.some((item) => item.checked != null || item.status !== undefined)) {
    return 'task list';
  }
  return BLOCK_NAMES[node.type] ?? node.type;
}

/**
 * A guess at what the editor rewrites in `source` (the file as stored, a
 * byte-order mark included), for the report's index: the kinds of the blocks
 * the notice names, `frontmatter` when the frontmatter changes,
 * and whether the change is whitespace only.
 */
export function constructGuess(source: string, trip: RoundTrip): string {
  const kinds = new Set(trip.changedBlocks.map((i) => blockName(trip.blockNodes[i] ?? { type: 'unknown' })));
  if (openDocument(trip.saved).frontmatter !== trip.doc.frontmatter) kinds.add('frontmatter');
  if (source.charCodeAt(0) === 0xfeff && trip.saved.charCodeAt(0) !== 0xfeff) kinds.add('byte-order mark');
  let guess = [...kinds].join(', ') || 'document whitespace';
  if (trip.saved.replace(/\s+/g, '') === source.replace(/\s+/g, '')) guess += ' (whitespace only)';
  return guess;
}
