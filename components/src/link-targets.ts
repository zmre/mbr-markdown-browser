/**
 * Note-link targets from `site.json`'s `markdown_files`: the one definition
 * of "what a `[[Name]]` link to this note is called", shared by the body
 * editor's `[[` autocomplete (`editor-link-autocomplete.ts`) and the
 * properties form's note-link fields (`frontmatter/mbr-widgets.ts`).
 *
 * The name is the note's `title`, else its filename stem: the first and last
 * of the renderer's resolution order (title → alias → stem,
 * `relationships.rs`), so a link written from it resolves.
 *
 * Pure: no DOM, no fetches, no editor imports.
 */

import { normalizeUrl } from './editor-picker-shared.js';

/** A `site.json` markdown file, as far as link targets read it. */
export interface LinkTargetSite {
  readonly url_path: string;
  readonly frontmatter?: { readonly title?: unknown; readonly [key: string]: unknown } | null;
}

/** A note one can link to. */
export interface LinkTarget {
  /** Normalized directory-style URL (`/people/jane/`). */
  readonly url: string;
  /** The name a wiki link uses: the title, else the filename stem. */
  readonly wiki: string;
  /** Text shown for the note (the same name). */
  readonly title: string;
  /** Display path for disambiguation (the URL). */
  readonly path: string;
}

/** The display name (stem) of a normalized directory-style URL. */
function stemOf(url: string): string {
  const segs = url.split('/').filter((s) => s.length > 0);
  return segs[segs.length - 1] ?? '';
}

/** One target per site file, in site.json order. */
export function buildLinkTargets(files: readonly LinkTargetSite[]): LinkTarget[] {
  return files.map((f) => {
    const url = normalizeUrl(f.url_path);
    const title = typeof f.frontmatter?.title === 'string' ? f.frontmatter.title.trim() : '';
    const name = title || stemOf(url) || url;
    return { url, wiki: name, title: name, path: url };
  });
}

/** `[[Name]]` for a target. */
export function wikilinkText(target: Pick<LinkTarget, 'wiki'>): string {
  return `[[${target.wiki}]]`;
}

/**
 * The name inside a `[[Name]]` or `[[Name|Alias]]` value (the target before
 * the pipe, as the renderer reads it), or `null` when `value` is not one.
 */
export function wikilinkName(value: string): string | null {
  const m = /^\[\[([^[\]]+)\]\]$/.exec(value.trim());
  if (!m) return null;
  const name = m[1].split('|')[0].trim();
  return name || null;
}
