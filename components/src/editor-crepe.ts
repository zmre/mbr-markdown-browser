/**
 * Heavy editor chunk: builds the editing modal and drives Milkdown/Crepe.
 *
 * This module pulls in Crepe (ProseMirror) and is therefore built as a
 * SEPARATE bundle (`mbr-editor.min.js`) that the lightweight `<mbr-editor>`
 * trigger loads on demand via a runtime dynamic import — keeping normal page
 * loads free of the editor's weight.
 *
 * The modal is rendered into the light DOM (appended to `document.body`) rather
 * than a shadow root, because Crepe relies on globally-scoped CSS which we
 * inject here as inlined stylesheet strings.
 */

import { Crepe, CrepeFeature } from '@milkdown/crepe';
import crepeCommonCss from '@milkdown/crepe/theme/common/style.css?inline';
import { editorViewCtx, schemaCtx } from '@milkdown/kit/core';
import { TextSelection } from '@milkdown/kit/prose/state';
import type { EditorView } from '@milkdown/kit/prose/view';
import type { Ctx, MilkdownPlugin } from '@milkdown/kit/ctx';
import { lineEndingOf, recombine, splitFrontmatter, toLf, withLineEnding } from './editor-frontmatter.js';
import { PICKER_CSS, createNestedModal } from './editor-picker-shared.js';
import { openPathPicker, type SiteMarkdownFile } from './editor-path-picker.js';
import { openMediaPicker, type MediaFile, type MediaPickResult } from './editor-media-picker.js';
import { createLinkAutocompletePlugin, AUTOCOMPLETE_CSS } from './editor-link-autocomplete.js';
import type { EditTarget } from './editor-media-target.js';
import { createMediaEditPlugin, detectEditTarget } from './editor-media-edit.js';
import { noteDir } from './editor-upload.js';
import { mediaProxyURL } from './editor-media-proxy.js';
import { alerts } from './editor-plugins/alerts.js';
import { taskMarkers } from './editor-plugins/task-markers.js';
import { headingAttrsPlugin } from './editor-plugins/heading-attrs.js';
import { sectionAttrsPlugin } from './editor-plugins/section-attrs.js';
import { wikilinksPlugin } from './editor-plugins/wikilinks.js';
import { mathPlugin } from './editor-plugins/math.js';
import { escapesPlugin } from './editor-plugins/escapes.js';
import { referencesPlugin } from './editor-plugins/references.js';
import { imagesPlugin } from './editor-plugins/images.js';

export interface OpenEditorOptions {
  /** URL of the raw-markdown endpoint for the current file. */
  rawUrl: string;
  /** URL of the save endpoint for the current file. */
  saveUrl: string;
  /** Human-readable file path shown in the header. */
  filePath: string;
  /**
   * The bearer token the page already knows, used to prefill the footer field.
   *
   * This chunk deliberately owns no token state of its own. The main bundle
   * holds it in memory (`edit-token.ts`) so that a task checkbox — which is
   * over there, behind the same `check_edit_access` policy — can use it, and so
   * that nothing durable is written to web storage. The chunk cannot import
   * that module (chunks must not import main-bundle state), so the value comes
   * in here and goes back out through {@link onToken}.
   */
  token?: string;
  /**
   * Show the token field from the start.
   *
   * Normally the field stays hidden until there is a reason for it — a 401 on
   * save, or a token already in hand. The main bundle sets this when a task
   * write has already been refused for want of one, so that "open the editor
   * and enter it" leads somewhere.
   */
  tokenRequired?: boolean;
  /** Hand a token the user typed back to the main bundle (memory only). */
  onToken?: (token: string) => void;
  /** Called as soon as the editor modal is visible (hides the trigger spinner). */
  onReady?: () => void;
  /** Called when the modal is dismissed so the trigger can reset its state. */
  onClose: () => void;
  /**
   * Called after a save the reader asked for (Save, Cmd/Ctrl+S, or "Save" in
   * the unsaved-changes prompt on close) succeeded. Not called for the save a
   * New/Rename/Move makes on its way to another page, which navigates anyway.
   *
   * The trigger uses it to reload the page where no live-reload socket will
   * (the iOS app's `mbr:` scheme), so the page under the editor shows the edit.
   */
  onSaved?: () => void;
}

/**
 * The Crepe feature switches the editor runs with.
 *
 * CodeMirror (and its dependent LaTeX feature) are disabled. CodeMirror
 * statically pulls in @codemirror/language-data (≈50 lazy language imports) —
 * bloating the bundle and, when inlined into a single chunk, breaking module
 * init order. The LaTeX feature depends on CodeMirror, so it must be disabled
 * too. Neither is essential here: code blocks still save as fenced code and
 * math still saves as `$…$` (rendered on the page by the existing katex
 * component); only the in-editor helpers are dropped.
 *
 * Exported so `editor-roundtrip.test.ts` and `editor-parity.test.ts` parse and
 * serialize with exactly the configuration a real save uses.
 */
export const CREPE_FEATURES = {
  [CrepeFeature.CodeMirror]: false,
  [CrepeFeature.Latex]: false,
} as const;

/**
 * The Milkdown plugins that teach the editor mbr's markdown extensions
 * (definition lists, `[-]`/`[>]` tasks, `--- {attrs}`, wikilinks, …), one per
 * `editor-plugins/<name>.ts`.
 *
 * **This list is the registration point.** `openEditor` and the parity
 * harness (`editor-parity.test.ts`) both install exactly these, so a plugin is
 * held to the byte-identical round trip and to the renderer's block outline
 * the moment it is listed here. Plugins that are editing aids rather than
 * syntax (link autocomplete, media double-click) are registered in
 * `openEditor` only.
 */
export const MARKDOWN_EXTENSION_PLUGINS: readonly (MilkdownPlugin | MilkdownPlugin[])[] = [
  sectionAttrsPlugin,
  headingAttrsPlugin,
  wikilinksPlugin,
  taskMarkers,
  alerts,
  mathPlugin,
  escapesPlugin,
  referencesPlugin,
  imagesPlugin,
];

/** Installs {@link MARKDOWN_EXTENSION_PLUGINS} on `crepe`, before `create()`. */
export function useMarkdownExtensions(crepe: Crepe): void {
  for (const plugin of MARKDOWN_EXTENSION_PLUGINS) crepe.editor.use(plugin);
}

/**
 * The body a save writes: Crepe's serialization plus mbr's post-processing.
 * The one definition, shared by the editor's dirty check / save and the
 * round-trip tests, so the tests cannot drift from what a save does.
 */
export function serializeBody(crepe: Crepe): string {
  return crepe.getMarkdown();
}

let stylesInjected = false;

function injectStyles(): void {
  if (stylesInjected) return;
  stylesInjected = true;
  const style = document.createElement('style');
  style.id = 'mbr-editor-styles';
  // `common/style.css` is Crepe's structural styling; it consumes the
  // `--crepe-color-*`/font variables that a theme file would normally supply.
  // Instead of Crepe's hardcoded Nord palette, THEME_CSS maps those variables
  // onto the page's Pico variables, so the editor inherits the active theme —
  // color variant, `.mbr/theme.css`/`user.css` overrides, and light/dark — all
  // of which Pico already switches. THEME_CSS comes after common so it wins.
  style.textContent = [
    crepeCommonCss,
    THEME_CSS,
    FOOTNOTE_CSS,
    MODAL_CSS,
    HEADER_CSS,
    PICKER_CSS,
    AUTOCOMPLETE_CSS,
    // Last, so its narrow-screen overrides win over everything above.
    NARROW_CSS,
  ].join('\n');
  document.head.appendChild(style);
}

