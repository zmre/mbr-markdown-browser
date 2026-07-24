/**
 * `[[`-triggered link autocomplete for the Crepe editor.
 *
 * A Milkdown plugin (a ProseMirror plugin wrapped with `$prose`) registered on
 * the Crepe instance via `crepe.editor.use(...)` before `.create()`. While the
 * caret sits just after an unclosed `[[`, it shows a fuzzy-filtered list of
 * markdown files (from a fresh site.json fetch). Selecting one inserts either:
 *   - a wiki link `[[Name]]` (default), or
 *   - a relative markdown link `[title](../rel/path/)` computed from the current
 *     page to the target (Tab toggles the mode).
 *
 * Interactive keys are handled by a **capture-phase** keydown listener on the
 * editor DOM, so navigation/selection wins over Crepe's own keymaps, and
 * `Escape` closes the popup before the editor's document-level handler can see
 * it (via `stopPropagation`). The `$prose` injection API matches the plan's
 * assumption for `@milkdown/crepe` 7.21.3 (`Editor.use(plugins)` before
 * `create()`).
 */

import { $prose } from '@milkdown/kit/utils';
import { Plugin, PluginKey, TextSelection } from '@milkdown/kit/prose/state';
import type { EditorView } from '@milkdown/kit/prose/view';
import type { MilkdownPlugin } from '@milkdown/kit/ctx';
import { fuzzyFilter } from './fuzzy.js';
import { relativeUrlPath, encodeLinkDestination, normalizeUrl } from './editor-picker-shared.js';

/** Minimal markdown-file shape read from site.json. */
export interface LinkAutocompleteSite {
  url_path: string;
  frontmatter?: { title?: string; [key: string]: unknown };
}

export interface LinkAutocompleteServices {
  /** Returns the fresh `markdown_files` array from site.json. */
  fetchSiteFiles: () => Promise<LinkAutocompleteSite[]>;
  /** Returns the current page's canonical directory-style URL. */
  currentUrl: () => string;
}

/** Insertion form for a selection. */
type LinkMode = 'wiki' | 'link';

/** Precomputed target derived from a site.json entry. */
interface Target {
  /** Normalized directory-style URL. */
  url: string;
  /** Name inserted for a wiki link. */
  wiki: string;
  /** Text shown for a relative markdown link. */
  title: string;
  /** Display path for the list. */
  path: string;
}

const linkAutocompleteKey = new PluginKey('mbr-link-autocomplete');

/** Styles for the floating suggestion popup (injected by the editor chunk). */
export const AUTOCOMPLETE_CSS = `
.mbr-autocomplete {
  position: fixed; z-index: 2200; min-width: 16rem; max-width: 28rem;
  max-height: 15rem; overflow-y: auto;
  background: var(--pico-background-color, #fff); color: var(--pico-color, #1a1a1a);
  border: 1px solid var(--pico-muted-border-color, #ccc); border-radius: 6px;
  box-shadow: 0 8px 28px rgba(0,0,0,0.28); font-size: 0.88rem;
}
.mbr-autocomplete-head {
  padding: 0.35rem 0.55rem; font-size: 0.72rem; opacity: 0.7;
  border-bottom: 1px solid var(--pico-muted-border-color, #eee);
  position: sticky; top: 0; background: inherit;
}
.mbr-autocomplete-head kbd {
  padding: 0 0.25rem; border: 1px solid var(--pico-muted-border-color, #ccc);
  border-radius: 3px; font-size: 0.7rem;
}
.mbr-autocomplete-item {
  display: flex; align-items: baseline; gap: 0.5rem;
  padding: 0.35rem 0.55rem; cursor: pointer;
}
.mbr-autocomplete-item.selected, .mbr-autocomplete-item:hover {
  background: var(--pico-primary-focus, rgba(99,102,241,0.15));
}
.mbr-autocomplete-item .title { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.mbr-autocomplete-item .path { opacity: 0.55; font-size: 0.75rem; font-family: var(--pico-font-family-monospace, monospace); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 55%; }
.mbr-autocomplete-empty { padding: 0.6rem; opacity: 0.6; }
`;

