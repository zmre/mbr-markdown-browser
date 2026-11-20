/**
 * Where an editor's floating UI goes: the `[[` link autocomplete and the
 * wikilink / section-attribute source popovers. Shared by `<mbr-editor>` and
 * `<mbr-markdown-field>`; no state, no page globals.
 */

/**
 * The attribute that marks an editor's floating-UI host. The `[[` link
 * autocomplete and the wikilink / section-attribute source popovers attach to
 * the nearest ancestor carrying it, else to `<body>` (`<mbr-editor>`'s modal
 * lives in the page's light DOM, where `<body>` is right). `<mbr-markdown-field>`
 * sets it on its themed wrapper so its popovers inherit the field's colours.
 */
export const POPOVER_HOST_ATTR = 'data-mbr-popover-host';

/** Where floating editor UI anchored at `anchor` is appended (see {@link POPOVER_HOST_ATTR}). */
export function popoverHostFor(anchor: Element): HTMLElement {
  return anchor.closest<HTMLElement>(`[${POPOVER_HOST_ATTR}]`) ?? document.body;
}
