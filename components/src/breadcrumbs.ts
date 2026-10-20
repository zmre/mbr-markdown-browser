/**
 * The "…" in collapsed breadcrumbs (main bundle, no custom element: it enhances
 * the server-rendered `_breadcrumbs.html` in place, as `nav-title.ts` does for
 * the title).
 *
 * Under 600px, theme.css shows a breadcrumb trail on one line: the parent and
 * the current page, the rest hidden. This adds a "…" button in front, only
 * when something is hidden (more than two items), and toggles
 * `.mbr-crumbs-expanded` on the `<nav>`, which shows the whole path, wrapped.
 * The button is in the markup at every width but theme.css shows it only in
 * the narrow layout, so a desktop page is unchanged.
 */

import { ICON_SM, iconElement, iconEllipsis } from './icon-dom.ts';

export const CRUMBS_SELECTOR = 'nav.breadcrumbs';
export const MORE_CLASS = 'mbr-crumbs-more';
export const EXPANDED_CLASS = 'mbr-crumbs-expanded';

/** Adds the "…" toggle to one breadcrumb `<nav>`. Idempotent. */
export function enhanceBreadcrumbs(nav: HTMLElement): void {
  const list = nav.querySelector(':scope > ul');
  if (!list || list.querySelector(`:scope > li.${MORE_CLASS}`)) return;
  // The collapsed line keeps the last two items; only then is anything hidden.
  if (list.querySelectorAll(':scope > li').length <= 2) return;

  const item = document.createElement('li');
  item.className = MORE_CLASS;
  const small = document.createElement('small');
  const button = document.createElement('button');
  button.type = 'button';
  button.append(iconElement(iconEllipsis, { size: ICON_SM }));
  button.setAttribute('aria-label', 'Show the full path');
  button.setAttribute('aria-expanded', 'false');
  button.addEventListener('click', () => {
    const expanded = nav.classList.toggle(EXPANDED_CLASS);
    button.setAttribute('aria-expanded', String(expanded));
  });
  small.append(button);
  item.append(small);
  list.prepend(item);
}

/** Enhances every breadcrumb trail under `root`. */
export function enhanceAllBreadcrumbs(root: ParentNode = document): void {
  root.querySelectorAll<HTMLElement>(CRUMBS_SELECTOR).forEach(enhanceBreadcrumbs);
}

if (typeof document !== 'undefined') {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => enhanceAllBreadcrumbs(), { once: true });
  } else {
    enhanceAllBreadcrumbs();
  }
}
