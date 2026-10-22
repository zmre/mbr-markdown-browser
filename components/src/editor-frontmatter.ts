/**
 * Pure helpers for separating YAML frontmatter from the markdown body and
 * recombining them. Milkdown/Crepe edits only the body; the frontmatter is
 * edited as raw YAML. Keeping these pure and dependency-free lets us unit-test
 * the round-trip without loading the editor.
 */

export interface SplitResult {
  /** Inner YAML (without the `---` fences), or `null` if there is no frontmatter. */
  frontmatter: string | null;
  /** The markdown body following the frontmatter (or the whole document). */
  body: string;
}

/** The opening fence: exactly `---`, then only whitespace (pulldown-cmark's `scan_metadata_block`). */
const OPENING_FENCE = /^---[ \t\f\r]*$/;
/** A closing fence: `---` or `...`, then only spaces (`scan_closing_metadata_block`). */
const CLOSING_FENCE = /^(?:---|\.\.\.) *\r?$/;
/** A blank line (`scan_blank_line`). */
const BLANK_LINE = /^[ \t]*\r?$/;

/**
 * The index of the line closing the frontmatter that opens `lines` (a
 * document split on `\n`, a byte-order mark already removed), or -1 when the
 * document has none.
 *
 * The renderer's rule exactly (pulldown-cmark's YAML metadata block, which is
 * what mbr reads frontmatter from): `---` on the first line, closed by the
 * next `---` or `...` line, and the line after the opening fence is neither
 * blank nor a closing fence. So `---`/`---` is two rules, not an empty block,
 * and `---`, blank, `title: x`, `---` is a rule and a heading: the editor
 * shows what the page shows, and an outline cannot disagree about where the
 * body starts. CR before the `\n` is tolerated, so a CRLF document need not be
 * converted first.
 */
export function frontmatterCloseLine(lines: readonly string[]): number {
  if (lines.length < 3 || !OPENING_FENCE.test(lines[0])) return -1;
  if (BLANK_LINE.test(lines[1]) || CLOSING_FENCE.test(lines[1])) return -1;
  for (let i = 2; i < lines.length; i++) {
    if (CLOSING_FENCE.test(lines[i])) return i;
  }
  return -1;
}

/**
 * Splits a raw markdown document into its leading YAML frontmatter block and
 * the remaining body ({@link frontmatterCloseLine} decides whether there is
 * one). Without a valid leading block the whole document is returned as the
 * body (so a `---` horizontal rule in the body is never mistaken for
 * frontmatter). A leading byte-order mark is tolerated for detection.
 */
export function splitFrontmatter(raw: string): SplitResult {
  const text = raw.charCodeAt(0) === 0xfeff ? raw.slice(1) : raw;
  const lines = text.split('\n');
  const closeIdx = frontmatterCloseLine(lines);
  if (closeIdx === -1) {
    return { frontmatter: null, body: raw };
  }
  const frontmatter = lines.slice(1, closeIdx).join('\n');
  const body = lines.slice(closeIdx + 1).join('\n');
  return { frontmatter, body };
}

/**
 * Recombines a (possibly empty) frontmatter block with a body into a single
 * document: `---`, the YAML without trailing newlines, `---`. Omits the fence
 * block entirely when the frontmatter is empty.
 *
 * This is how an *edited* frontmatter is written ({@link documentToSave});
 * an unedited one is written as loaded.
 */
export function recombine(frontmatter: string | null, body: string): string {
  const fm = frontmatter && frontmatter.trim() ? frontmatter.replace(/\n+$/, '') : '';
  if (!fm) {
    return body;
  }
  return `---\n${fm}\n---\n${body}`;
}

/** A document's line ending: what a save writes between lines. */
export type LineEnding = '\n' | '\r\n';

/**
 * The line ending `raw` is written with: CRLF when most of its line breaks
 * are CRLF, else LF (also for a file with no line break at all).
 *
 * The editor works in LF — the properties editor is opened on LF YAML and
 * Crepe serializes with LF — so a CRLF file is opened as LF ({@link toLf})
 * and saved back with {@link withLineEnding}. A file mixing both is saved
 * with its majority ending: it cannot round-trip byte for byte, but it no
 * longer silently becomes all-LF.
 */
