/**
 * "Top-level" blocks of a rendered note: a list that is not nested inside
 * another markdown container.
 *
 * Pure DOM, no imports, no state: safe for the main bundle and every lazy
 * chunk. Two features read the page's own structure this way and must agree
 * on what "nested" means:
 *
 * - flashcards (`flashcards/dom.ts`): only top-level definition lists are a
 *   deck, matching what `flashcards.rs` will write to;
 * - the calendar agenda (`calendar/agenda-scope.ts`): only top-level list items
 *   are agenda entries, nested bullets being their details.
 *
 * `<section>` is deliberately absent: it is the renderer's own wrapper
 * (`enable_sections`), not markdown nesting. `.mbr-chat` is present although it
 * is not markdown nesting either: a chat bubble's body is rendered markdown,
 * but in the source it is the text of a ```chat code fence.
 */
export const MARKDOWN_NESTING_SELECTOR = 'li, blockquote, dd, td, th, .footnote-definition, .mbr-chat'

/**
 * True when no ancestor of `el` (above it, up to the document) matches
 * `nesting`. The element itself is not tested, so a `<ul>` is top-level even
 * though it would match `li`'s parent rule.
 */
export function isTopLevel(el: Element, nesting: string = MARKDOWN_NESTING_SELECTOR): boolean {
  return el.parentElement?.closest(nesting) == null
}
