/**
 * The icon module: `icon-shapes.ts` must match its source of truth
 * (`crates/mbr-core/templates/icons/*.svg`), every icon must render as a real
 * `<svg>` that draws something, and theme.css's mask icons must draw the same
 * shapes as the files they name.
 */
import { render } from 'lit'
import { describe, expect, it } from 'vitest'
import { exportNameFor, renderIconShapesModule } from './icon-codegen.ts'
import * as shapes from './icon-shapes.ts'
import { icon, iconElement, iconMarkup, iconChevronRight, iconPlay, iconX } from './icons.ts'
import iconShapesSource from './icon-shapes.ts?raw'

declare const __MBR_ICON_SVGS__: Record<string, string>
declare const __MBR_THEME_CSS__: string

const files = __MBR_ICON_SVGS__
const allShapes = Object.entries(shapes) as [string, string][]

describe('icon-shapes.ts', () => {
  it('is up to date with crates/mbr-core/templates/icons (run `bun scripts/gen-icons.ts`)', () => {
    expect(iconShapesSource).toBe(renderIconShapesModule(files))
  })

  it('exports one constant per icon file', () => {
    expect(Object.keys(shapes).sort()).toEqual(Object.keys(files).map(exportNameFor).sort())
  })

  it('names exports from file names', () => {
    expect(exportNameFor('chevron-right.svg')).toBe('iconChevronRight')
    expect(exportNameFor('building-2.svg')).toBe('iconBuilding2')
  })

  it('refuses a file that is not one line of shapes', () => {
    expect(() => renderIconShapesModule({ 'bad.svg': '<svg><path d="M0 0"/></svg>' })).toThrow()
    expect(() => renderIconShapesModule({ 'bad.svg': '<path d="M0 0"/>\n<path d="M1 1"/>' })).toThrow()
  })
})

describe('iconElement', () => {
  it.each(allShapes)('%s builds an SVG that draws shapes', (_name, shape) => {
    const svg = iconElement(shape)
    expect(svg.namespaceURI).toBe('http://www.w3.org/2000/svg')
    expect(svg.children.length).toBeGreaterThan(0)
  })

  it('applies the one wrapper', () => {
    const svg = iconElement(iconX, { size: 16, className: 'close' })
    expect(svg.getAttribute('class')).toBe('mbr-icon close')
    expect(svg.getAttribute('viewBox')).toBe('0 0 24 24')
    expect(svg.getAttribute('width')).toBe('16')
    expect(svg.getAttribute('fill')).toBe('none')
    expect(svg.getAttribute('stroke')).toBe('currentColor')
    expect(svg.getAttribute('aria-hidden')).toBe('true')
    expect(svg.getAttribute('focusable')).toBe('false')
    expect(svg.hasAttribute('stroke-width')).toBe(false)
    expect(svg.getAttribute('style')).toContain('--mbr-icon-stroke')
  })

  it('returns a fresh node each time', () => {
    const host = document.createElement('span')
    host.append(iconElement(iconX), iconElement(iconX))
    expect(host.querySelectorAll('svg')).toHaveLength(2)
    expect(host.textContent).toBe('')
  })

  it('fills only when asked', () => {
    expect(iconElement(iconPlay, { filled: true }).getAttribute('fill')).toBe('currentColor')
  })
})

describe('icon (Lit)', () => {
  it('renders the same svg as iconMarkup', () => {
    const host = document.createElement('div')
    render(icon(iconChevronRight, { size: 16, className: 'chevron' }), host)
    const svg = host.querySelector('svg')!
    expect(svg.getAttribute('class')).toBe('mbr-icon chevron')
    expect(svg.getAttribute('width')).toBe('16')
    expect(svg.querySelector('path')?.getAttribute('d')).toBe('m9 18 6-6-6-6')
    expect(iconMarkup(iconChevronRight)).toContain('<path d="m9 18 6-6-6-6"/>')
  })
})

/**
 * theme.css draws a few icons as masks (alerts, task chips, the breadcrumb
 * divider). Each `--mbr-icon-<prop>` must carry exactly the shapes of the file
 * it is mapped to here, so swapping an `.svg` cannot leave CSS drawing the old
 * artwork.
 */
const MASKS: Record<string, string> = {
  note: 'info',
  tip: 'lightbulb',
  important: 'message-square-warning',
  warning: 'triangle-alert',
  caution: 'octagon-alert',
  calendar: 'calendar',
  check: 'check',
  'arrow-right': 'arrow-right',
  'chevron-right': 'chevron-right',
}

describe('theme.css icon masks', () => {
  it.each(Object.entries(MASKS))('--mbr-icon-%s draws %s.svg', (prop, file) => {
    const match = new RegExp(`--mbr-icon-${prop}:\\s*url\\("data:image/svg\\+xml,([^"]*)"\\)`).exec(__MBR_THEME_CSS__)
    expect(match, `--mbr-icon-${prop} missing`).not.toBeNull()
    const svg = match![1].replaceAll('%3C', '<').replaceAll('%3E', '>')
    const inner = svg.replace(/^<svg[^>]*>/, '').replace(/<\/svg>$/, '')
    expect(inner).toBe(files[`${file}.svg`].trim().replaceAll('"', "'"))
  })

  it('has no other --mbr-icon-* data URIs', () => {
    const props = [...__MBR_THEME_CSS__.matchAll(/--mbr-icon-([a-z-]+):\s*url\(/g)].map((m) => m[1])
    expect(props.sort()).toEqual(Object.keys(MASKS).sort())
  })
})
