import { afterEach, describe, expect, it } from 'vitest'
import {
  buildTextIndex,
  compileQuery,
  findMatchOffsets,
  rangeForMatch,
  scrollRangeIntoView,
  type TextIndex,
} from './find-in-page.js'

/**
 * The substance of find-in-page lives here rather than in the Lit element, so
 * these tests cover the parts that decide whether a search is CORRECT: which
 * text is indexed, which is pruned, where match boundaries fall, and how an
 * offset maps back onto a live Range.
 *
 * happy-dom has no `CSS.highlights` and reports a zero-size rect for every
 * range, which is exactly why painting is not exercised here.
 */

/** Mount markup inside the real search container and index it. */
function index(markup: string): TextIndex {
  const wrapper = document.createElement('main')
  wrapper.id = 'wrapper'
  wrapper.innerHTML = markup
  document.body.appendChild(wrapper)
  return buildTextIndex(wrapper)
}

/** Every match of `query` in `markup`, as the text a Range would select. */
function matches(markup: string, query: string, caseSensitive = false): string[] {
  const textIndex = index(markup)
  const pattern = compileQuery(query, caseSensitive)
  if (!pattern) return []
  const { starts, ends } = findMatchOffsets(textIndex, pattern, 1000)
  return Array.from(starts, (start, i) => rangeForMatch(textIndex, start, ends[i])?.toString() ?? '<null>')
}

afterEach(() => {
  document.body.innerHTML = ''
})