// Maps Crepe's theme variables onto Pico's, with the Nord light values as
// fallbacks in case a Pico variable is absent.
const THEME_CSS = `
.milkdown {
  --crepe-color-background: var(--pico-background-color, #fdfcff);
  --crepe-color-on-background: var(--pico-color, #1b1c1d);
  --crepe-color-surface: var(--pico-card-background-color, #f8f9ff);
  --crepe-color-surface-low: var(--pico-card-sectioning-background-color, #f2f3fa);
  --crepe-color-on-surface: var(--pico-color, #191c20);
  --crepe-color-on-surface-variant: var(--pico-muted-color, #43474e);
  /* Crepe uses --crepe-color-outline for toolbar/menu icon fill as well as for
     borders (the border uses are mostly color-mixed to ~20% opacity). Map it to
     Pico's muted *text* color, not muted-border-color, so the icons stay legible
     in both light and dark — matching Nord's mid-gray outline. */
  --crepe-color-outline: var(--pico-muted-color, #73777f);
  --crepe-color-primary: var(--pico-primary, #37618e);
  --crepe-color-secondary: var(--pico-secondary-background, #d7e3f8);
  --crepe-color-on-secondary: var(--pico-secondary-inverse, #101c2b);
  --crepe-color-inverse: var(--pico-contrast-background, #2e3135);
  --crepe-color-on-inverse: var(--pico-contrast-inverse, #eff0f7);
  --crepe-color-inline-code: var(--pico-code-color, #ba1a1a);
  --crepe-color-error: var(--pico-del-color, #ba1a1a);
  --crepe-color-hover: var(--pico-secondary-hover-background, #eceef4);
  --crepe-color-selected: var(--pico-muted-border-color, #e1e2e8);
  --crepe-color-inline-area: var(--pico-code-background-color, #d8dae0);

  --crepe-font-title: var(--pico-font-family, Rubik, Cambria, 'Times New Roman', Times, serif);
  --crepe-font-default: var(--pico-font-family, Inter, Arial, Helvetica, sans-serif);
  --crepe-font-code: var(--pico-font-family-monospace, 'JetBrains Mono', Menlo, Monaco, 'Courier New', Courier, monospace);

  /* Elevation for floating menus (toolbar, block-edit, link tooltip). Crepe's
     Nord theme defined these; without them box-shadow resolves to nothing, and
     in light mode the menu surface is the same white as the page — leaving the
     icons with no container/edge. These match Crepe's original Nord elevation. */
  --crepe-shadow-1: 0 1px 3px 1px rgba(0, 0, 0, 0.15), 0 1px 2px 0 rgba(0, 0, 0, 0.3);
  --crepe-shadow-2: 0 2px 6px 2px rgba(0, 0, 0, 0.15), 0 1px 2px 0 rgba(0, 0, 0, 0.3);
}

/* Code block / inline code background: match the rendered page. Pico paints
   <pre>/<code> with --pico-code-background-color at full opacity, but Crepe's
   default is a translucent color-mix (≈60%), which looks washed out and lighter
   than the page. Reset's higher-specificity \`pre code { background: transparent }\`
   still keeps the inner <code> clear, so blocks don't double-layer. */
.milkdown .ProseMirror pre,
.milkdown .ProseMirror code {
  background: var(--pico-code-background-color);
}
`;

const MODAL_CSS = `
.mbr-editor-backdrop {
  position: fixed; inset: 0; background: rgba(0,0,0,0.5); z-index: 2000;
  display: flex; align-items: center; justify-content: center;
}
.mbr-editor-modal {
  background: var(--pico-background-color, #fff);
  color: var(--pico-color, #1a1a1a);
  width: min(920px, 96vw); height: min(90vh, 900px);
  display: flex; flex-direction: column;
  border-radius: 8px; box-shadow: 0 12px 40px rgba(0,0,0,0.35);
  overflow: hidden;
}
.mbr-editor-body { flex: 1; display: flex; flex-direction: column; overflow: auto; }
.mbr-editor-fm { border-bottom: 1px solid var(--pico-muted-border-color, #e0e0e0); }
.mbr-editor-fm summary { cursor: pointer; padding: 0.5rem 1rem; font-size: 0.85rem; opacity: 0.85; }
.mbr-editor-fm textarea {
  width: 100%; box-sizing: border-box; border: none; resize: vertical;
  min-height: 8rem; font-family: var(--pico-font-family-monospace, monospace);
  font-size: 0.85rem; padding: 0.5rem 1rem; background: var(--pico-code-background-color, #f6f8fa); color: inherit;
}
.mbr-editor-crepe { flex: 1; min-height: 12rem; overflow: auto; }
.mbr-editor-footer {
  display: flex; align-items: center; flex-wrap: wrap; gap: 0 0.5rem;
  padding: 0 0.5rem; min-height: 44px;
  border-top: 1px solid var(--pico-muted-border-color, #e0e0e0);
}
.mbr-editor-tools { display: flex; align-items: center; gap: 0.25rem; }
.mbr-editor-footer .status { flex: 1 1 8rem; min-width: 0; font-size: 0.85rem; padding: 0.25rem 0.5rem; }
.mbr-editor-footer .status:empty { padding: 0; }
.mbr-editor-footer .status.error { color: var(--pico-del-color, #b3261e); }
.mbr-editor-footer .status.ok { color: var(--pico-ins-color, #1a7f37); }
.mbr-editor-modal .mbr-editor-token {
  display: none; width: auto; margin: 0.35rem 0.5rem;
  font-family: var(--pico-font-family-monospace, monospace); font-size: 0.85rem;
  padding: 0.3rem 0.5rem; min-width: 16rem; height: auto;
}
.mbr-editor-modal .mbr-editor-token.show { display: inline-block; }
.mbr-editor-loading { padding: 2rem; text-align: center; opacity: 0.7; }
`;

/**
 * Phones (and any window under 600px). Crepe pads the document 60px/120px,
 * which on a 390px screen leaves a sliver of text, and its selection toolbar
 * is a no-wrap row of 44px items with `overflow: hidden`, so most of it was out
 * of reach. Here the modal is full-screen (clear of the notch and home
 * indicator), the document gets phone margins (wider on the left, where
 * Crepe's block handle sits), and the toolbar wraps onto a second row within
 * the screen. The header and footer are one layout at every width (see
 * HEADER_CSS); here the status takes its own row, every field the reader can
 * type into is 16px (anything smaller makes iOS zoom the page on focus), and
 * the nested pickers become top-anchored cards below the inset.
 */
