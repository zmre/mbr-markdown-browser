/**
 * Entry point for the small `mbr-flashcards-reading.min.js` chunk (built by
 * vite.flashcards-reading.config.ts).
 *
 * The reading view's one enhancement: collapsing each card's
 * `___Review History___` into a `Reviewed 4× · last: Easy, Oct 9` line. The
 * `<mbr-flashcards>` trigger imports this at idle on flashcard pages only, so
 * no other page pays for the history parser, and a flashcard page that is only
 * read never fetches the much larger deck chunk.
 *
 * Pure DOM work over the page, no state: safe to load in static builds, and it
 * imports nothing from the main bundle.
 */
export { decorateAllHistories } from './dom.js'
