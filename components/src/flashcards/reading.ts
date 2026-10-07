/**
 * Entry point for the small `mbr-flashcards-reading.min.js` chunk (built by
 * vite.flashcards-reading.config.ts).
 *
 * The reading view's enhancements: collapsing each card's
 * `___Review History___` into a `Reviewed 4× · last: Easy, Oct 9` line, and the
 * progress indicators (`progress.ts`: a rating-coloured border on reviewed
 * questions, a pie per heading). The `<mbr-flashcards>` trigger imports this at
 * idle on flashcard pages only, so no other page pays for the history parser,
 * and a flashcard page that is only read never fetches the much larger deck
 * chunk. The trigger skips `decorateProgress` when
 * `flashcards_progress_indicators` is off, and re-runs it after a deck session
 * that wrote reviews.
 *
 * Pure DOM work over the page, no state: safe to load in static builds, and it
 * imports nothing from the main bundle.
 */
export { decorateAllHistories } from './dom.js'
export { decorateProgress } from './progress.js'
