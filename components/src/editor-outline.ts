/**
 * The editor's block outline of a document, built from the mdast the editor
 * parses (remark with every plugin Milkdown/Crepe registers), in the format
 * mbr-core's `editor_parity` test writes for the renderer
 * (`tests/fixtures/editor-roundtrip/<name>.outline.json`). The parity test
 * compares the two, so the editor and the renderer cannot disagree about where
 * a card, a definition or a list item ends without a test failing.
 *
 * # The format (same text as `crates/mbr-core/src/editor_parity.rs`)
 *
 * A pre-order array of block nodes. Every node has `depth` (0 = top level),
 * `kind`, and `lines` — `[first, last]`, 1-based, inclusive, whole-file
 * (frontmatter lines count): the lines of the first and last non-whitespace
 * characters of the node's source range. Then only the fields its kind
 * defines, never `null`:
 *
 * | kind            | fields                                             |
 * |-----------------|----------------------------------------------------|
 * | `heading`       | `level`; `id`, `classes` from `{#id .class}`       |
 * | `paragraph`     | —                                                  |
 * | `blockquote`    | `alert` for a GitHub alert                         |
 * | `list`          | `ordered`, `start` (ordered only), `tight`         |
 * | `item`          | `task`: `" "` `"x"` `"-"` `">"`, on task items only |
 * | `dl` `dt` `dd`  | `dl`: `tight`                                      |
 * | `code`          | `fenced`, `lang` (first word of the info string)   |
 * | `table`         | `align` (`none`/`left`/`center`/`right`), `rows`   |
 * | `rule`          | —                                                  |
 * | `section_break` | — (`--- {attrs}` on a paragraph of its own)        |
 * | `html`          | — (a block of raw HTML)                            |
 * | `footnote`      | `label`                                            |
 *
 * Inline content is never descended into: paragraphs, headings, terms, table
 * cells, code and html are leaves. Link reference definitions are not nodes,
 * and Milkdown's paragraph-wrapped HTML block counts as `html`. An mdast type with no mapping becomes
 * `mdast:<type>`, so a plugin that introduces a node the outline does not know
 * fails loudly instead of vanishing.
 *
 * # What an editor plugin must put on its mdast nodes
 *
 * This is the contract the TODO "Editor: cover mbr's markdown extensions"
 * plugins implement against; the field names are the ones those plugins are
 * expected to use, so the outline needs no change when they land:
 *
 * - definition lists: `defList` / `defListTerm` / `defListDescription`
 *   (the `mdast-util-definition-list` names), `spread` as on lists.
 * - task items: `listItem.status` = the marker character (`' '`, `'x'`, `'-'`,
 *   `'>'`); without it, GFM's `checked` is read (`true` → `"x"`).
 * - section attributes: a `sectionBreak` node.
 * - heading attributes: `heading.data.id` / `heading.data.classes`.
 * - GitHub alerts: `blockquote.data.alert`.
 */

/** One node of the outline (see the module docs). */
export interface OutlineNode {
  depth: number;
  kind: string;
  lines: [number, number];
  level?: number;
  id?: string;
  classes?: string[];
  alert?: string;
  ordered?: boolean;
  start?: number;
  tight?: boolean;
  task?: string;
  fenced?: boolean;
  lang?: string;
  align?: string[];
  rows?: number;
  label?: string;
}

/** The slice of an mdast node the outline reads. */
export interface MdastNode {
  type: string;
  children?: MdastNode[];
  position?: { start: { offset?: number }; end: { offset?: number } };
  data?: { id?: unknown; classes?: unknown; alert?: unknown };
  depth?: number;
  ordered?: boolean | null;
  start?: number | null;
  spread?: boolean | null;
  checked?: boolean | null;
  status?: unknown;
  lang?: string | null;
  align?: (string | null)[] | null;
  label?: string | null;
  identifier?: string;
}

/** Flow containers whose children are blocks and are outlined too. */
const FLOW_CONTAINERS = new Set([
  'root',
  'blockquote',
  'list',
  'listItem',
  'footnoteDefinition',
  'defList',
  'defListDescription',
]);

/** mdast-util-to-hast's rule for a loose list, which is how the editor renders. */
function isLoose(list: MdastNode): boolean {
  if (list.spread) return true;
  return (list.children ?? []).some((item) =>
    item.spread === null || item.spread === undefined ? (item.children?.length ?? 0) > 1 : item.spread,
  );
}

function taskMarker(item: MdastNode): string | undefined {
  if (typeof item.status === 'string') return item.status;
  if (item.checked === true) return 'x';
  if (item.checked === false) return ' ';
  return undefined;
}

/**
 * The outline of `body` as parsed into `tree`. `lineOffset` is the number of
 * file lines before the body (the frontmatter the editor split off), so line
 * numbers are whole-file like the renderer's.
 */
