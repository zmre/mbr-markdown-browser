/**
 * Extended task markers in the editor: `[ ]` open, `[x]`/`[X]` done, `[-]`
 * cancelled, `[>]` deferred — the grammar `crates/mbr-core/src/tasks.rs`
 * reads and the renderer draws (TODO "Editor: cover mbr's markdown
 * extensions", item 2).
 *
 * GFM (`micromark-extension-gfm-task-list-item`, inside remark-gfm) knows only
 * space and `x`/`X`, so `[-]`/`[>]` used to parse as text and save as
 * `\[-]`/`\[>]` — no longer tasks — and `[X]` saved as `[x]`. Four layers fix
 * that, each the supported Milkdown/unified extension point:
 *
 * 1. **Syntax** ({@link taskMarkerSyntax}): a fork of GFM's `tasklistCheck`
 *    tokenizer that also accepts `-` and `>`. It emits GFM's own token names,
 *    so remark-gfm's from-markdown handlers still set `checked` and strip the
 *    space after the box; whichever of the two tokenizers matches a `[x]`, the
 *    tree is the same.
 * 2. **mdast** ({@link taskMarkerFromMarkdown}): on exit of the check, the
 *    list item gets `status` (the normalized marker, `' '`/`'x'`/`'-'`/`'>'` —
 *    what `editor-outline.ts` and the parity goldens compare) and `marker`
 *    (the authored character, so `[X]` stays `[X]`). `checked` is kept for
 *    everything that only knows GFM: `true` for done, `false` otherwise.
 * 3. **Markdown out** ({@link taskMarkerToMarkdown}): a `listItem` handler that
 *    prints the box from `marker`, replacing remark-gfm's `[x]`/`[ ]`-only one
 *    (registered later, so it wins).
 * 4. **ProseMirror**: `list_item` gains a `taskMarker` attr next to GFM's
 *    `checked` ({@link taskListItemSchema}), and a node view
 *    ({@link taskListItemView}) draws the four states and cycles them with
 *    the in-document checkbox's semantics (`task-toggle.ts`
 *    `nextToggleStatus`/`nextCancelStatus`, which write what `tasks.rs`
 *    `set_marker` writes): click completes ↔ reopens, right-click cancels ↔
 *    reopens.
 *
 * `checked` stays the source of truth for *whether* an item is a task (Crepe's
 * slash menu, GFM's `[ ] ` input rule and paste all set only `checked`);
 * `taskMarker` refines it and is ignored when it contradicts `checked`
 * ({@link effectiveMarker}), so no other plugin can leave a stale marker.
 *
 * Annotations (`@due(...)`, `@done(...)`, `#tag`, `!!`, `> date`) are plain
 * text to the editor and need nothing here; the tests pin that they survive
 * unescaped.
 */
import type { ListItem, Parents } from 'mdast';
import type { CompileContext, Extension as FromMarkdownExtension, Token } from 'mdast-util-from-markdown';
import { defaultHandlers, type Info, type Options as ToMarkdownExtension, type State } from 'mdast-util-to-markdown';
import { factorySpace } from 'micromark-factory-space';
import { markdownLineEnding, markdownLineEndingOrSpace, markdownSpace } from 'micromark-util-character';
import type { Code, Effects, Extension as SyntaxExtension, State as MicromarkState, TokenizeContext } from 'micromark-util-types';
import type { Processor } from 'unified';
import { listItemBlockConfig } from '@milkdown/kit/component/list-item-block';
import { commandsCtx } from '@milkdown/kit/core';
import { extendListItemSchemaForTask } from '@milkdown/kit/preset/gfm';
import type { Node as ProseNode, NodeType } from '@milkdown/kit/prose/model';
import { splitListItem } from '@milkdown/kit/prose/schema-list';
import type { Command } from '@milkdown/kit/prose/state';
import type { NodeView } from '@milkdown/kit/prose/view';
import { $command, $remark, $useKeymap, $view } from '@milkdown/kit/utils';
import { injectEditorCss } from './inject-css.js';

