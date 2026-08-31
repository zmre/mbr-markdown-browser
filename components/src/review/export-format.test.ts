import { beforeEach, describe, expect, it } from 'vitest'
import { fenceFor, formatLocation, formatReview, inlineCode, typeLabel } from './export-format.ts'
import { makeNote, resetNoteIds, T0 } from './test-fixtures.ts'

beforeEach(() => resetNoteIds())

describe('formatReview', () => {
  it('reproduces the reference export from TODO.md', () => {
    // The sample the whole feature is specified against, from the repo owner's
    // pwnvim review export. Byte-for-byte, including the three-space indent.
    const notes = [
      makeNote({
        file: 'flake.nix',
        line: 26,
        type: 'suggestion',
        suggestion: null,
        body:
          "Did you know that numtide's llm-agents repo https://github.com/numtide/llm-agents.nix has omp?  " +
          'And they build into binary caches, which would speed this up (probably) if you used that as the input.',
      }),
      makeNote({
        file: 'flake.nix',
        line: 35,
        type: 'note',
        body: 'If you use the numtide version, this update stuff can go away - you just need to update the flake lock',
      }),
      makeNote({
        file: 'PLAN.md',
        line: 7,
        type: 'question',
        body: 'Not sure what it means by the four-cli toolchain dead weight. The priv options? The other codex and gemini stuff?',
      }),
      makeNote({
        file: 'PLAN.md',
        line: 15,
        type: 'note',
        body: 'I dunno, I kind of like the predictable wrapup. When I use claude without it I just have a wall of text to sort through.',
      }),
    ]

    // Passed in the sample's order, which is also sortNotes order for these
    // (PLAN.md < flake.nix would reorder, so assert against the given order).
    expect(formatReview(notes)).toBe(
      [
        '# Code Review',
        '',
        "1. **[SUGGESTION]** `flake.nix:26`",
        "   Did you know that numtide's llm-agents repo https://github.com/numtide/llm-agents.nix has omp?  And they build into binary caches, which would speed this up (probably) if you used that as the input.",
        '',
        '2. **[NOTE]** `flake.nix:35`',
        '   If you use the numtide version, this update stuff can go away - you just need to update the flake lock',
        '',
        '3. **[QUESTION]** `PLAN.md:7`',
        '   Not sure what it means by the four-cli toolchain dead weight. The priv options? The other codex and gemini stuff?',
        '',
        '4. **[NOTE]** `PLAN.md:15`',
        '   I dunno, I kind of like the predictable wrapup. When I use claude without it I just have a wall of text to sort through.',
        '',
      ].join('\n')
    )
  })

  it('returns just the heading when there are no notes', () => {
    expect(formatReview([])).toBe('# Code Review\n')
  })

  it('ends with exactly one newline', () => {
    const out = formatReview([makeNote()])
    expect(out.endsWith('\n')).toBe(true)
    expect(out.endsWith('\n\n')).toBe(false)
  })

  it('numbers continuously across files, with no per-file headings', () => {
    const out = formatReview([
      makeNote({ file: 'a.md', line: 1 }),
      makeNote({ file: 'b.md', line: 1 }),
      makeNote({ file: 'c.md', line: 1 }),
    ])
    expect(out).toContain('1. **[NOTE]** `a.md:1`')
    expect(out).toContain('2. **[NOTE]** `b.md:1`')
    expect(out).toContain('3. **[NOTE]** `c.md:1`')
    expect(out).not.toContain('## ')
  })

  it('indents multi-line bodies with three spaces and leaves blank lines empty', () => {
    const out = formatReview([makeNote({ body: 'first\n\nsecond' })])
    expect(out).toContain('   first\n\n   second')
    // Trailing whitespace on the blank line would be invisible and would show
    // up as a spurious diff wherever the review is pasted.
    expect(out).not.toMatch(/[ \t]+\n/)
  })

  it('keeps the three-space indent past item nine', () => {
    // `10. ` is four characters wide, but CommonMark lazy continuation keeps an
    // under-indented line in the same paragraph — and three spaces is what
    // pwnvim emits, so the two exports stay identical.
    const notes = Array.from({ length: 10 }, (_, i) => makeNote({ line: i + 1 }))
    const out = formatReview(notes)
    expect(out).toContain('10. **[NOTE]** `doc.md:10`\n   A comment.')
  })

  it('normalizes CRLF and strips trailing whitespace inside a body', () => {
    const out = formatReview([makeNote({ body: 'one  \r\ntwo\r\n' })])
    expect(out).toContain('   one\n   two')
    expect(out).not.toContain('\r')
  })
})