export const NARROW_CSS = `
@media (max-width: 600px) {
  .mbr-editor-backdrop { align-items: stretch; }
  .mbr-editor-modal {
    width: 100vw; height: 100vh; height: 100dvh; border-radius: 0;
    padding: env(safe-area-inset-top, 0px) env(safe-area-inset-right, 0px)
      env(safe-area-inset-bottom, 0px) env(safe-area-inset-left, 0px);
    box-sizing: border-box;
  }
  .milkdown .ProseMirror { padding: 16px 14px 24px 30px; }
  .mbr-editor-footer .status { flex: 1 1 100%; order: -1; }
  .mbr-editor-footer .status:empty { display: none; }
  .mbr-editor-modal .mbr-editor-token { min-width: 0; flex: 1 1 100%; font-size: 16px; }
  .mbr-editor-fm textarea { font-size: 16px; }
  .mbr-picker-field input[type="text"], .mbr-picker-field textarea { font-size: 16px; }
  /* A card at a fixed distance below the top inset (the native bar in the
   * app, the notch in a browser), never a bottom sheet: the list shrinks as
   * the reader types, and a bottom-anchored box then moves its path field
   * around under their finger. Anchored at the top only the bottom edge moves.
   * The cap uses svh, which the on-screen keyboard does not change. */
  .mbr-picker-backdrop {
    align-items: flex-start;
    padding-top: calc(env(safe-area-inset-top, 0px) + 0.75rem);
  }
  .mbr-picker-modal {
    width: calc(100vw - 1rem); border-radius: 12px;
    max-height: calc(100vh - env(safe-area-inset-top, 0px) - 2rem);
    max-height: calc(100svh - env(safe-area-inset-top, 0px) - 2rem);
    box-sizing: border-box;
  }
  .mbr-picker-footer { flex-wrap: wrap; }
  .mbr-picker-footer button, .mbr-picker-close, .mbr-picker-confirm button { min-height: 44px; }
  .mbr-picker-close { min-width: 44px; }
  .milkdown .milkdown-toolbar {
    flex-wrap: wrap;
    max-width: calc(100vw - 16px);
  }
  .milkdown .milkdown-toolbar .toolbar-item { margin: 4px; }
  .milkdown .milkdown-toolbar .divider { margin: 10px 2px; }
}
`;

/**
 * The editor's bars, in the shape of a native editing sheet: Cancel on the
 * left, the note's name in the middle, a ⋯ menu (New / Rename / Move) and Save
 * on the right; a format bar of icon + label tools at the bottom. One layout at
 * every width. Scoped under `.mbr-editor-modal` so it outranks Pico's global
 * `button` rules (filled, bordered, full-width in places), which would
 * otherwise turn every bar button into a coloured block.
 */
const HEADER_CSS = `
.mbr-editor-header {
  display: grid; grid-template-columns: auto minmax(0, 1fr) auto; align-items: center;
  gap: 0.25rem; min-height: 44px; padding: 0 0.25rem;
  border-bottom: 1px solid var(--pico-muted-border-color, #e0e0e0);
}
.mbr-editor-modal .mbr-editor-bar-btn,
.mbr-editor-modal .mbr-editor-tool {
  appearance: none; -webkit-appearance: none;
  background: none; border: 0; box-shadow: none; margin: 0;
  padding: 0 0.75rem; min-height: 44px; min-width: 44px; width: auto;
  font: inherit; font-size: 1.0625rem; line-height: 1.2;
  color: var(--pico-primary, #0172ad); border-radius: 8px; cursor: pointer;
  display: inline-flex; align-items: center; justify-content: center; gap: 0.35rem;
}
.mbr-editor-modal .mbr-editor-bar-btn:hover,
.mbr-editor-modal .mbr-editor-tool:hover {
  background: color-mix(in srgb, currentColor 10%, transparent);
}
.mbr-editor-modal .mbr-editor-bar-btn:focus-visible,
.mbr-editor-modal .mbr-editor-tool:focus-visible,
.mbr-editor-modal .mbr-editor-menu [role="menuitem"]:focus-visible {
  outline: 2px solid var(--pico-primary, #0172ad); outline-offset: -2px;
}
.mbr-editor-modal .mbr-editor-save { font-weight: 600; }
.mbr-editor-modal .mbr-editor-more { font-size: 1.4rem; letter-spacing: 0.05em; }
.mbr-editor-title { min-width: 0; text-align: center; line-height: 1.2; }
.mbr-editor-title h2 {
  margin: 0; font-size: 1.0625rem; font-weight: 600; color: inherit;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.mbr-editor-title .path {
  display: block; font-size: 0.75rem; color: var(--pico-muted-color, #6b7280);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.mbr-editor-title .path:empty { display: none; }
.mbr-editor-actions { position: relative; display: flex; align-items: center; }
.mbr-editor-menu {
  position: absolute; top: calc(100% + 4px); right: 0; z-index: 10;
  min-width: 13rem; padding: 0.35rem;
  background: var(--pico-card-background-color, var(--pico-background-color, #fff));
  color: var(--pico-color, #1a1a1a);
  border: 1px solid var(--pico-muted-border-color, #e0e0e0); border-radius: 12px;
  box-shadow: 0 10px 32px rgba(0,0,0,0.22);
  display: flex; flex-direction: column;
}
.mbr-editor-menu[hidden] { display: none; }
.mbr-editor-modal .mbr-editor-menu [role="menuitem"] {
  appearance: none; -webkit-appearance: none;
  background: none; border: 0; box-shadow: none; margin: 0; width: 100%;
  min-height: 44px; padding: 0 0.75rem; border-radius: 8px;
  font: inherit; font-size: 1rem; color: inherit; text-align: left; cursor: pointer;
}
.mbr-editor-modal .mbr-editor-menu [role="menuitem"]:hover,
.mbr-editor-modal .mbr-editor-menu [role="menuitem"]:focus {
  background: color-mix(in srgb, currentColor 10%, transparent);
}
.mbr-editor-tool svg { width: 1.15em; height: 1.15em; flex: none; }
.mbr-editor-modal .mbr-editor-tool { font-size: 0.95rem; }
`;

// Footnote nodes (from the gfm preset) are unstyled in the editor by default.
// Style the inline reference as a raised citation badge and the definition as a
// clearly-delimited note, reusing the same Crepe theme variables as THEME_CSS.
const FOOTNOTE_CSS = `
.milkdown .ProseMirror sup[data-type="footnote_reference"] {
  font-size: 0.75em;
  line-height: 0;
  vertical-align: super;
  font-weight: 600;
  color: var(--crepe-color-primary);
  padding: 0 0.15em;
  cursor: default;
  user-select: none;
}
.milkdown .ProseMirror dl[data-type="footnote_definition"] {
  margin: 0.4rem 0;
  padding: 0.15rem 0 0.15rem 0.75rem;
  border-left: 2px solid var(--crepe-color-selected, var(--pico-muted-border-color, #e1e2e8));
  font-size: 0.9em;
  color: var(--crepe-color-on-surface-variant, var(--pico-muted-color, #43474e));
}
.milkdown .ProseMirror dl[data-type="footnote_definition"] dt {
  display: inline;
  margin-right: 0.35rem;
  font-weight: 700;
  color: var(--crepe-color-primary);
}
.milkdown .ProseMirror dl[data-type="footnote_definition"] dt::before { content: "["; }
.milkdown .ProseMirror dl[data-type="footnote_definition"] dt::after { content: "]"; }
.milkdown .ProseMirror dl[data-type="footnote_definition"] dd {
  display: inline;
  margin: 0;
}
`;

