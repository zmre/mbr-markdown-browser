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
 * layout, so the overflow fixes are pinned as the rules that carry them; a
 * substring check alone could match a comment that merely mentions a rule.
 * The overflow itself is measured in a browser by
 * `components/scripts/check-overflow.sh` (docs/reference/development.md).
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
  // Formatter-proof: a value wrapped over several lines (`calc(\n  a * b\n)`)
  // reads the same as the one-line spelling.
  return match?.[1]
    .replace(/\s+/g, ' ')
    .replace(/\(\s+/g, '(')
    .replace(/\s+\)/g, ')')
    .trim()
}

const NARROW = 'screen and (max-width: 600px)'

describe('theme.css: nothing widens the page', () => {
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

  it("gives the docked sidebar layout an inline gutter for Pico's negative nav margins", () => {
    // Without it the grid's containers fill their column exactly, and the
    // header's last `<ul>` (Pico's `nav ul:last-child` margin) and the
    // marginalia dagger (`left: 100%`) hang past the viewport at 1280px.
    for (const child of ['header', 'nav.breadcrumbs', 'main', 'footer']) {
      const selector = `body.mbr-has-sidebar > ${child}`
      const gutter = rulesFor(selector).find((r) => declaration(r, 'padding-inline'))
      expect(gutter && declaration(gutter, 'padding-inline'), selector).toBe('var(--pico-spacing)')
      expect(gutter?.conditions, selector).toEqual(['@media screen and (min-width: 1024px)'])
    }
  })

  it('gives the breadcrumbs a row of their own in the docked sidebar grid', () => {
    // Every cell of the old three-row grid was named, so the auto-placed
    // breadcrumbs landed in an implicit row below the footer.
    const grid = rulesFor('body.mbr-has-sidebar').find((r) => declaration(r, 'grid-template-areas'))
    const areas = grid ? (declaration(grid, 'grid-template-areas') ?? '') : ''
    expect(areas.match(/"[^"]*"/g)).toEqual([
      '"sidebar header"',
      '"sidebar crumbs"',
      '"sidebar main"',
      '"sidebar footer"',
    ])
    expect(grid && declaration(grid, 'grid-template-rows')).toBe('auto auto 1fr auto')
    const crumbs = rulesFor('body.mbr-has-sidebar > nav.breadcrumbs').find((r) => declaration(r, 'grid-area'))
    expect(crumbs && declaration(crumbs, 'grid-area')).toBe('crumbs')
  })

  it('leaves room at the end of a docked layout\'s main for the review-note marker', () => {
    // The marker hangs at `left: 100%` of its block; a 1rem gutter clipped it.
    const gutter = rulesFor(':root').find((r) => declaration(r, '--mbr-review-marker-gutter'))
    expect(gutter && declaration(gutter, '--mbr-review-marker-gutter')).toBe('2rem')
    for (const selector of [
      'body.mbr-has-sidebar > main',
      'html:not(.mbr-companion-hidden) body.mbr-has-companion > main',
    ]) {
      const end = rulesFor(selector).find((r) => declaration(r, 'padding-inline-end'))
      expect(end && declaration(end, 'padding-inline-end'), selector).toBe(
        'calc(var(--pico-spacing) + var(--mbr-review-marker-gutter))'
      )
    }
  })

  it('reserves the docked companion column by padding, with the same gutter, only at its widths', () => {
    // The width rules only say that the column docks; the layout is written once.
    const widths = [
      { media: '@media screen and (min-width: 1280px)', body: 'body.mbr-has-companion:not(:has(> mbr-browse-single))' },
      { media: '@media screen and (min-width: 1440px)', body: 'body.mbr-has-companion' },
    ]
    for (const { media, body } of widths) {
      const docks = rulesFor(body).find((r) => declaration(r, '--mbr-companion-docked'))
      expect(docks && declaration(docks, '--mbr-companion-docked'), body).toBe('1')
      expect(docks?.conditions).toEqual([media])
    }
    const docked = 'html:not(.mbr-companion-hidden) body.mbr-has-companion'
    const reserve = rulesFor(docked).find((r) => declaration(r, 'padding-right'))
    expect(reserve && declaration(reserve, 'padding-right')).toBe('var(--mbr-inset-right)')
    expect(reserve && declaration(reserve, '--mbr-inset-right')).toBe('calc(var(--mbr-companion-docked, 0) * var(--mbr-companion-width))')
    expect(reserve?.conditions).toEqual([])
    const column = rulesFor(`${docked} > mbr-calendar`)[0]
    expect(column && declaration(column, 'display')).toBe('var(--mbr-companion-display, contents)')
    expect(column && declaration(column, 'position')).toBe('fixed')
    expect(column && declaration(column, 'width')).toBe('var(--mbr-companion-width)')
    for (const child of ['header', 'nav.breadcrumbs', 'main', 'footer']) {
      const gutter = rulesFor(`${docked} > ${child}`).find((r) => declaration(r, 'padding-inline'))
      expect(gutter && declaration(gutter, 'padding-inline'), child).toBe('var(--pico-spacing)')
      expect(gutter?.conditions).toEqual(['@media screen and (min-width: 1280px)'])
    }
  })


  it('keeps the marginalia dagger narrower than the 16px sidebar gutter', () => {
    const dagger = rulesFor('blockquote blockquote blockquote::before').find(
      (r) => r.conditions.length === 0 && declaration(r, 'padding')
    )
    const padding = /^0\s+(\d+)px$/.exec(dagger ? (declaration(dagger, 'padding') ?? '') : '')
    expect(padding).not.toBeNull()
    expect(Number(padding?.[1])).toBeLessThanOrEqual(3)
  })

  it('moves margin notes into the gutter only once a 250px note fits there', () => {
    // 2000px overflowed by 18px without the sidebar and 158px with it.
    expect(rules.some((r) => r.conditions.some((c) => c.includes('min-width: 2000px')))).toBe(false)
    const NO_SIDEBAR = ':where(body:not(.mbr-has-sidebar)) blockquote blockquote blockquote p'
    const narrow = rulesFor(NO_SIDEBAR)
    expect(narrow).toHaveLength(1)
    expect(narrow[0].conditions).toEqual(['@media screen and (min-width: 2080px)'])
    expect(declaration(narrow[0], 'max-width')).toBe('250px')
    const wide = rulesFor('blockquote blockquote blockquote p').find((r) =>
      r.conditions.includes('@media screen and (min-width: 2320px)')
    )
    expect(wide && declaration(wide, 'max-width')).toBe('250px')
    // `screen and` on both: a window-relative `width: calc(100vw - 40px)`
    // must never reach a printed page (WebKit evaluates width queries against
    // the window while printing).
    for (const rule of rules) {
      for (const c of rule.conditions) {
        if (/min-width: 2(080|320)px/.test(c)) expect(c).toMatch(/^@media screen and /)
      }
    }
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
    // A math fence's <div> scrolls too: before KaTeX draws it (and when it
    // cannot) it shows its source with `white-space: pre`.
    const fence = rulesFor('div.math-display')
    expect(fence.some((r) => declaration(r, 'overflow-x') === 'auto')).toBe(true)
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

describe('kanban print rules', () => {
  // Same specificity as the screen rules and `@media print` adds none, so the
  // print block wins only by coming later. It once sat earlier and lost, and a
  // printed board kept its 85vh columns and dropped cards.
  it('come after the screen rules they override', () => {
    const rules = parseRules(__MBR_THEME_CSS__)
    const lastIndex = (pred: (r: Rule) => boolean) =>
      rules.reduce((last, r, i) => (pred(r) ? i : last), -1)
    const isPrint = (r: Rule) => r.conditions.some((c) => /^@media\s+print/.test(c))
    const lastScreen = lastIndex(
      (r: Rule) => r.selector === '.kanban' && r.conditions.length === 0
    )
    const print = rules.findIndex(
      (r) => r.selector === '.kanban' && isPrint(r) && /height:\s*auto/.test(r.body)
    )
    expect(lastScreen).toBeGreaterThanOrEqual(0)
    expect(print).toBeGreaterThan(lastScreen)

    const lastScreenBreak = lastIndex(
      (r: Rule) => r.selector.includes('.kanban ul:last-child') && !isPrint(r)
    )
    const printBreak = rules.findIndex(
      (r) => r.selector.includes('.kanban ul:last-child') && isPrint(r)
    )
    expect(printBreak).toBeGreaterThan(lastScreenBreak)
  })
})

describe('kanban card layout', () => {
  const screen = (selector: string) => rulesFor(selector).find((r) => r.conditions.length === 0)

  it('makes a card the full width of its column, with no marker', () => {
    const card = screen('.kanban > ul > li')
    expect(card).toBeDefined()
    expect(declaration(card!, 'display')).toBe('block')
    expect(declaration(card!, 'list-style')).toBe('none')
    expect(declaration(card!, 'margin-inline')).toBe('0')
    const list = screen('.kanban > ul')
    expect(declaration(list!, 'padding-left')).toBe('0')
    expect(declaration(list!, 'margin-left')).toBe('0')
  })

  it('rules off only the first level of a card\'s contents', () => {
    const contents = screen('.kanban > :is(ul, ol) > li > :is(ul, ol)')
    expect(contents).toBeDefined()
    expect(declaration(contents!, 'border-top')).toBe('1px solid var(--pico-muted-border-color)')
    // A card with no contents has no nested list, so no rule; deeper levels
    // carry no border of their own.
    expect(declaration(screen('.kanban li :is(ul, ol)')!, 'border-top')).toBeUndefined()
  })

  // A task line was indented twice: its <li> kept the bullet's padding and the
  // checkbox then pushed the text one checkbox further in. Every content level
  // now hangs bullets and checkboxes in one gutter, the checkbox exactly as
  // wide (with its gap) as that gutter, so the text of both kinds of line
  // starts at the same place.
  it('hangs task checkboxes in the same gutter as bullets', () => {
    const gutter = declaration(screen('.kanban')!, '--mbr-kanban-gutter')
    expect(gutter).toBe('1.7em')
    expect(declaration(screen('.kanban li :is(ul, ol)')!, 'padding-inline-start')).toBe(
      'var(--mbr-kanban-gutter)'
    )
    const check = screen('.kanban li :is(ul, ol) > li > .mbr-task-check:first-child')
    expect(check).toBeDefined()
    expect(selectorsOf(check!)).toContain(
      '.kanban li :is(ul, ol) > li > p:first-child > .mbr-task-check:first-child'
    )
    expect(declaration(check!, 'font-size')).toBe('inherit')
    expect(declaration(check!, 'width')).toBe('1.25em')
    expect(declaration(check!, 'margin-inline')).toBe('calc(-1 * var(--mbr-kanban-gutter)) 0.45em')
    // 1.25em box + 0.45em gap = the 1.7em gutter.
    expect(1.25 + 0.45).toBeCloseTo(parseFloat(gutter!))
  })
})

describe('listing file cards', () => {
  it("closes the empty body between a file card's title and its footer", () => {
    const card = 'main#wrapper > section > article'
    const header = rulesFor(`${card} > header:has(+ footer)`)[0]
    expect(header && declaration(header, 'margin-bottom')).toBe('0')
    const title = rulesFor(`${card} > header:has(+ footer) > :last-child`)[0]
    expect(title && declaration(title, 'margin-bottom')).toBe('0')
    const footer = rulesFor(`${card} > header + footer`)[0]
    expect(footer && declaration(footer, 'margin-top')).toBe('0')
    expect(footer && declaration(footer, 'border-top')).toBe('0')
  })
})
