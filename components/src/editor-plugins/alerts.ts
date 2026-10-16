/**
 * GitHub alerts in the editor: `> [!NOTE]`, `[!TIP]`, `[!IMPORTANT]`,
 * `[!WARNING]`, `[!CAUTION]` — the five kinds pulldown-cmark's `ENABLE_GFM`
 * recognizes and `html.rs` renders as `blockquote.markdown-alert-<kind>`.
 *
 * Without this, the tag line is the start of the blockquote's first paragraph
 * to remark, and remark escapes a leading `[` on save: `> \[!NOTE]`, which the
 * renderer no longer treats as an alert. Here:
 *
 * 1. **mdast** ({@link markAlerts}, a remark transform): each blockquote's
 *    opening line is checked against pulldown-cmark's own rule
 *    (`scan_blockquote_tag`: right after the `>` and at most one space,
 *    `[!kind]` case-insensitively, then only whitespace to the end of the
 *    line, on the line that *opens* the blockquote). A match sets
 *    `blockquote.data.alert` (the lowercase kind — what `editor-outline.ts`
 *    and the parity goldens compare) and `data.alertLine` (the line as
 *    authored after the `>`, so `>[!note]` stays `>[!note]`), and the tag line
 *    is taken out of the first paragraph, which then starts where the
 *    renderer's does.
 * 2. **Markdown out** ({@link alertToMarkdown}): a `blockquote` handler that
 *    prints the tag line back in front of the body.
 * 3. **ProseMirror** ({@link alertBlockquoteSchema}): `blockquote` gains
 *    `alert`/`alertLine` attrs and renders as a callout in the page theme's
 *    alert colours. Typing `[!note] ` at the start of a plain blockquote makes
 *    it an alert ({@link alertInputRule}); Backspace at the start of an
 *    alert's first block makes it a plain blockquote again
 *    ({@link alertBackspace}).
 */
import type { Blockquote, Paragraph, Parents, PhrasingContent, Root } from 'mdast';
import { defaultHandlers, type Info, type Options as ToMarkdownExtension, type State } from 'mdast-util-to-markdown';
import type { Processor } from 'unified';
import type { VFile } from 'vfile';
import { blockquoteSchema } from '@milkdown/kit/preset/commonmark';
import type { Node as ProseNode } from '@milkdown/kit/prose/model';
import { InputRule } from '@milkdown/kit/prose/inputrules';
import { Plugin } from '@milkdown/kit/prose/state';
import { $inputRule, $prose, $remark } from '@milkdown/kit/utils';
import { injectEditorCss } from './inject-css.js';

/** The alert kinds pulldown-cmark knows, lowercase. */
export const ALERT_KINDS = ['note', 'tip', 'important', 'warning', 'caution'] as const;
export type AlertKind = (typeof ALERT_KINDS)[number];

const KIND_PATTERN = ALERT_KINDS.join('|');

/**
 * A blockquote's opening line, from its `>` (after any indentation) to the end
 * of the line: `>`, at most one space, `[!kind]`, trailing spaces or tabs.
 * Group 1 is everything after the `>`, group 2 the kind as written.
 */
const OPENING_LINE = new RegExp(`^[ \\t]*>( ?\\[!(${KIND_PATTERN})\\][ \\t]*)\\r?(?:\\n|$)`, 'i');

/** The kind of a tag spelling, `undefined` if it is not one. */
export function alertKind(name: string): AlertKind | undefined {
  const lower = name.toLowerCase();
  return (ALERT_KINDS as readonly string[]).includes(lower) ? (lower as AlertKind) : undefined;
}

/** Recognizes an alert's opening line. `source` starts at the blockquote. */
export function parseAlertLine(source: string): { kind: AlertKind; line: string } | undefined {
  const match = OPENING_LINE.exec(source);
  const kind = match ? alertKind(match[2]) : undefined;
  return match && kind ? { kind, line: match[1] } : undefined;
}

/**
 * The text to print after `>` for an alert: the authored line while it still
 * names the same kind, else the canonical ` [!KIND]`.
 */