/**
 * Smallest positive integer (as a string) not already present in `used`.
 * Footnote labels default to sequential numbers; this keeps them unique.
 */
function nextFootnoteLabel(used: ReadonlySet<string>): string {
  let n = 1;
  while (used.has(String(n))) n++;
  return String(n);
}

/**
 * Insert a footnote in a single transaction: a `footnote_reference` at the
 * current selection and an empty `footnote_definition` appended to the end of
 * the document, then move the cursor into the new definition so the user can
 * type the note immediately.
 *
 * Run via `crepe.editor.action(insertFootnote)`.
 */
function insertFootnote(ctx: Ctx): void {
  const view = ctx.get(editorViewCtx);
  const schema = ctx.get(schemaCtx);
  const refType = schema.nodes.footnote_reference;
  const defType = schema.nodes.footnote_definition;
  const paragraph = schema.nodes.paragraph;
  if (!refType || !defType || !paragraph) return;

  const { state } = view;

  // Collect labels already used by references or definitions so the new one is
  // unique across the whole document.
  const used = new Set<string>();
  state.doc.descendants((node) => {
    if (node.type === refType || node.type === defType) {
      const label = node.attrs.label;
      if (typeof label === 'string' && label) used.add(label);
    }
    return true;
  });
  const label = nextFootnoteLabel(used);

  const refNode = refType.create({ label });
  const defNode = defType.create({ label }, paragraph.create());

  let tr = state.tr.replaceSelectionWith(refNode, false);
  // Append the definition at the end of the document (positions already
  // reflect the inserted reference because `tr` tracks them).
  const defStart = tr.doc.content.size;
  tr = tr.insert(defStart, defNode);
  // Cursor inside the definition's empty paragraph: +1 enters the <dl>, +1
  // more enters the paragraph.
  const cursorPos = Math.min(defStart + 2, tr.doc.content.size);
  tr = tr.setSelection(TextSelection.create(tr.doc, cursorPos)).scrollIntoView();

  view.dispatch(tr);
  view.focus();
}

