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
