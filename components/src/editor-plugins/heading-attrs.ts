/**
 * Heading attributes — `# Title {#id .class}` — for the editor (TODO "Editor:
 * cover mbr's markdown extensions", inventory item 5).
 *
 * The bytes already round-trip: the block is heading text to remark and is
 * printed back as it was typed (neither `{` nor a mid-line `#` is escaped).
 * What the editor lacked is the *structure* — the renderer gives the heading
 * an `id` and `classes` (pulldown-cmark's `ENABLE_HEADING_ATTRIBUTES`) — so
 * this remark transform reads the trailing block with pulldown-cmark's own
 * rules and records it as `heading.data.id` / `heading.data.classes`, the
 * fields the parity outline compares (`editor-outline.ts`). The text is left
 * alone, so the save stays verbatim.
 */
import type { MilkdownPlugin } from '@milkdown/kit/ctx';
import { $remark } from '@milkdown/kit/utils';
import type { MdNode } from './remark-support.js';

/** A heading's attributes as pulldown-cmark parses them. */
export interface HeadingAttributes {
  id?: string;
  classes: string[];
}

/** Bytes pulldown-cmark refuses inside a heading attribute block. */
const NOT_IN_BLOCK = new Set(['{', '}', '<', '>', '\\', '\n', '\r']);

/**
 * The trailing attribute block of heading text, or `null` — pulldown-cmark's
 * `extract_attribute_block_content_from_header_text` +
 * `parse_inside_attribute_block`: after trailing whitespace the text must end
 * in `}`, the matching `{` is the nearest one before it with none of
 * `{ } < > \` or a line break in between, and the content splits on ASCII
 * whitespace into `#id` (last wins) and `.class` tokens; a one-character token
 * and `key=value` pairs give neither.
 */
export function parseHeadingAttributes(content: string): HeadingAttributes | null {
  const trimmed = content.replace(/[\n\r \t]+$/, '');
  if (!trimmed.endsWith('}')) return null;
  let open = trimmed.length - 2;
  while (open >= 0 && !NOT_IN_BLOCK.has(trimmed[open])) open--;
  if (open < 0 || trimmed[open] !== '{') return null;
  const result: HeadingAttributes = { classes: [] };
  for (const token of trimmed.slice(open + 1, -1).split(/[\t\n\f\r ]+/)) {
    if (token.length <= 1) continue;
    if (token[0] === '#') result.id = token.slice(1);
    else if (token[0] === '.') result.classes.push(token.slice(1));
  }
  return result;
}

/**
 * The text pulldown-cmark looks for the block in, given a heading's source:
 * an ATX heading's line after its `#`s, or a setext heading's content lines
 * (the underline excluded).
 */
export function headingContent(source: string): string {
  const atx = /^[ \t]*#{1,6}(?:[ \t]+|$)/.exec(source);
  if (atx) {
    const rest = source.slice(atx[0].length);
    const eol = rest.search(/\r?\n/);
    return eol === -1 ? rest : rest.slice(0, eol);
  }
  const lastBreak = source.search(/\r?\n[^\n]*$/);
  return lastBreak === -1 ? source : source.slice(0, lastBreak);
}

/** Records the attributes of every heading in `tree` as `data.id` / `data.classes`. */
export function annotateHeadingAttributes(tree: MdNode, source: string): void {
  for (const child of tree.children ?? []) {
    const start = child.position?.start.offset;
    const end = child.position?.end.offset;
    if (child.type === 'heading' && start !== undefined && end !== undefined) {
      const attrs = parseHeadingAttributes(headingContent(source.slice(start, end)));
      if (attrs) {
        child.data = { ...child.data, classes: attrs.classes };
        if (attrs.id !== undefined) child.data.id = attrs.id;
      }
    } else {
      annotateHeadingAttributes(child, source);
    }
  }
}

const headingAttrsRemark = $remark('mbrHeadingAttrs', () => () => (tree: unknown, file: { value?: unknown }) => {
  annotateHeadingAttributes(tree as MdNode, String(file.value ?? ''));
});

/** Everything heading attributes register: `MARKDOWN_EXTENSION_PLUGINS` lists this. */
export const headingAttrsPlugin: MilkdownPlugin[] = [...headingAttrsRemark];
