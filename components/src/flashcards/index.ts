/**
 * Entry point for the lazy `mbr-flashcards.min.js` chunk (built by
 * vite.flashcards.config.ts; loaded by the `<mbr-flashcards>` trigger the first
 * time a deck is opened).
 *
 * Importing this module registers `<mbr-flashcard-deck>`.
 *
 * IMPORTANT: nothing in this chunk may import stateful main-bundle modules —
 * `shared.ts`, `task-toggle.ts`, `edit-token.ts`. A
 * second copy would hold a second source-line cache and a second self-write
 * window, and a review written through it would reload the page under the
 * overlay. Their state arrives as `ReviewServices`, passed to `makeReviewRecorder`.
 */
export { MbrFlashcardDeckElement, DECK_CLOSE_EVENT, type DeckMode } from './mbr-flashcard-deck.js'
// The writer is built here, from main-bundle services the trigger passes in.
export { makeReviewRecorder } from './review-writer.js'