/** Builds the editing modal, loads the file, and wires up save/auth/errors. */
export async function openEditor(opts: OpenEditorOptions): Promise<void> {
  injectStyles();

  // Bearer token for this editor session. Seeded from the page and published
  // straight back to it on every keystroke; never persisted here or anywhere.
  let sessionToken = opts.token ?? '';

  const backdrop = document.createElement('div');
  backdrop.className = 'mbr-editor-backdrop';
  const modal = document.createElement('div');
  modal.className = 'mbr-editor-modal';
  backdrop.appendChild(modal);

  let crepe: Crepe | null = null;
  let baseHash = '';
  // Normalized document content as of load / last successful save. Compared
  // against the live content to detect unsaved edits before File operations.
  let savedContent = '';
  // True once Crepe is up and `savedContent` holds its normalized baseline.
  // Until then nothing can be dirty and nothing can be saved.
  let ready = false;
  let closed = false;

  const close = () => {
    if (closed) return;
    closed = true;
    if (crepe) {
      try { crepe.destroy(); } catch { /* ignore */ }
    }
    document.removeEventListener('keydown', onKeydown);
    closeMenu();
    backdrop.remove();
    opts.onClose();
  };

  // Replaced once the editor is up (it needs the dirty check and the save).
  // Before that there is nothing to lose, so closing is immediate.
  let requestClose: () => Promise<void> = async () => close();
  let saveFromShortcut: () => void = () => {};

  const onKeydown = (e: KeyboardEvent) => {
    // A nested picker or the unsaved-changes prompt owns the keyboard.
    if (document.querySelector('.mbr-picker-backdrop')) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      void requestClose();
    } else if (
      (e.metaKey || e.ctrlKey) &&
      !e.altKey &&
      !e.shiftKey &&
      e.key.toLowerCase() === 's'
    ) {
      // Always swallow it: the browser's own "Save page as…" is never wanted here.
      e.preventDefault();
      saveFromShortcut();
    }
  };
  document.addEventListener('keydown', onKeydown);
  backdrop.addEventListener('mousedown', (e) => {
    if (e.target === backdrop) void requestClose();
  });

  // The ⋯ menu's open/close live up here because `close()` tears it down.
  // Both are rebound once the header exists.
  let closeMenu: () => void = () => {};

  // Loading placeholder while we fetch + spin up Crepe.
  const loading = document.createElement('div');
  loading.className = 'mbr-editor-loading';
  loading.textContent = 'Loading editor…';
  modal.appendChild(loading);
  document.body.appendChild(backdrop);
  // The modal is now visible — let the trigger hide its own loading spinner.
  opts.onReady?.();

  const authHeaders = (extra?: Record<string, string>): Record<string, string> => {
    const h: Record<string, string> = { 'X-MBR-Edit': '1', ...extra };
    if (sessionToken) h['Authorization'] = `Bearer ${sessionToken}`;
    return h;
  };

  // Uploads a pasted/dropped/picked image to the server and returns its
  // root-absolute URL, which Crepe uses as the image `src`. Wired into the
  // ImageBlock feature's `onUpload` (see the Crepe config below), replacing
  // Crepe's default client-only `blob:` object URL so images actually persist.
  // Assets land in the note's own folder; the endpoint returns a root-absolute
  // `url` (e.g. `/notes/image.png`) which we return as-is — a bare-relative path
  // would resolve against the editor page URL and break the preview + render.
  async function uploadFile(file: File): Promise<string> {
    const dir = noteDir(opts.filePath);
    const resp = await fetch(
      `/.mbr/upload?dir=${encodeURIComponent(dir)}&name=${encodeURIComponent(file.name)}`,
      {
        method: 'POST',
        headers: authHeaders({ 'Content-Type': file.type || 'application/octet-stream' }),
        credentials: 'same-origin',
        body: file,
      },
    );
    if (!resp.ok) throw new Error(describeUploadError(resp.status));
    const data = (await resp.json()) as { url: string; path: string; name: string };
    return data.url;
  }

  // Fetch raw markdown.
  let raw: string;
  try {
    const resp = await fetch(opts.rawUrl, {
      headers: authHeaders(),
      credentials: 'same-origin',
    });
    if (!resp.ok) {
      loading.textContent = describeError(resp.status, 'load');
      return;
    }
    baseHash = resp.headers.get('X-MBR-Content-Hash') ?? '';
    raw = await resp.text();
  } catch (err) {
    loading.textContent = `Failed to load file: ${(err as Error).message}`;
    return;
  }

  // The editor works in LF; a CRLF file is saved back as CRLF (`currentContent`).
  const lineEnding = lineEndingOf(raw);
  const { frontmatter, body } = splitFrontmatter(toLf(raw));
  loading.remove();

  // Build modal chrome: Cancel | title | ⋯ Save, as a native editing sheet.
  modal.innerHTML = '';
  const header = document.createElement('div');
  header.className = 'mbr-editor-header';

  const cancelBtn = barButton('Cancel', 'mbr-editor-cancel');
  cancelBtn.title = 'Close the editor (Esc)';
  cancelBtn.addEventListener('click', () => void requestClose());

  const { name, folder } = splitFilePath(opts.filePath);
  const title = document.createElement('div');
  title.className = 'mbr-editor-title';
  title.title = opts.filePath;
  const h2 = document.createElement('h2');
  h2.textContent = name;
  const pathLine = document.createElement('span');
  pathLine.className = 'path';
  pathLine.textContent = folder;
  title.append(h2, pathLine);

  // File actions (New / Rename / Move) behind ⋯. Rename and Move share the path
  // picker (rename = move to a new filename); they differ only in the initial
  // cursor selection. Each guards unsaved edits before navigating away.
  const actions = document.createElement('div');
  actions.className = 'mbr-editor-actions';
  const moreBtn = barButton('⋯', 'mbr-editor-more');
  moreBtn.setAttribute('aria-label', 'File actions');
  moreBtn.title = 'File actions';
  moreBtn.setAttribute('aria-haspopup', 'menu');
  moreBtn.setAttribute('aria-expanded', 'false');
  const menu = document.createElement('div');
  menu.className = 'mbr-editor-menu';
  menu.setAttribute('role', 'menu');
  menu.setAttribute('aria-label', 'File actions');
  menu.hidden = true;
  const menuItem = (label: string, hint: string, run: () => void): HTMLButtonElement => {
    const item = document.createElement('button');
    item.type = 'button';
    item.setAttribute('role', 'menuitem');
    item.tabIndex = -1;
    item.textContent = label;
    item.title = hint;
    item.addEventListener('click', () => {
      closeMenu();
      run();
    });
    return item;
  };
  const menuItems = [
    menuItem('New note…', 'Create a new markdown file', () => void handleNew()),
    menuItem('Rename…', 'Rename this file', () => void handleMove('basename')),
    menuItem('Move…', 'Move this file to another folder', () => void handleMove('folder')),
  ];
  menu.append(...menuItems);

  // While open, a capture-phase listener on the document sees Escape before
  // the editor's own (bubble-phase) handler and stops it, so Escape closes the
  // menu, not the editor; a pointerdown anywhere outside closes it too.
  const onMenuKeydown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      closeMenu();
      moreBtn.focus();
      return;
    }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const at = menuItems.indexOf(document.activeElement as HTMLButtonElement);
      const step = e.key === 'ArrowDown' ? 1 : -1;
      const next = (at + step + menuItems.length) % menuItems.length;
      menuItems[at < 0 && step < 0 ? menuItems.length - 1 : next].focus();
    }
  };
  const onOutsidePointer = (e: Event) => {
    const t = e.target as Node | null;
    if (t && (menu.contains(t) || moreBtn.contains(t))) return;
    closeMenu();
  };
  const openMenu = () => {
    menu.hidden = false;
    moreBtn.setAttribute('aria-expanded', 'true');
    document.addEventListener('keydown', onMenuKeydown, true);
    document.addEventListener('pointerdown', onOutsidePointer, true);
    menuItems[0].focus();
  };
  closeMenu = () => {
    if (menu.hidden) return;
    menu.hidden = true;
    moreBtn.setAttribute('aria-expanded', 'false');
    document.removeEventListener('keydown', onMenuKeydown, true);
    document.removeEventListener('pointerdown', onOutsidePointer, true);
  };
  moreBtn.addEventListener('click', () => (menu.hidden ? openMenu() : closeMenu()));

  const saveBtn = barButton('Save', 'mbr-editor-save');
  saveBtn.title = 'Save (⌘S / Ctrl+S)';
  actions.append(moreBtn, menu, saveBtn);

  header.append(cancelBtn, title, actions);
  modal.appendChild(header);

  const bodyWrap = document.createElement('div');
  bodyWrap.className = 'mbr-editor-body';

  // Frontmatter editor (raw YAML), collapsed when empty.
  const fmDetails = document.createElement('details');
  fmDetails.className = 'mbr-editor-fm';
  if (frontmatter) fmDetails.open = true;
  const fmSummary = document.createElement('summary');
  fmSummary.textContent = 'YAML frontmatter';
  const fmTextarea = document.createElement('textarea');
  fmTextarea.value = frontmatter ?? '';
  fmTextarea.spellcheck = false;
  fmTextarea.setAttribute('aria-label', 'YAML frontmatter');
  fmDetails.appendChild(fmSummary);
  fmDetails.appendChild(fmTextarea);
  bodyWrap.appendChild(fmDetails);

  // Crepe body editor.
  const crepeHost = document.createElement('div');
  crepeHost.className = 'mbr-editor-crepe';
  bodyWrap.appendChild(crepeHost);
  modal.appendChild(bodyWrap);

  // Footer: the format bar, status and the token field.
  const footer = document.createElement('div');
  footer.className = 'mbr-editor-footer';
  const status = document.createElement('span');
  status.className = 'status';
  status.setAttribute('role', 'status');
  const tokenInput = document.createElement('input');
  tokenInput.type = 'password';
  tokenInput.className = 'mbr-editor-token';
  tokenInput.placeholder = 'Edit token';
  tokenInput.value = sessionToken;
  tokenInput.autocomplete = 'off';
  tokenInput.setAttribute('aria-label', 'Edit token');
  // Publish as the user types, rather than only on save. Uploads, New, Rename
  // and Move all read `sessionToken` through `authHeaders`, and a token typed
  // for one of those used not to be picked up until a save happened to run.
  tokenInput.addEventListener('input', () => {
    sessionToken = tokenInput.value.trim();
    opts.onToken?.(sessionToken);
  });
  // Insert-footnote helper. Lives in the footer chrome we fully control (rather
  // than a slash-menu item) so the insert never has to reconcile with Crepe's
  // typed `/query` range. ProseMirror keeps the last selection even when focus
  // moves to this button, so the reference lands at the previous cursor.
  const footnoteBtn = toolButton(FOOTNOTE_ICON, 'Footnote', 'mbr-editor-footnote');
  footnoteBtn.title = 'Insert a footnote at the cursor';
  footnoteBtn.setAttribute('aria-label', 'Insert footnote');
  footnoteBtn.addEventListener('click', () => {
    if (!crepe) return;
    try {
      crepe.editor.action(insertFootnote);
    } catch (err) {
      setStatus(`Could not insert footnote: ${(err as Error).message}`, 'error');
    }
  });
  // Insert-media helper. Context-aware: with an embed at/adjacent to the cursor
  // it opens the picker in edit mode (prefilled) to replace it; otherwise it
  // inserts a new embed at the cursor. Its label tracks that context (see
  // `setMediaButtonMode`); double-clicking any embed is the primary edit path.
  const mediaBtn = toolButton(MEDIA_ICON, 'Media', 'mbr-editor-media');
  const mediaLabel = mediaBtn.querySelector('.label') as HTMLSpanElement;
  mediaBtn.title = 'Insert a media embed — double-click an existing embed to edit';
  mediaBtn.setAttribute('aria-label', 'Insert media');
  mediaBtn.addEventListener('click', () => void handleMedia());
  const tools = document.createElement('div');
  tools.className = 'mbr-editor-tools';
  tools.append(footnoteBtn, mediaBtn);
  footer.append(tools, status, tokenInput);
  modal.appendChild(footer);

  const setStatus = (msg: string, kind: '' | 'ok' | 'error' = '') => {
    status.textContent = msg;
    status.className = `status${kind ? ' ' + kind : ''}`;
  };

  // Reflects context on the media button: when an embed is the current target
  // (selection or cursor-adjacent) it reads "Edit media"; otherwise "Insert
  // media". Driven by the media-edit plugin's selection-change hook. Defined
  // before Crepe so the plugin can call it from its first `update`.
  const setMediaButtonMode = (target: EditTarget | null): void => {
    const editing = target !== null;
    mediaLabel.textContent = editing ? 'Edit media' : 'Media';
    mediaBtn.setAttribute('aria-label', editing ? 'Edit media' : 'Insert media');
    mediaBtn.title = editing
      ? 'Edit the selected embed — or double-click any embed to edit'
      : 'Insert a media embed — double-click an existing embed to edit';
  };

  // Instantiate Crepe (see CREPE_FEATURES for why CodeMirror/LaTeX are off).
  try {
    crepe = new Crepe({
      root: crepeHost,
      defaultValue: body,
      features: CREPE_FEATURES,
      // Route the ImageBlock uploader to our server endpoint. Crepe falls back
      // `inlineOnUpload ?? onUpload` and `blockOnUpload ?? onUpload`, and its
      // drop/paste uploader reads the same block `onUpload`, so this single
      // `onUpload` covers the upload button, drag-drop, and paste.
      //
      // `proxyDomURL` swaps heavy media (video/audio/PDF authored with image
      // syntax) for a lightweight inline-SVG icon in the displayed `<img>`, so
      // the browser never tries to download the whole file as an image. Crepe
      // forwards this single `proxyDomURL` to both inline and block images, and
      // it only touches the reactive `src` — the saved markdown is unchanged.
      featureConfigs: {
        [CrepeFeature.ImageBlock]: {
          onUpload: (file: File) => uploadFile(file),
          proxyDomURL: (url: string) => mediaProxyURL(url),
        },
      },
    });
    // mbr's markdown syntax first, so the editing aids below see its nodes.
    useMarkdownExtensions(crepe);
    // Register the `[[` link-autocomplete plugin on the underlying Milkdown
    // editor before create() — the Crepe 7.21.3 plugin-injection point.
    crepe.editor.use(
      createLinkAutocompletePlugin({
        fetchSiteFiles: fetchSiteMarkdownFiles,
        currentUrl: () => safeDecodePath(window.location.pathname),
      }),
    );
    // Double-click-to-edit for media embeds (same injection point). It calls
    // back into the picker flow below: `onEditMedia` for a double-clicked node,
    // `onTargetChange` to keep the footer button label in sync.
    crepe.editor.use(
      createMediaEditPlugin({
        onEditMedia: (target) => void handleMediaWith(target),
        onTargetChange: (target) => setMediaButtonMode(target),
      }),
    );
    await crepe.create();
  } catch (err) {
    console.error('Crepe failed to initialize:', err);
    setStatus(`Failed to start editor: ${(err as Error).message}`, 'error');
    return;
  }

  // Baseline for dirty-tracking: Crepe normalizes markdown on load, so capture
  // the normalized content now rather than comparing against the raw source.
  const currentContent = (): string =>
    crepe ? withLineEnding(recombine(fmTextarea.value, serializeBody(crepe)), lineEnding) : savedContent;
  const isDirty = (): boolean => ready && currentContent() !== savedContent;
  savedContent = currentContent();
  ready = true;

  const doSave = async (): Promise<boolean> => {
    if (!crepe || !ready) return false;
    // Belt and braces alongside the `input` listener: a value set some way that
    // fires no input event still reaches the request and the main bundle.
    sessionToken = tokenInput.value.trim();
    opts.onToken?.(sessionToken);
    const content = currentContent();
    saveBtn.setAttribute('aria-busy', 'true');
    setStatus('Saving…');
    try {
      const resp = await fetch(opts.saveUrl, {
        method: 'POST',
        headers: authHeaders({ 'Content-Type': 'application/json' }),
        credentials: 'same-origin',
        body: JSON.stringify({ content, base_hash: baseHash }),
      });
      saveBtn.removeAttribute('aria-busy');
      if (resp.ok) {
        baseHash = resp.headers.get('X-MBR-Content-Hash') ?? baseHash;
        savedContent = content;
        setStatus('Saved. The page will reload.', 'ok');
        return true;
      }
      if (resp.status === 401) {
        tokenInput.classList.add('show');
        tokenInput.focus();
      }
      setStatus(describeError(resp.status, 'save'), 'error');
      return false;
    } catch (err) {
      saveBtn.removeAttribute('aria-busy');
      setStatus(`Save failed: ${(err as Error).message}`, 'error');
      return false;
    }
  };
  /** A save the reader asked for: on success, let the trigger refresh the page. */
  const saveAndNotify = async (): Promise<void> => {
    if (saveBtn.hasAttribute('aria-busy')) return;
    // Nothing changed: write nothing. The baseline is Crepe's *normalized*
    // markdown, not the file, so posting it would replace the source with
    // remark's re-serialization of it — which is lossy for everything
    // CommonMark+GFM cannot represent (definition-list continuations,
    // `--- {attrs}`, `[-]`/`[>]` tasks; see editor-roundtrip.test.ts). Skipping
    // the request leaves the file byte-for-byte as it was.
    if (!isDirty()) {
      setStatus('No changes to save.');
      return;
    }
    if (await doSave()) opts.onSaved?.();
  };
  saveBtn.addEventListener('click', () => void saveAndNotify());
  saveFromShortcut = () => void saveAndNotify();

  // ---------------------------------------------------------------------------
  // File operations (New / Rename / Move) and media insertion.
  // ---------------------------------------------------------------------------

  /**
   * Prompts to save/discard/cancel when there are unsaved edits. Resolves
   * `'clean'`, `'saved'` or `'discarded'` when it's safe to proceed, `null` on
   * cancel (or a failed save, whose error is then on the status line).
   */
  let guarding = false;
  const guardUnsaved = async (): Promise<'clean' | 'saved' | 'discarded' | null> => {
    if (!isDirty()) return 'clean';
    if (guarding) return null;
    guarding = true;
    try {
      const choice = await confirmUnsaved();
      if (choice === 'cancel') return null;
      if (choice === 'save') return (await doSave()) ? 'saved' : null;
      return 'discarded';
    } finally {
      guarding = false;
    }
  };

  // Cancel, Escape and a backdrop click all close through the guard: unsaved
  // edits used to be dropped without a word.
  requestClose = async () => {
    const outcome = await guardUnsaved();
    if (outcome === null) return;
    close();
    if (outcome === 'saved') opts.onSaved?.();
  };

  const handleNew = async (): Promise<void> => {
    if ((await guardUnsaved()) === null) return;
    const dest = await openPathPicker({
      mode: 'new',
      currentFsPath: opts.filePath,
      fetchSiteFiles: fetchSiteMarkdownFiles,
    });
    if (dest) await doCreate(dest.path, dest.createDirs);
  };

  const handleMove = async (select: 'basename' | 'folder'): Promise<void> => {
    if ((await guardUnsaved()) === null) return;
    const dest = await openPathPicker({
      mode: 'move',
      currentFsPath: opts.filePath,
      select,
      fetchSiteFiles: fetchSiteMarkdownFiles,
    });
    if (dest) await doMove(dest.path, dest.createDirs);
  };

  const doCreate = async (path: string, createDirs: boolean): Promise<void> => {
    setStatus('Creating…');
    try {
      const result = await postCreate(path, createDirs, authHeaders());
      if (result.ok) {
        setStatus('Created. Opening…', 'ok');
        window.location.href = result.urlPath + EDIT_ON_ARRIVAL_HASH;
        return;
      }
      if (result.status === 401) {
        tokenInput.classList.add('show');
        tokenInput.focus();
      }
      setStatus(describeFileOpError(result.status, 'create'), 'error');
    } catch (err) {
      setStatus(`Create failed: ${(err as Error).message}`, 'error');
    }
  };

  const doMove = async (to: string, createDirs: boolean): Promise<void> => {
    setStatus('Moving…');
    try {
      const resp = await fetch(`/.mbr/move/${encodeFsPath(opts.filePath)}`, {
        method: 'POST',
        headers: authHeaders({ 'Content-Type': 'application/json' }),
        credentials: 'same-origin',
        body: JSON.stringify({ to, create_dirs: createDirs }),
      });
      if (resp.ok) {
        const data = (await resp.json()) as {
          url_path: string;
          rewritten?: string[];
          wikilinks_rewritten?: string[];
        };
        const n = (data.rewritten?.length ?? 0) + (data.wikilinks_rewritten?.length ?? 0);
        // Mark clean so we don't re-prompt; the target page reloads fresh anyway.
        savedContent = currentContent();
        setStatus(
          n > 0
            ? `Moved. ${n} link${n === 1 ? '' : 's'} rewritten. Opening…`
            : 'Moved. Opening…',
          'ok',
        );
        window.location.href = data.url_path;
        return;
      }
      if (resp.status === 401) {
        tokenInput.classList.add('show');
        tokenInput.focus();
      }
      setStatus(describeFileOpError(resp.status, 'move'), 'error');
    } catch (err) {
      setStatus(`Move failed: ${(err as Error).message}`, 'error');
    }
  };

  /**
   * Opens the media picker for `editTarget` (edit mode, prefilled) or, when
   * `null`, in insert mode at the cursor, then applies the result. Shared by the
   * footer button and the double-click-to-edit plugin.
   */
  const handleMediaWith = async (editTarget: EditTarget | null): Promise<void> => {
    if (!crepe) return;
    const result = await openMediaPicker({
      mode: editTarget ? 'edit' : 'insert',
      initial: editTarget ? { src: editTarget.src, caption: editTarget.caption } : undefined,
      fetchMediaFiles: fetchMediaOtherFiles,
    });
    if (!result) return;
    try {
      applyMediaResult(crepe, result, editTarget);
    } catch (err) {
      setStatus(`Could not insert media: ${(err as Error).message}`, 'error');
    }
  };

  /** Footer media button: edit the embed at the cursor, else insert a new one. */
  const handleMedia = async (): Promise<void> => {
    if (!crepe) return;
    await handleMediaWith(selectedImageNode(crepe));
  };

  /** Small nested Save/Discard/Cancel dialog for the unsaved-changes guard. */
  const confirmUnsaved = (): Promise<'save' | 'discard' | 'cancel'> =>
    new Promise((resolve) => {
      let done = false;
      const finish = (r: 'save' | 'discard' | 'cancel') => {
        if (done) return;
        done = true;
        shell.destroy();
        resolve(r);
      };
      const shell = createNestedModal({
        ariaLabel: 'Unsaved changes',
        onCancel: () => finish('cancel'),
      });
      const head = document.createElement('div');
      head.className = 'mbr-picker-header';
      head.innerHTML = '<h3>Unsaved changes</h3>';
      const body = document.createElement('div');
      body.className = 'mbr-picker-field';
      body.textContent = 'Save your changes before continuing?';
      const foot = document.createElement('div');
      foot.className = 'mbr-picker-footer';
      const spacer = document.createElement('span');
      spacer.className = 'mbr-picker-status';
      const cancel = document.createElement('button');
      cancel.type = 'button';
      cancel.textContent = 'Cancel';
      cancel.addEventListener('click', () => finish('cancel'));
      const discard = document.createElement('button');
      discard.type = 'button';
      discard.textContent = 'Discard';
      discard.addEventListener('click', () => finish('discard'));
      const save = document.createElement('button');
      save.type = 'button';
      save.textContent = 'Save';
      save.addEventListener('click', () => finish('save'));
      foot.append(spacer, cancel, discard, save);
      shell.modal.append(head, body, foot);
      save.focus();
    });

  // Show the field when a token is already in hand, or when the page has had a
  // write refused for want of one — the token lives only as long as the page,
  // so on a token-protected server the second case is the usual one.
  if (sessionToken || opts.tokenRequired) tokenInput.classList.add('show');
}

