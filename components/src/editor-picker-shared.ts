/**
 * Shared chrome + pure helpers for the editor's nested pickers
 * (path picker, media picker). Lives only in the lazy `mbr-editor.min.js`
 * chunk. Deliberately free of `shared.ts`/Lit imports so the chunk stays
 * self-contained (per the project bundle rule).
 *
 * The pure path helpers (`relativeUrlPath`, `deriveExistingFolders`,
 * `fsPathToApproxUrl`, …) are exported so they can be unit-tested without a
 * DOM; see `editor-picker-shared.test.ts`.
 */

/**
 * Styles for the nested picker modals. Injected alongside the editor's own
 * stylesheet (see `injectStyles` in `editor-crepe.ts`) so both share one
 * `<style>` element and the same Pico theme variables. The z-index sits above
 * the editor modal (`z-index: 2000`) so pickers stack on top of it.
 */
export const PICKER_CSS = `
.mbr-picker-backdrop {
  position: fixed; inset: 0; background: rgba(0,0,0,0.4); z-index: 2100;
  display: flex; align-items: flex-start; justify-content: center; padding-top: 8vh;
}
.mbr-picker-modal {
  background: var(--pico-background-color, #fff);
  color: var(--pico-color, #1a1a1a);
  width: min(640px, 94vw); max-height: 78vh;
  display: flex; flex-direction: column;
  border-radius: 8px; box-shadow: 0 16px 48px rgba(0,0,0,0.4);
  overflow: hidden;
}
.mbr-picker-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 0.6rem 0.9rem; border-bottom: 1px solid var(--pico-muted-border-color, #e0e0e0);
}
.mbr-picker-header h3 { margin: 0; font-size: 0.95rem; }
.mbr-picker-close { background: transparent; border: none; font-size: 1.3rem; cursor: pointer; color: inherit; line-height: 1; padding: 0.1rem 0.4rem; }
.mbr-picker-field { padding: 0.6rem 0.9rem; border-bottom: 1px solid var(--pico-muted-border-color, #e0e0e0); }
.mbr-picker-field label { display: block; font-size: 0.75rem; opacity: 0.75; margin-bottom: 0.25rem; }
.mbr-picker-field input[type="text"], .mbr-picker-field textarea {
  width: 100%; box-sizing: border-box; padding: 0.4rem 0.5rem; font-size: 0.9rem;
  border: 1px solid var(--pico-muted-border-color, #ccc); border-radius: 5px;
  background: var(--pico-form-element-background-color, var(--pico-background-color, #fff)); color: inherit;
  font-family: var(--pico-font-family-monospace, monospace);
}
.mbr-picker-field .mbr-picker-row { display: flex; gap: 0.5rem; }
.mbr-picker-field .mbr-picker-row > div { flex: 1; }
.mbr-picker-list {
  overflow-y: auto; flex: 1; min-height: 4rem; padding: 0.25rem;
}
.mbr-picker-item {
  display: flex; align-items: center; gap: 0.5rem;
  padding: 0.4rem 0.55rem; border-radius: 5px; cursor: pointer; font-size: 0.88rem;
}
.mbr-picker-item:hover, .mbr-picker-item.selected {
  background: var(--pico-primary-focus, rgba(99,102,241,0.15));
}
.mbr-picker-item .mbr-picker-item-main { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.mbr-picker-item .mbr-picker-item-sub { opacity: 0.6; font-size: 0.78rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 45%; font-family: var(--pico-font-family-monospace, monospace); }
.mbr-picker-badge {
  flex-shrink: 0; font-size: 0.65rem; text-transform: uppercase; letter-spacing: 0.03em;
  padding: 0.1rem 0.35rem; border-radius: 4px; font-weight: 600;
  background: var(--pico-secondary-background, #e0e0e0); color: var(--pico-secondary-inverse, #333);
}
.mbr-picker-badge.folder { background: var(--pico-muted-border-color, #d0d0d0); }
.mbr-picker-empty { padding: 1.2rem; text-align: center; opacity: 0.6; font-size: 0.85rem; }
.mbr-picker-footer {
  display: flex; align-items: center; gap: 0.6rem;
  padding: 0.55rem 0.9rem; border-top: 1px solid var(--pico-muted-border-color, #e0e0e0);
}
.mbr-picker-footer .mbr-picker-status { flex: 1; font-size: 0.82rem; }
.mbr-picker-footer .mbr-picker-status.warn { color: var(--pico-del-color, #b3261e); }
.mbr-picker-footer .mbr-picker-status.ok { color: var(--pico-ins-color, #1a7f37); }
.mbr-picker-footer button { padding: 0.32rem 0.85rem; cursor: pointer; }
.mbr-picker-hint { font-size: 0.72rem; opacity: 0.6; padding: 0 0.9rem 0.5rem; }
.mbr-picker-confirm {
  padding: 0.5rem 0.9rem; border-top: 1px dashed var(--pico-muted-border-color, #ccc);
  font-size: 0.85rem; display: flex; align-items: center; gap: 0.6rem;
}
.mbr-picker-confirm span { flex: 1; }
`;

