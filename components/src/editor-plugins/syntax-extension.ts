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
import type { Extension as FromMarkdownExtension } from 'mdast-util-from-markdown';
import type { Options as ToMarkdownExtension } from 'mdast-util-to-markdown';
import type { Extension as MicromarkExtension } from 'micromark-util-types';

/** The parts of a syntax extension; each is optional. */
export interface SyntaxExtension {
  micromark?: MicromarkExtension;
  fromMarkdown?: FromMarkdownExtension;
  toMarkdown?: ToMarkdownExtension;
}

/** The processor data keys remark-parse / remark-stringify read. */
interface ProcessorData {
  micromarkExtensions?: MicromarkExtension[];
  fromMarkdownExtensions?: FromMarkdownExtension[];
  toMarkdownExtensions?: ToMarkdownExtension[];
}

/** A Milkdown plugin installing `extension` on the editor's remark processor. */
export function $syntax<Id extends string>(id: Id, extension: SyntaxExtension) {
  return $remark(id, () =>
    function attach(this: { data(): unknown }) {
      const data = this.data() as ProcessorData;
      if (extension.micromark) (data.micromarkExtensions ??= []).push(extension.micromark);
      if (extension.fromMarkdown) (data.fromMarkdownExtensions ??= []).push(extension.fromMarkdown);
      if (extension.toMarkdown) (data.toMarkdownExtensions ??= []).push(extension.toMarkdown);
    },
  );
}
