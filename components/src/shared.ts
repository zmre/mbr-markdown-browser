import { buildFolderTree, type FolderNode, type MarkdownFile } from './sorting.js';
import type { IncludeFilter } from './tasks/types.js';

/**
 * Tag source configuration for linking tag values.
 */
export interface TagSourceConfig {
  field: string;
  urlSource: string;
  label: string;
  labelPlural: string;
}

/**
 * MBR configuration from the page.
 */
declare global {
  interface Window {
    __MBR_CONFIG__?: {
      serverMode: boolean;
      guiMode: boolean;
      editEnabled?: boolean;
      tasksEnabled?: boolean;
      reviewEnabled?: boolean;
      /**
       * Typed as `string`, not `IncludeFilter`: this is whatever the page's
       * `_head.html` emitted, and {@link getTasksDefaultInclude} is the thing
       * that decides whether it is one of the three legal values.
       */
      tasksDefaultInclude?: string;
      searchEndpoint?: string;
      basePath?: string;
      tagSources?: TagSourceConfig[];
      graphDepth?: number;
      flashcardsConcentricThreshold?: number;
      flashcardsProgressIndicators?: boolean;
      repoId?: string;
    };
  }
}

/** Bounds for the sidebar mini-graph BFS depth (mirrors the Rust config). */
const GRAPH_DEPTH_MIN = 1;
const GRAPH_DEPTH_MAX = 5;
const GRAPH_DEPTH_DEFAULT = 2;

/**
 * Get the configured relationship-graph BFS depth (`graph_depth` config).
 * Clamped to 1–5; a missing or non-numeric value yields the default of 2.
 */
export function getGraphDepth(): number {
  const raw = window.__MBR_CONFIG__?.graphDepth;
  if (typeof raw !== 'number' || !Number.isFinite(raw)) {
    return GRAPH_DEPTH_DEFAULT;
  }
  return Math.max(GRAPH_DEPTH_MIN, Math.min(GRAPH_DEPTH_MAX, Math.floor(raw)));
}

/** Default Concentric growth threshold (mirrors the Rust config default). */
const CONCENTRIC_THRESHOLD_DEFAULT = 0.7;

/**
 * The flashcard deck's Concentric growth threshold
 * (`flashcards_concentric_threshold`): a missing or non-numeric value yields
 * 0.7, anything else is clamped into (0, 1] as the Rust validation demands.
 */
export function getFlashcardsConcentricThreshold(): number {
  const raw = window.__MBR_CONFIG__?.flashcardsConcentricThreshold;
  if (typeof raw !== 'number' || !Number.isFinite(raw)) return CONCENTRIC_THRESHOLD_DEFAULT;
  return Math.min(1, Math.max(0.01, raw));
}

/** Whether flashcard notes draw reading-view progress indicators (default on). */
export function isFlashcardsProgressEnabled(): boolean {
  return window.__MBR_CONFIG__?.flashcardsProgressIndicators !== false;
}

/**
 * Check if running in GUI mode (native window).
 * GUI mode has no browser URL bar, so link destinations are hidden.
 */
export function isGuiMode(): boolean {
  return window.__MBR_CONFIG__?.guiMode ?? false;
}

/**
 * Check if in-browser editing is enabled for this page.
 * Driven by the server config (`edit_enabled`); never reflects the token hash.
 */
export function isEditEnabled(): boolean {
  return window.__MBR_CONFIG__?.editEnabled ?? false;
}

/**
 * Check if the task browser is available on this page.
 * Driven by the server config (`tasks_enabled`); always false in static builds,
 * because the task index is built from live files.
 */
export function isTasksEnabled(): boolean {
  return window.__MBR_CONFIG__?.tasksEnabled ?? false;
}

/**
 * Check whether this page's block elements carry `data-mbr-line` source lines,
 * which a review anchor needs to name `file.md:LINE`.
 *
 * Driven by the server config (`review_enabled`); always false in static
 * builds, the CLI and QuickLook, which pass `ReviewLines::Omit` unconditionally
 * because an anchor is only meaningful against a live file.
 */
export function isReviewEnabled(): boolean {
  return window.__MBR_CONFIG__?.reviewEnabled ?? false;
}