describe('buildTextIndex', () => {
  it('concatenates text across inline elements', () => {
    expect(matches('<p>a<em>b</em><strong>c</strong>d</p>', 'abcd')).toEqual(['abcd'])
  })

  it('keeps chunk starts monotonic and parallel to the node list', () => {
    const textIndex = index('<p>one</p><p>two</p>')
    expect(textIndex.nodes.length).toBe(textIndex.starts.length)
    for (let i = 1; i < textIndex.starts.length; i++) {
      expect(textIndex.starts[i]).toBeGreaterThan(textIndex.starts[i - 1])
    }
    // Every chunk's start plus its length is the next chunk's start.
    const last = textIndex.starts.length - 1
    expect(textIndex.starts[last] + (textIndex.nodes[last]?.data.length ?? 1)).toBe(textIndex.text.length)
  })

  it('refuses to match across a block boundary', () => {
    // The whole point of the U+0000 separator: "foo" and "bar" are adjacent in
    // the concatenated text but not adjacent on the page.
    expect(matches('<p>foo</p><p>bar</p>', 'oob')).toEqual([])
    expect(matches('<p>foo</p><p>bar</p>', 'foo bar')).toEqual([])
    expect(matches('<li>foo</li><li>bar</li>', 'foo bar')).toEqual([])
  })

  it('still finds each block individually', () => {
    expect(matches('<p>foo</p><p>bar</p>', 'foo')).toEqual(['foo'])
    expect(matches('<p>foo</p><p>bar</p>', 'bar')).toEqual(['bar'])
  })

  it('treats a <br> as whitespace, not as a boundary', () => {
    // The Range spans the <br>, so its toString() is the DOM's text ("foobar");
    // the newline only ever existed in the index, as a separator the query's
    // flexible whitespace can cross.
    expect(matches('<p>foo<br>bar</p>', 'foo bar')).toEqual(['foobar'])
    expect(matches('<p>foo<br>bar</p>', 'foobar')).toEqual([])
  })

  it('does not merge across a block that follows a <br>', () => {
    expect(matches('<p>foo<br></p><p>bar</p>', 'foo bar')).toEqual([])
  })

  it('prunes script, style, noscript and template subtrees', () => {
    const markup = '<script>secret</script><style>secret</style><noscript>secret</noscript>' +
      '<template><p>secret</p></template><p>visible</p>'
    expect(matches(markup, 'secret')).toEqual([])
    expect(matches(markup, 'visible')).toEqual(['visible'])
  })

  it('prunes SVG text so Mermaid diagrams do not report unpaintable matches', () => {
    expect(matches('<svg><text>diagram</text></svg><p>diagram</p>', 'diagram')).toEqual(['diagram'])
  })

  it('counts a KaTeX formula once, not once per representation', () => {
    const katex = '<span class="katex"><span class="katex-mathml">alpha</span>' +
      '<span class="katex-html">alpha</span></span>'
    expect(matches(katex, 'alpha')).toEqual(['alpha'])
  })

  it('ignores the .sr-only title duplicate index.html emits', () => {
    // The exact shape of templates/index.html: a hidden Pagefind-weighted copy
    // of the title immediately followed by the visible <h1>. Without the
    // .sr-only prune every title search opens on a phantom zero-size match.
    const markup = '<span class="sr-only" data-pagefind-weight="10">Release Notes</span>' +
      '<h1>Release Notes</h1><p>body</p>'
    expect(matches(markup, 'Release Notes')).toEqual(['Release Notes'])
  })

  it('ignores the # permalink mbr-heading-enhancer appends to every heading', () => {
    const markup = '<h2 id="intro">Intro<a class="mbr-heading-anchor" href="#intro" aria-label="Permalink">#</a></h2>'
    expect(matches(markup, '#')).toEqual([])
    expect(matches(markup, 'Intro')).toEqual(['Intro'])
  })

  it('ignores hidden content that has no rect to scroll to', () => {
    expect(matches('<p hidden>ghost</p><p>ghost</p>', 'ghost')).toEqual(['ghost'])
    expect(matches('<p aria-hidden="true">ghost</p><p>ghost</p>', 'ghost')).toEqual(['ghost'])
    expect(matches('<div class="sectionhidden"><p>ghost</p></div><p>ghost</p>', 'ghost')).toEqual(['ghost'])
  })

  it('does not descend into shadow roots', () => {
    // Free: document.createTreeWalker never crosses a shadow boundary, so every
    // <mbr-*> element's internals are invisible with no special-casing.
    const wrapper = document.createElement('main')
    wrapper.id = 'wrapper'
    wrapper.innerHTML = '<p>light</p><div id="host"></div>'
    document.body.appendChild(wrapper)
    const host = wrapper.querySelector('#host') as HTMLElement
    host.attachShadow({ mode: 'open' }).innerHTML = '<span>shadowed</span>'

    const textIndex = buildTextIndex(wrapper)
    expect(textIndex.text).toContain('light')
    expect(textIndex.text).not.toContain('shadowed')
  })

  it('returns an empty index for an empty container', () => {
    const textIndex = index('')
    expect(textIndex.text).toBe('')
    expect(textIndex.nodes).toEqual([])
    expect(textIndex.starts.length).toBe(0)
  })
})

describe('compileQuery', () => {
  it('returns null for a blank query', () => {
    expect(compileQuery('', false)).toBeNull()
    expect(compileQuery('   \n\t ', false)).toBeNull()
  })

  it('escapes regex metacharacters in the query', () => {
    // The query is text a reader typed, not a pattern.
    expect(matches('<p>axb</p>', 'a.b')).toEqual([])
    expect(matches('<p>a.b</p>', 'a.b')).toEqual(['a.b'])
    expect(matches('<p>aaa</p>', 'a+')).toEqual([])
    expect(matches('<p>a+</p>', 'a+')).toEqual(['a+'])
  })

  it('matches across a soft wrap without normalizing the text', () => {
    // Markdown soft-wraps, so <p>hello\nworld</p> is one text node with a
    // newline in it. Flexible separators in the PATTERN keep text offsets 1:1
    // with Text.data, so no inverse mapping is needed.
    expect(matches('<p>foo\nbar</p>', 'foo bar')).toEqual(['foo\nbar'])
    expect(matches('<p>foo&nbsp;bar</p>', 'foo bar')).toEqual(['foo bar'])
    expect(matches('<p>foo   bar</p>', 'foo bar')).toEqual(['foo   bar'])
    expect(matches('<p>foo bar</p>', 'foo\nbar')).toEqual(['foo bar'])
  })

  it('is case-insensitive by default and exact when asked', () => {
    expect(matches('<p>Hello</p>', 'hello')).toEqual(['Hello'])
    expect(matches('<p>Hello</p>', 'hello', true)).toEqual([])
    expect(matches('<p>Hello</p>', 'Hello', true)).toEqual(['Hello'])
  })

  it('produces a global regex so a scan finds every occurrence', () => {
    expect(compileQuery('x', false)?.global).toBe(true)
    expect(matches('<p>x x x</p>', 'x')).toEqual(['x', 'x', 'x'])
  })
})

