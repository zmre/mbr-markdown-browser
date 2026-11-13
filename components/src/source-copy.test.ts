import { afterEach, describe, expect, it } from 'vitest'
import {
  balanceSpan,
  blockSpan,
  fitSpan,
  narrowToSelection,
  numberedBlocks,
  pageMarkdown,
  selectionMarkdown,
  selectionSource,
  stripFrontmatter,
} from './source-copy.ts'

afterEach(() => {
  document.body.innerHTML = ''
})

/** The narrowed source text for `selected` in `source`, or null. */
function narrow(source: string, selected: string): string | null {
  const span = narrowToSelection(source, selected)
  return span === null ? null : source.slice(span.start, span.end)
}

describe('stripFrontmatter / pageMarkdown', () => {
  it('drops a leading YAML block and the blank lines around the body', () => {
    expect(pageMarkdown(['---', 'title: A', 'tags: [x]', '---', '', '# A', '', 'Body', '', ''])).toBe('# A\n\nBody')
  })

  it('accepts `...` as the closing fence and a BOM before the opening one', () => {
    expect(stripFrontmatter(['﻿---', 'a: 1', '...', 'Body'])).toEqual(['Body'])
  })

  it('leaves a note without frontmatter whole', () => {
    expect(pageMarkdown(['# A', '', '---', '', 'B'])).toBe('# A\n\n---\n\nB')
  })

  it('does not treat a rule followed by a blank line, or an unclosed block, as frontmatter', () => {
    expect(pageMarkdown(['---', '', 'Text'])).toBe('---\n\nText')
    expect(pageMarkdown(['---', 'not: closed', 'Text'])).toBe('---\nnot: closed\nText')
  })
})

describe('block spans', () => {
  function blocks(markup: string): Element[] {
    document.body.innerHTML = `<main id="wrapper">${markup}</main>`
    return numberedBlocks(document.querySelector('main')!)
  }

  it('runs a block to the line before the next block that is not nested in it', () => {
    const list = blocks(
      '<ul><li data-mbr-line="1">a<ul><li data-mbr-line="2">b</li></ul></li><li data-mbr-line="3">c</li></ul>' +
        '<p data-mbr-line="5">d</p>'
    )
    expect(blockSpan(list, 0)).toEqual({ start: 1, end: 2 })
    expect(blockSpan(list, 1)).toEqual({ start: 2, end: 2 })
    expect(blockSpan(list, 2)).toEqual({ start: 3, end: 4 })
    expect(blockSpan(list, 3, 9)).toEqual({ start: 5, end: 9 })
  })

  it('gives a definition term one line', () => {
    const list = blocks('<dl><dt data-mbr-line="1">Q</dt><dd data-mbr-line="2">A</dd></dl>')
    expect(blockSpan(list, 0)).toEqual({ start: 1, end: 1 })
  })

  it('ignores elements with a bad line', () => {
    expect(blocks('<p data-mbr-line="x">a</p><p data-mbr-line="0">b</p><p data-mbr-line="2">c</p>')).toHaveLength(1)
  })

  it('fits a span to the file: end of file, trailing blanks, a paragraph ends at a blank line', () => {
    const lines = ['para one', 'para two', '', '---', '', 'next', '']
    expect(fitSpan(lines, { start: 1, end: 5 })).toEqual({ start: 1, end: 4 })
    expect(fitSpan(lines, { start: 1, end: 5 }, true)).toEqual({ start: 1, end: 2 })
    expect(fitSpan(lines, { start: 6, end: 99 })).toEqual({ start: 6, end: 6 })
    expect(fitSpan(lines, { start: 9, end: 9 })).toBeNull()
  })
})

