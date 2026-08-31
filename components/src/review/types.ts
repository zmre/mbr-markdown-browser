/**
 * The review-note model, and the service interfaces the chunk is handed.
 *
 * The note types mirror `TYPE_DEFS` in the repo owner's `zmre/pwnvim`
 * (`pwnvim/plugins/review.lua`) exactly — same six ids, same order — so a
 * review exported from mbr and one exported from pwnvim are interchangeable.
 * The order is the wire order and is load-bearing: it is the dropdown order,
 * and `note-order.ts` does NOT sort by type, so nothing else pins it.
 *
 * Pure declarations only. This module is imported by both the main bundle and
 * the lazy chunk, so it must stay free of state, DOM and fetches.
 */

/**
 * Element-id prefix for a note's marker: `mbr-review-<id>`.
 *
 * Lives here because the two ends of it are in different bundles — the marker
 * layer writes the id in the main bundle, the panel builds `#mbr-review-<id>`
 * hrefs inside the lazy chunk — and this module is the one pure thing both
 * already import. Two literals, one per bundle, is exactly the drift that makes
 * a deep link silently land at the top of the page. Matches the `mbr-task-N`
 * and `mbr-marker-N` anchors the renderer emits.
 */
export const REVIEW_ANCHOR_PREFIX = 'mbr-review-'

/** One of the six review categories. */
export type NoteType = 'issue' | 'suggestion' | 'note' | 'praise' | 'question' | 'insight'

/**
 * How well a note's anchor still matches the document.
 *
 * `exact` — the quote was found and the line is unchanged.
 * `moved`  — the quote (or its prefix) was found somewhere else; the line has
 *            been updated.
 * `lost`   — the quote is gone. The note keeps its last known line and is
 *            never deleted; staleness is a badge, not a deletion.
 */
export type AnchorState = 'exact' | 'moved' | 'lost'

/** Presentation and copy for one note type. */
export interface NoteTypeDef {
  readonly id: NoteType
  readonly label: string
  readonly description: string
}

/**
 * The six types, in pwnvim's order.
 *
 * No artwork here on purpose. Each type's icon is an outline SVG defined once
 * in `templates/theme.css` as `--mbr-review-icon-<id>` and painted as a CSS
 * mask, so the in-document marker and the panel's badge cannot disagree and a
 * repository can retheme both by overriding those properties. Emoji lived here
 * first and were the wrong tool: their per-platform metrics differ enough that
 * no single box centres all six.
 */
export const TYPE_DEFS: readonly NoteTypeDef[] = [
  { id: 'issue', label: 'Issue', description: 'Problems to fix' },
  { id: 'suggestion', label: 'Suggestion', description: 'Improvements' },
  { id: 'note', label: 'Note', description: 'Observations' },
  { id: 'praise', label: 'Praise', description: 'Positive feedback' },
  { id: 'question', label: 'Question', description: 'Clarification needed' },
  { id: 'insight', label: 'Insight', description: 'Useful observations' },
] as const

/** The default type a new note starts on. */
export const DEFAULT_NOTE_TYPE: NoteType = 'note'

/** Lookup by id, or `undefined` for an unknown string. */
export function typeDef(id: string): NoteTypeDef | undefined {
  return TYPE_DEFS.find((def) => def.id === id)
}

/** True when `value` is one of the six ids. */
export function isNoteType(value: unknown): value is NoteType {
  return typeof value === 'string' && TYPE_DEFS.some((def) => def.id === value)
}

/**
 * A single review note.
 *
 * `file` is the repo-relative source path with its extension, `/`-separated —
 * the same value `currentDocumentPath()` returns from
 * `window.frontmatter['markdown_source']`. It is deliberately the *source*
 * path rather than the URL: the export names a file an editor or an AI has to
 * open, and `docs/index.md` is served at `/docs/`.
 *
 * `line` is 1-based, or `null` for a note about the file as a whole.
 * `endLine` is `null` whenever the note covers a single line, so the
 * `file:line` / `file:line-endLine` choice in the export is one null check with
 * no arithmetic.
 *
 * `quote` is the selected text in `find-in-page.ts` `TextIndex` form — verbatim,
 * unnormalized, with a U+0000 separator between blocks. Storing it in that form is what
 * makes re-anchoring a plain `indexOf` against a freshly built index.
 */
export interface ReviewNote {
  id: string
  file: string
  line: number | null
  endLine: number | null
  quote: string | null
  type: NoteType
  body: string
  /** Replacement text. Only meaningful, rendered and exported when `type` is `suggestion`. */
  suggestion: string | null
  /** `null` for a file-level note, which has nothing to re-anchor. */
  anchorState: AnchorState | null
  createdAt: number
  updatedAt: number
}

/** The fields a caller supplies when creating a note; the rest are generated. */
export type NoteDraft = Pick<ReviewNote, 'file' | 'type' | 'body'> &
  Partial<Pick<ReviewNote, 'line' | 'endLine' | 'quote' | 'suggestion' | 'anchorState'>>

/**
 * What a selection resolved to, before it becomes a note.
 *
 * Produced by `anchor.ts`. Every field may be absent: a selection in a block
 * with no `data-mbr-line` carrier (a tight definition list, a static build, a
 * stale custom template) degrades to a file-level note rather than failing.
 */
export interface NoteAnchor {
  file: string
  line: number | null
  endLine: number | null
  quote: string | null
}

/**
 * Source text for a note's suggestion prefill.
 *
 * `exact` is false when the read fell back to the rendered text because
 * `/.mbr/raw` was unavailable — it sits behind `check_edit_access`, so it
 * answers 403 on any server not started with `--edit`, which is the common
 * case rather than an error.
 */
export interface SourceRead {
  text: string
  exact: boolean
}

/** Reads source lines for a suggestion prefill. Injected; see `task-toggle.ts`. */
export type SourceReader = (
  file: string,
  line: number | null,
  endLine: number | null
) => Promise<SourceRead>

/** Maps a repo-relative source path to a page URL, or `null` when unknown. */
export type NoteHrefResolver = (file: string) => string | null

/**
 * The store, as the chunk sees it.
 *
 * The implementation (`review-store.ts`) is main-bundle state and holds the
 * only `localStorage` calls in the feature, so the chunk receives this
 * interface as a Lit property rather than importing it — the rule stated in
 * `tasks/index.ts`.
 */
export interface ReviewStoreApi {
  all(): readonly ReviewNote[]
  save(note: ReviewNote): boolean
  remove(id: string): boolean
  /** Delete every note, across every file. Returns false if the write failed. */
  clear(): boolean
  /** False when a newer mbr wrote a store this build must not clobber. */
  writable(): boolean
  subscribe(listener: () => void): () => void
}
