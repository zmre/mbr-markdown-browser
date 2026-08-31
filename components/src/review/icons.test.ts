/**
 * Tests for the note-type icons.
 *
 * These assert the **rendered element**, not stylesheet text. The icons went
 * through three failed CSS-mask implementations, and every string assertion
 * over `theme.css` passed against all three — a mask that renders and a mask
 * that silently renders nothing look identical as text. Only building the icon
 * and inspecting it can tell them apart.
 */
import { describe, expect, it } from 'vitest'
import { createIconSvg, ICON_SHAPES } from './icon-svg.ts'
import { TYPE_DEFS, type NoteType } from './types.ts'

const IDS = TYPE_DEFS.map((d) => d.id)

describe('createIconSvg', () => {
  it.each(IDS)('builds an <svg> for %s', (id) => {
    const svg = createIconSvg(id)
    expect(svg).not.toBeNull()
    expect(svg!.tagName.toLowerCase()).toBe('svg')
    expect(svg!.namespaceURI).toBe('http://www.w3.org/2000/svg')
  })

  it.each(IDS)('%s actually contains drawable shapes', (id) => {
    // The failure mode all three CSS attempts shared: an element that exists,
    // occupies space, and draws nothing.
    const svg = createIconSvg(id)!
    expect(svg.children.length).toBeGreaterThan(0)
    for (const shape of Array.from(svg.children)) {
      expect(['path', 'line', 'circle', 'polygon', 'polyline']).toContain(
        shape.tagName.toLowerCase()
      )
    }
  })

  it.each(IDS)('%s strokes with currentColor so CSS drives the colour', (id) => {
    const svg = createIconSvg(id)!
    expect(svg.getAttribute('stroke')).toBe('currentColor')
    expect(svg.getAttribute('fill')).toBe('none')
    expect(svg.getAttribute('viewBox')).toBe('0 0 24 24')
  })

  it.each(IDS)('%s is decorative, since the marker carries the label', (id) => {
    const svg = createIconSvg(id)!
    expect(svg.getAttribute('aria-hidden')).toBe('true')
    expect(svg.getAttribute('focusable')).toBe('false')
  })

  it('adds nothing to textContent, so a marker stays unselectable', () => {
    // The invariant that made the glyph generated content in the first place:
    // a marker must never join the paragraph's text run, or selecting a
    // paragraph would copy it and a note would quote its own marker.
    const host = document.createElement('span')
    host.appendChild(createIconSvg('issue')!)
    expect(host.textContent).toBe('')
  })

  it('returns a fresh node each time, not a shared one', () => {
    // Markers are rebuilt on every store change; handing out one cached node
    // would move the icon from the previous marker into the new one.
    const a = createIconSvg('note')!
    const b = createIconSvg('note')!
    expect(a).not.toBe(b)
    const host = document.createElement('span')
    host.appendChild(a)
    host.appendChild(b)
    expect(host.querySelectorAll('svg')).toHaveLength(2)
  })

  it('gives every type its own artwork', () => {
    const shapes = IDS.map((id) => ICON_SHAPES[id])
    expect(new Set(shapes).size).toBe(IDS.length)
  })

  it('covers exactly the six types, no more and no fewer', () => {
    expect(Object.keys(ICON_SHAPES).sort()).toEqual([...IDS].sort())
  })

  it('returns null for a type it does not know', () => {
    expect(createIconSvg('nonsense' as NoteType)).toBeNull()
  })
})