/** Extracts the display name (stem) from a normalized directory-style URL. */
function stemOf(url: string): string {
  const segs = url.split('/').filter((s) => s.length > 0);
  return segs[segs.length - 1] ?? '';
}

/** Builds the searchable target list from raw site.json entries. */
function buildTargets(files: LinkAutocompleteSite[]): Target[] {
  return files.map((f) => {
    const url = normalizeUrl(f.url_path);
    const stem = stemOf(url);
    const name = f.frontmatter?.title || stem || url;
    return { url, wiki: name, title: name, path: url };
  });
}

/**
 * Creates the `[[` link-autocomplete plugin. Register it with
 * `crepe.editor.use(createLinkAutocompletePlugin(services))` before `create()`.
 */
export function createLinkAutocompletePlugin(services: LinkAutocompleteServices): MilkdownPlugin {
  return $prose(() => {
    let popup: HTMLDivElement | null = null;
    let active = false;
    let query = '';
    let range = { from: 0, to: 0 };
    let mode: LinkMode = 'wiki';
    let index = 0;
    let filtered: Target[] = [];
    let targets: Target[] | null = null; // null = not yet loaded this session
    let session = 0;

    return new Plugin({
      key: linkAutocompleteKey,
      view(view) {
        const onKeyDown = (e: KeyboardEvent) => handleKey(view, e);
        // Capture phase: run before Crepe's keymaps and before the editor's
        // document-level Escape handler.
        view.dom.addEventListener('keydown', onKeyDown, true);

        const refilter = () => {
          if (!targets) {
            filtered = [];
          } else {
            filtered = fuzzyFilter(
              targets.map((t) => ({ item: t, haystacks: [t.wiki, t.path] })),
              query,
            ).slice(0, 8);
          }
          if (index >= filtered.length) index = Math.max(0, filtered.length - 1);
        };

        const positionPopup = () => {
          if (!popup) return;
          const coords = view.coordsAtPos(range.to);
          popup.style.left = `${Math.round(coords.left)}px`;
          popup.style.top = `${Math.round(coords.bottom + 4)}px`;
        };

        const render = () => {
          if (!popup) {
            popup = document.createElement('div');
            popup.className = 'mbr-autocomplete';
            popup.setAttribute('role', 'listbox');
            document.body.appendChild(popup);
          }
          popup.textContent = '';

          const head = document.createElement('div');
          head.className = 'mbr-autocomplete-head';
          head.innerHTML =
            mode === 'wiki'
              ? 'Wiki link <kbd>[[…]]</kbd> · <kbd>Tab</kbd> relative link'
              : 'Relative link <kbd>[…](…)</kbd> · <kbd>Tab</kbd> wiki link';
          popup.appendChild(head);

          if (!targets) {
            const loading = document.createElement('div');
            loading.className = 'mbr-autocomplete-empty';
            loading.textContent = 'Loading…';
            popup.appendChild(loading);
          } else if (filtered.length === 0) {
            const empty = document.createElement('div');
            empty.className = 'mbr-autocomplete-empty';
            empty.textContent = query ? 'No matching pages' : 'Type to search pages';
            popup.appendChild(empty);
          } else {
            filtered.forEach((t, i) => {
              const row = document.createElement('div');
              row.className = `mbr-autocomplete-item${i === index ? ' selected' : ''}`;
              row.setAttribute('role', 'option');
              const title = document.createElement('span');
              title.className = 'title';
              title.textContent = t.title;
              const path = document.createElement('span');
              path.className = 'path';
              path.textContent = t.path;
              row.append(title, path);
              // mousedown (not click) + preventDefault keeps the editor
              // selection/focus so the insert range stays valid.
              row.addEventListener('mousedown', (e) => {
                e.preventDefault();
                index = i;
                commit(view);
              });
              popup!.appendChild(row);
            });
          }
          positionPopup();
        };

        const open = () => {
          active = true;
          index = 0;
          mode = 'wiki';
          // Fresh site.json per `[[` session so a just-created/moved page shows.
          session += 1;
          const mySession = session;
          targets = null;
          void services
            .fetchSiteFiles()
            .then((files) => {
              if (!active || mySession !== session) return;
              targets = buildTargets(files);
              refilter();
              render();
            })
            .catch(() => {
              if (!active || mySession !== session) return;
              targets = [];
              render();
            });
          refilter();
          render();
        };

        const close = () => {
          active = false;
          targets = null;
          if (popup) {
            popup.remove();
            popup = null;
          }
        };

        const commit = (v: EditorView) => {
          const target = filtered[index];
          if (!target) return;
          const { state } = v;
          const schema = state.schema;
          let insertedLen: number;
          let tr = state.tr;
          if (mode === 'wiki') {
            const text = `[[${target.wiki}]]`;
            insertedLen = text.length;
            tr = tr.insertText(text, range.from, range.to);
          } else {
            const rel = encodeLinkDestination(relativeUrlPath(services.currentUrl(), target.url));
            const linkMark = schema.marks.link;
            if (linkMark) {
              const node = schema.text(target.title, [linkMark.create({ href: rel })]);
              insertedLen = node.nodeSize;
              tr = tr.replaceRangeWith(range.from, range.to, node);
            } else {
              const text = `[${target.title}](${rel})`;
              insertedLen = text.length;
              tr = tr.insertText(text, range.from, range.to);
            }
          }
          const caret = Math.min(range.from + insertedLen, tr.doc.content.size);
          tr = tr.setSelection(TextSelection.create(tr.doc, caret)).scrollIntoView();
          close();
          v.dispatch(tr);
          v.focus();
        };

        function handleKey(v: EditorView, e: KeyboardEvent) {
          if (!active) return;
          switch (e.key) {
            case 'ArrowDown':
              e.preventDefault();
              e.stopPropagation();
              index = Math.min(index + 1, Math.max(0, filtered.length - 1));
              render();
              break;
            case 'ArrowUp':
              e.preventDefault();
              e.stopPropagation();
              index = Math.max(index - 1, 0);
              render();
              break;
            case 'Tab':
              e.preventDefault();
              e.stopPropagation();
              mode = mode === 'wiki' ? 'link' : 'wiki';
              render();
              break;
            case 'Enter':
              e.preventDefault();
              e.stopPropagation();
              commit(v);
              break;
            case 'Escape':
              // Stop propagation so the editor's Escape handler doesn't also
              // close the whole editor — the popup closes first.
              e.preventDefault();
              e.stopPropagation();
              close();
              break;
            default:
              break;
          }
        }

        const onUpdate = (v: EditorView) => {
          const { selection } = v.state;
          if (!selection.empty) {
            if (active) close();
            return;
          }
          const $from = selection.$from;
          const start = Math.max(0, $from.parentOffset - 200);
          const textBefore = $from.parent.textBetween(start, $from.parentOffset, undefined, '￼');
          // Match the last unclosed `[[` and the query typed after it.
          const m = /\[\[([^[\]\n]*)$/.exec(textBefore);
          if (!m) {
            if (active) close();
            return;
          }
          const matchLen = m[0].length;
          const newFrom = $from.pos - matchLen;
          const newQuery = m[1];
          const wasActive = active;
          range = { from: newFrom, to: $from.pos };
          query = newQuery;
          if (!wasActive) {
            open();
          } else {
            refilter();
            render();
          }
        };

        return {
          update: (v) => onUpdate(v),
          destroy: () => {
            view.dom.removeEventListener('keydown', onKeyDown, true);
            close();
          },
        };
      },
    });
  });
}
