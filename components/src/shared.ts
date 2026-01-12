/**
 * MBR configuration from the page.
 */
declare global {
  interface Window {
    __MBR_CONFIG__?: {
      serverMode: boolean;
      guiMode: boolean;
      searchEndpoint?: string;
      basePath?: string;
    };
  }
}

/**
 * Check if running in GUI mode (native window).
 * GUI mode has no browser URL bar, so link destinations are hidden.
 */
export function isGuiMode(): boolean {
  return window.__MBR_CONFIG__?.guiMode ?? false;
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
export function resolveUrl(path: string): string {
  if (window.__MBR_CONFIG__?.serverMode) {
    return path; // Server mode - use absolute paths as-is
  }
  // Static mode - make relative by prepending basePath and stripping leading slash
  const basePath = getBasePath();
  return basePath + path.replace(/^\//, '');
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