/**
 * Stable per-repository id (a hash of the server's root directory), used to
 * scope browser storage — currently just review notes — per repository
 * rather than per origin. Several repos served from the same host:port
 * (GUI mode's fixed default port, or a reused dev port) must not share
 * state. Empty string when absent: static builds, the CLI, QuickLook, and
 * error pages, none of which have review notes to scope.
 */
export function getRepoId(): string {
  return window.__MBR_CONFIG__?.repoId ?? '';
}

/** The three legal Show values (mirrors Rust's `task_query::IncludeFilter`). */
const INCLUDE_FILTERS: readonly string[] = ['all', 'tasks', 'markers'];

/** Mirrors Rust's `config::default_tasks_default_include`. */
const TASKS_DEFAULT_INCLUDE: IncludeFilter = 'tasks';

/**
 * Where the task panel's Show filter starts (`tasks_default_include` config).
 *
 * Validated rather than trusted, like {@link getGraphDepth}: a stale custom
 * `.mbr/_head.html` can emit anything, and an unrecognised value reaching the
 * request body would come back a 422 the panel renders as a bare error. Missing
 * or unknown yields `tasks`, the Rust default.
 */
export function getTasksDefaultInclude(): IncludeFilter {
  const raw = window.__MBR_CONFIG__?.tasksDefaultInclude;
  return raw !== undefined && INCLUDE_FILTERS.includes(raw)
    ? (raw as IncludeFilter)
    : TASKS_DEFAULT_INCLUDE;
}

/**
 * Get the base path for resolving asset URLs.
 * In server mode, returns empty string (absolute paths work).
 * In static mode, returns the relative path to root (e.g., "./", "../../").
 * Note: At root level in static mode, returns "./" to ensure relative paths
 * work correctly for both fetch() and dynamic import().
 */
export function getBasePath(): string {
  if (window.__MBR_CONFIG__?.serverMode) {
    return ''; // Server mode uses absolute paths
  }
  // Static mode - return basePath or "./" for root level
  return window.__MBR_CONFIG__?.basePath || './';
}

/**
 * Resolve a root-relative URL path using the current base path.
 * E.g., resolveUrl("/docs/guide/") from depth 2 returns "../../docs/guide/"
 */
/**
 * Check if a keyboard event indicates "open in new tab" intent.
 * Returns true for Cmd+Enter (macOS) or Ctrl+Enter (all platforms).
 */
export function isNewTabModifier(e: KeyboardEvent): boolean {
  return e.metaKey || e.ctrlKey;
}

/**
 * Open a URL in a new tab.
 */
export function openInNewTab(url: string): void {
  window.open(url, '_blank');
}