export function alertLineFor(kind: AlertKind, authored: unknown): string {
  if (typeof authored === 'string') {
    const parsed = parseAlertLine(`>${authored}`);
    if (parsed?.kind === kind && parsed.line === authored) return authored;
  }
  return ` [!${kind.toUpperCase()}]`;
}

// ============================================================================
// 1. mdast
// ============================================================================

/** `blockquote.data` as this plugin fills it. */
export interface AlertData {
  alert?: AlertKind;
  alertLine?: string;
}

type AlertBlockquote = Blockquote & { data?: Blockquote['data'] & AlertData };

/**
 * Removes the tag line from the front of `paragraph`. Returns false (and
 * changes nothing) when the paragraph does not start with it, which can only
 * mean remark and pulldown-cmark disagree about the line — then it is not
 * treated as an alert at all. `nextLine` is the offset of the second source
 * line's content, the paragraph's new start.
 */
function stripTagLine(
  quote: Blockquote,
  paragraph: Paragraph,
  tag: string,
  nextLine: { line: number; column: number; offset: number } | undefined,
): boolean {
  const [head, second] = paragraph.children;
  if (head?.type !== 'text' || !head.value.startsWith(tag)) return false;
  const rest = head.value.slice(tag.length);
  const lineBreak = /^[ \t]*\r?\n/.exec(rest);
  let remaining: PhrasingContent[];
  if (lineBreak) {
    // `[!NOTE]\nbody…`: the body continues in the same text node.
    remaining = [{ ...head, value: rest.slice(lineBreak[0].length) }, ...paragraph.children.slice(1)];
    if (remaining[0].position && nextLine) remaining[0].position = { ...remaining[0].position, start: nextLine };
  } else if (/^[ \t]*$/.test(rest) && (second === undefined || second.type === 'break')) {
    // `[!NOTE]` alone in its text node: the paragraph ends there, or a hard
    // break (trailing spaces after the tag) ends the line.
    remaining = paragraph.children.slice(second ? 2 : 1);
  } else {
    return false;
  }
  if (remaining.length === 0 || (remaining.length === 1 && remaining[0].type === 'text' && !remaining[0].value)) {
    quote.children.splice(quote.children.indexOf(paragraph), 1);
    return true;
  }
  paragraph.children = remaining;
  const start = remaining[0].position?.start ?? nextLine;
  if (paragraph.position && start) paragraph.position = { ...paragraph.position, start };
  return true;
}

/**
 * Where the second line of `source` (a blockquote starting at `base`) has its
 * first content character, past indentation and `>` markers: where the
 * renderer's first paragraph starts.
 */
function secondLineStart(
  source: string,
  base: { line: number; offset?: number },
): { line: number; column: number; offset: number } | undefined {
  const newline = source.indexOf('\n');
  if (newline === -1 || base.offset === undefined) return undefined;
  const lineStart = newline + 1;
  const skipped = /^[ \t>]*/.exec(source.slice(lineStart))?.[0].length ?? 0;
  return { line: base.line + 1, column: skipped + 1, offset: base.offset + lineStart + skipped };
}

/**
 * Marks the alert in `quote` (see the module docs), given the document
 * `source` its positions index into.
 */
function markAlert(quote: AlertBlockquote, source: string): void {
  const start = quote.position?.start;
  const end = quote.position?.end.offset;
  if (start?.offset === undefined || end === undefined) return;
  const text = source.slice(start.offset, end);
  const alert = parseAlertLine(text);
  const first = quote.children[0];
  if (!alert || first?.type !== 'paragraph') return;
  if (stripTagLine(quote, first, alert.line.trim(), secondLineStart(text, start))) {
    quote.data = { ...quote.data, alert: alert.kind, alertLine: alert.line };
  }
}

/**
 * The transform: every blockquote, at any depth. It needs the source (the
 * parsed file's value, which Milkdown's parser hands to `runSync`) because
 * the tag line's exact spelling is not in the tree; without it nothing is
 * marked and the document is no worse off than without this plugin.
 *
 * A tree transform rather than a from-markdown exit handler because a
 * blockquote's source spans container prefixes, which micromark's
 * `sliceSerialize` refuses to slice.
 */
