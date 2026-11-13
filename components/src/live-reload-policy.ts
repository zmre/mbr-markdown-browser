/**
 * Which file changes make the page on screen out of date — the pure half of
 * `<mbr-live-reload>`.
 *
 * The server forwards every change in the repository to every open page
 * (`/.mbr/ws/changes`), so this is where "a save of some other note reloads
 * the note I am reading" is decided. A page depends on:
 *
 * - **its own note** (`markdown_source`), and only when the bytes on disk
 *   differ from the version it shows. The server puts the note's current
 *   fingerprint on every frame about it (`content_hash`, the same function as
 *   the render's `markdown_source_hash`), because one save arrives as several
 *   events — FSEvents reports an in-place write as created + three modified,
 *   and the echo of an atomic rename can come seconds later — and an event
 *   reaching the *reloaded* page used to reload it a second time;
 * - **the templates** (`.mbr/`, or the configured template folder): the server
 *   flags those frames `template`. The raw `.mbr/config.toml` event is not one
 *   of them: the server answers a configuration change with its own
 *   `reloaded` event once the new configuration is being served;
 * - for a page that is **not a note** (folder listing, home, a 404, a tag or
 *   media page — no `markdown_source`): the folder its URL names. A file or
 *   folder directly in it changes the listing, a subfolder's index note
 *   changes that folder's title, and a note named like the URL (`/x/` →
 *   `x.md`, `x/index.md`) is what a 404 there is waiting for.
 *
 * Nothing else: not a sibling `index.md`, not an `.html`/`.css`/`.js` file
 * elsewhere in the repository (both used to reload every note).
 */

/** One frame from `/.mbr/ws/changes`. */
export interface LiveReloadChange {
  relative_path: string
  /** `reloaded` is not a file: the server now serves a changed configuration. */
  event: 'modified' | 'created' | 'deleted' | 'reloaded'
  /** The page's own note's fingerprint on disk now (frames about it only). */
  content_hash?: string
  /** The change is in the template folder: every page renders differently. */
  template?: boolean
}

/** What the page on screen was rendered from. */
export interface LiveReloadPage {
  /** The page's own note, repo-relative (`markdown_source`), or null. */
  source: string | null
  /** The version of `source` the page shows, or null when unknown. */
  shownHash: string | null
  /** The page's URL path (`location.pathname`), for pages with no `source`. */
  pathname: string
}

/** The raw `config.toml` event, which the server's `reloaded` supersedes. */
const CONFIG_FILE = '.mbr/config.toml'

/**
 * A repo-relative path as the server spells it: `/`-separated, no leading `/`
 * or `./`, no trailing `/`, Unicode NFC (macOS can hand back decomposed names).
 */
export function normalizeRepoPath(path: string): string {
  return path
    .replace(/\\/g, '/')
    .replace(/^(\.\/|\/)+/, '')
    .replace(/\/+$/, '')
    .normalize('NFC')
}

/** The folder a URL path names, repo-relative (`/docs/a%20b/` → `docs/a b`). */
export function folderOfPathname(pathname: string): string {
  let decoded = pathname
  try {
    decoded = decodeURIComponent(pathname)
  } catch {
    // A malformed escape: compare the path as written.
  }
  return normalizeRepoPath(decoded)
}

function parentOf(path: string): string {
  const slash = path.lastIndexOf('/')
  return slash < 0 ? '' : path.slice(0, slash)
}

function baseName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1)
}

/** Whether a change at `changed` alters the page for folder `folder`. */
function affectsFolderPage(changed: string, folder: string): boolean {
  const parent = parentOf(changed)
  if (parent === folder) return true
  // A subfolder's index note names the subfolder in the listing.
  if (parentOf(parent) === folder && /^index\.[^/]+$/i.test(baseName(changed))) return true
  // `/x/` serves `x.md` once it exists: a 404 there waits for it.
  return folder !== '' && parentOf(folder) === parent && baseName(changed) === `${baseName(folder)}.md`
}

/**
 * Whether `change` makes `page` out of date. Self-writes (task toggles,
 * flashcard reviews) are the caller's business and are checked first.
 */
export function shouldReloadFor(change: LiveReloadChange, page: LiveReloadPage): boolean {
  if (change.event === 'reloaded') return true
  const changed = normalizeRepoPath(change.relative_path)
  if (changed === CONFIG_FILE) return false
  if (change.template === true) return true

  if (page.source !== null) {
    if (changed !== normalizeRepoPath(page.source)) return false
    // The disk holds exactly what the page shows: an echo of a save the page
    // has already reloaded for, or a touch. A deletion always reloads.
    const unchanged =
      change.event !== 'deleted' &&
      typeof change.content_hash === 'string' &&
      page.shownHash !== null &&
      change.content_hash === page.shownHash
    return !unchanged
  }

  return affectsFolderPage(changed, folderOfPathname(page.pathname))
}
