import { describe, expect, it } from 'vitest'
import {
  appliesToPrint,
  declaredPageBox,
  marginLonghands,
  mbrSheetMatcher,
  pageRuleDeclarations,
  parsePageSize,
  type RuleLike,
  type SheetLike,
} from './page-box.ts'

const MM = 1 / 25.4
const rounded = (size: { widthIn: number; heightIn: number } | null) =>
  size && [Math.round(size.widthIn * 1000) / 1000, Math.round(size.heightIn * 1000) / 1000]

/** A stylesheet as the CSSOM shows it (happy-dom drops `@page`, so tests build them). */
function sheet(rules: RuleLike[], extra: Partial<SheetLike> = {}): SheetLike {
  return { href: null, ownerNode: null, cssRules: rules, ...extra }
}
const rule = (cssText: string): RuleLike => ({ cssText })
const media = (mediaText: string, rules: RuleLike[]): RuleLike => ({
  cssText: `@media ${mediaText} { … }`,
  media: { mediaText },
  cssRules: rules,
})
const none = () => false

describe('UNIT parsePageSize', () => {
  it.each([
    ['A5', [5.827, 8.268]],
    ['a4', [8.268, 11.693]],
    ['A4 landscape', [11.693, 8.268]],
    ['landscape letter', [11, 8.5]],
    ['letter portrait', [8.5, 11]],
    ['legal', [8.5, 14]],
    ['ledger', [11, 17]],
    ['JIS-B5', [Math.round(182 * MM * 1000) / 1000, Math.round(257 * MM * 1000) / 1000]],
    ['148mm 210mm', [5.827, 8.268]],
    ['8.5in 11in', [8.5, 11]],
    ['6in', [6, 6]],
    ['21cm 29.7cm', [8.268, 11.693]],
    ['612pt 792pt', [8.5, 11]],
    ['816px 1056px', [8.5, 11]],
    ['A5 !important', [5.827, 8.268]],
  ] as const)('%s', (value, expected) => {
    expect(rounded(parsePageSize(value))).toEqual(expected)
  })

  it.each([
    ['auto'],
    ['portrait'],
    ['landscape'],
    [''],
    ['tabloid'],
    ['A4 A5'],
    ['A4 sideways'],
    ['10in 5in 3in'],
    ['0.5in 11in'],
    ['300in 11in'],
    ['8.5 11'],
    ['8.5em 11em'],
    ['calc(1in + 1in)'],
  ])('ignores %j', (value) => {
    expect(parsePageSize(value)).toBeNull()
  })
})

describe('UNIT pageRuleDeclarations', () => {
  it('reads a plain @page rule', () => {
    expect(pageRuleDeclarations('@page { size: A5; margin: 20mm 1in; }')).toEqual(
      new Map([
        ['size', 'A5'],
        ['margin', '20mm 1in'],
      ])
    )
    expect(pageRuleDeclarations('@page{MARGIN-TOP:0}')).toEqual(new Map([['margin-top', '0']]))
  })

  it.each([['@page :first { margin-top: 0 }'], ['@page cover { size: A4 }'], ['body { margin: 0 }'], ['@page']])(
    'ignores %j',
    (text) => {
      expect(pageRuleDeclarations(text)).toBeNull()
    }
  )
})

describe('UNIT appliesToPrint', () => {
  it.each([
    [undefined, true],
    ['', true],
    ['print', true],
    ['all', true],
    ['only print', true],
    ['screen, print', true],
    ['print and (orientation: landscape)', true],
    ['(min-width: 0px)', true],
    ['not screen', true],
    ['screen', false],
    ['only screen and (min-width: 600px)', false],
    ['not print', false],
  ])('%j → %s', (mediaText, expected) => {
    expect(appliesToPrint(mediaText)).toBe(expected)
  })
})