export function markAlerts(tree: Root, source: string): void {
  const visit = (node: Parents): void => {
    for (const child of node.children) {
      if (child.type === 'blockquote') markAlert(child, source);
      if ('children' in child) visit(child);
    }
  };
  visit(tree);
}

// ============================================================================
// 2. Markdown out
// ============================================================================

/** The default `blockquote` handler, with the tag line in front for an alert. */
function blockquoteWithAlert(node: AlertBlockquote, parent: Parents | undefined, state: State, info: Info): string {
  const kind = node.data?.alert;
  if (!kind) return defaultHandlers.blockquote(node, parent, state, info);
  const head = `>${alertLineFor(kind, node.data?.alertLine)}`;
  // Milkdown's serializer omits `children` on a node it pushed none into.
  if (!node.children?.length) return head;
  return `${head}\n${defaultHandlers.blockquote(node, parent, state, info)}`;
}

export function alertToMarkdown(): ToMarkdownExtension {
  return { handlers: { blockquote: blockquoteWithAlert } };
}

/** The unified plugin. */
export function remarkAlerts(this: Processor): (tree: Root, file: VFile) => void {
  const data = this.data();
  (data.toMarkdownExtensions ??= []).push(alertToMarkdown());
  return (tree, file) => {
    if (typeof file.value === 'string') markAlerts(tree, file.value);
  };
}

export const remarkAlertsPlugin = $remark('mbrAlerts', () => remarkAlerts);

// ============================================================================
// 3. ProseMirror
// ============================================================================

const ALERT_LABELS: Record<AlertKind, string> = {
  note: 'Note',
  tip: 'Tip',
  important: 'Important',
  warning: 'Warning',
  caution: 'Caution',
};

/** Theme colours from theme.css's `--mbr-alert-<kind>-*` (fallbacks for an older custom theme). */
const ALERTS_CSS = `
${ALERT_KINDS.map(
  (kind) => `.milkdown .ProseMirror blockquote[data-mbr-alert='${kind}'] {
  --mbr-editor-alert-bg: var(--mbr-alert-${kind}-bg, transparent);
  --mbr-editor-alert-border: var(--mbr-alert-${kind}-border, currentColor);
  --mbr-editor-alert-color: var(--mbr-alert-${kind}-icon-color, currentColor);
}`,
).join('\n')}
.milkdown .ProseMirror blockquote[data-mbr-alert] {
  background: var(--mbr-editor-alert-bg);
  border-radius: 4px;
  padding-top: 1.8em;
  padding-bottom: 0.25em;
  margin: 8px 0;
}
.milkdown .ProseMirror blockquote[data-mbr-alert]::before {
  background: var(--mbr-editor-alert-border);
}
.milkdown .ProseMirror blockquote[data-mbr-alert]::after {
  content: attr(data-mbr-alert-label);
  position: absolute;
  top: 0.3em;
  left: 40px;
  font-weight: 700;
  color: var(--mbr-editor-alert-color);
  pointer-events: none;
  user-select: none;
}
`;

type DomSpec = readonly [string, Record<string, unknown>, ...unknown[]];

/** The node's kind, if its `alert` attr is one. */
function nodeAlert(node: ProseNode): AlertKind | undefined {
  return typeof node.attrs.alert === 'string' ? alertKind(node.attrs.alert) : undefined;
}

