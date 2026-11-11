import { describe, it, expect } from 'vitest'
import { hasMeaningfulContent } from './slot-content.js'

function nodesOf(markup: string): Node[] {
  const host = document.createElement('div')
  host.innerHTML = markup
  return [...host.childNodes]
}

describe('hasMeaningfulContent', () => {
  it('is false for nothing at all', () => {
    expect(hasMeaningfulContent([])).toBe(false)
  })

  it('is false for the compiled-in partial: a comment wrapped in whitespace', () => {
    expect(hasMeaningfulContent(nodesOf('\n  <!-- here just for user overrides -->\n'))).toBe(false)
  })

  it('is false for blank text only', () => {
    expect(hasMeaningfulContent(nodesOf(' \n\t '))).toBe(false)
  })

  it('is true for any element', () => {
    expect(hasMeaningfulContent(nodesOf('<!-- c --> <a href="/">Home</a>'))).toBe(true)
    expect(hasMeaningfulContent(nodesOf('<span></span>'))).toBe(true)
  })

  it('is true for bare text', () => {
    expect(hasMeaningfulContent(nodesOf('<!-- c --> © 2026 '))).toBe(true)
  })
})
