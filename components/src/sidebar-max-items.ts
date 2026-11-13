/**
 * How many items a sidebar section or the browse middle pane renders per page
 * before "show more": the `sidebar_max_items` config option, which the page
 * templates write as a `max-items` attribute on `<mbr-browse>` and
 * `<mbr-browse-single>`. Shared so both elements read it the same way.
 */

/** The config option's default (`config.rs`'s `default_sidebar_max_items`). */
export const DEFAULT_SIDEBAR_MAX_ITEMS = 100;

/**
 * The page size a `max-items` attribute asks for: a positive integer, else
 * the default (absent, empty, zero, negative or not a number — a custom
 * template may omit it or write anything).
 */
export function sidebarMaxItems(attr: string | null): number {
  if (!attr) return DEFAULT_SIDEBAR_MAX_ITEMS;
  const parsed = Number.parseInt(attr, 10);
  return Number.isNaN(parsed) || parsed <= 0 ? DEFAULT_SIDEBAR_MAX_ITEMS : parsed;
}
