/**
 * A task hit's text as the panel shows it: wikilinks read as the names the
 * rendered page shows, and a marker word kept washable.
 *
 * The server sends a task's text with annotations stripped but its inline
 * markup verbatim, so `Call [[Maya Okafor]]` and `[[people/maya|Maya]]` came
 * through with their brackets. On the page pulldown-cmark draws a wikilink as
 * its alias, else its target; this reads them the same way.
 *
 * The names are plain text, not links: the whole text already sits inside the
 * card's own link to the task's line, and a link cannot hold another. (The
 * chunk could not resolve them anyway without site.json, which lives in the
 * main bundle.)
 *
 * A marker's `marker_start`/`marker_end` are UTF-16 offsets into the server's
 * text, so rewriting a wikilink before the marker would shift them. Segmenting
 * keeps every offset in the original string: the marker span is cut from the
 * original, and a wikilink overlapping it is left as written rather than
 * guessed at.
 *
 * Pure: no DOM, no imports, unit-tested on its own.
 */

/** One run of a hit's display text. */
export interface TextSegment {
  kind: 'text' | 'wikilink' | 'marker'
  text: string
}

/** `[[target]]` or `[[target|alias]]` on one line, no nested brackets. */
const WIKILINK = /\[\[([^[\]\n]+)\]\]/g

/**
 * The name the page draws for a wikilink's inner text (between the brackets):
 * the alias after the first `|`, else the target. `null` when both are blank,
 * so `[[ | ]]` stays as written.
 */
export function wikilinkDisplay(inner: string): string | null {
  const pipe = inner.indexOf('|')
  const target = (pipe === -1 ? inner : inner.slice(0, pipe)).trim()
  const alias = pipe === -1 ? '' : inner.slice(pipe + 1).trim()
  const shown = alias || target
  return shown.length > 0 ? shown : null
}

/**
 * Split `text` into plain runs, wikilink names and the marker word.
 *
 * `marker` is `[start, end)` in UTF-16 units of `text`, already checked by
 * the caller; `null` for a task (or a marker whose range is unusable).
 */
export function taskTextSegments(text: string, marker: readonly [number, number] | null): TextSegment[] {
  const segments: TextSegment[] = []
  const pushText = (value: string) => {
    if (value.length === 0) return
    const last = segments[segments.length - 1]
    if (last?.kind === 'text') last.text += value
    else segments.push({ kind: 'text', text: value })
  }

  // Cuts in document order: each wikilink that becomes a name, and the marker.
  const cuts: Array<{ start: number; end: number; segment: TextSegment }> = []
  for (const match of text.matchAll(WIKILINK)) {
    const start = match.index
    const end = start + match[0].length
    // An embed (`![[image.png]]`) is not a name: leave it as written.
    if (start > 0 && text[start - 1] === '!') continue
    // Overlapping the marker: rewriting it would move the marker's offsets.
    if (marker && start < marker[1] && end > marker[0]) continue
    const shown = wikilinkDisplay(match[1])
    if (shown === null) continue
    cuts.push({ start, end, segment: { kind: 'wikilink', text: shown } })
  }
  if (marker) {
    cuts.push({ start: marker[0], end: marker[1], segment: { kind: 'marker', text: text.slice(marker[0], marker[1]) } })
  }
  cuts.sort((a, b) => a.start - b.start)

  let at = 0
  for (const cut of cuts) {
    pushText(text.slice(at, cut.start))
    segments.push(cut.segment)
    at = cut.end
  }
  pushText(text.slice(at))
  return segments
}

/** The display text as one string (no marker wash): for labels and tooltips. */
export function taskDisplayText(text: string): string {
  return taskTextSegments(text, null)
    .map((s) => s.text)
    .join('')
}