declare module 'micromark-util-types' {
  // GFM's token names (declared by `micromark-extension-gfm-task-list-item`,
  // which this module does not import): the fork emits the same ones.
  interface TokenTypeMap {
    taskListCheck: 'taskListCheck';
    taskListCheckMarker: 'taskListCheckMarker';
    taskListCheckValueChecked: 'taskListCheckValueChecked';
    taskListCheckValueUnchecked: 'taskListCheckValueUnchecked';
  }
}

// ============================================================================
// The marker model (pure)
// ============================================================================

/** A task's normalized marker: what the renderer and the parity outline call its status. */
export type TaskStatusMarker = ' ' | 'x' | '-' | '>';

/** Every character the tokenizer accepts between the brackets (line endings are read as `' '`). */
const MARKER_CHARS = new Set([' ', '\t', 'x', 'X', '-', '>']);

/** The normalized status of an authored marker character, `undefined` if it is not one. */
export function statusOf(marker: string): TaskStatusMarker | undefined {
  switch (marker) {
    case ' ':
    case '\t':
      return ' ';
    case 'x':
    case 'X':
      return 'x';
    case '-':
      return '-';
    case '>':
      return '>';
    default:
      return undefined;
  }
}

/** Done is the only status GFM's `checked` calls checked. */
function isDone(marker: string): boolean {
  return statusOf(marker) === 'x';
}

/**
 * The marker to print for an item with GFM `checked` and an optional authored
 * `marker`. `checked` decides whether it is a task at all and whether it is
 * done; the authored marker is kept whenever it agrees with that, so `[X]`,
 * `[-]`, `[>]` and a tab survive, and anything that only set `checked` gets
 * GFM's `[x]`/`[ ]`.
 */
export function effectiveMarker(checked: boolean | null | undefined, marker: unknown): string | undefined {
  if (typeof checked !== 'boolean') return undefined;
  if (typeof marker === 'string' && MARKER_CHARS.has(marker) && isDone(marker) === checked) return marker;
  return checked ? 'x' : ' ';
}

/**
 * Left click / `Space` on a task: complete ↔ reopen. A cancelled or deferred
 * task completes, as in the document (`task-toggle.ts` `nextToggleStatus`).
 */
export function nextToggleMarker(marker: string): TaskStatusMarker {
  return statusOf(marker) === 'x' ? ' ' : 'x';
}

/**
 * Right click: cancel ↔ reopen (`nextCancelStatus`). Deferred `>` counts as
 * cancelled there — `tasks.rs` maps both to `Canceled` — so it reopens; and
 * cancelling always writes `-`, as `set_marker` does (`>` carries a
 * destination date a cancel cannot invent).
 */
export function nextCancelMarker(marker: string): TaskStatusMarker {
  const status = statusOf(marker);
  return status === '-' || status === '>' ? ' ' : '-';
}

// ============================================================================
// 1. Syntax: GFM's task check, forked to accept `-` and `>`
// ============================================================================

const LEFT_BRACKET = 91; // [
const RIGHT_BRACKET = 93; // ]
const LOWER_X = 120;
const UPPER_X = 88;
const DASH = 45;
const GREATER_THAN = 62;

/**
 * A micromark syntax extension for `[ ]` `[x]` `[X]` `[-]` `[>]` at the start
 * of a list item's first paragraph. Identical to GFM's tokenizer — the same
 * position rule (`_gfmTasklistFirstContentOfListItem`, set by micromark's
 * subtokenizer), the same "must be followed by whitespace and content" rule —
 * except for the two extra marker characters.
 */
export function taskMarkerSyntax(): SyntaxExtension {
  return { text: { [LEFT_BRACKET]: { name: 'mbrTaskListCheck', tokenize: tokenizeTaskListCheck } } };
}

