import { describe, expect, it } from 'vitest'
import { nextAnchorState, pickNearest, PREFIX_LEN, quoteMatches } from './reanchor.ts'

/**
 * A haystack built from numbered lines, plus the `lineAt` a caller would derive
 * from `rangeForMatch` + `closest('[data-mbr-line]')`. Line N starts at the Nth
 * newline, so offsets map back to 1-based lines exactly the way the DOM does.
 */
function document(lines: string[]): { text: string; lineAt: (offset: number) => number | null } {
  const text = lines.join('\n')
  const starts: number[] = []
  let at = 0
  for (const line of lines) {
    starts.push(at)
    at += line.length + 1
  }
  return {
    text,
    lineAt: (offset) => {
      if (offset < 0 || offset > text.length) return null
      let line = 0
      for (let i = 0; i < starts.length; i++) {
        if (starts[i]! <= offset) line = i + 1
      }
      return line === 0 ? null : line
    },
  }
}

describe('quoteMatches', () => {
  it('finds a single exact occurrence', () => {
    const m = quoteMatches('alpha beta gamma', 'beta')
    expect(m).toEqual({ starts: [6], length: 4, exact: true })
  })

  it('finds every non-overlapping occurrence', () => {
    const m = quoteMatches('ab ab ab', 'ab')
    expect(m.starts).toEqual([0, 3, 6])
    expect(m.exact).toBe(true)
  })

  it('finds nothing for an empty quote', () => {
    expect(quoteMatches('anything', '')).toEqual({ starts: [], length: 0, exact: false })
  })

  it('falls back to the prefix when the full quote is gone', () => {
    const quote = `${'x'.repeat(PREFIX_LEN)} TAIL THAT WAS EDITED`
    const haystack = `lead-in ${'x'.repeat(PREFIX_LEN)} a different tail`
    const m = quoteMatches(haystack, quote)
    expect(m.exact).toBe(false)
    expect(m.length).toBe(PREFIX_LEN)
    expect(m.starts).toEqual([8])
  })

  it('prefers a whole-quote match over a prefix match elsewhere', () => {
    // A document holding both an edited and an unedited copy must resolve to
    // the unedited one.
    const quote = `${'x'.repeat(PREFIX_LEN)} ORIGINAL TAIL`
    const haystack = `${'x'.repeat(PREFIX_LEN)} EDITED TAIL ... ${quote}`
    const m = quoteMatches(haystack, quote)
    expect(m.exact).toBe(true)
    expect(m.starts).toEqual([haystack.lastIndexOf(quote)])
  })

  it('does not re-search a quote shorter than the prefix', () => {
    expect(quoteMatches('nothing here', 'short quote')).toEqual({
      starts: [],
      length: 0,
      exact: false,
    })
  })
})

describe('pickNearest', () => {
  it('returns null for no candidates', () => {
    expect(pickNearest([], 5)).toBeNull()
  })

  it('returns the first candidate when there is no stored line', () => {
    const first = { start: 10, line: 3 }
    expect(pickNearest([first, { start: 40, line: 9 }], null)).toBe(first)
  })

  it('picks the candidate closest to the stored line', () => {
    // Repeated boilerplate: the note must not jump to the first copy.
    const candidates = [
      { start: 0, line: 2 },
      { start: 50, line: 40 },
      { start: 90, line: 80 },
    ]
    expect(pickNearest(candidates, 38)?.line).toBe(40)
  })

  it('follows a paragraph that moved down', () => {
    expect(pickNearest([{ start: 0, line: 120 }], 40)?.line).toBe(120)
  })

  it('prefers a candidate with a line over one without', () => {
    const withLine = { start: 99, line: 500 }
    expect(pickNearest([{ start: 0, line: null }, withLine], 1)).toBe(withLine)
  })

  it('breaks a tie toward the earlier offset', () => {
    const first = { start: 0, line: 4 }
    expect(pickNearest([first, { start: 80, line: 6 }], 5)).toBe(first)
  })
})

describe('nextAnchorState', () => {
  const doc = document(['# Title', '', 'The quick brown fox.', '', 'Another paragraph here.'])

  it('reports exact when nothing moved', () => {
    const result = nextAnchorState(
      { line: 3, endLine: null, quote: 'The quick brown fox.' },
      doc.text,
      doc.lineAt
    )
    expect(result).toEqual({ line: 3, endLine: null, anchorState: 'exact' })
  })

  it('reports moved and updates the line when the quote shifted', () => {
    const grown = document(['# Title', '', 'New intro.', '', 'The quick brown fox.'])
    const result = nextAnchorState(
      { line: 3, endLine: null, quote: 'The quick brown fox.' },
      grown.text,
      grown.lineAt
    )
    expect(result).toEqual({ line: 5, endLine: null, anchorState: 'moved' })
  })

  it('reports lost and keeps the stored line when the quote is gone', () => {
    // The note is never deleted and never renumbered to something invented —
    // the last known line is the best information available, and the export
    // has to stay pasteable.
    const result = nextAnchorState(
      { line: 3, endLine: null, quote: 'A sentence that was deleted.' },
      doc.text,
      doc.lineAt
    )
    expect(result).toEqual({ line: 3, endLine: null, anchorState: 'lost' })
  })

  it('reports lost for a note with no quote', () => {
    expect(
      nextAnchorState({ line: 3, endLine: null, quote: null }, doc.text, doc.lineAt)
    ).toEqual({ line: 3, endLine: null, anchorState: 'lost' })
  })

  it('recomputes endLine for a multi-line span', () => {
    const spanning = document(['alpha', 'beta', 'gamma'])
    const result = nextAnchorState(
      { line: 1, endLine: 3, quote: 'alpha\nbeta\ngamma' },
      spanning.text,
      spanning.lineAt
    )
    expect(result).toEqual({ line: 1, endLine: 3, anchorState: 'exact' })
  })

  it('counts a prefix-only match as moved even at the same line', () => {
    const quote = `${'x'.repeat(PREFIX_LEN)} ORIGINAL`
    const edited = document([`${'x'.repeat(PREFIX_LEN)} REWRITTEN`])
    const result = nextAnchorState({ line: 1, endLine: null, quote }, edited.text, edited.lineAt)
    expect(result.anchorState).toBe('moved')
    expect(result.line).toBe(1)
  })

  it('reports lost when the match lands somewhere with no line', () => {
    const result = nextAnchorState(
      { line: 2, endLine: null, quote: 'orphan' },
      'orphan',
      () => null
    )
    expect(result).toEqual({ line: 2, endLine: null, anchorState: 'lost' })
  })
})