describe('UNIT declaredPageBox', () => {
  it('finds size and margins in the page`s own sheets, later rules winning', () => {
    const box = declaredPageBox(
      [
        sheet([rule('@page { size: letter; margin-top: 1in }')]),
        sheet([rule('body { margin: 0 }'), rule('@page { size: A5 }')]),
      ],
      none
    )
    expect(rounded(box.size)).toEqual([5.827, 8.268])
    expect([...box.margins]).toEqual(['margin-top'])
  })

  it('treats the margin shorthand as all four sides', () => {
    const box = declaredPageBox([sheet([rule('@page { margin: 20mm }')])], none)
    expect(box.size).toBeNull()
    expect([...box.margins].sort()).toEqual(['margin-bottom', 'margin-left', 'margin-right', 'margin-top'])
  })

  it('reads inside @media print and @supports, never inside @media screen', () => {
    const supports: RuleLike = { cssText: '@supports (display: grid) { … }', cssRules: [rule('@page { size: A4 }')] }
    const box = declaredPageBox(
      [
        sheet([
          media('print', [rule('@page { margin: 1cm }')]),
          supports,
          media('screen', [rule('@page { size: legal; margin-left: 9in }')]),
        ]),
      ],
      none
    )
    expect(rounded(box.size)).toEqual([8.268, 11.693])
    expect(box.margins.size).toBe(4)
  })

  it('ignores :first and named pages, `size: auto`, screen-only sheets and unreadable ones', () => {
    const crossOrigin = {
      href: 'https://cdn.example.com/x.css',
      get cssRules(): ArrayLike<RuleLike> {
        throw new DOMException('SecurityError')
      },
    } as SheetLike
    const box = declaredPageBox(
      [
        sheet([rule('@page :first { margin-top: 0; size: A4 }'), rule('@page { size: auto }')]),
        sheet([rule('@page { margin: 0 }')], { media: { mediaText: 'screen' } }),
        crossOrigin,
      ],
      none
    )
    expect(box.size).toBeNull()
    expect(box.margins.size).toBe(0)
  })

  it('leaves mbr`s own sheets out: theme.css`s print `@page { margin: 0 }` is not the page`s', () => {
    const isMbr = mbrSheetMatcher('/.mbr/', 'http://127.0.0.1:5000/notes/a/', 'mbr-export-page')
    const theme = sheet([media('print', [rule('@page { margin: 0 }')])], { href: 'http://127.0.0.1:5000/.mbr/theme.css' })
    const exportPage = sheet([rule('@page { size: 8.5in 11in; margin: 1in }')], { ownerNode: { id: 'mbr-export-page' } })
    const user = sheet([rule('@page { margin: 2cm }')], { href: 'http://127.0.0.1:5000/.mbr/user.css' })
    expect(declaredPageBox([theme, exportPage], isMbr).margins.size).toBe(0)
    // user.css is the repository's own, so it counts.
    expect(declaredPageBox([theme, user], isMbr).margins.size).toBe(4)
  })

  it('matches mbr`s sheets at a static build`s relative asset base', () => {
    const isMbr = mbrSheetMatcher('../../.mbr/', 'https://example.com/site/a/b/', 'mbr-export-page')
    expect(isMbr(sheet([], { href: 'https://example.com/site/.mbr/theme.css' }))).toBe(true)
    expect(isMbr(sheet([], { href: 'https://example.com/site/.mbr/pico.min.css' }))).toBe(true)
    expect(isMbr(sheet([], { href: 'https://example.com/site/.mbr/invoice.css' }))).toBe(false)
    expect(isMbr(sheet([], { href: 'https://example.com/site/a/theme.css' }))).toBe(false)
  })
})

describe('UNIT marginLonghands', () => {
  it.each([
    ['1in', ['1in', '1in', '1in', '1in']],
    ['0.6in 0.5in', ['0.6in', '0.5in', '0.6in', '0.5in']],
    ['1in 2in 3in', ['1in', '2in', '3in', '2in']],
    ['1in 2in 3in 4in', ['1in', '2in', '3in', '4in']],
  ])('%s', (shorthand, [top, right, bottom, left]) => {
    expect(marginLonghands(shorthand)).toEqual({
      'margin-top': top,
      'margin-right': right,
      'margin-bottom': bottom,
      'margin-left': left,
    })
  })
})