function tokenizeTaskListCheck(
  this: TokenizeContext,
  effects: Effects,
  ok: MicromarkState,
  nok: MicromarkState,
): MicromarkState {
  const self = this;
  return open;

  function open(code: Code): MicromarkState | undefined {
    if (self.previous !== null || !self._gfmTasklistFirstContentOfListItem) return nok(code);
    effects.enter('taskListCheck');
    effects.enter('taskListCheckMarker');
    effects.consume(code);
    effects.exit('taskListCheckMarker');
    return inside;
  }

  function inside(code: Code): MicromarkState | undefined {
    // `-` and `>` are not done, so they reuse GFM's "unchecked" token: its
    // from-markdown handler then sets `checked: false` and strips the space,
    // and ours reads the actual character off the source.
    if (code === DASH || code === GREATER_THAN) {
      // `[-]`/`[>]` are valid link labels (`[ ]` is not, and GFM's check
      // outranks a `[x]` reference): when the document defines one, the
      // renderer reads a shortcut reference link there and no task, so we
      // decline and let the reference parse.
      if (self.parser.defined.includes(String.fromCharCode(code))) return nok(code);
    }
    if (markdownLineEndingOrSpace(code) || code === DASH || code === GREATER_THAN) {
      effects.enter('taskListCheckValueUnchecked');
      effects.consume(code);
      effects.exit('taskListCheckValueUnchecked');
      return close;
    }
    if (code === LOWER_X || code === UPPER_X) {
      effects.enter('taskListCheckValueChecked');
      effects.consume(code);
      effects.exit('taskListCheckValueChecked');
      return close;
    }
    return nok(code);
  }

  function close(code: Code): MicromarkState | undefined {
    if (code !== RIGHT_BRACKET) return nok(code);
    effects.enter('taskListCheckMarker');
    effects.consume(code);
    effects.exit('taskListCheckMarker');
    effects.exit('taskListCheck');
    return after;
  }

  function after(code: Code): MicromarkState | undefined {
    if (markdownLineEnding(code)) return ok(code);
    if (markdownSpace(code)) return effects.check({ tokenize: spaceThenNonSpace }, ok, nok)(code);
    return nok(code);
  }
}

function spaceThenNonSpace(effects: Effects, ok: MicromarkState, nok: MicromarkState): MicromarkState {
  return factorySpace(effects, (code: Code) => (code === null ? nok(code) : ok(code)), 'whitespace');
}

// ============================================================================
// 2. mdast: `status` and `marker` on the list item
// ============================================================================

/** A GFM list item with this plugin's fields. */
export type TaskListItem = ListItem & { status?: TaskStatusMarker; marker?: string };

/**
 * Sets `status`/`marker` on the list item when its check closes. Keyed on the
 * outer `taskListCheck` token, which remark-gfm has no handler for, so this
 * composes with GFM's `checked` handling rather than replacing it.
 */
export function taskMarkerFromMarkdown(): FromMarkdownExtension {
  return {
    exit: {
      taskListCheck(this: CompileContext, token: Token): undefined {
        // `[`, the marker, `]`. A line ending between the brackets is GFM's
        // odd "unchecked" spelling; it is printed back as a space.
        const raw = this.sliceSerialize(token).slice(1, -1);
        const marker = MARKER_CHARS.has(raw) ? raw : ' ';
        // We are in the paragraph, in the list item.
        const item = this.stack[this.stack.length - 2] as TaskListItem | undefined;
        if (item?.type !== 'listItem') return;
        item.marker = marker;
        item.status = statusOf(marker) ?? ' ';
        item.checked = isDone(marker);
      },
    },
  };
}

// ============================================================================
// 3. Markdown out: print the authored box
// ============================================================================

/** remark-gfm's `listItem` handler, but the box comes from {@link effectiveMarker}. */
function listItemWithTaskMarker(node: TaskListItem, parent: Parents | undefined, state: State, info: Info): string {
  const marker = effectiveMarker(node.checked, node.marker);
  const head = node.children[0];
  // A definition list's term is the item's first paragraph to the renderer
  // (`- [ ] Term` / `  : def`), so the box goes in front of it as well.
  const checkable = marker !== undefined && (head?.type === 'paragraph' || (head?.type as string) === 'defList');
  const checkbox = `[${marker}] `;
  const tracker = state.createTracker(info);
  if (checkable) tracker.move(checkbox);
  const value = defaultHandlers.listItem(node, parent, state, { ...info, ...tracker.current() });
  // GFM's pattern only knows `1.`; `1)` items are tasks too.
  return checkable ? value.replace(/^(?:[*+-]|\d+[.)])(?:[\r\n]| {1,3})/, (bullet) => bullet + checkbox) : value;
}