describe('findMatchOffsets', () => {
  it('counts every match exactly even past the cap', () => {
    const textIndex = index('<p>aaaaaaaaaa</p>')
    const result = findMatchOffsets(textIndex, compileQuery('a', false)!, 3)
    expect(result.total).toBe(10)
    expect(result.starts.length).toBe(3)
    expect(result.ends.length).toBe(3)
  })

  it('terminates on a pattern that can match nothing', () => {
    // A zero-length match leaves lastIndex where it was; without the nudge in
    // the scan loop this test hangs rather than fails.
    const textIndex = index('<p>abc</p>')
    const result = findMatchOffsets(textIndex, /x*/g, 100)
    expect(result.total).toBe(0)
    expect(result.starts.length).toBe(0)
  })

  it('ignores a stale lastIndex on the pattern it is handed', () => {
    const textIndex = index('<p>abab</p>')
    const pattern = compileQuery('ab', false)!
    pattern.lastIndex = 3
    expect(findMatchOffsets(textIndex, pattern, 10).total).toBe(2)
  })
})

describe('rangeForMatch', () => {
  it('spans several text nodes without mutating the DOM', () => {
    // No <mark> wrapping, no split text nodes, so none of the enhancers' cached
    // node references are invalidated.
    const markup = '<p>hello <b>wo</b>rld</p>'
    const before = index(markup).nodes.length
    expect(matches(markup, 'world')).toEqual(['world'])
    expect(index(markup).nodes.length).toBe(before)
  })

  it('resolves an offset to the right node and offset within it', () => {
    const textIndex = index('<p>alpha</p><p>beta</p>')
    const start = textIndex.text.indexOf('eta')
    const range = rangeForMatch(textIndex, start, start + 3)!
    expect(range.toString()).toBe('eta')
    expect((range.startContainer as Text).data).toBe('beta')
    expect(range.startOffset).toBe(1)
  })

  it('returns null for an out-of-bounds or empty span', () => {
    const textIndex = index('<p>alpha</p>')
    expect(rangeForMatch(textIndex, -1, 2)).toBeNull()
    expect(rangeForMatch(textIndex, 2, 2)).toBeNull()
    expect(rangeForMatch(textIndex, 0, textIndex.text.length + 1)).toBeNull()
  })

  it('returns null when the span lands on a synthetic separator', () => {
    const textIndex = index('<p>alpha</p><p>beta</p>')
    // No leading separator (nothing to separate from), then one between the
    // two paragraphs, which owns no node.
    expect(textIndex.nodes[0]).not.toBeNull()
    expect(textIndex.nodes[1]).toBeNull()
    const separator = textIndex.starts[1]
    expect(rangeForMatch(textIndex, separator, separator + 1)).toBeNull()
  })
})

describe('scrollRangeIntoView', () => {
  it('does nothing for a range with no rect', () => {
    // happy-dom reports all-zero rects, so this pins the guard rather than the
    // scrolling itself.
    const textIndex = index('<p>alpha</p>')
    const start = textIndex.text.indexOf('alpha')
    const range = rangeForMatch(textIndex, start, start + 5)!
    expect(() => scrollRangeIntoView(range, 40)).not.toThrow()
    expect(window.scrollY).toBe(0)
  })
})
