/**
 * The page's own `@page` box: whether the page (a custom template, a
 * stylesheet it links, `user.css`) declares the sheet size or margins itself.
 * What it declares wins over the export's `[pdf]` paper and margin
 * (docs/features/pdf-export.md, "Paper and margins set by the page"):
 *
 * - `prepare.ts` writes into `<style id="mbr-export-page">` only the
 *   descriptors the page leaves out;
 * - a declared size goes back to Rust in the ready message (`page`), which
 *   prints on that paper — WebKit on macOS ignores `@page { size }` and
 *   prints on the print system's paper (measured), so CSS alone cannot do it.
 *
 * mbr's own sheets do not count: theme.css's print rule is
 * `@page { margin: 0 }` (for the browser print panel), which would otherwise
 * read as "the page sets its margins" on every ordinary note.
 *
 * Reads the CSSOM by duck typing (`cssText`, `cssRules`, `media`), because
 * happy-dom drops `@page` rules entirely; the tests hand it plain objects.
 */

/** A sheet size in inches, as the page lays out (orientation applied). */
export interface PageSize {
  widthIn: number
  heightIn: number
}

/** The four margin longhands, in `margin` shorthand order. */
export const MARGIN_SIDES = ['margin-top', 'margin-right', 'margin-bottom', 'margin-left'] as const
export type MarginSide = (typeof MARGIN_SIDES)[number]

/** What the page's own plain `@page` rules declare. */
export interface DeclaredPageBox {
  /** The last declared `size` that is a real sheet (`auto` and nonsense are ignored). */
  size: PageSize | null
  /** Every margin side some rule declares (`margin` declares all four). */
  margins: Set<MarginSide>
}

/** The CSSOM shapes read here, loosely typed (see the module comment). */
export interface RuleLike {
  readonly cssText: string
  readonly cssRules?: ArrayLike<RuleLike>
  readonly media?: { readonly mediaText: string }
}
export interface SheetLike {
  readonly href: string | null
  readonly ownerNode?: { readonly id?: string } | null
  readonly media?: { readonly mediaText: string }
  readonly cssRules: ArrayLike<RuleLike>
}

const MM_PER_IN = 25.4

/** CSS Paged Media's `<page-size>` keywords, portrait, in inches. */
const NAMED_SIZES: Readonly<Record<string, readonly [number, number]>> = {
  a5: [148 / MM_PER_IN, 210 / MM_PER_IN],
  a4: [210 / MM_PER_IN, 297 / MM_PER_IN],
  a3: [297 / MM_PER_IN, 420 / MM_PER_IN],
  b5: [176 / MM_PER_IN, 250 / MM_PER_IN],
  b4: [250 / MM_PER_IN, 353 / MM_PER_IN],
  'jis-b5': [182 / MM_PER_IN, 257 / MM_PER_IN],
  'jis-b4': [257 / MM_PER_IN, 364 / MM_PER_IN],
  letter: [8.5, 11],
  legal: [8.5, 14],
  ledger: [11, 17],
}

/** Inches per CSS absolute length unit. */
const UNIT_IN: Readonly<Record<string, number>> = {
  in: 1,
  cm: 1 / 2.54,
  mm: 1 / MM_PER_IN,
  q: 1 / (4 * MM_PER_IN),
  pt: 1 / 72,
  pc: 1 / 6,
  px: 1 / 96,
}

/** The same bounds Rust puts on a paper (`pdf_config.rs`): 1–200 inches a side. */
const MIN_SIDE_IN = 1
const MAX_SIDE_IN = 200

function lengthIn(token: string): number | null {
  const match = /^(\d+(?:\.\d+)?|\.\d+)([a-z]+)$/.exec(token)
  if (!match) return null
  const unit = UNIT_IN[match[2]]
  return unit === undefined ? null : Number(match[1]) * unit
}

const inRange = (side: number) => Number.isFinite(side) && side >= MIN_SIDE_IN && side <= MAX_SIDE_IN

/**
 * A `size` descriptor's value as a sheet in inches, or `null` for `auto`,
 * anything mbr cannot print on, and anything malformed. Pure, for the tests.
 *
 * `A5`, `A4 landscape`, `landscape letter`, `portrait`/`landscape` alone
 * (`null`: they keep the configured paper, which `[pdf] orientation` turns),
 * one length (a square), two lengths (width height).
 */