export function taskMarkerToMarkdown(): ToMarkdownExtension {
  return { handlers: { listItem: listItemWithTaskMarker } };
}

/** The unified plugin: all three halves on the processor remark-gfm configured. */
export function remarkTaskMarkers(this: Processor): void {
  const data = this.data();
  (data.micromarkExtensions ??= []).push(taskMarkerSyntax());
  (data.fromMarkdownExtensions ??= []).push(taskMarkerFromMarkdown());
  (data.toMarkdownExtensions ??= []).push(taskMarkerToMarkdown());
}

// ============================================================================
// 4. ProseMirror: the `taskMarker` attr and the four-state checkbox
// ============================================================================

export const remarkTaskMarkersPlugin = $remark('mbrTaskMarkers', () => remarkTaskMarkers);

type DomSpec = readonly [string, Record<string, unknown>, ...unknown[]];

/**
 * GFM's task `list_item`, plus `taskMarker`: the authored character (`' '`,
 * `'\t'`, `'x'`, `'X'`, `'-'`, `'>'`), `null` on a plain item or one whose
 * marker nothing has set yet.
 */
export const taskListItemSchema = extendListItemSchemaForTask.extendSchema((prev) => (ctx) => {
  const base = prev(ctx);
  return {
    ...base,
    attrs: { ...base.attrs, taskMarker: { default: null, validate: 'string|null' } },
    parseDOM: (base.parseDOM ?? []).map((rule) => {
      const getAttrs = rule.getAttrs;
      if (!getAttrs) return rule;
      return {
        ...rule,
        getAttrs: (dom: HTMLElement | string) => {
          const attrs = getAttrs(dom as HTMLElement);
          if (!attrs || !(dom instanceof HTMLElement)) return attrs;
          return { ...attrs, taskMarker: dom.dataset.taskMarker ?? null };
        },
      };
    }),
    toDOM: (node: ProseNode) => {
      const spec = base.toDOM?.(node) as DomSpec | undefined;
      const marker = effectiveMarker(node.attrs.checked, node.attrs.taskMarker);
      if (!Array.isArray(spec) || marker === undefined) return spec as never;
      const [tag, attrs, ...rest] = spec;
      return [tag, { ...attrs, 'data-task-marker': marker }, ...rest] as never;
    },
    parseMarkdown: {
      match: ({ type }) => type === 'listItem',
      runner: (state, node, type) => {
        const item = node as TaskListItem & { label?: unknown };
        if (item.checked == null) {
          base.parseMarkdown.runner(state, node, type);
          return;
        }
        state.openNode(type, {
          label: item.label != null ? `${String(item.label)}.` : '•',
          listType: item.label != null ? 'ordered' : 'bullet',
          spread: item.spread ?? true,
          checked: Boolean(item.checked),
          taskMarker: typeof item.marker === 'string' ? item.marker : null,
        });
        state.next(node.children);
        state.closeNode();
      },
    },
    toMarkdown: {
      match: (node) => node.type.name === 'list_item',
      runner: (state, node) => {
        const marker = effectiveMarker(node.attrs.checked, node.attrs.taskMarker);
        if (marker === undefined) {
          base.toMarkdown.runner(state, node);
          return;
        }
        state.openNode('listItem', undefined, {
          label: node.attrs.label,
          listType: node.attrs.listType,
          spread: node.attrs.spread,
          checked: node.attrs.checked,
          marker,
          status: statusOf(marker) ?? ' ',
        });
        state.next(node.content);
        state.closeNode();
      },
    },
  };
});