/**
 * Fragment a page is opened with to start editing on arrival. `<mbr-editor>`
 * strips it and opens the editor, so a freshly created note opens ready to
 * type. Mirrored as a literal in `mbr-editor.ts`, which cannot import this
 * chunk.
 */
export const EDIT_ON_ARRIVAL_HASH = '#mbr-edit';

/** Percent-encodes each segment of a repo-relative path for a URL. */
function encodeFsPath(p: string): string {
  return p.split('/').map(encodeURIComponent).join('/');
}

/** Minimal starter body for a newly created file (an H1 of its stem). */
export function newFileTemplate(path: string): string {
  const stem = (path.split('/').pop() ?? '').replace(/\.[^.]+$/, '');
  return `# ${stem}\n`;
}

/** Outcome of {@link postCreate}. */
export type CreateResult = { ok: true; urlPath: string } | { ok: false; status: number };

/**
 * `POST /.mbr/create/<path>` with the starter template. The one create request,
 * shared by the editor's New and {@link createNote}. Network failures reject.
 */
export async function postCreate(
  path: string,
  createDirs: boolean,
  headers: Record<string, string>,
): Promise<CreateResult> {
  const resp = await fetch(`/.mbr/create/${encodeFsPath(path)}`, {
    method: 'POST',
    headers: { ...headers, 'Content-Type': 'application/json' },
    credentials: 'same-origin',
    body: JSON.stringify({ content: newFileTemplate(path), create_dirs: createDirs }),
  });
  if (!resp.ok) return { ok: false, status: resp.status };
  const data = (await resp.json()) as { url_path: string };
  return { ok: true, urlPath: data.url_path };
}

