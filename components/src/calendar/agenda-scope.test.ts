import { describe, expect, it } from 'vitest'
import { AGENDA_PREFILTER, mayHaveAgenda, topLevelListItems } from './agenda-scope.ts'
import { isTopLevel, MARKDOWN_NESTING_SELECTOR } from '../top-level.ts'

function main(html: string): HTMLElement {
  const el = document.createElement('main')
  el.innerHTML = html
  return el
}

describe('topLevelListItems', () => {
  it('takes the items of lists not nested in a markdown or HTML container', () => {
    const root = main(`
<ul><li>a<ul><li>nested</li></ul></li><li>b</li></ul>
<section><ol><li>c</li></ol></section>
<blockquote><ul><li>quoted</li></ul></blockquote>
<aside><ul><li>aside</li></ul></aside>
<div class="footnote-definition"><ul><li>footnote</li></ul></div>`)
    expect(topLevelListItems(root).map((li) => li.firstChild?.textContent)).toEqual(['a', 'b', 'c'])
  })
})

describe('isTopLevel', () => {
  it('shares the markdown nesting selector with flashcards', () => {
    const root = main('<dl id="top"></dl><blockquote><dl id="quoted"></dl></blockquote>')
    expect(isTopLevel(root.querySelector('#top')!)).toBe(true)
    expect(isTopLevel(root.querySelector('#quoted')!)).toBe(false)
    expect(MARKDOWN_NESTING_SELECTOR).toContain('.mbr-chat')
  })
})

describe('mayHaveAgenda (the trigger prefilter)', () => {
  it('spots a time-like top-level bullet, plain or task', () => {
    expect(mayHaveAgenda(main('<ul><li>09:00 - 09:15 Standup</li></ul>'))).toBe(true)
    expect(
      mayHaveAgenda(
        main('<ul><li><input class="mbr-task-check" type="checkbox"> <span class="mbr-task-text">9am Run</span></li></ul>')
      )
    ).toBe(true)
    expect(mayHaveAgenda(main('<ul><li>\n<p>noon Lunch</p>\n</li></ul>'))).toBe(true)
  })

  it('stays quiet on ordinary notes', () => {
    expect(mayHaveAgenda(main('<p>09:00 in prose</p><ul><li>2026 goals</li><li>10 reasons</li></ul>'))).toBe(false)
    expect(mayHaveAgenda(main('<ul><li>Projects<ul><li>09:00 nested</li></ul></li></ul>'))).toBe(false)
  })

  it('is cheap regardless of how long an item is', () => {
    const long = '<li>Outline<ul>' + '<li>x</li>'.repeat(2000) + '</ul></li>'
    expect(mayHaveAgenda(main(`<ul>${long}</ul>`))).toBe(false)
  })

  it('matches forms the spec draft missed', () => {
    expect(AGENDA_PREFILTER.test('9:00am Standup')).toBe(true)
    expect(AGENDA_PREFILTER.test('9  am Standup')).toBe(true)
  })
})
