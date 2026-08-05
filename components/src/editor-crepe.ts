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
import type { Ctx } from '@milkdown/kit/ctx';
import { recombine, splitFrontmatter, unescapeWikilinks } from './editor-frontmatter.js';
import { PICKER_CSS, createNestedModal } from './editor-picker-shared.js';
import { openPathPicker, type SiteMarkdownFile } from './editor-path-picker.js';
import { openMediaPicker, type MediaFile, type MediaPickResult } from './editor-media-picker.js';
import { createLinkAutocompletePlugin, AUTOCOMPLETE_CSS } from './editor-link-autocomplete.js';
import type { EditTarget } from './editor-media-target.js';
import { createMediaEditPlugin, detectEditTarget } from './editor-media-edit.js';
import { noteDir } from './editor-upload.js';
import { mediaProxyURL } from './editor-media-proxy.js';

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
.mbr-editor-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 0.75rem 1rem; border-bottom: 1px solid var(--pico-muted-border-color, #e0e0e0);
}
.mbr-editor-header h2 { margin: 0; font-size: 1rem; }
.mbr-editor-header .path { font-weight: normal; opacity: 0.7; font-size: 0.85rem; margin-left: 0.5rem; }
.mbr-editor-close { background: transparent; border: none; font-size: 1.4rem; cursor: pointer; color: inherit; line-height: 1; padding: 0.25rem 0.5rem; }
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
  display: flex; align-items: center; gap: 0.75rem;
  padding: 0.6rem 1rem; border-top: 1px solid var(--pico-muted-border-color, #e0e0e0);
}
.mbr-editor-footer .status { flex: 1; font-size: 0.85rem; }
.mbr-editor-footer .status.error { color: var(--pico-del-color, #b3261e); }
.mbr-editor-footer .status.ok { color: var(--pico-ins-color, #1a7f37); }
.mbr-editor-token { display: none; font-family: var(--pico-font-family-monospace, monospace); font-size: 0.85rem; padding: 0.3rem 0.5rem; min-width: 16rem; }
.mbr-editor-token.show { display: inline-block; }
.mbr-editor-footer button { padding: 0.35rem 0.9rem; cursor: pointer; }
.mbr-editor-loading { padding: 2rem; text-align: center; opacity: 0.7; }
`;

// Header file actions (New / Rename / Move) and the footer "Insert media"
// button. Kept visually light so they don't compete with the title/close.
const HEADER_CSS = `
.mbr-editor-fileactions { display: flex; align-items: center; gap: 0.3rem; margin-left: auto; margin-right: 0.5rem; }
.mbr-editor-fileactions button {
  font-size: 0.8rem; padding: 0.2rem 0.55rem; cursor: pointer;
  background: transparent; color: inherit;
  border: 1px solid var(--pico-muted-border-color, #ccc); border-radius: 5px;
}
.mbr-editor-fileactions button:hover { background: var(--pico-secondary-hover-background, #eceef4); }
.mbr-editor-media { }
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
.mbr-editor-footnote { margin-right: auto; }
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

  const close = () => {
    if (crepe) {
      try { crepe.destroy(); } catch { /* ignore */ }
    }
    document.removeEventListener('keydown', onKeydown);
    backdrop.remove();
    opts.onClose();
  };

  const onKeydown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
    }
  };
  document.addEventListener('keydown', onKeydown);
  backdrop.addEventListener('mousedown', (e) => {
    if (e.target === backdrop) close();
  });

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

  const { frontmatter, body } = splitFrontmatter(raw);
  loading.remove();

  // Build modal chrome.
  modal.innerHTML = '';
  const header = document.createElement('div');
  header.className = 'mbr-editor-header';
  header.innerHTML = `<h2>Edit<span class="path"></span></h2>`;
  header.querySelector('.path')!.textContent = opts.filePath;

  // File actions (New / Rename / Move). Rename and Move share the path picker
  // (rename = move to a new filename); they differ only in the initial cursor
  // selection. Each guards unsaved edits before navigating away.
  const fileActions = document.createElement('div');
  fileActions.className = 'mbr-editor-fileactions';
  const mkFileBtn = (label: string, title: string, onClick: () => void): HTMLButtonElement => {
    const b = document.createElement('button');
    b.type = 'button';
    b.textContent = label;
    b.title = title;
    b.addEventListener('click', onClick);
    return b;
  };
  fileActions.append(
    mkFileBtn('New', 'Create a new markdown file', () => void handleNew()),
    mkFileBtn('Rename', 'Rename this file', () => void handleMove('basename')),
    mkFileBtn('Move', 'Move this file to another folder', () => void handleMove('folder')),
  );
  header.appendChild(fileActions);

  const closeBtn = document.createElement('button');
  closeBtn.className = 'mbr-editor-close';
  closeBtn.setAttribute('aria-label', 'Close editor');
  closeBtn.textContent = '×';
  closeBtn.addEventListener('click', close);
  header.appendChild(closeBtn);
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

  // Footer: status, token field, actions.
  const footer = document.createElement('div');
  footer.className = 'mbr-editor-footer';
  const status = document.createElement('span');
  status.className = 'status';
  const tokenInput = document.createElement('input');
  tokenInput.type = 'password';
  tokenInput.className = 'mbr-editor-token';
  tokenInput.placeholder = 'Edit token';
  tokenInput.value = sessionToken;
  tokenInput.autocomplete = 'off';
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
  const footnoteBtn = document.createElement('button');
  footnoteBtn.type = 'button';
  footnoteBtn.className = 'mbr-editor-footnote';
  footnoteBtn.textContent = 'Footnote';
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
  const mediaBtn = document.createElement('button');
  mediaBtn.type = 'button';
  mediaBtn.className = 'mbr-editor-media';
  mediaBtn.textContent = 'Insert media';
  mediaBtn.title = 'Insert a media embed — double-click an existing embed to edit';
  mediaBtn.setAttribute('aria-label', 'Insert media');
  mediaBtn.addEventListener('click', () => void handleMedia());
  const cancelBtn = document.createElement('button');
  cancelBtn.textContent = 'Cancel';
  cancelBtn.addEventListener('click', close);
  const saveBtn = document.createElement('button');
  saveBtn.textContent = 'Save';
  footer.append(footnoteBtn, mediaBtn, status, tokenInput, cancelBtn, saveBtn);
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
    mediaBtn.textContent = editing ? 'Edit media' : 'Insert media';
    mediaBtn.setAttribute('aria-label', editing ? 'Edit media' : 'Insert media');
    mediaBtn.title = editing
      ? 'Edit the selected embed — or double-click any embed to edit'
      : 'Insert a media embed — double-click an existing embed to edit';
  };

  // Instantiate Crepe with CodeMirror (and its dependent LaTeX feature)
  // disabled. CodeMirror statically pulls in @codemirror/language-data (≈50
  // lazy language imports) — bloating the bundle and, when inlined into a single
  // chunk, breaking module init order. The LaTeX feature depends on CodeMirror,
  // so it must be disabled too. Neither is essential here: code blocks still
  // save as fenced code and math still saves as `$…$` (rendered on the page by
  // the existing katex component); only the in-editor helpers are dropped.
  try {
    crepe = new Crepe({
      root: crepeHost,
      defaultValue: body,
      features: {
        [CrepeFeature.CodeMirror]: false,
        [CrepeFeature.Latex]: false,
      },
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
    crepe ? recombine(fmTextarea.value, unescapeWikilinks(crepe.getMarkdown())) : savedContent;
  const isDirty = (): boolean => crepe !== null && currentContent() !== savedContent;
  savedContent = currentContent();

  const doSave = async (): Promise<boolean> => {
    if (!crepe) return false;
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
  saveBtn.addEventListener('click', () => void doSave());

  // ---------------------------------------------------------------------------
  // File operations (New / Rename / Move) and media insertion.
  // ---------------------------------------------------------------------------

  const encodeFsPath = (p: string): string =>
    p.split('/').map(encodeURIComponent).join('/');

  /** Minimal starter body for a newly created file (an H1 of its stem). */
  const newFileTemplate = (path: string): string => {
    const stem = (path.split('/').pop() ?? '').replace(/\.[^.]+$/, '');
    return `# ${stem}\n`;
  };

  /**
   * Prompts to save/discard/cancel when there are unsaved edits. Resolves
   * `true` when it's safe to proceed (saved or discarded), `false` on cancel.
   */
  const guardUnsaved = async (): Promise<boolean> => {
    if (!isDirty()) return true;
    const choice = await confirmUnsaved();
    if (choice === 'cancel') return false;
    if (choice === 'save') return doSave();
    return true; // discard
  };

  const handleNew = async (): Promise<void> => {
    if (!(await guardUnsaved())) return;
    const dest = await openPathPicker({
      mode: 'new',
      currentFsPath: opts.filePath,
      fetchSiteFiles: fetchSiteMarkdownFiles,
    });
    if (dest) await doCreate(dest.path, dest.createDirs);
  };

  const handleMove = async (select: 'basename' | 'folder'): Promise<void> => {
    if (!(await guardUnsaved())) return;
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
      const resp = await fetch(`/.mbr/create/${encodeFsPath(path)}`, {
        method: 'POST',
        headers: authHeaders({ 'Content-Type': 'application/json' }),
        credentials: 'same-origin',
        body: JSON.stringify({ content: newFileTemplate(path), create_dirs: createDirs }),
      });
      if (resp.ok) {
        const data = (await resp.json()) as { url_path: string };
        setStatus('Created. Opening…', 'ok');
        window.location.href = data.url_path;
        return;
      }
      if (resp.status === 401) {
        tokenInput.classList.add('show');
        tokenInput.focus();
      }
      setStatus(describeFileOpError(resp.status, 'create'), 'error');
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
