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
import { recombine, splitFrontmatter } from './editor-frontmatter.js';

export interface OpenEditorOptions {
  /** URL of the raw-markdown endpoint for the current file. */
  rawUrl: string;
  /** URL of the save endpoint for the current file. */
  saveUrl: string;
  /** Human-readable file path shown in the header. */
  filePath: string;
  /** Called as soon as the editor modal is visible (hides the trigger spinner). */
  onReady?: () => void;
  /** Called when the modal is dismissed so the trigger can reset its state. */
  onClose: () => void;
}

/** In-memory (never persisted) bearer token for this page session. */
let sessionToken = '';

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
  style.textContent = [crepeCommonCss, THEME_CSS, MODAL_CSS].join('\n');
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
  min-height: 4rem; font-family: var(--pico-font-family-monospace, monospace);
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

/** Builds the editing modal, loads the file, and wires up save/auth/errors. */
export async function openEditor(opts: OpenEditorOptions): Promise<void> {
  injectStyles();

  const backdrop = document.createElement('div');
  backdrop.className = 'mbr-editor-backdrop';
  const modal = document.createElement('div');
  modal.className = 'mbr-editor-modal';
  backdrop.appendChild(modal);

  let crepe: Crepe | null = null;
  let baseHash = '';

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
  const cancelBtn = document.createElement('button');
  cancelBtn.textContent = 'Cancel';
  cancelBtn.addEventListener('click', close);
  const saveBtn = document.createElement('button');
  saveBtn.textContent = 'Save';
  footer.append(status, tokenInput, cancelBtn, saveBtn);
  modal.appendChild(footer);

  const setStatus = (msg: string, kind: '' | 'ok' | 'error' = '') => {
    status.textContent = msg;
    status.className = `status${kind ? ' ' + kind : ''}`;
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
    });
    await crepe.create();
  } catch (err) {
    console.error('Crepe failed to initialize:', err);
    setStatus(`Failed to start editor: ${(err as Error).message}`, 'error');
    return;
  }

  const doSave = async () => {
    if (!crepe) return;
    sessionToken = tokenInput.value.trim();
    const content = recombine(fmTextarea.value, crepe.getMarkdown());
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
        setStatus('Saved. The page will reload.', 'ok');
        return;
      }
      if (resp.status === 401) {
        tokenInput.classList.add('show');
        tokenInput.focus();
      }
      setStatus(describeError(resp.status, 'save'), 'error');
    } catch (err) {
      saveBtn.removeAttribute('aria-busy');
      setStatus(`Save failed: ${(err as Error).message}`, 'error');
    }
  };
  saveBtn.addEventListener('click', doSave);

  // If we already know a token is needed (revealed on a prior 401), keep it shown.
  if (sessionToken) tokenInput.classList.add('show');
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
