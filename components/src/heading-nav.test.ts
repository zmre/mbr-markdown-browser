import { describe, it, expect } from 'vitest'
import { adjacentHeadingIndex, jumpStillInFlight, HEADING_SELECTOR } from './heading-nav.js'

describe('adjacentHeadingIndex', () => {
  // Landing line at 50px (the sticky header); tolerance 4px.
  const LANDING = 50

  it('picks the first heading below the landing line on next', () => {
    expect(adjacentHeadingIndex([-200, 20, 300, 800], LANDING, 'next')).toBe(2)
  })

  it('skips the heading already resting on the landing line', () => {
    // Sub-pixel scroll leaves it at 52, within tolerance: it is the current one.
    expect(adjacentHeadingIndex([-200, 52, 300], LANDING, 'next')).toBe(2)
    expect(adjacentHeadingIndex([-200, 48, 300], LANDING, 'prev')).toBe(0)
  })

  it('picks the last heading above the landing line on prev', () => {
    expect(adjacentHeadingIndex([-500, -200, 20, 300], LANDING, 'prev')).toBe(2)
  })

  it('returns -1 when there is nothing that way', () => {
    expect(adjacentHeadingIndex([100, 300], LANDING, 'prev')).toBe(-1)
    expect(adjacentHeadingIndex([-300, 50], LANDING, 'next')).toBe(-1)
    expect(adjacentHeadingIndex([], LANDING, 'next')).toBe(-1)
  })

  it('steps on from the anchor of a jump still in flight', () => {
    // Mid smooth-scroll toward heading 2 (still at 120): geometry alone would
    // pick heading 2 again; the anchor moves past it.
    const tops = [-400, -100, 120, 600, 900]
    expect(adjacentHeadingIndex(tops, LANDING, 'next')).toBe(2)
    expect(adjacentHeadingIndex(tops, LANDING, 'next', 2)).toBe(3)
    expect(adjacentHeadingIndex(tops, LANDING, 'prev', 2)).toBe(1)
  })

  it('steps on from the anchor at the end of a page that cannot scroll further', () => {
    // The last headings never reach the landing line: without the anchor, `]`
    // would choose heading 1 forever.
    const tops = [-50, 300, 500]
    expect(adjacentHeadingIndex(tops, LANDING, 'next', 1)).toBe(2)
    expect(adjacentHeadingIndex(tops, LANDING, 'next', 2)).toBe(-1)
  })

  it('ignores an anchor outside the list', () => {
    expect(adjacentHeadingIndex([-10, 300], LANDING, 'next', 7)).toBe(1)
  })
})

describe('jumpStillInFlight', () => {
  it('is true between where the jump started and where it aimed, either way', () => {
    expect(jumpStillInFlight(100, 0, 400)).toBe(true)
    expect(jumpStillInFlight(400, 0, 400)).toBe(true)
    expect(jumpStillInFlight(250, 400, 100)).toBe(true)
    expect(jumpStillInFlight(402, 0, 400)).toBe(true)
  })

  it('is false once the reader scrolled elsewhere', () => {
    expect(jumpStillInFlight(900, 0, 400)).toBe(false)
    expect(jumpStillInFlight(0, 100, 400)).toBe(false)
  })
})

describe('HEADING_SELECTOR', () => {
  it('matches h1–h6 in the note body only', () => {
    document.body.innerHTML = `
      <header><h1>Site</h1></header>
      <main id="wrapper"><h1>A</h1><p>x</p><h3>B</h3><section><h6>C</h6></section></main>`
    const found = Array.from(document.querySelectorAll(HEADING_SELECTOR)).map((h) => h.textContent)
    expect(found).toEqual(['A', 'B', 'C'])
    document.body.innerHTML = ''
  })
})