/** Handle returned by {@link createNestedModal}. */
export interface NestedModal {
  backdrop: HTMLDivElement;
  modal: HTMLDivElement;
  /** Tears down the modal and removes its capture-phase key listener. */
  destroy: () => void;
}

/**
 * Creates a nested modal (backdrop + panel) on top of the editor.
 *
 * Mirrors how `openEditor` manages keys, but uses a **capture-phase** keydown
 * listener that calls `onCancel` on Escape and `stopPropagation()`s so the
 * innermost picker closes first — the editor's own (bubble-phase) Escape never
 * sees the event while a picker is open. Clicking the backdrop also cancels.
 */
export function createNestedModal(opts: {
  ariaLabel: string;
  onCancel: () => void;
}): NestedModal {
  const backdrop = document.createElement('div');
  backdrop.className = 'mbr-picker-backdrop';
  backdrop.setAttribute('role', 'dialog');
  backdrop.setAttribute('aria-modal', 'true');
  backdrop.setAttribute('aria-label', opts.ariaLabel);

  const modal = document.createElement('div');
  modal.className = 'mbr-picker-modal';
  backdrop.appendChild(modal);

  const onKeydown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      // Capture phase on THIS backdrop + stopPropagation: because backdrops are
      // siblings on <body>, only the picker containing the focused element sees
      // the event, so the innermost closes first and the editor's document-level
      // (bubble-phase) Escape handler never fires while a picker is open.
      e.preventDefault();
      e.stopPropagation();
      opts.onCancel();
    }
  };
  backdrop.addEventListener('keydown', onKeydown, true);

  backdrop.addEventListener('mousedown', (e) => {
    if (e.target === backdrop) opts.onCancel();
  });

  const destroy = () => {
    backdrop.removeEventListener('keydown', onKeydown, true);
    backdrop.remove();
  };

  document.body.appendChild(backdrop);
  return { backdrop, modal, destroy };
}

// ============================================================================
// Pure path helpers (repo-relative filesystem paths + directory-style URLs)
// ============================================================================

/** Default markdown extensions when the server config isn't surfaced. */
export const DEFAULT_MARKDOWN_EXTENSIONS = ['md', 'markdown'] as const;

/** Normalizes a directory-style URL to a leading + trailing slash form. */
export function normalizeUrl(url: string): string {
  let u = url.trim();
  if (!u.startsWith('/')) u = '/' + u;
  if (!u.endsWith('/')) u = u + '/';
  return u.replace(/\/{2,}/g, '/');
}

/** Splits a path/URL into its non-empty segments. */
function segments(path: string): string[] {
  return path.split('/').filter((s) => s.length > 0);
}

