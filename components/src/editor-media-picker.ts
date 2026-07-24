/**
 * Media picker for the editor: browse media.json, pick a file, and insert (or
 * edit) an embed. Output is plain markdown that mbr renders with its media
 * magic:
 *   - image / audio / pdf → `![caption](url)`
 *   - video               → `![caption](url)`, or the `{{ vid(...) }}` shortcode
 *                           when a start/end timestamp is given (the only form
 *                           that supports timestamps).
 *
 * media.json is fetched fresh on open (server-mode only). This module keeps its
 * own minimal media types and does NOT import `types.ts`/`shared.ts`, because
 * those run top-level site/media fetches at import time — which must not happen
 * inside the lazy editor chunk (project bundle rule).
 *
 * The picker only computes the embed; the caller (`editor-crepe.ts`) performs
 * the ProseMirror insertion/replacement.
 */

import { fuzzyFilter } from './fuzzy.js';
import { createNestedModal } from './editor-picker-shared.js';

/** Media kinds the picker surfaces (mirrors StaticFileKind's media variants). */
export type MediaKind = 'image' | 'video' | 'audio' | 'pdf';

/** Minimal media entry read from media.json `other_files[]`. */
export interface MediaFile {
  url_path: string;
  metadata: {
    kind: { type: string; title?: string; duration?: string };
    file_size_bytes?: number;
  };
}

export interface MediaPickerOptions {
  mode: 'insert' | 'edit';
  /** For `edit`: the existing embed's src + caption to prefill. */
  initial?: { src: string; caption: string };
  /** Fetches the fresh `other_files` array from media.json. */
  fetchMediaFiles: () => Promise<MediaFile[]>;
}

/**
 * What to insert/replace. `image` → an image embed (inline image node);
 * `shortcode` → a raw `{{ vid(...) }}` paragraph (video with timestamps).
 */
export type MediaPickResult =
  | { form: 'image'; src: string; caption: string }
  | { form: 'shortcode'; shortcode: string };

// ============================================================================
// Pure output builder (unit-tested)
// ============================================================================

export interface MediaOutputParams {
  url: string;
  kind: MediaKind;
  caption: string;
  /** Video-only playback start (seconds or timestamp). */
  start?: string;
  /** Video-only playback end. */
  end?: string;
}