/** The label CSS class per status (Crepe's `checked`/`unchecked`, plus ours). */
const LABEL_CLASS: Record<TaskStatusMarker, string> = {
  ' ': 'unchecked',
  x: 'checked',
  '-': 'unchecked mbr-task-canceled',
  '>': 'unchecked mbr-task-deferred',
};

const TASK_TITLE = 'Click to complete · right-click to cancel';

/** Box with a dash (cancelled) and box with an arrow (deferred), in Crepe's 24px icon style. */
const CANCELED_ICON =
  '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d="M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zm0 2v14h14V5H5zm2 6h10v2H7v-2z"/></svg>';
const DEFERRED_ICON =
  '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d="M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zm0 2v14h14V5H5zm4.6 2.6L14 12l-4.4 4.4-1.4-1.4 3-3-3-3 1.4-1.4z"/></svg>';

const TASK_MARKERS_CSS = `
.milkdown .milkdown-list-item-block[data-task-marker='-'] > .list-item > .children,
.milkdown .milkdown-list-item-block[data-task-marker='>'] > .list-item > .children {
  text-decoration: line-through;
  opacity: 0.65;
}
`;

/**
 * Crepe's list-item node view, as plain DOM, with the four task states.
 * Same markup and classes (`milkdown-list-item-block` > `li.list-item` >
 * `.label-wrapper` + `.children`), so Crepe's stylesheet applies unchanged,
 * and the bullet/number/`[ ]`/`[x]` labels still come from Crepe's
 * `listItemBlockConfig` (its icons); `[-]`/`[>]` get their own.
 */
export const taskListItemView = $view(taskListItemSchema.node, (ctx) => (initialNode, view, getPos): NodeView => {
  injectEditorCss('mbr-editor-task-markers', TASK_MARKERS_CSS);
  const config = ctx.get(listItemBlockConfig.key);

  const dom = document.createElement('div');
  dom.className = 'milkdown-list-item-block';
  const li = document.createElement('li');
  li.className = 'list-item';
  const labelWrapper = document.createElement('div');
  labelWrapper.className = 'label-wrapper';
  labelWrapper.contentEditable = 'false';
  const label = document.createElement('span');
  labelWrapper.appendChild(label);
  const children = document.createElement('div');
  children.className = 'children';
  const contentDOM = document.createElement('div');
  contentDOM.setAttribute('data-content-dom', 'true');
  contentDOM.className = 'content-dom';
  children.appendChild(contentDOM);
  li.append(labelWrapper, children);
  dom.appendChild(li);

  let node = initialNode;

  const render = (): void => {
    const { checked, label: text, listType } = node.attrs;
    const marker = effectiveMarker(checked, node.attrs.taskMarker);
    const status = marker === undefined ? undefined : statusOf(marker);
    const readonly = !view.editable;
    let icon: string;
    if (status === '-') icon = CANCELED_ICON;
    else if (status === '>') icon = DEFERRED_ICON;
    else icon = config.renderLabel({ label: text, listType, checked: status === undefined ? undefined : status === 'x', readonly });
    const kind = status === undefined ? (listType === 'bullet' ? 'bullet' : 'ordered') : LABEL_CLASS[status];
    label.className = `milkdown-icon label ${kind}${readonly ? ' readonly' : ''}`;
    // Icons are the editor's own SVG constants; anything else (an ordered
    // item's `3.`) is text.
    if (icon.trimStart().startsWith('<svg')) label.innerHTML = icon;
    else label.textContent = icon;
    if (marker === undefined) {
      dom.removeAttribute('data-task-marker');
      labelWrapper.removeAttribute('title');
    } else {
      dom.setAttribute('data-task-marker', marker);
      labelWrapper.title = TASK_TITLE;
    }
  };

  const setMarker = (next: (marker: string) => TaskStatusMarker): void => {
    const marker = effectiveMarker(node.attrs.checked, node.attrs.taskMarker);
    const pos = getPos();
    if (marker === undefined || !view.editable || pos === undefined) return;
    const taskMarker = next(marker);
    if (!view.hasFocus()) view.focus();
    view.dispatch(view.state.tr.setNodeMarkup(pos, undefined, { ...node.attrs, taskMarker, checked: taskMarker === 'x' }));
  };

  // Pointerdown, as Crepe does, so the click never moves the selection into
  // the label; the right button is left to `contextmenu`.
  labelWrapper.addEventListener('pointerdown', (event) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    setMarker(nextToggleMarker);
  });
  labelWrapper.addEventListener('contextmenu', (event) => {
    if (effectiveMarker(node.attrs.checked, node.attrs.taskMarker) === undefined) return;
    event.preventDefault();
    event.stopPropagation();
    setMarker(nextCancelMarker);
  });

  render();

  return {
    dom,
    contentDOM,
    update: (updated) => {
      if (updated.type !== initialNode.type) return false;
      node = updated;
      render();
      return true;
    },
    ignoreMutation: (mutation) => {
      if (mutation.type === 'selection') return false;
      if (mutation.target === contentDOM && mutation.type === 'attributes') return true;
      return !contentDOM.contains(mutation.target);
    },
    selectNode: () => {
      dom.classList.add('selected');
      li.classList.add('ProseMirror-selectednode');
    },
    deselectNode: () => {
      dom.classList.remove('selected');
      li.classList.remove('ProseMirror-selectednode');
    },
  };
});