/** Returns the parent directory of a repo-relative fs path ('' = repo root). */
export function parentFolder(fsPath: string): string {
  const segs = segments(fsPath);
  segs.pop();
  return segs.join('/');
}

/** True when the fs path's leaf ends in one of the markdown extensions. */
export function hasMarkdownExtension(
  fsPath: string,
  exts: readonly string[] = DEFAULT_MARKDOWN_EXTENSIONS,
): boolean {
  const leaf = segments(fsPath).pop() ?? '';
  const dot = leaf.lastIndexOf('.');
  if (dot <= 0) return false;
  return exts.includes(leaf.slice(dot + 1).toLowerCase());
}

/**
 * Computes the set of repo-relative folders that certainly exist, given every
 * markdown file's directory-style `url_path`.
 *
 * A URL like `/a/b/guide/` maps to a file whose containing directory is at
 * least `a/b` — regardless of whether the file is `a/b/guide.md` or
 * `a/b/guide/index.md` — so every ancestor of `segments.slice(0, -1)` is a
 * confident folder. This deliberately *under*-approximates (an index folder
 * such as `a/b/guide` may be omitted), which only ever causes a harmless,
 * idempotent "create folder?" prompt — never a missing one. Always includes the
 * repo root (`''`).
 */
export function deriveExistingFolders(urlPaths: readonly string[]): Set<string> {
  const folders = new Set<string>(['']);
  for (const url of urlPaths) {
    const segs = segments(url);
    segs.pop(); // drop the file's own slug segment
    let acc = '';
    for (const seg of segs) {
      acc = acc ? `${acc}/${seg}` : seg;
      folders.add(acc);
    }
  }
  return folders;
}

/**
 * Approximates the directory-style URL a repo-relative markdown fs path would
 * be served at, for advisory collision checks against existing `url_path`s.
 * Mirrors the backend's `build_markdown_url_path` for the default `index.md`
 * (the server's 409 remains the authoritative collision guard).
 */
export function fsPathToApproxUrl(fsPath: string, indexFile = 'index.md'): string {
  const segs = segments(fsPath);
  let leaf = segs.pop() ?? '';
  // Strip a single extension from the leaf.
  const dot = leaf.lastIndexOf('.');
  if (dot > 0) leaf = leaf.slice(0, dot);
  const indexStem = indexFile.replace(/\.[^.]+$/, '');
  if (leaf !== indexStem) segs.push(leaf);
  const path = segs.join('/');
  return normalizeUrl('/' + path);
}

/**
 * Computes a relative URL from the page at `fromUrl` to `toUrl`, both
 * directory-style (`/a/b/guide/`). Because mbr serves pages with a trailing
 * slash, a relative href resolves against the page URL itself, so every "from"
 * segment is a directory level. Returns a `./`- or `../`-prefixed path; a
 * self-reference yields `./`.
 */
export function relativeUrlPath(fromUrl: string, toUrl: string): string {
  const from = segments(fromUrl);
  const to = segments(toUrl);
  let i = 0;
  while (i < from.length && i < to.length && from[i] === to[i]) i++;
  const parts: string[] = [];
  for (let k = i; k < from.length; k++) parts.push('..');
  for (let k = i; k < to.length; k++) parts.push(to[k]);
  if (parts.length === 0) return './';
  let rel = parts.join('/') + '/';
  if (!rel.startsWith('.')) rel = './' + rel;
  return rel;
}

/**
 * Encodes a relative URL path for use as a markdown link destination, escaping
 * only spaces (as `%20`) and parentheses so `[t](dest)` parses correctly while
 * staying human-readable. Slashes and dots are preserved.
 */
export function encodeLinkDestination(path: string): string {
  return path
    .replace(/%/g, '%25')
    .replace(/ /g, '%20')
    .replace(/\(/g, '%28')
    .replace(/\)/g, '%29');
}
