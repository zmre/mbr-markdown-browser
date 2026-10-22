/**
 * The YAML source editor: CodeMirror 6 with YAML highlighting, undo history,
 * line wrapping, frontmatter-aware completion and parse-error lint
 * (`yaml-assist.ts`). One factory serves both the properties editor's YAML
 * tab (the whole block) and "Edit as YAML" on one value (its slice, with
 * `basePath` so completion knows where it is).
 *
 * The editor modal around it owns Escape (close) and Cmd/Ctrl+S (save) on
 * the document, in the bubble phase. So Escape that closes a completion list
 * stops there (`stopPropagation`), and a slice's own
 * Escape (cancel) does too; any other Escape, and Cmd/Ctrl+S (which nothing
 * here binds), reach the modal as they did from the old textarea.
 *
 * The CodeMirror packages are the copies Crepe already bundles (one
 * `@codemirror/state`/`view` in the chunk); `@codemirror/lang-yaml` is
 * imported directly, never through `@codemirror/language-data`, which the
 * editor build stubs out.
 */

import { autocompletion, closeCompletion, completionKeymap, type CompletionSource } from '@codemirror/autocomplete';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { yaml } from '@codemirror/lang-yaml';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { linter } from '@codemirror/lint';
import { EditorState, Prec, type Extension } from '@codemirror/state';
import { EditorView, keymap, placeholder } from '@codemirror/view';
import { tags } from '@lezer/highlight';
import { yamlCompletionSource, yamlDiagnostics, type YamlAssistOptions } from './yaml-assist.js';

export interface YamlSourceOptions extends YamlAssistOptions {
  readonly parent: HTMLElement;
  readonly doc: string;
  /** The editor's accessible name. */
  readonly label: string;
  /** Called with the whole text after every change the user makes. */
  readonly onChange: (text: string) => void;
  /** Escape when nothing in the editor wanted it (a slice cancels); unset leaves Escape to the page. */
  readonly onEscape?: () => void;
  /** Cmd/Ctrl+Enter (a slice applies). */
  readonly onSubmit?: () => void;
  readonly placeholder?: string;
}

export interface YamlSourceEditor {
  readonly view: EditorView;
  /** The current text. */
  readonly text: string;
  /** Replaces the text (not reported through `onChange`). */
  setText(text: string): void;
  focus(): void;
  destroy(): void;
}

/** Colours from the page's Pico theme, so light and dark both follow it. */
const theme = EditorView.theme({
  '&': {
    color: 'var(--pico-color, #1a1a1a)',
    backgroundColor: 'var(--pico-code-background-color, #f6f8fa)',
    border: '1px solid var(--pico-muted-border-color, #d0d0d0)',
    borderRadius: '6px',
  },
  '&.cm-focused': {
    outline: 'none',
    borderColor: 'var(--pico-primary, #0172ad)',
    boxShadow: '0 0 0 1px var(--pico-primary, #0172ad)',
  },
  '.cm-scroller': { fontFamily: 'var(--pico-font-family-monospace, ui-monospace, monospace)', lineHeight: '1.5' },
  '.cm-content': { padding: '0.35rem 0', caretColor: 'var(--pico-color, #1a1a1a)' },
  '.cm-line': { padding: '0 0.5rem' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--pico-color, #1a1a1a)' },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': {
    backgroundColor: 'var(--pico-primary-focus, rgba(1, 114, 173, 0.25))',
  },
  '.cm-placeholder': { color: 'var(--pico-muted-color, #6b7280)' },
  '.cm-tooltip': {
    backgroundColor: 'var(--pico-card-background-color, var(--pico-background-color, #fff))',
    color: 'var(--pico-color, #1a1a1a)',
    border: '1px solid var(--pico-muted-border-color, #e0e0e0)',
    borderRadius: '8px',
    boxShadow: '0 8px 24px rgba(0, 0, 0, 0.18)',
  },
  // The list is clipped to the rounded box; the info panel sits outside the box, so the box itself must not clip.
  '.cm-tooltip.cm-tooltip-autocomplete > ul': {
    borderRadius: '8px',
    fontFamily: 'var(--pico-font-family-monospace, ui-monospace, monospace)',
    maxHeight: '15rem',
    minWidth: '12rem',
  },
  '.cm-tooltip.cm-tooltip-autocomplete > ul > li': { padding: '0.2rem 0.5rem', lineHeight: '1.5' },
  '.cm-tooltip-autocomplete ul li[aria-selected]': {
    backgroundColor: 'var(--pico-primary-focus, rgba(1, 114, 173, 0.18))',
    color: 'var(--pico-color, #1a1a1a)',
  },
  '.cm-completionDetail': { color: 'var(--pico-muted-color, #6b7280)', fontStyle: 'normal', marginLeft: '0.75em' },
  '.cm-completionMatchedText': { textDecoration: 'none', fontWeight: '600' },
  '.cm-tooltip.cm-completionInfo': { padding: '0.35rem 0.6rem', maxWidth: '20rem', fontSize: '0.85em' },
  '.cm-diagnostic': { padding: '0.25rem 0.5rem' },
  '.cm-diagnostic-error': { borderLeftColor: 'var(--pico-del-color, #b3261e)' },
  '.cm-diagnostic-warning': { borderLeftColor: 'var(--mbr-warning-color, #b7791f)' },
  '.cm-panels': { backgroundColor: 'transparent', color: 'inherit' },
  '.cm-panels.cm-panels-bottom': { borderTop: '1px solid var(--pico-muted-border-color, #e0e0e0)' },
});