export function resolveUrl(path: string): string {
  if (window.__MBR_CONFIG__?.serverMode) {
    return path; // Server mode - use absolute paths as-is
  }
  // Static mode - make relative by prepending basePath and stripping leading slash
  const basePath = getBasePath();
  return basePath + path.replace(/^\//, '');
}

/**
 * Get tag sources configuration for linking tag values.
 * Used by mbr-info to create clickable links for tag fields.
 */
export function getTagSources(): TagSourceConfig[] {
  return window.__MBR_CONFIG__?.tagSources ?? [];
}

/**
 * Percent-decode a URL path, falling back to the raw string on a malformed
 * escape sequence (e.g. a lone `%`) so a bad URL never throws.
 */
export function safeDecodePath(p: string): string {
  try {
    return decodeURIComponent(p);
  } catch {
    return p;
  }
}

/**
 * The canonical site path (`/docs/guide/`, the form site.json's `url_path` and
 * links.json's `to`/`from` use) of the page at `pathname`.
 *
 * Server mode serves the site at `/`, so the pathname already is canonical
 * (returned decoded and otherwise untouched). A
 * static build may be deployed anywhere — at a host's root or under a prefix
 * such as GitHub Pages' `/my-site/` — and the page only knows `basePath`, the
 * relative path from itself to the site root (`""` or `"./"` at the root,
 * `"../../"` two levels down). Resolving `basePath` against the pathname, as
 * the browser does for every relative URL on the page, yields the site root's
 * pathname whatever the deployment prefix; the canonical path is what follows
 * it.
 *
 * This replaced counting `../` and keeping that many trailing segments, which
 * had no answer for depth 0: the root page of a site under `/my-site/` came
 * back as `/my-site/`, so its links.json was requested at
 * `my-site/links.json` (a 404, read as "link tracking off") and its info
 * panel, fuzzy-nav links and sidebar highlight were all empty.
 *
 * A trailing `index.html` (a page opened by its file name) is dropped and a
 * missing trailing slash added, so both name the same page as its directory
 * URL. The result is percent-DECODED, because site.json stores paths decoded.
 */
export function canonicalPathFor(pathname: string, basePath: string, serverMode: boolean): string {
  // `window.location.pathname` is percent-encoded (e.g. spaces → `%20`), but
  // site.json stores `url_path`/`neighbor` DECODED (literal spaces). Decode
  // once, last, so the prefix arithmetic below runs on one consistent form.
  if (serverMode) {
    return safeDecodePath(pathname);
  }

  // Every mbr page is a directory URL, so a missing trailing slash (a host
  // that serves `/a/b` without redirecting) still names directory `b`; without
  // it `basePath` would resolve one level too high.
  const stripped = pathname.replace(/\/index\.html$/, '/');
  const page = stripped.endsWith('/') ? stripped : `${stripped}/`;

  // Any origin will do: only the path arithmetic matters, and resolving
  // through `URL` normalizes `./`, `../` and an empty base the way the
  // browser does for the page's own relative links.
  let root: string;
  try {
    root = new URL(basePath || './', `http://mbr.invalid${page.startsWith('/') ? '' : '/'}${page}`).pathname;
  } catch {
    root = '/';
  }

  // `page` ends in `/`, so `rest` is empty (the root) or ends in `/` too.
  const rest = page.startsWith(root) ? page.slice(root.length) : page.replace(/^\/+/, '');
  return `/${safeDecodePath(rest)}`;
}

/** {@link canonicalPathFor} for the current page. */
export function getCanonicalPath(): string {
  return canonicalPathFor(
    window.location.pathname,
    window.__MBR_CONFIG__?.basePath ?? '',
    Boolean(window.__MBR_CONFIG__?.serverMode),
  );
}

/**
 * Reactive state for site navigation loading.
 * Components can subscribe to changes via the callback pattern.
 */
interface SiteNavState {
  isLoading: boolean;
  data: any | null;
  error: string | null;
}

const siteNavState: SiteNavState = {
  isLoading: true,
  data: null,
  error: null,
};

const siteNavListeners: Set<(state: SiteNavState) => void> = new Set();

/**
 * Subscribe to site navigation state changes.
 * Returns an unsubscribe function.
 */
export function subscribeSiteNav(callback: (state: SiteNavState) => void): () => void {
  siteNavListeners.add(callback);
  // Immediately notify with current state
  callback(siteNavState);
  return () => siteNavListeners.delete(callback);
}

/**
 * Get current site navigation loading state.
 */
export function getSiteNavState(): SiteNavState {
  return { ...siteNavState };
}

// Determine the URL for site.json based on mode
function getSiteJsonUrl(): string {
  if (window.__MBR_CONFIG__?.serverMode) {
    return '/.mbr/site.json'; // Absolute path in server mode
  }
  return getBasePath() + '.mbr/site.json'; // Relative path in static mode
}

const siteJsonUrl = getSiteJsonUrl();

/**
 * Promise-based access to site navigation data (for backwards compatibility).
 */
export const siteNav = fetch(siteJsonUrl)
  .then((resp) => {
    if (!resp.ok) {
      throw new Error(`Failed to load site data: ${resp.status}`);
    }
    return resp.json();
  })
  .then((data) => {
    siteNavState.isLoading = false;
    siteNavState.data = data;
    siteNavState.error = null;
    // Notify all listeners
    siteNavListeners.forEach(cb => cb({ ...siteNavState }));
    return data;
  })
  .catch((err) => {
    siteNavState.isLoading = false;
    siteNavState.data = null;
    siteNavState.error = err.message || 'Failed to load site data';
    // Notify all listeners
    siteNavListeners.forEach(cb => cb({ ...siteNavState }));
    throw err;
  })

/**
 * Memoized folder tree derived from site.json.
 *
 * Building the tree is O(files) and every consumer needs the *same* tree, so it
 * is computed once off the shared `siteNav` promise instead of once per
 * component (mbr-nav used to rebuild the whole tree just to render two buttons).
 * The repo's configured `index_file` is honored, so repos using `_index.md`
 * attach their landing page to the folder node rather than the parent.
 */
let siteFolderTreePromise: Promise<FolderNode> | null = null;

/**
 * Get the shared folder tree for the whole site (memoized).
 * Rejects if site.json failed to load.
 */
export function getSiteFolderTree(): Promise<FolderNode> {
  if (!siteFolderTreePromise) {
    siteFolderTreePromise = siteNav.then((data: any) => {
      const files: MarkdownFile[] = data?.markdown_files ?? [];
      const indexFile: string = data?.index_file || 'index.md';
      return buildFolderTree(files, indexFile);
    });
  }
  return siteFolderTreePromise;
}

// ============================================================================
// Media Navigation (separate endpoint for media/static file metadata)
// ============================================================================

/**
 * Reactive state for media navigation loading.
 * Components can subscribe to changes via the callback pattern.
 */
interface MediaNavState {
  isLoading: boolean;
  data: any | null;
  error: string | null;
}

const mediaNavState: MediaNavState = {
  isLoading: true,
  data: null,
  error: null,
};

const mediaNavListeners: Set<(state: MediaNavState) => void> = new Set();

/**
 * Subscribe to media navigation state changes.
 * Returns an unsubscribe function.
 *
 * The first subscription lazily kicks off the media.json fetch; the catalog is
 * only needed by the media browser, which lives behind the search popup.
 */
export function subscribeMediaNav(callback: (state: MediaNavState) => void): () => void {
  mediaNavListeners.add(callback);
  // Errors are surfaced through mediaNavState, so swallow the rejection here to
  // avoid an unhandled promise rejection for subscribers that never await.
  loadMediaNav().catch(() => {});
  // Immediately notify with current state
  callback(mediaNavState);
  return () => mediaNavListeners.delete(callback);
}

/**
 * Get current media navigation loading state.
 */
export function getMediaNavState(): MediaNavState {
  return { ...mediaNavState };
}

// Determine the URL for media.json based on mode
function getMediaJsonUrl(): string {
  if (window.__MBR_CONFIG__?.serverMode) {
    return '/.mbr/media.json'; // Absolute path in server mode
  }
  return getBasePath() + '.mbr/media.json'; // Relative path in static mode
}

let mediaNavPromise: Promise<any> | null = null;

/**
 * Promise-based access to media navigation data (memoized, lazy).
 *
 * In server mode, fetches from the dedicated /.mbr/media.json endpoint.
 * In static mode, fetches from .mbr/media.json (generated during build).
 *
 * This is deliberately NOT fetched at module scope: shared.ts ships in the
 * always-loaded main bundle, but media.json is only consumed by the media
 * browser inside the search popup, and in server mode the endpoint blocks on
 * media probing (PDF parsing, video metadata). The first `subscribeMediaNav`
 * triggers it; later calls reuse the same promise.
 */
export function loadMediaNav(): Promise<any> {
  if (!mediaNavPromise) {
    mediaNavState.isLoading = true;
    mediaNavPromise = fetch(getMediaJsonUrl())
      .then((resp) => {
        if (!resp.ok) {
          throw new Error(`Failed to load media data: ${resp.status}`);
        }
        return resp.json();
      })
      .then((data) => {
        mediaNavState.isLoading = false;
        mediaNavState.data = data;
        mediaNavState.error = null;
        mediaNavListeners.forEach(cb => cb({ ...mediaNavState }));
        return data;
      })
      .catch((err) => {
        mediaNavState.isLoading = false;
        mediaNavState.data = null;
        mediaNavState.error = err.message || 'Failed to load media data';
        mediaNavListeners.forEach(cb => cb({ ...mediaNavState }));
        throw err;
      });
  }
  return mediaNavPromise;
}