/** Strips the `/videos/` prefix (or a leading slash) for the `vid` shortcode. */
export function toVidPath(url: string): string {
  if (url.startsWith('/videos/')) return url.slice('/videos/'.length);
  return url.replace(/^\//, '');
}

/**
 * Builds the embed for a chosen media file. Videos with a start or end become a
 * `{{ vid(...) }}` shortcode (the only form that carries timestamps); everything
 * else is a plain `![caption](url)` embed.
 */
export function buildMediaOutput(p: MediaOutputParams): MediaPickResult {
  const caption = p.caption.trim();
  const start = p.start?.trim();
  const end = p.end?.trim();
  if (p.kind === 'video' && (start || end)) {
    const parts = [`path="${toVidPath(p.url)}"`];
    if (start) parts.push(`start="${start}"`);
    if (end) parts.push(`end="${end}"`);
    if (caption) parts.push(`caption="${caption}"`);
    return { form: 'shortcode', shortcode: `{{ vid(${parts.join(', ')}) }}` };
  }
  return { form: 'image', src: p.url, caption };
}

/** Narrows a media.json kind string to a displayable {@link MediaKind}. */
export function mediaKindOf(file: MediaFile): MediaKind | null {
  const t = file.metadata?.kind?.type;
  return t === 'image' || t === 'video' || t === 'audio' || t === 'pdf' ? t : null;
}

/** Display title: metadata title if present, else the filename from the URL. */
function mediaTitle(file: MediaFile): string {
  const title = file.metadata?.kind?.title;
  if (title) return title;
  const parts = file.url_path.split('/');
  return parts[parts.length - 1] || file.url_path;
}

// ============================================================================
// Picker UI
// ============================================================================

/** Opens the media picker. Resolves with the embed to apply, or `null`. */
export function openMediaPicker(opts: MediaPickerOptions): Promise<MediaPickResult | null> {
  return new Promise<MediaPickResult | null>((resolve) => {
    let settled = false;
    const finish = (result: MediaPickResult | null) => {
      if (settled) return;
      settled = true;
      shell.destroy();
      resolve(result);
    };

    const shell = createNestedModal({
      ariaLabel: opts.mode === 'edit' ? 'Edit media embed' : 'Insert media',
      onCancel: () => finish(null),
    });
    const { modal } = shell;

    let mediaFiles: MediaFile[] = [];
    let selected: MediaFile | null = null;
    let filtered: MediaFile[] = [];
    let selectedIndex = -1;

    // --- Chrome -------------------------------------------------------------
    const header = document.createElement('div');
    header.className = 'mbr-picker-header';
    const h3 = document.createElement('h3');
    h3.textContent = opts.mode === 'edit' ? 'Edit media embed' : 'Insert media';
    const closeBtn = document.createElement('button');
    closeBtn.className = 'mbr-picker-close';
    closeBtn.setAttribute('aria-label', 'Cancel');
    closeBtn.textContent = '×';
    closeBtn.addEventListener('click', () => finish(null));
    header.append(h3, closeBtn);

    const searchField = document.createElement('div');
    searchField.className = 'mbr-picker-field';
    const searchLabel = document.createElement('label');
    searchLabel.textContent = 'Find media';
    const search = document.createElement('input');
    search.type = 'text';
    search.spellcheck = false;
    search.autocomplete = 'off';
    search.placeholder = 'Filter by name…';
    search.setAttribute('aria-label', 'Filter media');
    searchField.append(searchLabel, search);

    const list = document.createElement('div');
    list.className = 'mbr-picker-list';

    // Caption + optional video timestamps.
    const detailField = document.createElement('div');
    detailField.className = 'mbr-picker-field';
    const captionLabel = document.createElement('label');
    captionLabel.textContent = 'Caption (optional)';
    const caption = document.createElement('input');
    caption.type = 'text';
    caption.spellcheck = true;
    caption.setAttribute('aria-label', 'Caption');
    const timeRow = document.createElement('div');
    timeRow.className = 'mbr-picker-row';
    timeRow.style.marginTop = '0.5rem';
    timeRow.style.display = 'none';
    const startWrap = document.createElement('div');
    const startLabel = document.createElement('label');
    startLabel.textContent = 'Start (video)';
    const start = document.createElement('input');
    start.type = 'text';
    start.placeholder = 'e.g. 10 or 0:10';
    start.setAttribute('aria-label', 'Video start');
    startWrap.append(startLabel, start);
    const endWrap = document.createElement('div');
    const endLabel = document.createElement('label');
    endLabel.textContent = 'End (video)';
    const end = document.createElement('input');
    end.type = 'text';
    end.placeholder = 'e.g. 30 or 0:30';
    end.setAttribute('aria-label', 'Video end');
    endWrap.append(endLabel, end);
    timeRow.append(startWrap, endWrap);
    detailField.append(captionLabel, caption, timeRow);

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
    confirmBtn.textContent = opts.mode === 'edit' ? 'Replace' : 'Insert';
    confirmBtn.disabled = true;
    footer.append(status, cancelBtn, confirmBtn);

    modal.append(header, searchField, list, detailField, footer);

    const setStatus = (msg: string, kind: '' | 'ok' | 'warn' = '') => {
      status.textContent = msg;
      status.className = `mbr-picker-status${kind ? ' ' + kind : ''}`;
    };

    const selectFile = (file: MediaFile | null) => {
      selected = file;
      const isVideo = !!file && mediaKindOf(file) === 'video';
      timeRow.style.display = isVideo ? 'flex' : 'none';
      confirmBtn.disabled = !file;
      renderList();
    };

    const renderList = () => {
      const query = search.value.trim();
      filtered = fuzzyFilter(
        mediaFiles.map((f) => ({ item: f, haystacks: [mediaTitle(f), f.url_path] })),
        query,
      ).slice(0, 60);
      if (selectedIndex >= filtered.length) selectedIndex = filtered.length - 1;

      list.textContent = '';
      if (filtered.length === 0) {
        const empty = document.createElement('div');
        empty.className = 'mbr-picker-empty';
        empty.textContent = mediaFiles.length === 0 ? 'Loading…' : 'No matching media';
        list.appendChild(empty);
        return;
      }
      filtered.forEach((file, i) => {
        const kind = mediaKindOf(file);
        const isSelected = file === selected || i === selectedIndex;
        const row = document.createElement('div');
        row.className = `mbr-picker-item${isSelected ? ' selected' : ''}`;
        row.setAttribute('role', 'option');
        const badge = document.createElement('span');
        badge.className = 'mbr-picker-badge';
        badge.textContent = kind ?? 'file';
        const main = document.createElement('span');
        main.className = 'mbr-picker-item-main';
        main.textContent = mediaTitle(file);
        const sub = document.createElement('span');
        sub.className = 'mbr-picker-item-sub';
        sub.textContent = file.url_path;
        row.append(badge, main, sub);
        row.addEventListener('mouseenter', () => {
          selectedIndex = i;
        });
        row.addEventListener('click', () => {
          selectedIndex = i;
          selectFile(file);
        });
        list.appendChild(row);
      });
    };

    const submit = () => {
      if (!selected) {
        setStatus('Pick a media file first.', 'warn');
        return;
      }
      const kind = mediaKindOf(selected);
      if (!kind) {
        setStatus('Unsupported media type.', 'warn');
        return;
      }
      finish(
        buildMediaOutput({
          url: selected.url_path,
          kind,
          caption: caption.value,
          start: start.value,
          end: end.value,
        }),
      );
    };

    // --- Keyboard -----------------------------------------------------------
    search.addEventListener('input', () => {
      selectedIndex = -1;
      renderList();
    });
    const listNav = (e: KeyboardEvent) => {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        selectedIndex = Math.min(selectedIndex + 1, filtered.length - 1);
        if (filtered[selectedIndex]) selectFile(filtered[selectedIndex]);
        scrollSelectedIntoView();
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        selectedIndex = Math.max(selectedIndex - 1, 0);
        if (filtered[selectedIndex]) selectFile(filtered[selectedIndex]);
        scrollSelectedIntoView();
      } else if (e.key === 'Enter') {
        e.preventDefault();
        submit();
      }
    };
    search.addEventListener('keydown', listNav);
    caption.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        submit();
      }
    });
    confirmBtn.addEventListener('click', submit);

    const scrollSelectedIntoView = () => {
      const el = list.querySelector('.mbr-picker-item.selected');
      el?.scrollIntoView({ block: 'nearest' });
    };

    // --- Prefill (edit) + load ---------------------------------------------
    if (opts.initial) {
      caption.value = opts.initial.caption;
    }
    setStatus('Loading media…');
    search.focus();

    void opts
      .fetchMediaFiles()
      .then((files) => {
        if (settled) return;
        mediaFiles = files.filter((f) => mediaKindOf(f) !== null);
        // In edit mode, preselect the entry matching the current src.
        if (opts.initial) {
          const match = mediaFiles.find((f) => f.url_path === opts.initial!.src);
          if (match) {
            selectedIndex = filtered.indexOf(match);
            selectFile(match);
          } else {
            setStatus('Current embed is not in media.json; pick a replacement.', '');
          }
        }
        setStatus('');
        renderList();
      })
      .catch(() => {
        if (settled) return;
        setStatus('Could not load media.json.', 'warn');
      });
  });
}
