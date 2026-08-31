/**
 * Shared contract between `mbr-keys` (which owns the global keydown handler)
 * and the overlays it has to know about.
 *
 * `mbr-keys` must suppress bare-letter shortcuts while an overlay owns the
 * keyboard, and open overlays on `/`, `F2` and `f`/`F`/`T`. It used to do both
 * by reaching into TS-private fields (`(el as any)._isOpen`, `_isDrawerOpen`),
 * which discarded the element types that `HTMLElementTagNameMap` already
 * provides: renaming a private field still compiled, and the guard silently
 * reported "closed" forever after. Every overlay element now declares
 * `implements MbrOverlay`, so a rename is a compile error at both ends.
 */
export interface MbrOverlay {
  /** True while the overlay is visible and owns the keyboard. */
  readonly isOpen: boolean;
  /** Show the overlay. */
  open(): void;
  /** Hide the overlay. */
  close(): void;
}

/**
 * Tag names of every element implementing {@link MbrOverlay}.
 *
 * Their backing state differs (`_isOpen` vs `_isDrawerOpen`), which is exactly
 * why detection goes through the interface instead of a per-element branch.
 */
export const OVERLAY_TAGS = [
  'mbr-search',
  'mbr-browse',
  'mbr-browse-single',
  'mbr-fuzzy-nav',
  'mbr-find-bar',
  'mbr-tasks',
  'mbr-review',
] as const satisfies readonly (keyof HTMLElementTagNameMap)[];

/** Tag name of an element implementing {@link MbrOverlay}. */
export type OverlayTag = (typeof OVERLAY_TAGS)[number];

/**
 * True when any overlay currently in `root` reports itself open.
 *
 * An element can be in the DOM before its custom-element definition has run
 * (lazy chunks, a failed bundle), in which case it has no `isOpen` at all —
 * hence the explicit `=== true` rather than a truthiness coercion.
 */
export function isAnyOverlayOpen(root: ParentNode = document): boolean {
  return OVERLAY_TAGS.some((tag) => root.querySelector(tag)?.isOpen === true);
}

/**
 * Find an overlay element that is present AND upgraded, so callers can drive it
 * through the contract without risking a `TypeError` on a plain `HTMLElement`.
 */
export function findOverlay(tag: OverlayTag, root: ParentNode = document): MbrOverlay | null {
  const el = root.querySelector(tag);
  return el && typeof el.open === 'function' && typeof el.close === 'function' ? el : null;
}
