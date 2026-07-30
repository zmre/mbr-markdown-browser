/**
 * Benchmarks for the find-in-page hot path.
 *
 * `buildTextIndex` runs once per open (and once per coalesced content change);
 * `findMatchOffsets` runs once per settled keystroke. Both must stay well under
 * a frame on the kind of document mbr is pointed at, which is why the sizes go
 * up to a million characters — mbr is used on repositories with tens of
 * thousands of files and some of them are very long.
 *
 * `rangeForMatch` is deliberately not benchmarked: happy-dom's `Range` is
 * orders of magnitude slower than a real engine's, so the number would measure
 * happy-dom rather than the binary search it is meant to cover.
 */

import { bench, describe } from 'vitest'
import { buildTextIndex, compileQuery, findMatchOffsets } from './find-in-page'

const WORDS = [
  'markdown', 'browser', 'render', 'template', 'section', 'anchor', 'wikilink',
  'frontmatter', 'genealogy', 'relationship', 'transcode', 'oembed', 'needle',
]

/**
 * A document of roughly `chars` characters, shaped like rendered markdown:
 * headings, paragraphs with inline elements, list items and a code block, plus
 * the pruned nodes every real page carries (`.sr-only` title, heading
 * permalinks, a KaTeX double-encoding, an SVG diagram).
 */
function generateDocument(chars: number): HTMLElement {
  const parts: string[] = ['<span class="sr-only" data-pagefind-weight="10">Benchmark Page</span>']
  let length = 0
  let n = 0
  while (length < chars) {
    const word = (i: number) => WORDS[(n + i) % WORDS.length]
    const block = n % 8 === 0
      ? `<h2 id="h${n}">Section ${n} ${word(1)}<a class="mbr-heading-anchor" href="#h${n}">#</a></h2>`
      : n % 8 === 3
        ? `<ul><li>${word(1)} ${word(2)}</li><li>${word(3)} <code>${word(4)}</code></li></ul>`
        : n % 8 === 5
          ? `<pre><code>let ${word(1)} = ${word(2)}(${word(3)});</code></pre>`
          : `<p>${word(1)} ${word(2)} <em>${word(3)}</em> ${word(4)}\n${word(5)} ` +
            `<strong>${word(6)}</strong> ${word(7)} ${word(8)} ${word(9)} ${word(10)}.</p>`
    parts.push(block)
    length += block.length
    n++
  }
  parts.push('<span class="katex"><span class="katex-mathml">x^2</span><span class="katex-html">x2</span></span>')
  parts.push('<svg><text>diagram label</text></svg>')

  const wrapper = document.createElement('main')
  wrapper.id = 'wrapper'
  wrapper.innerHTML = parts.join('')
  document.body.appendChild(wrapper)
  return wrapper
}

const SIZES = [10_000, 100_000, 1_000_000]

describe('buildTextIndex', () => {
  for (const size of SIZES) {
    const root = generateDocument(size)
    bench(`index a ${size.toLocaleString('en-US')} char document`, () => {
      buildTextIndex(root)
    })
  }
})

describe('findMatchOffsets', () => {
  for (const size of SIZES) {
    const index = buildTextIndex(generateDocument(size))

    // Rare single word: the common case, a handful of hits.
    bench(`scan ${size.toLocaleString('en-US')} chars, rare term`, () => {
      findMatchOffsets(index, compileQuery('genealogy', false)!, 10_000)
    })

    // Two words with a flexible separator, which is the expensive pattern.
    bench(`scan ${size.toLocaleString('en-US')} chars, two-word phrase`, () => {
      findMatchOffsets(index, compileQuery('markdown browser', false)!, 10_000)
    })

    // Pathological: a single letter matching a large fraction of the document.
    bench(`scan ${size.toLocaleString('en-US')} chars, one letter`, () => {
      findMatchOffsets(index, compileQuery('e', false)!, 10_000)
    })
  }
})
