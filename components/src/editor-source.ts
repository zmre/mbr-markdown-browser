/**
 * The editor's source mode: the whole file — frontmatter and body — as plain
 * text in CodeMirror 6, with markdown highlighting (GFM, and the frontmatter
 * as YAML), undo history, soft wrap, and Enter continuing a list or quote.
 *
 * Nothing between this text and the disk rewrites it: a save in source mode
 * writes exactly what is here (in the file's own line ending), which is the
 * point of the mode — the rich editor re-prints the markdown it loads.
 *
 * `markdownLanguage` is imported rather than `markdown()`: the latter brings
 * `@codemirror/lang-html` (and with it CSS and JavaScript) for highlighting
 * embedded HTML, which a note's source does not need. `@codemirror/language-data`
 * stays stubbed (`vite.editor.config.ts`), so fenced code is not highlighted
 * by language. The CodeMirror core packages are the copies Crepe and the
 * properties editor already put in the chunk.
 *
 * The modal around it owns Escape (close) and Cmd/Ctrl+S (save) on the
 * document, in the bubble phase; nothing here binds either.
 */

import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { markdownKeymap, markdownLanguage } from '@codemirror/lang-markdown';
import { yamlFrontmatter } from '@codemirror/lang-yaml';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { EditorState, type Extension } from '@codemirror/state';
import { EditorView, keymap } from '@codemirror/view';
import { tags } from '@lezer/highlight';

export interface SourceEditorOptions {
  readonly parent: HTMLElement;
  /** The file's text, in LF. */
  readonly doc: string;
  /** Called with the whole text after every change the user makes. */
  readonly onChange?: (text: string) => void;
}

export interface SourceEditor {
  readonly view: EditorView;
  /** The current text (LF). */
  readonly text: string;
  focus(): void;
  destroy(): void;
}

/** The page's Pico theme: a full-height document, not a boxed field. */
const theme = EditorView.theme({
  '&': { color: 'var(--pico-color, #1a1a1a)', backgroundColor: 'transparent' },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': {
    fontFamily: 'var(--pico-font-family-monospace, ui-monospace, monospace)',
    lineHeight: '1.6',
  },
  '.cm-content': { padding: '1rem 0 40vh', caretColor: 'var(--pico-color, #1a1a1a)' },
  '.cm-line': { padding: '0 1rem' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--pico-color, #1a1a1a)' },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': {
    backgroundColor: 'var(--pico-primary-focus, rgba(1, 114, 173, 0.25))',
  },
});

/** Markdown token colours from theme variables: structure in the primary colour, markup muted. */
const highlight = HighlightStyle.define([
  { tag: tags.heading, fontWeight: '700', color: 'var(--pico-h1-color, inherit)' },
  { tag: tags.strong, fontWeight: '700' },
  { tag: tags.emphasis, fontStyle: 'italic' },
  { tag: tags.strikethrough, textDecoration: 'line-through' },
  { tag: [tags.link, tags.url], color: 'var(--pico-primary, #0172ad)' },
  { tag: tags.monospace, color: 'var(--pico-code-color, #6b5ea8)' },
  { tag: tags.quote, color: 'var(--pico-muted-color, #6b7280)' },
  { tag: [tags.processingInstruction, tags.contentSeparator, tags.meta], color: 'var(--pico-muted-color, #6b7280)' },
  { tag: tags.comment, color: 'var(--pico-muted-color, #6b7280)', fontStyle: 'italic' },
  // The frontmatter, through lang-yaml.
  { tag: tags.definition(tags.propertyName), color: 'var(--pico-primary, #0172ad)' },
  { tag: tags.string, color: 'var(--pico-ins-color, #1d7a3a)' },
]);

/** The extensions of the source editor (no view needed: the tests build states from these). */
export function sourceEditorExtensions(onDocChange: (text: string) => void): Extension[] {
  return [
    yamlFrontmatter({ content: markdownLanguage }),
    history(),
    keymap.of([...markdownKeymap, ...defaultKeymap, ...historyKeymap]),
    EditorView.lineWrapping,
    // Keeps the line being typed clear of the footer bar and, on a phone, of the keyboard.
    EditorView.scrollMargins.of(() => ({ top: 16, bottom: 96 })),
    syntaxHighlighting(highlight),
    theme,
    EditorView.contentAttributes.of({
      'aria-label': 'Markdown source',
      'aria-multiline': 'true',
      spellcheck: 'true',
      autocapitalize: 'sentences',
    }),
    EditorView.updateListener.of((update) => {
      if (update.docChanged) onDocChange(update.state.doc.toString());
    }),
  ];
}

/** Mounts the source editor in `opts.parent`. */
export function createSourceEditor(opts: SourceEditorOptions): SourceEditor {
  const view = new EditorView({
    parent: opts.parent,
    state: EditorState.create({
      doc: opts.doc,
      extensions: sourceEditorExtensions((text) => opts.onChange?.(text)),
    }),
  });
  return {
    view,
    get text() {
      return view.state.doc.toString();
    },
    focus() {
      view.focus();
    },
    destroy() {
      view.destroy();
    },
  };
}
