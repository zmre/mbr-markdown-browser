import { css, html, type TemplateResult } from 'lit'

/**
 * Whether light-DOM content handed to a `<slot>` is worth showing.
 *
 * `<mbr-browse>` and `<mbr-browse-single>` slot the repository's
 * `_browse_footer.html` into the bottom of the sidebar. The compiled-in partial
 * is only an HTML comment, and the template adds whitespace around it, so
 * "something was assigned" is not enough: a footer drawn for a comment would be
 * an empty bordered strip on every repository that never asked for one.
 *
 * Meaningful = any element, or a text node with non-whitespace text. Comments,
 * processing instructions and blank text are not. Pure: no state.
 */
export function hasMeaningfulContent(nodes: Iterable<Node>): boolean {
  for (const node of nodes) {
    if (node.nodeType === Node.ELEMENT_NODE) return true
    if (node.nodeType === Node.TEXT_NODE && (node.textContent ?? '').trim() !== '') return true
  }
  return false
}

/**
 * The sidebar footer both browse elements draw below their scrolling list: a
 * default `<slot>` for `_browse_footer.html`, hidden (no border, no padding)
 * while `hasContent` is false. `onSlotChange` should recompute `hasContent`
 * from the slot's `assignedNodes()`; the element seeds it from its own
 * `childNodes` when it connects, since a slot rendered later (a drawer opened
 * after load) is not guaranteed a `slotchange`.
 */
export function renderSidebarFooter(hasContent: boolean, onSlotChange: (e: Event) => void): TemplateResult {
  return html`
    <div class="sidebar-footer" ?hidden=${!hasContent}>
      <slot @slotchange=${onSlotChange}></slot>
    </div>
  `
}

/** `slotchange` → whether the slot now holds anything worth showing. */
export function slotHasMeaningfulContent(e: Event): boolean {
  const slot = e.target as HTMLSlotElement
  return hasMeaningfulContent(slot.assignedNodes())
}

/**
 * Styles for {@link renderSidebarFooter}. The links themselves are light DOM,
 * styled by theme.css (`mbr-browse > a`), which a repository's user.css can
 * override; this only lays out the strip they sit in.
 */
export const SIDEBAR_FOOTER_CSS = css`
  .sidebar-footer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.25rem 0.5rem;
    flex-shrink: 0;
    padding: 0.5rem 0.75rem;
    border-top: 1px solid var(--pico-muted-border-color, #eee);
  }

  .sidebar-footer[hidden] {
    display: none;
  }
`
