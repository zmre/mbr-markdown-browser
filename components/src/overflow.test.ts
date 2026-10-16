import { describe, it, expect } from 'vitest'

// Read by vitest.config.ts (outside components/, and CSS imports are stubbed).
declare const __MBR_THEME_CSS__: string

interface Rule {
  /** The rule's selector list, whitespace collapsed. */
  selector: string
  /** Declarations, whitespace collapsed. */
  body: string
  /** Preludes of the at-rules it is nested in, outermost first. */
  conditions: string[]
}

/**
 * A just-enough CSS reader for theme.css: comments removed, then every style
 * rule with the `@media`/`@supports` blocks around it. happy-dom does no
 * layout, so the narrow-screen overflow fixes are pinned as the rules that
 * carry them; a substring check alone could match a comment that merely
 * mentions a rule.
 */
function parseRules(css: string): Rule[] {
  const source = css.replace(/\/\*[\s\S]*?\*\//g, '')
  const rules: Rule[] = []
  const stack: string[] = []
  let start = 0
  for (let i = 0; i < source.length; i++) {
    const ch = source[i]
    if (ch === '{') {
      const prelude = source.slice(start, i).trim().replace(/\s+/g, ' ')
      const close = source.indexOf('}', i)
      const nextOpen = source.indexOf('{', i + 1)
      if (prelude.startsWith('@') || (nextOpen !== -1 && nextOpen < close)) {
        stack.push(prelude)
        start = i + 1
      } else {
        rules.push({
          selector: prelude,
          body: source.slice(i + 1, close).trim().replace(/\s+/g, ' '),
          conditions: [...stack],
        })
        i = close
        start = close + 1
      }
    } else if (ch === '}') {
      stack.pop()
      start = i + 1
    }
  }
  return rules
}

const rules = parseRules(__MBR_THEME_CSS__)

/** The selector list split on top-level commas (not those inside `:is(…)`). */
function selectorsOf(rule: Rule): string[] {
  const parts: string[] = []
  let depth = 0
  let current = ''
  for (const ch of rule.selector) {
    if (ch === ',' && depth === 0) {
      parts.push(current.trim())
      current = ''
      continue
    }
    if (ch === '(') depth++
    else if (ch === ')') depth--
    current += ch
  }
  parts.push(current.trim())
  return parts
}

/** Every rule one of whose selectors is exactly `selector`. */
function rulesFor(selector: string): Rule[] {
  return rules.filter((rule) => selectorsOf(rule).includes(selector))
}

function declaration(rule: Rule, property: string): string | undefined {
  const match = new RegExp(`(?:^|;)\\s*${property}\\s*:\\s*([^;]+)`).exec(rule.body)
  return match?.[1].trim()
}

const NARROW = 'screen and (max-width: 600px)'

describe('theme.css narrow-screen overflow', () => {
  it('parses the stylesheet into rules (guards the reader itself)', () => {
    expect(rules.length).toBeGreaterThan(200)
    expect(rulesFor('.mbr-table-scroll').length).toBeGreaterThan(0)
  })

  it('makes the markdown table wrapper a horizontal scroll box on every screen', () => {
    const wrapper = rulesFor('.mbr-table-scroll').find((r) => r.conditions.length === 0)
    expect(wrapper).toBeDefined()
    expect(declaration(wrapper!, 'overflow-x')).toBe('auto')
    expect(declaration(wrapper!, 'max-width')).toBe('100%')
    // The block margin lives on the wrapper, not doubled inside it.
    expect(declaration(wrapper!, 'margin-bottom')).toBe('var(--pico-typography-spacing-vertical)')
    const inner = rulesFor('.mbr-table-scroll > table').find((r) => r.conditions.length === 0)
    expect(declaration(inner!, 'margin-bottom')).toBe('0')
    // And the table is never turned into a block: that is the VoiceOver cost
    // the wrapper exists to avoid.
    for (const rule of rulesFor('.mbr-table-scroll > table')) {
      expect(declaration(rule, 'display')).toBeUndefined()
    }
  })

  it('lets the docked sidebar layout shrink its content column below a wide table', () => {
    // A bare `1fr` has an `auto` minimum: the widest table sized the column
    // and the page scrolled sideways instead of the table's wrapper.
    const grid = rulesFor('body.mbr-has-sidebar').find((r) => declaration(r, 'grid-template-columns'))
    expect(grid && declaration(grid, 'grid-template-columns')).toBe('var(--mbr-sidebar-width) minmax(0, 1fr)')
  })

  it('lets a printed table run to full width', () => {
    const print = rulesFor('.mbr-table-scroll').find((r) => r.conditions.includes('@media print'))
    expect(print && declaration(print, 'overflow')).toBe('visible')
  })

  it('falls back to a block scroll box only for unwrapped tables on a phone', () => {
    // Never inside a playing slide deck: Reveal lays slides out at 960px and
    // scales them, so the phone's width says nothing about room on a slide.
    const fallback = rulesFor('main table:not(.mbr-table-scroll > table, .reveal table)')
    expect(fallback).toHaveLength(1)
    expect(fallback[0].conditions).toEqual([`@media ${NARROW}`])
    expect(declaration(fallback[0], 'display')).toBe('block')
    expect(declaration(fallback[0], 'overflow-x')).toBe('auto')
  })

  it('breaks long inline code and link text in the note, but not inside tables', () => {
    for (const selector of ['main :not(pre) > code', 'main a']) {
      const rule = rulesFor(selector).find((r) => declaration(r, 'overflow-wrap'))
      expect(rule && declaration(rule, 'overflow-wrap'), selector).toBe('anywhere')
    }
    const cells = rulesFor('main :is(td, th) :is(code, a)')
    expect(cells.some((r) => declaration(r, 'overflow-wrap') === 'normal')).toBe(true)
  })

  it('scrolls wide display math and caps media at the column width', () => {
    const katex = rulesFor('.katex-display')
    expect(katex.some((r) => declaration(r, 'overflow-x') === 'auto')).toBe(true)
    const media = rulesFor('main :where(img, video, iframe, embed, object)')
    expect(media.some((r) => declaration(r, 'max-width') === '100%')).toBe(true)
  })

  it('keeps marginalia on screen under 600px', () => {
    const narrow = rules.filter((r) => r.conditions.includes(`@media ${NARROW}`))
    const indicator = narrow.find((r) => r.selector === 'blockquote blockquote blockquote')
    expect(indicator && declaration(indicator, 'left')).toBe('auto')
    expect(indicator && declaration(indicator, 'right')).toBe('0')
    const note = narrow.find((r) => r.selector === 'blockquote blockquote blockquote p')
    expect(note && declaration(note, 'min-width')).toBe('0')
    expect(note && declaration(note, 'width')).toContain('100vw')
  })

  it('never hides body overflow (that breaks position: sticky)', () => {
    const bodyRules = rules.filter((r) => selectorsOf(r).some((s) => /(^|[\s>])body$/.test(s)))
    expect(bodyRules.length).toBeGreaterThan(0)
    for (const rule of bodyRules) {
      for (const property of ['overflow', 'overflow-x']) {
        expect(declaration(rule, property), `${rule.selector} { ${property} }`).not.toBe('hidden')
      }
    }
  })

  it('clips sideways overflow only in the app', () => {
    const clipping = rules.filter((r) => declaration(r, 'overflow-x') === 'clip')
    expect(clipping.map((r) => r.selector)).toEqual(['html.mbr-embedded body'])
    expect(clipping[0].conditions).toEqual([])
  })
})
