/**
 * The small slice of the unified/mdast API the editor plugins use, typed
 * structurally so the plugins need no direct dependency on the (transitive)
 * `unified`, `micromark-util-types` or `mdast-util-*` packages.
 */

/** An mdast node, as loosely as the plugins read and build them. */
export interface MdNode {
  type: string;
  children?: MdNode[];
  value?: string;
  position?: { start: { offset?: number }; end: { offset?: number } };
  data?: Record<string, unknown>;
  [field: string]: unknown;
}

/** Syntax extensions a remark plugin adds, by the `data()` key each goes to. */
export interface RemarkExtensions {
  /** A micromark syntax extension (`micromarkExtensions`). */
  micromark?: unknown;
  /** An `mdast-util-from-markdown` extension (`fromMarkdownExtensions`). */
  fromMarkdown?: unknown;
  /** An `mdast-util-to-markdown` extension (`toMarkdownExtensions`). */
  toMarkdown?: unknown;
}

interface DataHolder {
  data: () => Record<string, unknown[] | undefined>;
}

/**
 * Registers `extensions` on a unified processor — `this` inside a remark
 * plugin — the way `remark-gfm` and friends do.
 */
export function addRemarkExtensions(processor: unknown, extensions: RemarkExtensions): void {
  const data = (processor as DataHolder).data();
  const push = (key: string, value: unknown): void => {
    if (value === undefined) return;
    const list = data[key] ?? (data[key] = []);
    list.push(value);
  };
  push('micromarkExtensions', extensions.micromark);
  push('fromMarkdownExtensions', extensions.fromMarkdown);
  push('toMarkdownExtensions', extensions.toMarkdown);
}

/**
 * Marks a lazy continuation line of a paragraph (recorded by `authored-style.ts`) in a list item or quote
 * (`- item` then `more` unindented): the line as typed, which the item and
 * quote printers leave without their prefix. The `root` printer
 * (`minimal-escapes.ts`) removes the mark, so it never reaches any output.
 */
export const LAZY_LINE = '\u0000mbr-lazy\u0000';

/** `text` without {@link LAZY_LINE} marks. */
export function withoutLazyMarks(text: string): string {
  return text.includes(LAZY_LINE) ? text.split(LAZY_LINE).join('') : text;
}