export function parsePageSize(value: string): PageSize | null {
  const tokens = value.trim().toLowerCase().replace(/\s*!important$/, '').split(/\s+/).filter(Boolean)
  if (tokens.length === 0 || tokens.length > 2) return null
  const lengths = tokens.map(lengthIn)
  let width: number
  let height: number
  if (lengths.every((l) => l !== null)) {
    width = lengths[0] as number
    height = (lengths[1] ?? lengths[0]) as number
  } else {
    const names = tokens.filter((t) => t in NAMED_SIZES)
    const turns = tokens.filter((t) => t === 'landscape' || t === 'portrait')
    if (names.length !== 1 || names.length + turns.length !== tokens.length) return null
    const [w, h] = NAMED_SIZES[names[0]]
    ;[width, height] = turns[0] === 'landscape' ? [h, w] : [w, h]
  }
  return inRange(width) && inRange(height) ? { widthIn: width, heightIn: height } : null
}

/**
 * The declarations of a plain `@page` rule's text (`@page { a: b; c: d }`),
 * lower-cased names to values; `null` for anything else, including the
 * `:first`/`:left`/named page selectors, which never change the sheet.
 */
export function pageRuleDeclarations(cssText: string): Map<string, string> | null {
  const open = cssText.indexOf('{')
  const close = cssText.lastIndexOf('}')
  if (open < 0 || close < open) return null
  if (cssText.slice(0, open).trim().toLowerCase() !== '@page') return null
  const declarations = new Map<string, string>()
  for (const part of cssText.slice(open + 1, close).split(';')) {
    const colon = part.indexOf(':')
    if (colon < 0) continue
    const name = part.slice(0, colon).trim().toLowerCase()
    const value = part.slice(colon + 1).trim()
    if (name && value) declarations.set(name, value)
  }
  return declarations
}

/** Whether a media list applies on paper (`print`, `all`, or none given). */
export function appliesToPrint(mediaText: string | undefined): boolean {
  const text = (mediaText ?? '').trim().toLowerCase()
  if (text === '') return true
  return text.split(',').some((query) => {
    const q = query.trim()
    if (q.startsWith('not ')) return !/\bprint\b/.test(q)
    return q === '' || /^(only\s+)?(print|all)\b/.test(q) || q.startsWith('(')
  })
}

/** Folds one rule list (recursing into `@media print`, `@supports`, `@layer`). */
function collect(rules: ArrayLike<RuleLike>, box: DeclaredPageBox): void {
  for (const rule of Array.from(rules)) {
    const declarations = pageRuleDeclarations(rule.cssText)
    if (declarations) {
      const size = declarations.get('size')
      if (size !== undefined) {
        const parsed = parsePageSize(size)
        if (parsed) box.size = parsed
      }
      for (const [name] of declarations) {
        if (name === 'margin') MARGIN_SIDES.forEach((side) => box.margins.add(side))
        else if ((MARGIN_SIDES as readonly string[]).includes(name)) box.margins.add(name as MarginSide)
      }
    } else if (rule.cssRules && (rule.media === undefined || appliesToPrint(rule.media.mediaText))) {
      collect(rule.cssRules, box)
    }
  }
}

/**
 * Every plain `@page` rule in `sheets` that is not one of mbr's own
 * (`isMbrSheet`), in document order: later rules win, as in the cascade.
 * Sheets whose rules cannot be read (cross-origin) are skipped.
 */
export function declaredPageBox(sheets: ArrayLike<SheetLike>, isMbrSheet: (sheet: SheetLike) => boolean): DeclaredPageBox {
  const box: DeclaredPageBox = { size: null, margins: new Set() }
  for (const sheet of Array.from(sheets)) {
    if (isMbrSheet(sheet) || !appliesToPrint(sheet.media?.mediaText)) continue
    let rules: ArrayLike<RuleLike>
    try {
      rules = sheet.cssRules
    } catch {
      continue
    }
    collect(rules, box)
  }
  return box
}

/** mbr's stylesheets under `/.mbr/` whose `@page` rules are mbr's, not the page's. */
const MBR_SHEETS = ['pico.min.css', 'theme.css', 'export.css']

/**
 * Whether `sheet` is mbr's: theme.css, pico.min.css or export.css at
 * `assetBase` (resolved against `baseUrl`), or the export's own
 * `<style id="mbr-export-page">`. `user.css` is the page's.
 */
export function mbrSheetMatcher(assetBase: string, baseUrl: string, pageStyleId: string): (sheet: SheetLike) => boolean {
  const own = new Set(
    MBR_SHEETS.map((name) => {
      try {
        return new URL(`${assetBase}${name}`, baseUrl).href
      } catch {
        return ''
      }
    })
  )
  return (sheet) => sheet.ownerNode?.id === pageStyleId || (sheet.href !== null && own.has(sheet.href))
}

/**
 * A `margin` shorthand (1–4 values, already validated) as its four longhands.
 * Pure, for the tests.
 */
export function marginLonghands(shorthand: string): Record<MarginSide, string> {
  const v = shorthand.trim().split(/\s+/)
  const [top, right = top, bottom = top, left = right] = v
  return { 'margin-top': top, 'margin-right': right, 'margin-bottom': bottom, 'margin-left': left }
}
