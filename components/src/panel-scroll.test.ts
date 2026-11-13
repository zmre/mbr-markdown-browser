import { describe, it, expect } from 'vitest'
import { panelScrollAmount, PANEL_SCROLL_LINE } from './panel-scroll.js'

function key(k: string, mods: Partial<Pick<KeyboardEvent, 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>> = {}) {
  return { key: k, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods }
}

describe('panelScrollAmount', () => {
  const PANE = 600

  it('scrolls a line on j / k and the arrows', () => {
    expect(panelScrollAmount(key('j'), PANE, false)).toBe(PANEL_SCROLL_LINE)
    expect(panelScrollAmount(key('ArrowDown'), PANE, false)).toBe(PANEL_SCROLL_LINE)
    expect(panelScrollAmount(key('k'), PANE, false)).toBe(-PANEL_SCROLL_LINE)
    expect(panelScrollAmount(key('ArrowUp'), PANE, false)).toBe(-PANEL_SCROLL_LINE)
  })

  it('scrolls half the pane on Ctrl+d / Ctrl+u', () => {
    expect(panelScrollAmount(key('d', { ctrlKey: true }), PANE, false)).toBe(300)
    expect(panelScrollAmount(key('u', { ctrlKey: true }), PANE, false)).toBe(-300)
  })

  it('scrolls a full pane less context on Ctrl+b, and Ctrl+f on macOS only', () => {
    expect(panelScrollAmount(key('b', { ctrlKey: true }), PANE, false)).toBe(-550)
    expect(panelScrollAmount(key('f', { ctrlKey: true }), PANE, true)).toBe(550)
    expect(panelScrollAmount(key('f', { ctrlKey: true }), PANE, false)).toBeNull()
  })

  it('ignores Cmd, Alt, Ctrl+Shift and unrelated keys', () => {
    expect(panelScrollAmount(key('d', { metaKey: true }), PANE, true)).toBeNull()
    expect(panelScrollAmount(key('j', { altKey: true }), PANE, false)).toBeNull()
    expect(panelScrollAmount(key('G', { ctrlKey: true, shiftKey: true }), PANE, false)).toBeNull()
    expect(panelScrollAmount(key('j', { ctrlKey: true }), PANE, false)).toBeNull()
    expect(panelScrollAmount(key('x'), PANE, false)).toBeNull()
  })
})