export interface CreateNoteOptions {
  /** Repo-relative folder to prefill (`''` for the root). */
  folder: string;
  /** The page's in-memory edit token, if it has one (see `edit-token.ts`). */
  token?: string;
  /** Called on a 401, so the main bundle can show the token field next time. */
  onAuthRequired?: () => void;
}

/**
 * "New note" without an open editor: pick a path, create the file, and open
 * it with {@link EDIT_ON_ARRIVAL_HASH} so the editor comes up on arrival.
 *
 * There is no editor footer to type a token into here, so a refusal is an
 * `alert` (native in the iOS app). On a token-protected server the token the
 * page holds lives only in memory and does not survive the navigation: the new
 * note's page asks for it again on its first save.
 */
export async function createNote(opts: CreateNoteOptions): Promise<void> {
  injectStyles();
  const dest = await openPathPicker({
    mode: 'new',
    currentFsPath: '',
    initialFolder: opts.folder,
    fetchSiteFiles: fetchSiteMarkdownFiles,
  });
  if (!dest) return;
  const headers: Record<string, string> = { 'X-MBR-Edit': '1' };
  if (opts.token) headers['Authorization'] = `Bearer ${opts.token}`;
  let result: CreateResult;
  try {
    result = await postCreate(dest.path, dest.createDirs, headers);
  } catch (err) {
    window.alert(`Create failed: ${(err as Error).message}`);
    return;
  }
  if (result.ok) {
    window.location.href = result.urlPath + EDIT_ON_ARRIVAL_HASH;
    return;
  }
  if (result.status === 401) opts.onAuthRequired?.();
  window.alert(describeFileOpError(result.status, 'create'));
}