export function lineEndingOf(raw: string): LineEnding {
  let crlf = 0;
  let lf = 0;
  for (let i = raw.indexOf('\n'); i !== -1; i = raw.indexOf('\n', i + 1)) {
    if (i > 0 && raw.charCodeAt(i - 1) === 13) crlf += 1;
    else lf += 1;
  }
  return crlf > lf ? '\r\n' : '\n';
}

/** `text` with every CRLF turned into LF. */
export function toLf(text: string): string {
  return text.replace(/\r\n/g, '\n');
}

/** `text` (LF, as the editor produces it) written with `ending`. */
export function withLineEnding(text: string, ending: LineEnding): string {
  return ending === '\n' ? text : toLf(text).replace(/\n/g, '\r\n');
}

/**
 * `text` as a `<textarea>` hands it back from `.value`: every CR or CRLF a
 * line feed (HTML's "API value").
 */
export function textareaValue(text: string | null): string {
  return (text ?? '').replace(/\r\n?/g, '\n');
}

/** A file as the editor opens it: LF, split, and how to write it back. */
export interface OpenedDocument {
  /** The ending a save writes ({@link lineEndingOf} of the file as loaded). */
  lineEnding: LineEnding;
  /** The YAML the properties editor starts with, `null` without frontmatter. */
  frontmatter: string | null;
  /** The markdown body Crepe is opened on (LF, no byte-order mark, no leading blank lines). */
  body: string;
  /** Lines before the body (the frontmatter), so body line N is file line N + this. */
  bodyLineOffset: number;
  /**
   * Everything before the body exactly as loaded: a byte-order mark, the
   * fences, the YAML, blank lines at its end, the blank lines separating it
   * from the body, its line endings. A save that leaves the frontmatter alone
   * writes this back unchanged.
   */
  head: string;
  /** The blank lines between the frontmatter (or the file's start) and the body, in LF. */
  separator: string;
}

/** The blank lines a text starts with (CRLF or LF). */
const BLANK_LINE_AT_START = /^(?:[ \t]*\r?\n)+/;

/**
 * Opens `raw` the way the editor does: in LF, split into frontmatter and body.
 * The one derivation shared by `openEditor`, the tests and the
 * `roundtrip-check` tool, so none of them can disagree about where the body
 * starts or which ending a save writes.
 *
 * Blank lines before the body go into {@link OpenedDocument.head}: Crepe
 * never prints a document starting with one, so the body it is opened on must
 * not start with one either. So does a byte-order mark. (`fetch(…).text()`
 * removes one before the editor sees the file, so in the editor that only
 * matters to callers that pass one in.)
 */
export function openDocument(raw: string): OpenedDocument {
  const lineEnding = lineEndingOf(raw);
  const bom = raw.charCodeAt(0) === 0xfeff ? '\uFEFF' : '';
  const lines = raw.slice(bom.length).split('\n');
  const closeIdx = frontmatterCloseLine(lines);
  const afterFrontmatter = closeIdx === -1 ? raw.slice(bom.length) : lines.slice(closeIdx + 1).join('\n');
  const rawSeparator = BLANK_LINE_AT_START.exec(afterFrontmatter)?.[0] ?? '';
  const rawBody = afterFrontmatter.slice(rawSeparator.length);
  const separator = toLf(rawSeparator);
  const head = raw.slice(0, raw.length - rawBody.length);
  return {
    lineEnding,
    frontmatter: closeIdx === -1 ? null : toLf(`${lines.slice(1, closeIdx).join('\n')}\n`).slice(0, -1),
    body: toLf(rawBody),
    bodyLineOffset: head.split('\n').length - 1,
    head,
    separator,
  };
}

/** Whether `yaml` (the properties editor's text) is the frontmatter `doc` was opened with. */
export function frontmatterUnchanged(doc: OpenedDocument, yaml: string): boolean {
  return textareaValue(yaml) === textareaValue(doc.frontmatter);
}

/** The text before the body when the frontmatter is `yaml` (edited): {@link recombine}'s form. */
function editedHead(doc: OpenedDocument, yaml: string): string {
  const bom = doc.head.charCodeAt(0) === 0xfeff ? '\uFEFF' : '';
  // Cleared frontmatter takes its separator with it: a file does not start with blank lines.
  const separator = yaml.trim() ? doc.separator : '';
  return bom + withLineEnding(recombine(yaml, separator), doc.lineEnding);
}

