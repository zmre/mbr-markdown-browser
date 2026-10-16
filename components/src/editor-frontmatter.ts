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

/**
 * Splits a raw markdown document into its leading YAML frontmatter block and
 * the remaining body.
 *
 * A frontmatter block must start on the very first line with a `---` fence and
 * be terminated by a later `---` (or `...`) fence line. If there is no valid
 * leading block, the whole document is returned as the body (so a `---`
 * horizontal rule in the body is never mistaken for frontmatter).
 */
export function splitFrontmatter(raw: string): SplitResult {
  // Tolerate a leading BOM for detection.
  const text = raw.charCodeAt(0) === 0xfeff ? raw.slice(1) : raw;
  const lines = text.split('\n');

  if (lines.length === 0 || lines[0].trimEnd() !== '---') {
    return { frontmatter: null, body: raw };
  }

  let closeIdx = -1;
  for (let i = 1; i < lines.length; i++) {
    const trimmed = lines[i].trimEnd();
    if (trimmed === '---' || trimmed === '...') {
      closeIdx = i;
      break;
    }
  }
  if (closeIdx === -1) {
    return { frontmatter: null, body: raw };
  }

  const frontmatter = lines.slice(1, closeIdx).join('\n');
  const body = lines.slice(closeIdx + 1).join('\n');
  return { frontmatter, body };
}

/**
 * Recombines a (possibly empty) frontmatter block with a body into a single
 * document. Omits the fence block entirely when the frontmatter is empty.
 *
 * `recombine(splitFrontmatter(x))` round-trips for LF-newline documents.
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
 * The editor works in LF — the frontmatter textarea normalizes to LF and
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

/** A file as the editor opens it: LF, split, and how to write it back. */
export interface OpenedDocument {
  /** The ending a save writes ({@link lineEndingOf} of the file as loaded). */
  lineEnding: LineEnding;
  /** The YAML the frontmatter textarea starts with, `null` without frontmatter. */
  frontmatter: string | null;
  /** The markdown body Crepe is opened on (LF). */
  body: string;
  /** Lines before the body (the frontmatter), so body line N is file line N + this. */
  bodyLineOffset: number;
}

/**
 * Opens `raw` the way the editor does: in LF, split into frontmatter and body.
 * The one derivation shared by `openEditor`, the tests and the
 * `roundtrip-check` tool, so none of them can disagree about where the body
 * starts or which ending a save writes.
 */
export function openDocument(raw: string): OpenedDocument {
  const lineEnding = lineEndingOf(raw);
  const lf = toLf(raw);
  const { frontmatter, body } = splitFrontmatter(lf);
  const bodyLineOffset = lf.slice(0, lf.length - body.length).split('\n').length - 1;
  return { lineEnding, frontmatter, body, bodyLineOffset };
}

/**
 * The file a save writes for `doc`: the frontmatter textarea's `yaml` and the
 * body, recombined and written with the file's own line ending.
 */
export function documentToSave(doc: OpenedDocument, yaml: string, body: string): string {
  return withLineEnding(recombine(yaml, body), doc.lineEnding);
}
