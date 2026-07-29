/**
 * Destination-path picker for the editor's File menu (New / Rename / Move).
 *
 * A combobox over the markdown folder hierarchy derived from site.json. The
 * user edits a repo-relative filesystem path (with a markdown extension) — the
 * exact shape the `/.mbr/create` and `/.mbr/move` endpoints expect. site.json
 * is fetched fresh on open (server-mode only) so the hierarchy is current after
 * a prior create/move. The static overlay is ignored (markdown files only).
 *
 * The picker only *selects* a destination: it resolves `{ path, createDirs }`.
 * The caller (`editor-crepe.ts`) performs the authenticated create/move POST
 * and any navigation, reusing the editor's `authHeaders()`.
 */

import { fuzzyFilter } from './fuzzy.js';
import {
  createNestedModal,
  deriveExistingFolders,
  fsPathToApproxUrl,
  hasMarkdownExtension,
  normalizeUrl,
  parentFolder,
  DEFAULT_MARKDOWN_EXTENSIONS,
} from './editor-picker-shared.js';

/** Minimal markdown-file shape read from site.json. */
export interface SiteMarkdownFile {
  url_path: string;
  /**
   * Repo-relative, `/`-separated source path (`docs/guide/index.md`) as shipped
   * by site.json (`MarkdownInfo::raw_path`). Authoritative: it keeps the real
   * extension and the real `index.md` leaf, neither of which is recoverable
   * from the directory-style `url_path`. Optional only so a stale cached
   * site.json predating the field still works — see {@link fileFsPath}.
   */
  raw_path?: string;
  frontmatter?: { title?: string; [key: string]: unknown };
}

export interface PathPickerOptions {
  /** `new` creates a file; `move` renames/moves the current file. */
  mode: 'new' | 'move';
  /** Repo-relative filesystem path (with extension) of the current file. */
  currentFsPath: string;
  /** Markdown extensions accepted for the leaf (defaults to md/markdown). */
  markdownExtensions?: string[];
  /**
   * Initial selection for the prefilled `move` path: `basename` (rename — the
   * default) selects the filename stem; `folder` selects the directory portion.
   */
  select?: 'basename' | 'folder';
  /** Fetches the fresh `markdown_files` array from site.json. */
  fetchSiteFiles: () => Promise<SiteMarkdownFile[]>;
}

export interface PathPickResult {
  /** Repo-relative filesystem path (with extension) to create/move to. */
  path: string;
  /** True when the destination's parent folder must be created. */
  createDirs: boolean;
}

/** A steering suggestion shown in the list (an existing folder or file). */
interface Suggestion {
  kind: 'folder' | 'file';
  /** Value applied to the input when chosen. */
  value: string;
  /** Primary label. */
  label: string;
  /** Secondary (path) label. */
  sub: string;
}

/**
 * Opens the path picker on top of the editor. Resolves with the chosen
 * destination, or `null` if the user cancels.
 */