/** YAML token colours from theme variables (keys in the primary colour, comments muted). */
const highlight = HighlightStyle.define([
  { tag: tags.definition(tags.propertyName), color: 'var(--pico-primary, #0172ad)' },
  { tag: tags.string, color: 'var(--pico-ins-color, #1d7a3a)' },
  { tag: [tags.special(tags.string), tags.labelName, tags.typeName], color: 'var(--pico-secondary, #6b5ea8)' },
  { tag: tags.lineComment, color: 'var(--pico-muted-color, #6b7280)', fontStyle: 'italic' },
  { tag: [tags.separator, tags.punctuation, tags.squareBracket, tags.brace], color: 'var(--pico-muted-color, #6b7280)' },
  { tag: [tags.meta, tags.keyword], color: 'var(--pico-muted-color, #6b7280)' },
]);

/**
 * On a narrow screen the help panel beside the list has no room (CodeMirror
 * lays it over the list), so a completion's help becomes its inline detail.
 */
function narrowAware(source: CompletionSource): CompletionSource {
  return (context) => {
    const result = source(context);
    const narrow = typeof window !== 'undefined' && window.matchMedia?.('(max-width: 600px)').matches;
    if (!narrow || !result || result instanceof Promise) return result;
    return { ...result, options: result.options.map((o) => (typeof o.info === 'string' ? { ...o, detail: o.detail ?? o.info, info: undefined } : o)) };
  };
}

/** The extensions of a frontmatter YAML editor (no view needed: the tests build states from these). */
export function yamlSourceExtensions(opts: Omit<YamlSourceOptions, 'parent' | 'doc'>, onDocChange: (text: string) => void): Extension[] {
  const escapeOnce: Extension = Prec.highest(
    keymap.of([
      // Escape that closes a popup is the editor's; the modal never sees it.
      { key: 'Escape', run: closeCompletion, stopPropagation: true },
      ...(opts.onSubmit
        ? [
            {
              key: 'Mod-Enter',
              run: () => {
                opts.onSubmit!();
                return true;
              },
              stopPropagation: true,
            },
          ]
        : []),
    ]),
  );
  const onEscape = opts.onEscape;
  return [
    yaml(),
    history(),
    escapeOnce,
    onEscape
      ? Prec.low(
          keymap.of([
            {
              key: 'Escape',
              run: () => {
                onEscape();
                return true;
              },
              stopPropagation: true,
            },
          ]),
        )
      : [],
    keymap.of([...completionKeymap, ...defaultKeymap, ...historyKeymap]),
    EditorView.lineWrapping,
    EditorState.tabSize.of(2),
    autocompletion({ override: [narrowAware(yamlCompletionSource(opts))], icons: false, defaultKeymap: false }),
    linter((view) => yamlDiagnostics(view.state.doc.toString()), { delay: 250 }),
    syntaxHighlighting(highlight),
    theme,
    opts.placeholder ? placeholder(opts.placeholder) : [],
    EditorView.contentAttributes.of({ 'aria-label': opts.label, spellcheck: 'false', autocapitalize: 'off', autocorrect: 'off' }),
    EditorView.updateListener.of((update) => {
      if (update.docChanged) onDocChange(update.state.doc.toString());
    }),
  ];
}

/** Mounts a YAML editor in `opts.parent`. */
export function createYamlEditor(opts: YamlSourceOptions): YamlSourceEditor {
  let external = false;
  const view = new EditorView({
    parent: opts.parent,
    state: EditorState.create({
      doc: opts.doc,
      extensions: yamlSourceExtensions(opts, (text) => {
        if (!external) opts.onChange(text);
      }),
    }),
  });
  return {
    view,
    get text() {
      return view.state.doc.toString();
    },
    setText(text: string) {
      if (text === view.state.doc.toString()) return;
      external = true;
      try {
        view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
      } finally {
        external = false;
      }
    },
    focus() {
      view.focus();
    },
    destroy() {
      view.destroy();
    },
  };
}