describe('formatLocation', () => {
  it('omits the line for a file-level note', () => {
    expect(formatLocation(makeNote({ line: null }))).toBe('doc.md')
  })

  it('uses file:line for a single-line note', () => {
    expect(formatLocation(makeNote({ line: 12 }))).toBe('doc.md:12')
  })

  it('uses file:line-endLine for a span', () => {
    expect(formatLocation(makeNote({ line: 12, endLine: 18 }))).toBe('doc.md:12-18')
  })

  it('ignores an endLine that is not past the start', () => {
    expect(formatLocation(makeNote({ line: 12, endLine: 12 }))).toBe('doc.md:12')
    expect(formatLocation(makeNote({ line: 12, endLine: 9 }))).toBe('doc.md:12')
  })
})

describe('suggestion payload', () => {
  it('emits an indented ```suggestion fence', () => {
    const out = formatReview([
      makeNote({ type: 'suggestion', body: 'Tighten this.', suggestion: 'The fox jumps quickly.' }),
    ])
    expect(out).toBe(
      [
        '# Code Review',
        '',
        '1. **[SUGGESTION]** `doc.md:10`',
        '   Tighten this.',
        '',
        '   ```suggestion',
        '   The fox jumps quickly.',
        '   ```',
        '',
      ].join('\n')
    )
  })

  it('is omitted for a non-suggestion note even when text is present', () => {
    const out = formatReview([makeNote({ type: 'note', suggestion: 'ignored' })])
    expect(out).not.toContain('suggestion')
  })

  it('is omitted when the suggestion is empty', () => {
    const out = formatReview([makeNote({ type: 'suggestion', suggestion: '' })])
    expect(out).not.toContain('```')
  })

  it('lengthens the fence past a fenced block inside the suggestion', () => {
    const out = formatReview([
      makeNote({ type: 'suggestion', suggestion: '```js\nconst a = 1\n```' }),
    ])
    expect(out).toContain('   ````suggestion')
    expect(out).toContain('   ````\n')
  })
})

describe('inlineCode', () => {
  it('uses a single backtick for ordinary text', () => {
    expect(inlineCode('doc.md:12')).toBe('`doc.md:12`')
  })

  it('lengthens the run past backticks in the content', () => {
    expect(inlineCode('a`b')).toBe('``a`b``')
  })

  it('pads when the content starts or ends with a backtick', () => {
    expect(inlineCode('`x')).toBe('`` `x ``')
    expect(inlineCode('x`')).toBe('`` x` ``')
  })
})

describe('fenceFor', () => {
  it('is three backticks by default', () => {
    expect(fenceFor('plain text')).toBe('```')
  })

  it('is never shorter than three', () => {
    expect(fenceFor('a`b')).toBe('```')
  })

  it('clears the longest run inside', () => {
    expect(fenceFor('```\nx\n```')).toBe('````')
    expect(fenceFor('`````')).toBe('``````')
  })
})

describe('typeLabel', () => {
  it('uppercases the type', () => {
    expect(typeLabel(makeNote({ type: 'suggestion' }))).toBe('SUGGESTION')
    expect(typeLabel(makeNote({ type: 'insight' }))).toBe('INSIGHT')
  })
})

describe('fixture sanity', () => {
  it('uses a fixed clock so output is byte-stable', () => {
    expect(makeNote().createdAt).toBe(T0)
  })
})
