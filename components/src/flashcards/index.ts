/**
 * Entry point for the lazy `mbr-flashcards.min.js` chunk (built by
 * vite.flashcards.config.ts; loaded by the `<mbr-flashcards>` trigger the first
 * time a deck is opened).
 *
 * Importing this module registers `<mbr-flashcard-deck>`.
 *
 * IMPORTANT: nothing in this chunk may import stateful main-bundle modules —
 * `shared.ts`, `task-toggle.ts`, `flashcard-review.ts`, `edit-token.ts`. A
 * second copy would hold a second source-line cache and a second self-write
 * window, and a review written through it would reload the page under the
 * overlay. The writer arrives as the deck's `recordReview` property.
 */
export { MbrFlashcardDeckElement, DECK_CLOSE_EVENT, type DeckMode } from './mbr-flashcard-deck.js'