export function openPathPicker(opts: PathPickerOptions): Promise<PathPickResult | null> {
  const exts = opts.markdownExtensions?.length
    ? opts.markdownExtensions
    : [...DEFAULT_MARKDOWN_EXTENSIONS];

  return new Promise<PathPickResult | null>((resolve) => {
    let settled = false;
    const finish = (result: PathPickResult | null) => {
      if (settled) return;
      settled = true;
      shell.destroy();
      resolve(result);
    };

    const shell = createNestedModal({
      ariaLabel: opts.mode === 'new' ? 'New markdown file' : 'Rename or move file',
      onCancel: () => finish(null),
    });
    const { modal } = shell;

    // Data derived once site.json arrives.
    let existingUrls = new Set<string>();
    let existingFolders = new Set<string>(['']);
    let suggestions: Suggestion[] = [];
    const currentUrl = fsPathToApproxUrl(opts.currentFsPath);

    // --- Chrome -------------------------------------------------------------
    const header = document.createElement('div');
    header.className = 'mbr-picker-header';
    const h3 = document.createElement('h3');
    h3.textContent = opts.mode === 'new' ? 'New markdown file' : 'Rename or move file';
    const closeBtn = document.createElement('button');
    closeBtn.className = 'mbr-picker-close';
    closeBtn.setAttribute('aria-label', 'Cancel');
    closeBtn.textContent = '×';
    closeBtn.addEventListener('click', () => finish(null));
    header.append(h3, closeBtn);

    const field = document.createElement('div');
    field.className = 'mbr-picker-field';
    const label = document.createElement('label');
    label.textContent = 'Destination path (repo-relative)';
    const input = document.createElement('input');
    input.type = 'text';
    input.spellcheck = false;
    input.autocomplete = 'off';
    input.setAttribute('aria-label', 'Destination path');
    field.append(label, input);

    const list = document.createElement('div');
    list.className = 'mbr-picker-list';

    const hint = document.createElement('div');
    hint.className = 'mbr-picker-hint';
    hint.textContent = 'Type a path, or pick a folder to steer. Enter confirms.';

    // Inline "create folder?" confirmation bar (hidden until needed).
    const confirmBar = document.createElement('div');
    confirmBar.className = 'mbr-picker-confirm';
    confirmBar.style.display = 'none';

    const footer = document.createElement('div');
    footer.className = 'mbr-picker-footer';
    const status = document.createElement('span');
    status.className = 'mbr-picker-status';
    const cancelBtn = document.createElement('button');
    cancelBtn.type = 'button';
    cancelBtn.textContent = 'Cancel';
    cancelBtn.addEventListener('click', () => finish(null));
    const confirmBtn = document.createElement('button');
    confirmBtn.type = 'button';
    confirmBtn.textContent = opts.mode === 'new' ? 'Create' : 'Move';
    footer.append(status, cancelBtn, confirmBtn);

    modal.append(header, field, list, hint, confirmBar, footer);

    const setStatus = (msg: string, kind: '' | 'ok' | 'warn' = '') => {
      status.textContent = msg;
      status.className = `mbr-picker-status${kind ? ' ' + kind : ''}`;
    };

    // --- Suggestion list ----------------------------------------------------
    let selectedIndex = -1;
    let filtered: Suggestion[] = [];

    const applySuggestion = (s: Suggestion) => {
      input.value = s.value;
      input.focus();
      // Place caret at end so the user can keep typing a filename.
      const end = input.value.length;
      input.setSelectionRange(end, end);
      renderList();
      validate();
    };

    const renderList = () => {
      // The query is the segment being typed after the last slash for files,
      // but for steering we fuzzy-match the whole input against folder/file
      // paths — this keeps folder discovery forgiving.
      const query = input.value.trim();
      filtered = fuzzyFilter(
        suggestions.map((s) => ({ item: s, haystacks: [s.value, s.label] })),
        query,
      ).slice(0, 40);
      if (selectedIndex >= filtered.length) selectedIndex = filtered.length - 1;

      list.textContent = '';
      if (filtered.length === 0) {
        const empty = document.createElement('div');
        empty.className = 'mbr-picker-empty';
        empty.textContent = suggestions.length === 0 ? 'Loading…' : 'No matching folders or files';
        list.appendChild(empty);
        return;
      }
      filtered.forEach((s, i) => {
        const row = document.createElement('div');
        row.className = `mbr-picker-item${i === selectedIndex ? ' selected' : ''}`;
        row.setAttribute('role', 'option');
        const badge = document.createElement('span');
        badge.className = `mbr-picker-badge ${s.kind}`;
        badge.textContent = s.kind;
        const main = document.createElement('span');
        main.className = 'mbr-picker-item-main';
        main.textContent = s.label;
        const sub = document.createElement('span');
        sub.className = 'mbr-picker-item-sub';
        sub.textContent = s.sub;
        row.append(badge, main, sub);
        row.addEventListener('mouseenter', () => {
          selectedIndex = i;
        });
        row.addEventListener('click', () => applySuggestion(s));
        list.appendChild(row);
      });
    };

    // --- Validation + confirm ----------------------------------------------
    /** Normalizes the typed value into a candidate fs path (auto-adds `.md`). */
    const candidatePath = (): string => {
      let p = input.value.trim().replace(/^\/+/, '');
      if (!p) return '';
      // If the leaf has no extension at all, default to `.md`.
      if (!hasMarkdownExtension(p, exts) && !/\.[^./]+$/.test(p.split('/').pop() ?? '')) {
        p = `${p}.${exts[0]}`;
      }
      return p;
    };

    /** Returns a blocking error message, or `null` when the path is valid. */
    const validationError = (path: string): string | null => {
      if (!path) return 'Enter a destination path.';
      if (path.split('/').some((seg) => seg === '..' || seg === '.')) {
        return 'Path may not contain "." or ".." segments.';
      }
      if (/\/$/.test(input.value.trim()) || !(path.split('/').pop() ?? '')) {
        return 'Include a filename, not just a folder.';
      }
      if (!hasMarkdownExtension(path, exts)) {
        return `Filename must end in .${exts.join(' or .')}`;
      }
      const destUrl = fsPathToApproxUrl(path);
      if (opts.mode === 'move' && normalizeUrl(destUrl) === normalizeUrl(currentUrl)) {
        return 'Choose a different destination than the current file.';
      }
      if (existingUrls.has(normalizeUrl(destUrl))) {
        return 'A file already exists at that location.';
      }
      return null;
    };

    /** Live validation → status + confirm-button enablement. Returns validity. */
    const validate = (): boolean => {
      const path = candidatePath();
      const err = validationError(path);
      if (err) {
        setStatus(err, 'warn');
        confirmBtn.disabled = true;
        return false;
      }
      const folder = parentFolder(path);
      if (folder && !existingFolders.has(folder)) {
        setStatus(`New folder “${folder}” will be created.`, '');
      } else {
        setStatus('');
      }
      confirmBtn.disabled = false;
      return true;
    };

    const submit = () => {
      if (!validate()) return;
      const path = candidatePath();
      const folder = parentFolder(path);
      const needsFolder = folder !== '' && !existingFolders.has(folder);
      if (needsFolder) {
        askCreateFolder(folder, path);
        return;
      }
      finish({ path, createDirs: false });
    };

    /** Renders the inline "create folder?" confirmation before committing. */
    const askCreateFolder = (folder: string, path: string) => {
      confirmBtn.disabled = true;
      confirmBar.style.display = 'flex';
      confirmBar.textContent = '';
      const msg = document.createElement('span');
      msg.textContent = `Folder “${folder}” doesn't exist yet. Create it?`;
      const noBtn = document.createElement('button');
      noBtn.type = 'button';
      noBtn.textContent = 'No';
      noBtn.addEventListener('click', () => {
        confirmBar.style.display = 'none';
        confirmBtn.disabled = false;
        input.focus();
      });
      const yesBtn = document.createElement('button');
      yesBtn.type = 'button';
      yesBtn.textContent = `Create “${folder}”`;
      yesBtn.addEventListener('click', () => finish({ path, createDirs: true }));
      confirmBar.append(msg, noBtn, yesBtn);
      yesBtn.focus();
    };

    // --- Keyboard -----------------------------------------------------------
    input.addEventListener('input', () => {
      selectedIndex = -1;
      confirmBar.style.display = 'none';
      renderList();
      validate();
    });
    input.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        selectedIndex = Math.min(selectedIndex + 1, filtered.length - 1);
        renderList();
        scrollSelectedIntoView();
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        selectedIndex = Math.max(selectedIndex - 1, -1);
        renderList();
        scrollSelectedIntoView();
      } else if (e.key === 'Enter') {
        e.preventDefault();
        if (selectedIndex >= 0 && filtered[selectedIndex]) {
          applySuggestion(filtered[selectedIndex]);
        } else {
          submit();
        }
      }
    });
    confirmBtn.addEventListener('click', submit);

    const scrollSelectedIntoView = () => {
      const el = list.querySelector('.mbr-picker-item.selected');
      el?.scrollIntoView({ block: 'nearest' });
    };

    // --- Prefill + load -----------------------------------------------------
    if (opts.mode === 'move') {
      input.value = opts.currentFsPath;
    } else {
      const folder = parentFolder(opts.currentFsPath);
      input.value = folder ? `${folder}/` : '';
    }
    renderList();
    setStatus('Loading site index…');
    confirmBtn.disabled = true;

    input.focus();
    if (opts.mode === 'move') {
      const val = input.value;
      const slash = val.lastIndexOf('/');
      if (opts.select === 'folder' && slash >= 0) {
        // Select the directory portion (incl. trailing slash) for a move.
        input.setSelectionRange(0, slash + 1);
      } else {
        // Select the basename (without extension) for a quick rename.
        const dot = val.lastIndexOf('.');
        const start = slash + 1;
        const end = dot > start ? dot : val.length;
        input.setSelectionRange(start, end);
      }
    } else {
      const end = input.value.length;
      input.setSelectionRange(end, end);
    }

    void opts
      .fetchSiteFiles()
      .then((files) => {
        if (settled) return;
        const urls = files.map((f) => normalizeUrl(f.url_path));
        existingUrls = new Set(urls);
        // A `url_path` alone only pins down a file's *ancestor* folders, while
        // `raw_path` names the containing folder exactly (`/docs/guide/` could
        // be `docs/guide.md` or `docs/guide/index.md`). Feeding both in unions
        // the confident set with the exact one, so an index file's own folder
        // no longer triggers a spurious "create folder?" prompt.
        existingFolders = deriveExistingFolders([
          ...urls,
          ...files.map((f) => fileFsPath(f, exts)),
        ]);
        suggestions = buildSuggestions(files, exts);
        renderList();
        validate();
      })
      .catch(() => {
        if (settled) return;
        // Fail open: the server's collision/parent checks remain authoritative.
        setStatus('Could not load site index; server checks still apply.', 'warn');
        confirmBtn.disabled = false;
      });
  });
}