describe('narrowToSelection', () => {
  it('finds plain text', () => {
    expect(narrow('The quick brown fox.', 'quick brown')).toBe('quick brown')
  })

  it('steps over emphasis and keeps a wrapper the selection fills', () => {
    expect(narrow('Some **bold** text', 'bold')).toBe('**bold**')
    expect(narrow('Some **bold** text', 'Some bold')).toBe('Some **bold**')
    expect(narrow('a *b* and _c_ d', 'b and c')).toBe('*b* and _c_')
    expect(narrow('run `code` here', 'code')).toBe('`code`')
    expect(narrow('~~gone~~ now', 'gone now')).toBe('~~gone~~ now')
  })

  it('widens to the closer of an emphasis opened inside the selection', () => {
    expect(narrow('x **bold words** y', 'x bold')).toBe('x **bold words**')
    expect(narrow('x **bold words** y', 'words y')).toBe('**bold words** y')
  })

  it('steps over a link destination and keeps the link whole', () => {
    expect(narrow('See [the docs](https://example.com/a_(b)) here.', 'the docs here')).toBe(
      '[the docs](https://example.com/a_(b)) here'
    )
    expect(narrow('See [the docs](u) here.', 'docs')).toBe('[the docs](u)')
    expect(narrow('An ![alt text](img.png) image', 'alt text')).toBe('![alt text](img.png)')
    expect(narrow('A [ref link][r] here', 'ref link here')).toBe('[ref link][r] here')
  })

  it('steps over a wikilink target and inline HTML', () => {
    expect(narrow('Go to [[Some Note|the note]] now', 'the note now')).toBe('[[Some Note|the note]] now')
    expect(narrow('Press <kbd>Ctrl</kbd>+<kbd>K</kbd> to search', 'Ctrl+K to')).toBe('Ctrl</kbd>+<kbd>K</kbd> to')
  })

  it('reads escapes and entities as the characters they render', () => {
    expect(narrow('2 \\* 3 = 6', '2 * 3')).toBe('2 \\* 3')
    expect(narrow('Tom &amp; Jerry', 'Tom & Jerry')).toBe('Tom &amp; Jerry')
  })

  it('maps smart punctuation back to what was typed', () => {
    expect(narrow(`It's "quoted" -- and more...`, 'It’s “quoted” – and more…')).toBe(`It's "quoted" -- and more...`)
    expect(narrow('a --- b', 'a — b')).toBe('a --- b')
  })

  it('crosses soft line breaks and blockquote prefixes', () => {
    expect(narrow('> first line\n> second line', 'line second')).toBe('line\n> second')
    expect(narrow('one\ntwo', 'one two')).toBe('one\ntwo')
  })

  it('crosses table cells', () => {
    expect(narrow('| a | b |\n|---|---|\n| one | two |', 'one two')).toBe('one | two')
  })

  it('is null when the text is not in the source', () => {
    expect(narrow('Due @due(2026-01-01)', 'Due Jan 1')).toBeNull()
    expect(narrow('abc', '')).toBeNull()
  })
})

describe('balanceSpan', () => {
  it('leaves a balanced span alone', () => {
    const source = 'a **b** c'
    expect(balanceSpan(source, { start: 0, end: source.length })).toEqual({ start: 0, end: source.length })
  })

  it('takes the opening bracket of a link closed inside the span', () => {
    const source = 'x [one two](u) y'
    // "two](u) y"
    const span = balanceSpan(source, { start: 7, end: source.length })
    expect(source.slice(span.start, span.end)).toBe('[one two](u) y')
  })
})

