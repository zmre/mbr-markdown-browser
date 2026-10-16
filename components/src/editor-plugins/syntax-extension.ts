/**
 * Registers a markdown syntax extension on the editor's remark processor —
 * the micromark tokenizer (parse), the mdast-util-from-markdown handlers
 * (tokens → mdast) and the mdast-util-to-markdown handlers (mdast → text) —
 * the same three hooks `remark-gfm` uses, wrapped as a Milkdown `$remark`
 * plugin so it is installed with `crepe.editor.use(...)`.
 *
 * remark-parse and remark-stringify read these arrays from the processor's
 * data on every parse/serialize, so registering at attach time is enough.
 */
import { $remark } from '@milkdown/kit/utils';
import { addRemarkExtensions } from './remark-support.js';
import type { Extension as FromMarkdownExtension } from 'mdast-util-from-markdown';
import type { Options as ToMarkdownExtension } from 'mdast-util-to-markdown';
import type { Extension as MicromarkExtension } from 'micromark-util-types';

/** The parts of a syntax extension; each is optional. */
export interface SyntaxExtension {
  micromark?: MicromarkExtension;
  fromMarkdown?: FromMarkdownExtension;
  toMarkdown?: ToMarkdownExtension;
}

/**
 * A Milkdown plugin installing `extension` on the editor's remark processor
 * (`addRemarkExtensions`; plugins that also transform the tree call that
 * from their own `$remark`).
 */
export function $syntax<Id extends string>(id: Id, extension: SyntaxExtension) {
  return $remark(id, () =>
    function attach(this: unknown) {
      addRemarkExtensions(this, extension);
    },
  );
}
