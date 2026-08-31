/**
 * Finding a note's quote again after the document under it changed.
 *
 * The WYSIWYG editor re-serializes the whole file on save (`editor-crepe.ts`
 * hands `crepe.getMarkdown()` back through a normalizing serializer), so line
 * numbers shift even when nothing semantic changed. A note anchored purely by
 * line would silently point at the wrong sentence; one anchored by its quoted
 * text can find itself again.
 *
 * Everything here is pure over strings and numbers — no DOM, no fetch, no
 * clock — which is what makes the whole drift algorithm testable directly. The
 * caller supplies the haystack (a fresh `find-in-page.ts` `TextIndex.text`) and
 * a line lookup; this module decides *where* the quote went.
 *
 * # Why the search is verbatim
 *
 * The quote is stored in `TextIndex` form: exactly the characters
 * `buildTextIndex` produced, with no normalization. So relocating it is a plain
 * `indexOf` and there is no inverse mapping to get wrong — the point
 * `find-in-page.ts` argues at length about its own text buffer. A normalized
 * quote would need to map an offset in normalized space back to a DOM position,
 * which is the single most bug-prone part of any find implementation.
 *
 * # Why this needs no network
 *
 * The search runs against the rendered page, not the markdown source, so it
 * works on a server started without `--edit` — where `/.mbr/raw` answers 403.
 * Rendered text and source text are never crossed: a rendered quote is matched
 * against a rendered index, and the line comes from the `data-mbr-line`
 * attribute the renderer emitted.
 */

import type { AnchorState } from './types.ts'

/**
 * How much of a quote must still match for a note to count as relocated.
 *
 * A prefix rather than a fuzzy edit distance: an edit that changed the *start*
 * of the quoted sentence has changed what the note was about, and should read
 * as lost rather than be dragged somewhere plausible. An edit to its tail
 * usually has not.
 */
export const PREFIX_LEN = 60

/** Where a quote was found, and how confidently. */
export interface QuoteMatches {
  /** Offsets into the haystack, ascending. Empty when nothing matched. */
  starts: number[]
  /** Length of the needle actually matched, so the caller can build a range. */
  length: number
  /** True when the whole quote matched; false when only its prefix did. */
  exact: boolean
}

/**
 * Every place `quote` occurs in `haystack`, preferring whole-quote matches.
 *
 * Falls back to the first {@link PREFIX_LEN} characters only when the full
 * quote is nowhere to be found, so a document containing both an edited and an
 * unedited copy still resolves to the unedited one.
 */
export function quoteMatches(
  haystack: string,
  quote: string,
  prefixLen: number = PREFIX_LEN
): QuoteMatches {
  if (quote.length === 0) return { starts: [], length: 0, exact: false }

  const full = occurrences(haystack, quote)
  if (full.length > 0) return { starts: full, length: quote.length, exact: true }

  // A quote shorter than the prefix has already been searched in full.
  if (quote.length <= prefixLen) return { starts: [], length: 0, exact: false }

  const prefix = quote.slice(0, prefixLen)
  const partial = occurrences(haystack, prefix)
  if (partial.length > 0) return { starts: partial, length: prefix.length, exact: false }

  return { starts: [], length: 0, exact: false }
}

/** All non-overlapping occurrences of `needle`, ascending. */
function occurrences(haystack: string, needle: string): number[] {
  const found: number[] = []
  let at = haystack.indexOf(needle)
  while (at !== -1) {
    found.push(at)
    at = haystack.indexOf(needle, at + needle.length)
  }
  return found
}

/** A candidate location for a relocated quote. */
export interface Candidate {
  /** Offset into the haystack. */
  start: number
  /** 1-based source line that offset resolves to, or `null` if unknown. */
  line: number | null
}

/**
 * The candidate closest to where the note used to be.
 *
 * Repeated prose is the case this exists for — a boilerplate sentence appearing
 * in five places must not send a note to the first one. Distance is measured in
 * source lines, so a note follows its own paragraph as the document grows above
 * it.
 *
 * A candidate with no line loses to any candidate that has one, and ties break
 * toward the earlier offset so the result is deterministic.
 */
export function pickNearest(
  candidates: readonly Candidate[],
  storedLine: number | null
): Candidate | null {
  if (candidates.length === 0) return null
  if (storedLine === null) return candidates[0] ?? null

  let best: Candidate | null = null
  let bestDistance = Number.POSITIVE_INFINITY
  for (const candidate of candidates) {
    const distance = candidate.line === null ? Number.MAX_SAFE_INTEGER : Math.abs(candidate.line - storedLine)
    if (distance < bestDistance) {
      best = candidate
      bestDistance = distance
    }
  }
  return best
}

/** A note's anchor before re-anchoring. */
export interface StoredAnchor {
  line: number | null
  endLine: number | null
  quote: string | null
}

/** A note's anchor after re-anchoring. */
export interface ResolvedAnchor {
  line: number | null
  endLine: number | null
  anchorState: AnchorState
}

/**
 * Where a note's anchor should now point.
 *
 * `lineAt` maps a haystack offset to the source line of the block containing
 * it — in practice `rangeForMatch` followed by `closest('[data-mbr-line]')`,
 * which is the DOM half the caller owns. Returning `null` from it is fine and
 * simply means the block carries no line.
 *
 * A quote that cannot be found is reported `lost` **with its stored line
 * intact**. The note is never dropped and never renumbered to something
 * invented: the last known line is the best information available, the export
 * stays pasteable, and the staleness shows as a badge instead.
 */
export function nextAnchorState(
  stored: StoredAnchor,
  haystack: string,
  lineAt: (offset: number) => number | null,
  prefixLen: number = PREFIX_LEN
): ResolvedAnchor {
  const unchanged: ResolvedAnchor = {
    line: stored.line,
    endLine: stored.endLine,
    anchorState: 'lost',
  }

  if (stored.quote === null || stored.quote.length === 0) return unchanged

  const matches = quoteMatches(haystack, stored.quote, prefixLen)
  if (matches.starts.length === 0) return unchanged

  const candidates = matches.starts.map((start) => ({ start, line: lineAt(start) }))
  const best = pickNearest(candidates, stored.line)
  if (best === null || best.line === null) return unchanged

  const endLine = lineAt(best.start + Math.max(0, matches.length - 1))
  const resolvedEnd = endLine !== null && endLine > best.line ? endLine : null

  const moved = !matches.exact || best.line !== stored.line || resolvedEnd !== stored.endLine
  return {
    line: best.line,
    endLine: resolvedEnd,
    anchorState: moved ? 'moved' : 'exact',
  }
}
