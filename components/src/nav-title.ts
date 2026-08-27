/**
 * Ellipsis + tooltip for the page title in the header.
 *
 * The title sits in the middle of a Pico `nav`, between two groups of buttons.
 * A long title -- or a narrow window, or a large font -- used to push those
 * buttons off the edge. `theme.css` now clips it to one line with an ellipsis;
 * this module is the other half, giving the reader a way to see the full text.
 *
 * It uses Pico's own `data-tooltip`, the same mechanism `mbr-link-enhancement`
 * uses, because Pico styles that on **both** `:hover` and `:focus` -- so one
 * attribute plus a tab stop covers pointer and touch alike, with no tooltip
 * implementation of our own. A native `title` attribute would have covered only
 * hover, which is exactly the case a phone does not have.
 *
 * The attribute is applied *only while the text is actually clipped*. A tooltip
 * that repeats a title already fully on screen is noise, and CSS cannot tell the
 * difference -- comparing `scrollWidth` to `clientWidth` is the only way to know,
 * and it has to be re-checked whenever the box changes width.
 */

/** The `<li>` wrapping the title, as emitted by `templates/_nav.html`. */
const TITLE_SELECTOR = '.mbr-nav-title';

/**
 * Sub-pixel slack for the overflow test.
 *
 * `scrollWidth` and `clientWidth` are integers rounded from fractional layout,
 * so a title that fits exactly can report a 1px overflow and flicker a tooltip
 * onto a title the reader can already read in full.
 */
const OVERFLOW_SLACK_PX = 1;

/** Whether `el`'s content is wider than the box drawn for it. */
export function isClipped(el: HTMLElement): boolean {
  return el.scrollWidth > el.clientWidth + OVERFLOW_SLACK_PX;
}

/**
 * Give `container` a tooltip if its text is clipped, or take one away if not.
 *
 * Idempotent, so it can be called on every resize.
 */
export function syncTitleTooltip(container: HTMLElement): void {
  const text = container.querySelector('strong');
  if (!(text instanceof HTMLElement)) return;

  const full = text.textContent?.trim() ?? '';
  if (full && isClipped(text)) {
    container.setAttribute('data-tooltip', full);
    // Below the title: the header is at the top of the viewport, and Pico's
    // default placement would put the bubble off-screen.
    container.setAttribute('data-placement', 'bottom');
    // A tab stop is what lets a touch device raise the tooltip at all, since
    // tapping focuses. Only while there is something to reveal -- an untruncated
    // title should not be in the tab order.
    container.setAttribute('tabindex', '0');
  } else {
    container.removeAttribute('data-tooltip');
    container.removeAttribute('data-placement');
    container.removeAttribute('tabindex');
  }
}

/**
 * Watch the header title for the whole life of the page.
 *
 * Returns a teardown for tests; nothing in the page calls it.
 */
export function observeNavTitle(root: ParentNode = document): () => void {
  const container = root.querySelector(TITLE_SELECTOR);
  if (!(container instanceof HTMLElement)) return () => {};

  const sync = () => syncTitleTooltip(container);
  sync();

  // ResizeObserver rather than a window `resize` listener: the title's box also
  // changes when a sibling button appears (the page-errors badge, the task
  // clipboard), which resizes nothing.
  if (typeof ResizeObserver === 'undefined') {
    return () => {};
  }
  const observer = new ResizeObserver(sync);
  observer.observe(container);
  return () => observer.disconnect();
}

// Fonts land after first paint and change the measurement, so re-check once they
// are ready. `document.fonts` is absent under happy-dom and in older engines.
function start(): void {
  observeNavTitle();
  const fonts = (document as Document & { fonts?: { ready?: Promise<unknown> } }).fonts;
  void fonts?.ready?.then(() => {
    const container = document.querySelector(TITLE_SELECTOR);
    if (container instanceof HTMLElement) syncTitleTooltip(container);
  });
}

if (typeof document !== 'undefined') {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start, { once: true });
  } else {
    start();
  }
}