/**
 * The file a save writes for `doc`: the properties editor's `yaml` and the
 * body, written with the file's own line ending.
 *
 * Frontmatter left alone is written back byte for byte as loaded
 * ({@link OpenedDocument.head}: fences, blank lines at its end and after it,
 * its line endings). Edited frontmatter is {@link recombine}d into the
 * normalized `---` form, which {@link frontmatterRewrites} names at open; the
 * blank lines separating it from the body are kept. A byte-order mark is kept
 * either way. The body ends as the loaded one did ({@link withAuthoredEnd}).
 */
export function documentToSave(doc: OpenedDocument, yaml: string, body: string): string {
  const head = frontmatterUnchanged(doc, yaml) ? doc.head : editedHead(doc, yaml);
  const ended = withAuthoredEnd(withAuthoredBlankLines(body, doc.body), doc.body);
  return head + withLineEnding(ended, doc.lineEnding);
}

/**
 * Calls `visit(index, key)` for each blank line of `lines`: its non-blank
 * neighbours and place in its run. The empty string after a final line break
 * is not a line: filling it with a whitespace-only last line took the line
 * break away (`p⏎ ⏎` saved as `p⏎ `).
 */
function eachBlankLine(lines: readonly string[], visit: (index: number, key: string) => void): void {
  let before = '\u0001start';
  let run: number[] = [];
  const flush = (after: string) => {
    run.forEach((index, k) => visit(index, `${before}\u0000${after}\u0000${k}`));
    run = [];
  };
  const count = lines.length > 1 && lines[lines.length - 1] === '' ? lines.length - 1 : lines.length;
  lines.slice(0, count).forEach((line, index) => {
    if (line.trim() === '') {
      run.push(index);
    } else {
      flush(line);
      before = line;
    }
  });
  flush('\u0001end');
}

/**
 * `printed` (a serialization, LF) with the whitespace of `original`'s
 * whitespace-only lines put back: a printed empty line between the same two
 * non-blank lines, at the same place among the blank lines there, as a
 * whitespace-only line of the loaded body becomes that line again. Either is
 * a blank line to markdown; the printer writes every blank line empty. A
 * blank line whose neighbours were edited stays empty.
 */
export function withAuthoredBlankLines(printed: string, original: string): string {
  if (!/^[ \t]+$/m.test(original)) return printed;
  const source = original.split('\n');
  const wanted = new Map<string, string[]>();
  eachBlankLine(source, (index, key) => {
    if (source[index] === '') return;
    const queue = wanted.get(key) ?? [];
    queue.push(source[index]);
    wanted.set(key, queue);
  });
  if (wanted.size === 0) return printed;
  const out = printed.split('\n');
  eachBlankLine(out, (index, key) => {
    const whitespace = out[index] === '' ? wanted.get(key)?.shift() : undefined;
    if (whitespace !== undefined) out[index] = whitespace;
  });
  return out.join('\n');
}

/**
 * `body`, a serialization ending in one line break (the printer's), ending
 * the way `original` (the body as loaded, LF) did: without a final line
 * break, or with the blank lines after its last line. A body ending any
 * other way (the safety net already kept the loaded end) is left alone.
 */
export function withAuthoredEnd(body: string, original: string): string {
  if (!body.endsWith('\n') || body.endsWith('\n\n') || original === '') return body;
  const end = /\n[ \t\n]*$/.exec(original)?.[0];
  return body.slice(0, -1) + (end ?? '');
}

/**
 * What editing `doc`'s frontmatter would change besides the edit itself —
 * {@link recombine}'s normalization of the block as loaded — in words, for the
 * open-time notice. Empty when an edit writes the block exactly as it was
 * spelled (the usual case) or there is no frontmatter.
 */
export function frontmatterRewrites(doc: OpenedDocument): string[] {
  if (doc.frontmatter === null) return [];
  const edited = editedHead(doc, doc.frontmatter);
  // A closing fence ending the file gains a line break, which is no rewrite worth naming.
  if (doc.head === edited || (doc.body === '' && doc.head === edited.replace(/\r?\n$/, ''))) return [];
  const reasons: string[] = [];
  const lines = toLf(doc.head.replace(/^\uFEFF/, '')).split('\n');
  const closer = lines[doc.frontmatter.split('\n').length + 1];
  if (lines[0] !== '---' || closer !== '---') reasons.push('its fences are written `---`');
  if (doc.frontmatter.endsWith('\n')) reasons.push('blank lines at its end are dropped');
  if (reasons.length === 0) reasons.push('its line endings are made uniform');
  return reasons;
}
