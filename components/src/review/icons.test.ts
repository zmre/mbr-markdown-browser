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
import { NOTE_TYPE_ICONS, TYPE_DEFS, noteTypeIcon, type NoteType } from './types.ts'

const IDS = TYPE_DEFS.map((d) => d.id)

describe('noteTypeIcon', () => {
  it.each(IDS)('builds an <svg> for %s', (id) => {
    const svg = noteTypeIcon(id)
    expect(svg).not.toBeNull()
    expect(svg!.tagName.toLowerCase()).toBe('svg')
    expect(svg!.namespaceURI).toBe('http://www.w3.org/2000/svg')
  })

  it.each(IDS)('%s actually contains drawable shapes', (id) => {
    // The failure mode all three CSS attempts shared: an element that exists,
    // occupies space, and draws nothing.
    const svg = noteTypeIcon(id)!
    expect(svg.children.length).toBeGreaterThan(0)
    for (const shape of Array.from(svg.children)) {
      expect(['path', 'line', 'circle', 'polygon', 'polyline', 'rect']).toContain(shape.tagName.toLowerCase())
    }
  })

  it.each(IDS)('%s strokes with currentColor so CSS drives the colour', (id) => {
    const svg = noteTypeIcon(id)!
    expect(svg.getAttribute('stroke')).toBe('currentColor')
    expect(svg.getAttribute('fill')).toBe('none')
    expect(svg.getAttribute('viewBox')).toBe('0 0 24 24')
  })

  it.each(IDS)('%s is decorative, since the marker carries the label', (id) => {
    const svg = noteTypeIcon(id)!
    expect(svg.getAttribute('aria-hidden')).toBe('true')
    expect(svg.getAttribute('focusable')).toBe('false')
  })

  it('adds nothing to textContent, so a marker stays unselectable', () => {
    const host = document.createElement('span')
    host.appendChild(noteTypeIcon('issue')!)
    expect(host.textContent).toBe('')
  })

  it('returns a fresh node each time, not a shared one', () => {
    // Markers are rebuilt on every store change; handing out one cached node
    // would move the icon from the previous marker into the new one.
    const a = noteTypeIcon('note')!
    const b = noteTypeIcon('note')!
    expect(a).not.toBe(b)
    const host = document.createElement('span')
    host.appendChild(a)
    host.appendChild(b)
    expect(host.querySelectorAll('svg')).toHaveLength(2)
  })

  it('gives every type its own artwork', () => {
    const shapes = IDS.map((id) => NOTE_TYPE_ICONS[id])
    expect(new Set(shapes).size).toBe(IDS.length)
  })

  it('covers exactly the six types, no more and no fewer', () => {
    expect(Object.keys(NOTE_TYPE_ICONS).sort()).toEqual([...IDS].sort())
  })

  it('returns null for a type it does not know', () => {
    expect(noteTypeIcon('nonsense' as NoteType)).toBeNull()
  })
})