export function mdastOutline(tree: MdastNode, body: string, lineOffset: number): OutlineNode[] {
  const newlines: number[] = [];
  for (let i = body.indexOf('\n'); i !== -1; i = body.indexOf('\n', i + 1)) newlines.push(i);
  // 1-based line of `offset`: newlines strictly before it, plus one.
  const lineOf = (offset: number): number => {
    let lo = 0;
    let hi = newlines.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (newlines[mid] < offset) lo = mid + 1;
      else hi = mid;
    }
    return lo + 1 + lineOffset;
  };
  const span = (start: number, end: number): [number, number] => {
    const text = body.slice(start, end);
    const content = text.trim();
    if (!content) return [lineOf(start), lineOf(start)];
    const first = start + (text.length - text.trimStart().length);
    return [lineOf(first), lineOf(first + content.length - 1)];
  };

  const out: OutlineNode[] = [];
  const visit = (node: MdastNode, depth: number): void => {
    const start = node.position?.start.offset;
    const end = node.position?.end.offset;
    // A node with no position was synthesized by a transform; it has no
    // source to compare. A link reference definition is not a block to the
    // renderer (pulldown-cmark consumes it), so it is not one here either
    // (`mbrDefinition`: the editor's name for it, `editor-plugins/references.ts`).
    if (start === undefined || end === undefined || node.type === 'definition' || node.type === 'mbrDefinition') return;
    const base = { depth, kind: '', lines: span(start, end) };
    const entry: OutlineNode = describe(node, base, body.slice(start, end));
    out.push(entry);
    if (FLOW_CONTAINERS.has(node.type)) {
      for (const child of node.children ?? []) visit(child, depth + 1);
    }
  };
  for (const child of tree.children ?? []) visit(child, 0);
  return out;
}

/**
 * Milkdown has no block HTML node: its `remarkHtmlTransformer` turns a block
 * `html` node into a paragraph holding a copy of it, same position. That is
 * the editor's representation of an HTML block, not a structural
 * disagreement (it serializes back verbatim), so it outlines as `html`.
 */
function isWrappedHtmlBlock(paragraph: MdastNode): boolean {
  const only = paragraph.children?.length === 1 ? paragraph.children[0] : undefined;
  return (
    only?.type === 'html' &&
    only.position?.start.offset === paragraph.position?.start.offset &&
    only.position?.end.offset === paragraph.position?.end.offset
  );
}

function describe(node: MdastNode, base: OutlineNode, source: string): OutlineNode {
  switch (node.type) {
    case 'heading': {
      const entry: OutlineNode = { ...base, kind: 'heading', level: node.depth };
      if (typeof node.data?.id === 'string') entry.id = node.data.id;
      if (Array.isArray(node.data?.classes) && node.data.classes.length > 0) {
        entry.classes = node.data.classes.map(String);
      }
      return entry;
    }
    case 'paragraph':
      return isWrappedHtmlBlock(node) ? { ...base, kind: 'html' } : { ...base, kind: 'paragraph' };
    case 'blockquote':
      return typeof node.data?.alert === 'string'
        ? { ...base, kind: 'blockquote', alert: node.data.alert }
        : { ...base, kind: 'blockquote' };
    case 'list':
      return node.ordered
        ? { ...base, kind: 'list', ordered: true, start: node.start ?? 1, tight: !isLoose(node) }
        : { ...base, kind: 'list', ordered: false, tight: !isLoose(node) };
    case 'listItem': {
      const task = taskMarker(node);
      return task === undefined ? { ...base, kind: 'item' } : { ...base, kind: 'item', task };
    }
    case 'defList':
      return { ...base, kind: 'dl', tight: !isLoose(node) };
    case 'defListTerm':
      return { ...base, kind: 'dt' };
    case 'defListDescription':
      return { ...base, kind: 'dd' };
    case 'code': {
      const fence = source.trimStart();
      const entry: OutlineNode = {
        ...base,
        kind: 'code',
        fenced: fence.startsWith('```') || fence.startsWith('~~~'),
      };
      if (node.lang) entry.lang = node.lang;
      return entry;
    }
    case 'table':
      return {
        ...base,
        kind: 'table',
        align: (node.align ?? []).map((a) => a ?? 'none'),
        rows: node.children?.length ?? 0,
      };
    case 'thematicBreak':
      return { ...base, kind: 'rule' };
    case 'sectionBreak':
      return { ...base, kind: 'section_break' };
    case 'html':
      return { ...base, kind: 'html' };
    case 'footnoteDefinition':
      return { ...base, kind: 'footnote', label: node.label ?? node.identifier ?? '' };
    default:
      return { ...base, kind: `mdast:${node.type}` };
  }
}