/** Commonmark's `blockquote`, plus `alert` (lowercase kind or null) and `alertLine` (authored, or null). */
export const alertBlockquoteSchema = blockquoteSchema.extendSchema((prev) => (ctx) => {
  const base = prev(ctx);
  return {
    ...base,
    attrs: {
      ...base.attrs,
      alert: { default: null, validate: 'string|null' },
      alertLine: { default: null, validate: 'string|null' },
    },
    parseDOM: [
      {
        tag: 'blockquote[data-mbr-alert]',
        getAttrs: (dom: HTMLElement) => {
          const kind = alertKind(dom.dataset.mbrAlert ?? '');
          return kind ? { alert: kind, alertLine: null } : false;
        },
      },
      ...(base.parseDOM ?? []),
    ],
    toDOM: (node: ProseNode) => {
      const spec = base.toDOM?.(node) as DomSpec | undefined;
      const kind = nodeAlert(node);
      if (!Array.isArray(spec) || !kind) return spec as never;
      const [tag, attrs, ...rest] = spec;
      return [tag, { ...attrs, 'data-mbr-alert': kind, 'data-mbr-alert-label': ALERT_LABELS[kind] }, ...rest] as never;
    },
    parseMarkdown: {
      match: ({ type }) => type === 'blockquote',
      runner: (state, node, type) => {
        const data = (node.data ?? {}) as AlertData;
        const kind = typeof data.alert === 'string' ? alertKind(data.alert) : undefined;
        state.openNode(type, kind ? { alert: kind, alertLine: data.alertLine ?? null } : undefined);
        state.next(node.children);
        state.closeNode();
      },
    },
    toMarkdown: {
      match: (node) => node.type.name === 'blockquote',
      runner: (state, node) => {
        const kind = nodeAlert(node);
        const data: AlertData | undefined = kind
          ? { alert: kind, alertLine: alertLineFor(kind, node.attrs.alertLine) }
          : undefined;
        state.openNode('blockquote', undefined, data ? { data: { ...data } } : undefined);
        // ProseMirror cannot hold an empty blockquote, so `> [!NOTE]` alone
        // comes back holding one empty paragraph, which Milkdown would print
        // as `<br />`. An alert's tag line stands on its own.
        const onlyEmpty = node.childCount === 1 && node.firstChild?.isTextblock && node.firstChild.content.size === 0;
        if (!(kind && onlyEmpty)) state.next(node.content);
        state.closeNode();
      },
    },
  };
});

/**
 * `[!note] ` (any kind, any case) typed at the start of a plain blockquote's
 * first block turns the blockquote into that alert; the typed tag goes away,
 * since the callout shows it.
 */
export const alertInputRule = $inputRule(
  () =>
    new InputRule(new RegExp(`^\\[!(${KIND_PATTERN})\\]\\s$`, 'i'), (state, match, start, end) => {
      const kind = alertKind(match[1]);
      const $start = state.doc.resolve(start);
      const depth = $start.depth - 1;
      if (!kind || depth < 0) return null;
      const quote = $start.node(depth);
      if (quote.type.name !== 'blockquote' || nodeAlert(quote) || $start.index(depth) !== 0) return null;
      return state.tr
        .delete(start, end)
        .setNodeMarkup($start.before(depth), undefined, { ...quote.attrs, alert: kind, alertLine: null });
    }),
);

/** Backspace at the very start of an alert's first block drops the alert (the blockquote stays). */
export const alertBackspace = $prose(
  () =>
    new Plugin({
      props: {
        handleKeyDown(view, event) {
          if (event.key !== 'Backspace' || event.metaKey || event.ctrlKey || event.altKey) return false;
          const { selection } = view.state;
          const { $from } = selection;
          if (!selection.empty || $from.parentOffset !== 0 || $from.depth < 2) return false;
          const depth = $from.depth - 1;
          const quote = $from.node(depth);
          if (quote.type.name !== 'blockquote' || !nodeAlert(quote) || $from.index(depth) !== 0) return false;
          view.dispatch(
            view.state.tr.setNodeMarkup($from.before(depth), undefined, { ...quote.attrs, alert: null, alertLine: null }),
          );
          return true;
        },
      },
      view: () => {
        injectEditorCss('mbr-editor-alerts', ALERTS_CSS);
        return {};
      },
    }),
);

/** Everything the alerts plugin registers, for `MARKDOWN_EXTENSION_PLUGINS`. */
export const alerts = [remarkAlertsPlugin, alertBlockquoteSchema, alertInputRule, alertBackspace].flat();
