/**
 * Tests for the shared anchored-card positioner.
 *
 * Every assertion goes through `positionRects`, which is why it exists as a
 * separate export: happy-dom reports `getBoundingClientRect()` as all-zeros, so
 * `positionAnchored` cannot be meaningfully exercised here. What *can* be
 * checked here is the one thing `positionAnchored` adds — that the card is
 * moved to a neutral origin before it is measured.
 */
import { describe, expect, it } from 'vitest'
import { GAP_PX, positionAnchored, positionRects, type Rect } from './anchored-popover.ts'

function rect(left: number, top: number, width: number, height: number): Rect {
  return { left, top, width, height, right: left + width, bottom: top + height }
}

const VW = 1000
const VH = 800

describe('positionRects', () => {
  it('centres the card horizontally on the anchor', () => {
    // Anchor spans 400..440, centre 420. A 100-wide card wants left = 370.
    const { left } = positionRects(rect(400, 300, 40, 20), rect(0, 0, 100, 50), VW, VH)
    expect(left).toBe(370)
  })

  it('prefers to sit above the anchor', () => {
    // Anchor top 300, card 50 tall => 300 - 50 - 8.
    const { top } = positionRects(rect(400, 300, 40, 20), rect(0, 0, 100, 50), VW, VH)
    expect(top).toBe(300 - 50 - GAP_PX)
  })

  it('flips below when there is no room above', () => {
    // Anchor at the very top: above would be negative, so use anchor.bottom.
    const anchor = rect(400, 4, 40, 20)
    const { top } = positionRects(anchor, rect(0, 0, 50, 50), VW, VH)
    expect(top).toBe(anchor.bottom + GAP_PX)
  })

  it('clamps to the left edge', () => {
    const { left } = positionRects(rect(0, 300, 10, 20), rect(0, 0, 200, 50), VW, VH)
    expect(left).toBe(GAP_PX)
  })

  it('clamps to the right edge', () => {
    const { left } = positionRects(rect(990, 300, 10, 20), rect(0, 0, 200, 50), VW, VH)
    expect(left).toBe(VW - 200 - GAP_PX)
  })

  it('clamps to the bottom edge when flipped below', () => {
    // Anchor near the bottom with nothing above it: flips below, then clamps.
    const { top } = positionRects(rect(400, 0, 40, 780), rect(0, 0, 100, 100), VW, VH)
    expect(top).toBe(VH - 100 - GAP_PX)
  })

  it('keeps a card larger than the viewport on screen rather than off it', () => {
    // The max-then-min order matters: min alone would yield a negative offset.
    const { left, top } = positionRects(
      rect(400, 300, 40, 20),
      rect(0, 0, VW + 400, VH + 400),
      VW,
      VH
    )
    expect(left).toBe(GAP_PX)
    expect(top).toBe(GAP_PX)
  })

  it('honours a caller-supplied gap', () => {
    const { top } = positionRects(rect(400, 300, 40, 20), rect(0, 0, 100, 50), VW, VH, 20)
    expect(top).toBe(300 - 50 - 20)
  })
})

describe('positionAnchored', () => {
  it('measures the card at a neutral origin before placing it', () => {
    // The bug this guards: measuring a card that is still positioned near an
    // edge reads a box max-width has not been applied to yet, so the card
    // creeps a few pixels every time it is shown.
    const anchor = document.createElement('span')
    const card = document.createElement('div')
    card.style.left = '900px'
    card.style.top = '700px'
    document.body.append(anchor, card)

    const seen: string[] = []
    card.getBoundingClientRect = () => {
      seen.push(`${card.style.left},${card.style.top}`)
      return rect(0, 0, 0, 0) as DOMRect
    }

    positionAnchored(anchor, card)

    expect(seen).toEqual(['0px,0px'])
    anchor.remove()
    card.remove()
  })
})
