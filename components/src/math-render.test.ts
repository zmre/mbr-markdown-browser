import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import {
  type AsciiMathConverter,
  type KatexLike,
  ASCIIMATH_CLASS,
  MATH_STATE_ATTR,
  createAsciiMathConverter,
  hasAsciiMath,
  pendingMathElements,
  renderMathElements,
  texFor,
} from './math-render.ts'

/** The vendored `asciimath2tex.<version>.min.js`, read by vitest.config.ts. */
declare const __MBR_ASCIIMATH2TEX_JS__: string

/**
 * Markup exactly as `html.rs` writes it: inline and `$$` math are spans, a
 * math fence is a div, and an AsciiMath fence also carries `math-asciimath`.
 */
const PAGE = `
<main id="wrapper">
  <p>Inline <span class="math math-inline">x^2</span> and
  <span class="math math-display">\\int_0^1 x</span></p>
  <div class="math math-display" data-mbr-line="5">\\begin{aligned}
a &amp;= b \\\\
- c &amp;= d
\\end{aligned}
</div>
  <div class="math math-display math-asciimath">sum_(i=1)^n i
</div>
  <pre><code class="language-tex">\\documentclass{article}</code></pre>
</main>`

/** A KaTeX stand-in that records calls and writes a marker the tests can see. */
function fakeKatex(): KatexLike & { calls: Array<{ tex: string; displayMode: boolean }> } {
  const calls: Array<{ tex: string; displayMode: boolean }> = []
  return {
    calls,
    render(tex, element, options) {
      calls.push({ tex, displayMode: options.displayMode })
      element.innerHTML = `<span class="katex">${tex}</span>`
    },
  }
}

/** Runs the vendored UMD build as a CommonJS module and returns its export. */
function loadVendoredParserClass(): new () => AsciiMathConverter {
  const module: { exports: unknown } = { exports: {} }
  new Function('module', 'exports', __MBR_ASCIIMATH2TEX_JS__)(module, module.exports)
  return module.exports as new () => AsciiMathConverter
}

