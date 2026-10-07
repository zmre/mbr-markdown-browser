import { afterEach, describe, expect, it } from 'vitest'
import { decorateAllHistories } from './dom.js'
import { PIE_CLASS, clearProgress, decorateProgress, pieCounts, pieElement, pieLabel } from './progress.js'
import { DECK_HTML, SECTIONED_DECK_HTML, installDeckPage } from './test-fixtures.js'

afterEach(() => {
  document.body.className = ''
  document.body.innerHTML = ''
})

const NOW = new Date(2026, 11, 1)

/** Every text node under `el`, joined — what find-in-page and copy would see. */
function textNodes(el: Node): string[] {
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
  const out: string[] = []
  while (walker.nextNode()) out.push(walker.currentNode.textContent ?? '')
  return out
}

const pieOf = (root: ParentNode, id: string) => root.querySelector(`#${id} > .${PIE_CLASS}`)
const slices = (pie: Element | null) => Array.from(pie?.querySelectorAll('path, circle') ?? [], (p) => p.getAttribute('class'))

describe('pie helpers', () => {
  it('counts latest ratings with unreviewed as its own bucket', () => {
    expect(pieCounts(['easy', null, 'again', 'easy'])).toEqual({ again: 1, hard: 0, good: 0, easy: 2, unreviewed: 1 })
  })

  it('labels best first, omits empty buckets, and counts cards', () => {
    expect(pieLabel({ again: 2, hard: 1, good: 2, easy: 4, unreviewed: 1 })).toBe(
      'Easy 40% · Good 20% · Hard 10% · Again 20% · Not reviewed 10% (10 cards)'
    )
    expect(pieLabel({ again: 0, hard: 0, good: 1, easy: 0, unreviewed: 0 })).toBe('Good 100% (1 card)')
  })

  it('draws slices in again, hard, good, easy, unreviewed order; a whole pie is a circle', () => {
    const pie = pieElement({ again: 1, hard: 1, good: 1, easy: 1, unreviewed: 1 })
    expect(slices(pie)).toEqual([
      'mbr-fc-pie-again',
      'mbr-fc-pie-hard',
      'mbr-fc-pie-good',
      'mbr-fc-pie-easy',
      'mbr-fc-pie-unreviewed',
    ])
    const whole = pieElement({ again: 0, hard: 0, good: 0, easy: 3, unreviewed: 0 })
    expect(whole.querySelector('circle')).not.toBeNull()
    expect(pie.getAttribute('role')).toBe('img')
    expect(pie.getAttribute('aria-label')).toBe(pie.title)
    expect(pie.querySelector('svg')!.getAttribute('aria-hidden')).toBe('true')
    expect(textNodes(pie)).toEqual([])
  })
})

describe('decorateProgress', () => {
  it('borders each reviewed question in its latest rating, with a title', () => {
    const root = installDeckPage(SECTIONED_DECK_HTML)
    decorateProgress(root, NOW)
    const dts = Array.from(root.querySelectorAll<HTMLElement>('dt'))
    expect(dts.map((dt) => Array.from(dt.classList).filter((c) => c.startsWith('mbr-fc-last-')))).toEqual([
      ['mbr-fc-last-easy'],
      [],
      ['mbr-fc-last-hard'],
      ['mbr-fc-last-good'],
      [],
    ])
    expect(dts[0].title).toMatch(/^Last review: Easy, Oct 9/)
    expect(dts[1].hasAttribute('title')).toBe(false)
  })

  it('puts a pie on every heading with cards, none on one without', () => {
    const root = installDeckPage(SECTIONED_DECK_HTML)
    decorateProgress(root, NOW)
    for (const id of ['deck', 'geo', 'rivers', 'math']) expect(pieOf(root, id)).not.toBeNull()
    expect(pieOf(root, 'empty')).toBeNull()
    expect(pieOf(root, 'geo')!.getAttribute('aria-label')).toBe('Easy 33% · Hard 33% · Not reviewed 33% (3 cards)')
    expect(pieOf(root, 'rivers')!.getAttribute('aria-label')).toBe('Hard 100% (1 card)')
    // Unreviewed is its own grey slice.
    expect(slices(pieOf(root, 'math'))).toEqual(['mbr-fc-pie-good', 'mbr-fc-pie-unreviewed'])
  })

  it('gives the H1 the whole note, even when the cards are not under it', () => {
    // A template-generated title: no id, and the cards sit outside it.
    const root = installDeckPage(DECK_HTML.replace('<main id="wrapper">', '<main id="wrapper"><h1>Deck</h1>'))
    decorateProgress(root, NOW)
    const pie = root.querySelector(`h1 > .${PIE_CLASS}`)!
    expect(pie.getAttribute('aria-label')).toBe('Easy 33% · Good 33% · Not reviewed 33% (3 cards)')
  })

  it('places the pie before the permalink and adds no text to the heading', () => {
    const root = installDeckPage(SECTIONED_DECK_HTML)
    const before = root.querySelectorAll('h1, h2, h3').length
    const textBefore = Array.from(root.querySelectorAll('h1, h2, h3'), (h) => h.textContent)
    decorateProgress(root, NOW)
    expect(root.querySelectorAll('h1, h2, h3')).toHaveLength(before)
    expect(Array.from(root.querySelectorAll('h1, h2, h3'), (h) => h.textContent)).toEqual(textBefore)
    const h1 = root.querySelector('h1')!
    expect(h1.lastElementChild!.classList.contains('mbr-heading-anchor')).toBe(true)
    expect(h1.lastElementChild!.previousElementSibling!.classList.contains(PIE_CLASS)).toBe(true)
  })

  it('is idempotent and follows new history', () => {
    const root = installDeckPage(SECTIONED_DECK_HTML)
    decorateAllHistories(root)
    decorateProgress(root, NOW)
    decorateProgress(root, NOW)
    expect(root.querySelectorAll(`.${PIE_CLASS}`)).toHaveLength(4)
    expect(root.querySelector('dt')!.className).toBe('mbr-fc-last-easy')
    // A new entry for France: the border and pies follow.
    root.querySelector('dd[data-mbr-line="7"] ul')!.insertAdjacentHTML('beforeend', '<li>2026-11-01 09:00 - Again</li>')
    decorateProgress(root, NOW)
    expect(root.querySelector('dt')!.className).toBe('mbr-fc-last-again')
    expect(root.querySelectorAll(`.${PIE_CLASS}`)).toHaveLength(4)
  })

  it('draws nothing on a note with no history, and clears what was there', () => {
    const root = installDeckPage(SECTIONED_DECK_HTML.replace(/<li[^>]*>\d{4}-[^<]*<\/li>/g, ''))
    decorateProgress(root, NOW)
    expect(root.querySelector(`.${PIE_CLASS}`)).toBeNull()
    expect(root.querySelector('[class*="mbr-fc-last-"]')).toBeNull()

    const full = installDeckPage(SECTIONED_DECK_HTML)
    decorateProgress(full, NOW)
    clearProgress(full)
    expect(full.querySelector(`.${PIE_CLASS}`)).toBeNull()
    expect(full.querySelector('dt[title]')).toBeNull()
  })

  it('never removes a title the author wrote', () => {
    const root = installDeckPage(SECTIONED_DECK_HTML.replace('<dt data-mbr-line="5">', '<dt data-mbr-line="5" title="mine">'))
    decorateProgress(root, NOW)
    clearProgress(root)
    expect(root.querySelector('dt')!.title).toBe('mine')
  })
})