// ============================================================================
// 5. Enter: a new task starts open
// ============================================================================

/**
 * Enter in a task item's text: ProseMirror's `splitListItem`, but the new
 * item is an open task (`[ ]`) instead of a copy of the old item's status —
 * the split copies every attribute, so Enter after a `[-]` or `[x]` item
 * used to make another cancelled or done one. Enter at the very start of
 * the item inserts the new (empty) item above, which is then the open one;
 * the item the cursor was in keeps its status either way.
 *
 * Declines (for the list's own Enter) outside a task item, in an empty item
 * (which Enter lifts out of the list), with a selection, and below the item's
 * first level (a nested list or definition list has its own Enter).
 */
export function splitTaskItem(itemType: NodeType): Command {
  return (state, dispatch) => {
    const { $from, empty } = state.selection;
    const depth = $from.depth - 1;
    if (!empty || depth < 1) return false;
    const item = $from.node(depth);
    if (item.type !== itemType || item.attrs.checked == null || $from.parent.content.size === 0) return false;
    const atStart = $from.parentOffset === 0 && $from.index(depth) === 0;
    return splitListItem(itemType)(
      state,
      dispatch &&
        ((tr) => {
          const $cursor = tr.selection.$from;
          const lower = $cursor.before($cursor.depth - 1);
          const target = atStart ? lower - (tr.doc.resolve(lower).nodeBefore?.nodeSize ?? 0) : lower;
          const node = tr.doc.nodeAt(target);
          if (node?.type === itemType) {
            tr.setNodeMarkup(target, undefined, { ...node.attrs, checked: false, taskMarker: ' ' });
          }
          dispatch(tr);
        }),
    );
  };
}

const splitTaskItemCommand = $command('MbrSplitTaskItem', (ctx) => () => splitTaskItem(taskListItemSchema.type(ctx)));

/** Above the list's own Enter (50), below the definition list's (100), which owns a term in a task item. */
const SPLIT_TASK_PRIORITY = 60;

const taskItemKeymap = $useKeymap('mbrTaskItemKeymap', {
  SplitTaskItem: {
    shortcuts: 'Enter',
    priority: SPLIT_TASK_PRIORITY,
    command: (ctx) => () => ctx.get(commandsCtx).call(splitTaskItemCommand.key),
  },
});

/** Everything item 2 registers, for `MARKDOWN_EXTENSION_PLUGINS`. */
export const taskMarkers = [
  remarkTaskMarkersPlugin,
  taskListItemSchema,
  taskListItemView,
  splitTaskItemCommand,
  taskItemKeymap,
].flat();