describe('math-render', () => {
  beforeEach(() => {
    document.body.innerHTML = PAGE
  })

  afterEach(() => {
    document.body.innerHTML = ''
    delete (window as unknown as { AsciiMathParser?: unknown }).AsciiMathParser
    vi.restoreAllMocks()
  })

  describe('selection', () => {
    it('finds spans and fenced divs, never highlighted code', () => {
      const els = pendingMathElements()
      expect(els.map((el) => el.tagName)).toEqual(['SPAN', 'SPAN', 'DIV', 'DIV'])
      expect(els.some((el) => el.closest('pre'))).toBe(false)
    })

    it('skips elements already handled', () => {
      const first = pendingMathElements()[0]
      first.setAttribute(MATH_STATE_ATTR, 'rendered')
      expect(pendingMathElements()).not.toContain(first)
      expect(pendingMathElements()).toHaveLength(3)
    })

    it('needs the AsciiMath converter only when an AsciiMath block exists', () => {
      expect(hasAsciiMath(pendingMathElements())).toBe(true)
      document.querySelector(`.${ASCIIMATH_CLASS}`)?.remove()
      expect(hasAsciiMath(pendingMathElements())).toBe(false)
    })
  })

  describe('texFor', () => {
    it('gives a TeX element its trimmed, unescaped text, blank lines and "- " lines included', () => {
      const fence = document.querySelector('div.math-display:not(.math-asciimath)')!
      expect(texFor(fence, null)).toBe('\\begin{aligned}\na &= b \\\\\n- c &= d\n\\end{aligned}')
    })

    it('converts AsciiMath through the converter', () => {
      const block = document.querySelector(`.${ASCIIMATH_CLASS}`)!
      const parse = vi.fn((s: string) => `TEX(${s})`)
      expect(texFor(block, { parse })).toBe('TEX(sum_(i=1)^n i)')
      expect(parse).toHaveBeenCalledOnce()
    })

    it('returns null for AsciiMath without a converter, or when it throws', () => {
      const block = document.querySelector(`.${ASCIIMATH_CLASS}`)!
      expect(texFor(block, null)).toBeNull()
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      const throwing: AsciiMathConverter = {
        parse: () => {
          throw new Error('bad input')
        },
      }
      expect(texFor(block, throwing)).toBeNull()
      expect(warn).toHaveBeenCalled()
    })

    it('returns null for an empty element', () => {
      const empty = document.createElement('div')
      empty.className = 'math math-display'
      expect(texFor(empty, null)).toBeNull()
    })
  })

  describe('renderMathElements', () => {
    it('renders every element once with the right display mode', () => {
      const katex = fakeKatex()
      const converter: AsciiMathConverter = { parse: (s) => `TEX(${s})` }
      const els = pendingMathElements()

      expect(renderMathElements(els, katex, converter)).toBe(4)
      expect(katex.calls.map((c) => c.displayMode)).toEqual([false, true, true, true])
      expect(katex.calls[3].tex).toBe('TEX(sum_(i=1)^n i)')
      expect(els.every((el) => el.getAttribute(MATH_STATE_ATTR) === 'rendered')).toBe(true)

      // A second pass (another <mbr-katex>, a re-scan) must not read KaTeX's
      // own output back as TeX.
      expect(renderMathElements(els, katex, converter)).toBe(0)
      expect(pendingMathElements()).toHaveLength(0)
      expect(katex.calls).toHaveLength(4)
    })

    it('leaves AsciiMath source visible and warns when the converter is unavailable', () => {
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      const katex = fakeKatex()
      const block = document.querySelector(`.${ASCIIMATH_CLASS}`)!

      expect(renderMathElements(pendingMathElements(), katex, null)).toBe(3)
      expect(block.textContent?.trim()).toBe('sum_(i=1)^n i')
      expect(block.getAttribute(MATH_STATE_ATTR)).toBe('source')
      expect(warn).toHaveBeenCalledOnce()
    })

    it('keeps going when KaTeX throws on one element', () => {
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      let n = 0
      const katex: KatexLike = {
        render: () => {
          if (n++ === 0) throw new Error('boom')
        },
      }
      expect(renderMathElements(pendingMathElements(), katex, { parse: (s) => s })).toBe(3)
      expect(warn).toHaveBeenCalledOnce()
    })
  })

  describe('createAsciiMathConverter', () => {
    it('returns null and warns when the global is missing', () => {
      const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
      expect(createAsciiMathConverter(window)).toBeNull()
      expect(warn).toHaveBeenCalledOnce()
    })

    it('uses the global the vendored UMD build defines in a browser', () => {
      // As a classic <script>: `this` is the window and there is no `module`.
      new Function(__MBR_ASCIIMATH2TEX_JS__).call(window)
      const converter = createAsciiMathConverter(window)
      expect(converter).not.toBeNull()
      expect(converter!.parse('x^2')).toBe('x^{2}')
    })
  })

  describe('vendored asciimath2tex', () => {
    it('converts typical AsciiMath to TeX KaTeX accepts', () => {
      const Parser = loadVendoredParserClass()
      const parser = new Parser()
      expect(parser.parse('sum_(i=1)^n i^3=((n(n+1))/2)^2')).toBe(
        '\\sum_{i = 1}^{n} i^{3} = \\left ( \\frac{n \\left ( n + 1 \\right )}{2} \\right )^{2}'
      )
      expect(parser.parse('[[a,b],[c,d]]')).toBe('\\left [ \\begin{matrix} a & b \\\\ c & d \\end{matrix} \\right ]')
      // A newline is whitespace: a block is one expression.
      expect(parser.parse('a = b\nc = d')).toBe('a = b c = d')
    })

    it('feeds the converted TeX to KaTeX end to end', () => {
      const Parser = loadVendoredParserClass()
      const katex = fakeKatex()
      const block = document.querySelector(`.${ASCIIMATH_CLASS}`)!
      renderMathElements([block], katex, new Parser())
      expect(katex.calls).toEqual([{ tex: '\\sum_{i = 1}^{n} i', displayMode: true }])
    })
  })
})
