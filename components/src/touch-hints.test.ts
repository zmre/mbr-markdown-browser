import { describe, it, expect } from 'vitest'
import tasksPanelSource from './tasks/mbr-tasks-panel.ts?raw'
import searchSource from './mbr-search.ts?raw'
import { DECK_CSS } from './flashcards/styles.ts'

/**
 * Keyboard-shortcut footers are hidden on touch screens without a keyboard
 * (`(hover: none) and (pointer: coarse)`), where they only take space; desktop
 * keeps them, and anything tappable stays. happy-dom evaluates no media
 * queries, so these pin the rules themselves.
 */
const TOUCH = '@media (hover: none) and (pointer: coarse)'

/** The body of the first `TOUCH` block in `css`. */
function touchBlock(css: string): string {
  const start = css.indexOf(TOUCH)
  expect(start, 'has a touch rule').toBeGreaterThan(-1)
  return css.slice(start, css.indexOf('}\n', css.indexOf('}', start) + 1) + 2)
}

describe('keyboard hints on touch screens', () => {
  it('hides the tasks overlay footer (hints only)', () => {
    expect(touchBlock(tasksPanelSource)).toMatch(/\.tasks-footer \{\s*display: none;/)
  })

  it('hides the search hints and the "=" key cap, but keeps the Browse Media button', () => {
    const block = touchBlock(searchSource)
    expect(block).toContain('.footer-hint')
    expect(block).toContain('.media-browser-button kbd')
    expect(block).not.toMatch(/\.media-browser-button\s*[,{]/)
  })

  it('hides the deck hint line', () => {
    expect(touchBlock(DECK_CSS)).toMatch(/\.mbr-fc-hints \{ display: none; \}/)
  })
})