/**
 * The repo-relative filesystem path of a markdown file from site.json.
 *
 * Prefers the authoritative `raw_path`; only a site.json old enough to predate
 * the field falls back to reconstructing a path from the directory-style
 * `url_path`, which cannot distinguish `docs/guide.md` from
 * `docs/guide/index.md` and has to guess the extension.
 */
export function fileFsPath(f: SiteMarkdownFile, exts: readonly string[]): string {
  const raw = (f.raw_path ?? '').trim().replace(/^\/+/, '');
  if (raw) return raw;
  const segs = normalizeUrl(f.url_path)
    .split('/')
    .filter((s) => s.length > 0);
  return (segs.length ? segs.join('/') : 'index') + `.${exts[0] ?? 'md'}`;
}

/**
 * Builds the folder + file steering suggestions from the markdown files. Files
 * are shown by title/stem; folders are the confident directory set. A trailing
 * slash on a folder value lets the user keep typing the filename after picking.
 */
function buildSuggestions(files: SiteMarkdownFile[], exts: string[]): Suggestion[] {
  const folders = new Set<string>();
  const fileSuggestions: Suggestion[] = [];

  for (const f of files) {
    const url = normalizeUrl(f.url_path);
    const segs = url.split('/').filter((s) => s.length > 0);
    const stem = segs[segs.length - 1] ?? '';
    const fsPath = fileFsPath(f, exts);
    // Accumulate every folder the file proves exists. With `raw_path` this is
    // exact (an index file contributes its own folder too); without it, it
    // degrades to the file's confident URL ancestors.
    let acc = '';
    const folderSegs = fsPath.split('/').filter((s) => s.length > 0);
    for (let i = 0; i < folderSegs.length - 1; i++) {
      acc = acc ? `${acc}/${folderSegs[i]}` : folderSegs[i];
      folders.add(acc);
    }
    const title = f.frontmatter?.title;
    fileSuggestions.push({
      kind: 'file',
      value: fsPath,
      label: title || stem || fsPath,
      sub: fsPath,
    });
  }

  const folderSuggestions: Suggestion[] = [...folders]
    .sort()
    .map((folder) => ({
      kind: 'folder',
      value: `${folder}/`,
      label: `${folder}/`,
      sub: 'folder',
    }));

  // Folders first (steering), then files (collision awareness).
  return [...folderSuggestions, ...fileSuggestions];
}