describe('selectionSource / selectionMarkdown', () => {
  const SOURCE = [
    '---', // 1
    'title: T', // 2
    '---', // 3
    '# Heading', // 4
    '', // 5
    'First **para**', // 6
    'continues here.', // 7
    '', // 8
    '---', // 9
    '', // 10
    '- item one', // 11
    '- item [two](u)', // 12
    '', // 13
    '```', // 14
    'code', // 15
    '```', // 16
    '', // 17
  ]

  function page(): HTMLElement {
    document.body.innerHTML =
      '<main id="wrapper">' +
      '<div class="mbr-contact-card"><p>Card text</p></div>' +
      '<h1 id="h" data-mbr-line="4">Heading<a class="mbr-heading-anchor" href="#h">#</a></h1>' +
      '<p data-mbr-line="6">First <strong>para</strong>\ncontinues here.</p>' +
      '<hr>' +
      '<ul><li data-mbr-line="11">item one</li><li data-mbr-line="12">item <a href="u">two</a></li></ul>' +
      '<pre data-mbr-line="14"><code>code\n</code></pre>' +
      '</main>'
    return document.querySelector('main')!
  }

  const text = (el: Element): string => (el.textContent ?? '').replace('#', '')

  function rangeOver(startNode: Node, startOffset: number, endNode: Node, endOffset: number): Range {
    const range = document.createRange()
    range.setStart(startNode, startOffset)
    range.setEnd(endNode, endOffset)
    return range
  }

  function copy(range: Range, root: HTMLElement): string | null {
    const where = selectionSource(range, root, range.toString().replace('#', ''), text)
    return where === null ? null : selectionMarkdown(SOURCE, where)
  }

  it('copies the lines of every block a selection spans, whole', () => {
    const root = page()
    const p = root.querySelector('p[data-mbr-line]')!
    const li = root.querySelectorAll('li')[0]!
    // From the middle of the paragraph to the middle of the first item.
    const range = rangeOver(p.lastChild!, 3, li.firstChild!, 4)
    expect(copy(range, root)).toBe('First **para**\ncontinues here.\n\n---\n\n- item one')
  })

  it('narrows a selection inside one block to its exact source', () => {
    const root = page()
    const strong = root.querySelector('strong')!
    const range = rangeOver(strong.firstChild!, 0, strong.parentElement!.lastChild!, 10)
    expect(copy(range, root)).toBe('**para**\ncontinues')
  })

  it('copies a whole block whole, markers included', () => {
    const root = page()
    const h1 = root.querySelector('h1')!
    const range = document.createRange()
    range.selectNodeContents(h1)
    expect(copy(range, root)).toBe('# Heading')
    const li = root.querySelectorAll('li')[1]!
    range.selectNodeContents(li)
    expect(copy(range, root)).toBe('- item [two](u)')
  })

  it('keeps a paragraph to its own lines (not the rule after it)', () => {
    const root = page()
    const range = document.createRange()
    range.selectNodeContents(root.querySelector('p[data-mbr-line]')!)
    expect(copy(range, root)).toBe('First **para**\ncontinues here.')
  })

  it('narrows inside a code block', () => {
    const root = page()
    const code = root.querySelector('code')!
    expect(copy(rangeOver(code.firstChild!, 1, code.firstChild!, 3), root)).toBe('od')
  })

  it('ignores a triple-click overshoot into the next block', () => {
    const root = page()
    const h1 = root.querySelector('h1')!
    const p = root.querySelector('p[data-mbr-line]')!
    expect(copy(rangeOver(h1.firstChild!, 0, p.firstChild!, 0), root)).toBe('# Heading')
  })

  it('does not map a selection that includes text outside numbered blocks', () => {
    const root = page()
    const card = root.querySelector('.mbr-contact-card p')!
    const h1 = root.querySelector('h1')!
    const range = rangeOver(card.firstChild!, 0, h1.firstChild!, 3)
    expect(selectionSource(range, root, range.toString(), text)).toBeNull()
  })

  it('ignores the permalink, which has no source of its own', () => {
    const root = page()
    const h1 = root.querySelector('h1')!
    const range = rangeOver(h1.firstChild!, 0, h1.querySelector('a')!.firstChild!, 1)
    expect(copy(range, root)).toBe('# Heading')
  })

  it('maps nothing on a page without line numbers', () => {
    document.body.innerHTML = '<main id="wrapper"><p>plain</p></main>'
    const root = document.querySelector('main')!
    const range = document.createRange()
    range.selectNodeContents(root.querySelector('p')!)
    expect(selectionSource(range, root, 'plain', text)).toBeNull()
  })

  it('is null when the page is newer than the source it read', () => {
    const root = page()
    const range = document.createRange()
    range.selectNodeContents(root.querySelector('pre')!)
    const where = selectionSource(range, root, 'code', text)!
    expect(selectionMarkdown(['short'], where)).toBeNull()
  })

  it('falls back to the whole block when the selection cannot be found in it', () => {
    const lines = ['- [ ] Pay rent @due(2026-01-01)']
    document.body.innerHTML =
      '<main id="wrapper"><ul><li data-mbr-line="1">Pay rent <span class="chip">Jan 1</span></li></ul></main>'
    const root = document.querySelector('main')!
    const li = root.querySelector('li')!
    const range = rangeOver(li.firstChild!, 4, li.querySelector('.chip')!.firstChild!, 3)
    const where = selectionSource(range, root, range.toString(), text)!
    expect(selectionMarkdown(lines, where)).toBe('- [ ] Pay rent @due(2026-01-01)')
  })
})
