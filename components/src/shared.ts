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

/**
 * Promise-based access to site navigation data (for backwards compatibility).
 */
export const siteNav = fetch("/.mbr/site.json")
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
