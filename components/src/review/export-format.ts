/**
 * Renders a set of review notes as the markdown a reader copies out.
 *
 * The shape is `zmre/pwnvim`'s `markdown()` in `pwnvim/plugins/review.lua`,
 * matched deliberately so a review assembled in mbr can be pasted alongside one
 * assembled in the editor:
 *
 * ```markdown
 * # Code Review
 *
 * 1. **[SUGGESTION]** `flake.nix:26`
 *    The comment body.
 * ```
 *
 * Pure: no DOM, no Lit, no state, no clock. Every input arrives as an argument
 * so the output is a function of the notes alone and can be asserted verbatim.
 */

import type { ReviewNote } from './types.ts'

/** The document heading, and the whole output when there are no notes. */
const HEADING = '# Code Review'

/**
 * Indent for a note's body and its suggestion fence.
 *
 * Exactly three spaces regardless of the ordinal's width, which is what pwnvim
 * emits. It stays correct past item 9 even though `10. ` is four characters
 * wide: CommonMark's lazy-continuation rule keeps an under-indented line in the
 * same paragraph, and the fence only has to clear the three-space limit at
 * which an indented code block would start.
 */
const INDENT = '   '

/**
 * Wrap `value` in a code span that survives backticks inside it.
 *
 * A repo path may legally contain a backtick, and `` `a`b` `` would end the
 * span early and spill raw markdown into the location. CommonMark's rule: use a
 * longer run of backticks than any inside, and pad with a space when the
 * content starts or ends with one.
 */
export function inlineCode(value: string): string {
  const longest = (value.match(/`+/g) ?? []).reduce((max, run) => Math.max(max, run.length), 0)
  const fence = '`'.repeat(longest + 1)
  const pad = value.startsWith('`') || value.endsWith('`') ? ' ' : ''
  return `${fence}${pad}${value}${pad}${fence}`
}

/**
 * The opening/closing fence for a code block containing `value`.
 *
 * Same escaping problem as {@link inlineCode}: a suggestion may itself contain
 * a fenced block, and three backticks would close early.
 */
export function fenceFor(value: string): string {
  const longest = (value.match(/`+/g) ?? []).reduce((max, run) => Math.max(max, run.length), 0)
  return '`'.repeat(Math.max(3, longest + 1))
}

/** `ISSUE`, `SUGGESTION`, … — the bracketed label in an item's first line. */
export function typeLabel(note: ReviewNote): string {
  return note.type.toUpperCase()
}

/**
 * `file`, `file:line`, or `file:line-endLine`.
 *
 * `endLine` is `null` for a single-line note by construction (see
 * `ReviewNote`), so this needs no comparison — but a note edited by hand or
 * recovered from an older store could still carry an equal or inverted pair,
 * hence the guard.
 */
export function formatLocation(note: ReviewNote): string {
  if (note.line === null) return note.file
  if (note.endLine !== null && note.endLine > note.line) {
    return `${note.file}:${note.line}-${note.endLine}`
  }
  return `${note.file}:${note.line}`
}

/**
 * Normalize a body or suggestion for output: one line-ending convention, no
 * trailing whitespace, no leading or trailing blank lines.
 */
function normalizeBlock(value: string): string[] {
  return value
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map((line) => line.replace(/[ \t]+$/, ''))
    .join('\n')
    .replace(/^\n+|\n+$/g, '')
    .split('\n')
}

/**
 * Indent one body line.
 *
 * A blank line is emitted **empty** rather than as three spaces: trailing
 * whitespace is invisible, survives a round trip through most editors, and
 * shows up as a diff nobody asked for.
 */
function indent(line: string): string {
  return line.length === 0 ? '' : INDENT + line
}

/**
 * The `suggestion`-fenced replacement block for a suggestion note.
 *
 * ` ```suggestion ` rather than a bare fence or a diff: pwnvim has no
 * suggestion payload to copy, and this is the tag GitHub's review UI and every
 * coding agent already recognise — so the export *does* something at the other
 * end rather than just describing itself.
 */
function suggestionLines(note: ReviewNote): string[] {
  if (note.type !== 'suggestion') return []
  const text = (note.suggestion ?? '').replace(/\r\n?/g, '\n').replace(/\n+$/, '')
  if (text.length === 0) return []
  const fence = fenceFor(text)
  return ['', indent(`${fence}suggestion`), ...text.split('\n').map(indent), indent(fence)]
}

/** One numbered item, without its trailing blank line. */
function formatNote(note: ReviewNote, ordinal: number): string[] {
  const head = `${ordinal}. **[${typeLabel(note)}]** ${inlineCode(formatLocation(note))}`
  const body = normalizeBlock(note.body).map(indent)
  return [head, ...body, ...suggestionLines(note)]
}

/**
 * Render the whole review.
 *
 * Notes are numbered continuously across files, with no per-file headings —
 * pwnvim's sample spans two files with unbroken numbering, and the numbering is
 * how a reader refers to an item in conversation.
 *
 * Callers pass notes already in {@link sortNotes} order; this function does not
 * reorder, so the panel's on-screen order and the copied order cannot diverge.
 */
export function formatReview(notes: readonly ReviewNote[]): string {
  const lines = [HEADING, '']
  notes.forEach((note, i) => {
    lines.push(...formatNote(note, i + 1), '')
  })
  // `lines` already ends with '' after the last item (or after the heading when
  // there are none), so a single join gives exactly one trailing newline.
  return lines.join('\n')
}