/** A header bar button (Cancel / ⋯ / Save): text only, native-sheet styling. */
function barButton(label: string, className: string): HTMLButtonElement {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = `mbr-editor-bar-btn ${className}`;
  b.textContent = label;
  return b;
}

/** A format-bar tool: a small inline icon plus a text label. */
function toolButton(iconSvg: string, label: string, className: string): HTMLButtonElement {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = `mbr-editor-tool ${className}`;
  b.innerHTML = iconSvg;
  const text = document.createElement('span');
  text.className = 'label';
  text.textContent = label;
  b.appendChild(text);
  return b;
}

const SVG_ATTRS =
  'xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
  'stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"';
/** Lines of text with a raised mark: a footnote. */
const FOOTNOTE_ICON = `<svg ${SVG_ATTRS}><path d="M3 8h11M3 13h11M3 18h7"/><path d="M18 4.5l1.5-1v6"/></svg>`;
/** A framed picture: media. */
const MEDIA_ICON = `<svg ${SVG_ATTRS}><rect x="3" y="4.5" width="18" height="15" rx="2"/><circle cx="8.5" cy="9.5" r="1.5"/><path d="M21 15.5l-5-5-9 9"/></svg>`;

/**
 * The note's name and its folder, for the title: `notes/2026/plan.md` →
 * `{ name: 'plan.md', folder: 'notes/2026' }`.
 */
export function splitFilePath(path: string): { name: string; folder: string } {
  const slash = path.lastIndexOf('/');
  return slash < 0
    ? { name: path, folder: '' }
    : { name: path.slice(slash + 1), folder: path.slice(0, slash) };
}

function describeError(status: number, phase: 'load' | 'save'): string {
  switch (status) {
    case 401:
      return 'Authentication required — enter your edit token and try again.';
    case 403:
      return 'Editing is disabled or this request was blocked.';
    case 409:
      return 'This file changed on disk since it was loaded. Reload the page before saving.';
    case 404:
      return 'File not found.';
    default:
      return `Failed to ${phase} (HTTP ${status}).`;
  }
}

/** Maps create/move HTTP failures to friendly messages. */
function describeFileOpError(status: number, op: 'create' | 'move'): string {
  switch (status) {
    case 400:
      return 'The destination folder does not exist. Retry and allow creating it.';
    case 401:
      return 'Authentication required — enter your edit token and try again.';
    case 403:
      return 'Editing is disabled or this request was blocked.';
    case 404:
      return op === 'move' ? 'The source file was not found.' : 'Destination not found.';
    case 409:
      return op === 'create'
        ? 'A file already exists at that path.'
        : 'Something already exists at the destination.';
    default:
      return `Failed to ${op} (HTTP ${status}).`;
  }
}

/**
 * Maps image-upload HTTP failures to friendly messages. Crepe surfaces the
 * thrown Error's message in the image block, so keep it self-explanatory.
 */
function describeUploadError(status: number): string {
  switch (status) {
    case 401:
      return 'Authentication required — enter your edit token and save once, then retry.';
    case 403:
      return 'Editing is disabled or this upload was blocked.';
    default:
      return `Upload failed (HTTP ${status}).`;
  }
}

/** Percent-decodes a URL path, tolerating a malformed escape sequence. */
function safeDecodePath(p: string): string {
  try {
    return decodeURIComponent(p);
  } catch {
    return p;
  }
}

/** Fetches the fresh `markdown_files` array from site.json (server mode). */
async function fetchSiteMarkdownFiles(): Promise<SiteMarkdownFile[]> {
  const resp = await fetch('/.mbr/site.json', { credentials: 'same-origin' });
  if (!resp.ok) throw new Error(`site.json ${resp.status}`);
  const data = (await resp.json()) as { markdown_files?: SiteMarkdownFile[] };
  return data.markdown_files ?? [];
}

/** Fetches the fresh `other_files` array from media.json (server mode). */
async function fetchMediaOtherFiles(): Promise<MediaFile[]> {
  const resp = await fetch('/.mbr/media.json', { credentials: 'same-origin' });
  if (!resp.ok) throw new Error(`media.json ${resp.status}`);
  const data = (await resp.json()) as { other_files?: MediaFile[] };
  return data.other_files ?? [];
}

/**
 * The embed at/adjacent to the current selection that the media picker can edit
 * in place, or `null`. See {@link detectEditTarget}: it matches a strict
 * `NodeSelection` on an image as well as an image just after/before the cursor,
 * so the footer button also works for Crepe's atom `image-block` (which rarely
 * yields a `NodeSelection`).
 */
function selectedImageNode(crepe: Crepe): EditTarget | null {
  return crepe.editor.action((ctx): EditTarget | null =>
    detectEditTarget(ctx.get(editorViewCtx).state.selection),
  );
}

/**
 * Applies a media picker result to the document: inserts a new embed at the
 * cursor, or (with `editTarget`) replaces the selected image. A `shortcode`
 * result (video with timestamps) is written as literal `{{ vid(...) }}` text —
 * a paragraph when replacing a block image, inline otherwise.
 */
function applyMediaResult(
  crepe: Crepe,
  result: MediaPickResult,
  editTarget: EditTarget | null,
): void {
  crepe.editor.action((ctx) => {
    const view: EditorView = ctx.get(editorViewCtx);
    const schema = ctx.get(schemaCtx);
    const { state } = view;
    let tr = state.tr;

    if (editTarget) {
      const from = editTarget.pos;
      const to = editTarget.pos + editTarget.nodeSize;
      if (result.form === 'image') {
        const node = state.doc.nodeAt(from);
        if (!node) return;
        const attrs =
          editTarget.typeName === 'image-block'
            ? { ...node.attrs, src: result.src, caption: result.caption }
            : { ...node.attrs, src: result.src, alt: result.caption };
        tr = tr.setNodeMarkup(from, undefined, attrs);
      } else if (editTarget.typeName === 'image-block') {
        const para = schema.nodes.paragraph.create(null, schema.text(result.shortcode));
        tr = tr.replaceRangeWith(from, to, para);
      } else {
        tr = tr.insertText(result.shortcode, from, to);
      }
    } else if (result.form === 'image') {
      const imageType = schema.nodes.image;
      if (!imageType) return;
      const node = imageType.create({ src: result.src, alt: result.caption, title: null });
      tr = tr.replaceSelectionWith(node, false);
    } else {
      tr = tr.insertText(result.shortcode);
    }

    view.dispatch(tr.scrollIntoView());
    view.focus();
  });
}
