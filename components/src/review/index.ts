/**
 * Entry point for the lazy `mbr-review.min.js` chunk (loaded on demand by the
 * `<mbr-review>` trigger the first time a note is written or the panel is
 * opened).
 *
 * Importing this module registers `<mbr-review-panel>` and `<mbr-review-form>`.
 * The form is registered independently of the panel on purpose: `r` on a
 * selection opens the form with no panel at all.
 *
 * IMPORTANT: nothing in this chunk may import stateful main-bundle modules.
 * Specifically **not** `review-store.ts` (the only web-storage user in the
 * feature, and the holder of the in-memory note list and its subscriber set),
 * **not** `shared.ts` (a top-level `site.json` fetch and the URL resolver built
 * from it) and **not** `task-toggle.ts` (a per-page raw-file line cache and the
 * live-reload suppression window). A second copy of any of them inside the
 * chunk would be a second cache and a second subscriber set, diverging from the
 * one the page is already using. Pure modules — `../safe-href.ts` and
 * everything else under `review/` — are fine; everything stateful arrives
 * through element properties set by the trigger.
 */
export { MbrReviewPanelElement } from './mbr-review-panel.ts'
// The marker-id prefix is shared with the main bundle's marker layer, so it
// lives in the pure types module rather than in either bundle's element.
export { REVIEW_ANCHOR_PREFIX } from './types.ts'
export { MbrReviewFormElement } from './mbr-review-form.ts'
export type { ReviewSaveDetail } from './mbr-review-form.ts'
export { renderNoteCard } from './note-card.ts'
export type { NoteCardOptions } from './note-card.ts'
export type {
  AnchorState,
  NoteAnchor,
  NoteDraft,
  NoteHrefResolver,
  NoteType,
  NoteTypeDef,
  ReviewNote,
  ReviewStoreApi,
  SourceRead,
  SourceReader,
} from './types.ts'
