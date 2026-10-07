import { afterEach, describe, expect, it } from 'vitest'
import {
  HISTORY_CLASS,
  appendHistoryEntry,
  cardSections,
  decorateAllHistories,
  decorateHistory,
  deckCards,
  deckLists,
  hasDeck,
  headingText,
  historyEntriesOf,
  isHistoryDefinition,
  shiftSourceLines,
} from './dom.js'
import { DECK_HTML, SECTIONED_DECK_HTML, installDeckPage } from './test-fixtures.js'
import { DECK_CSS } from './styles.js'

afterEach(() => {
  document.body.className = ''
  document.body.innerHTML = ''
})

describe('deck discovery', () => {
  it('collects top-level lists only', () => {
    const root = installDeckPage()
    expect(deckLists(root)).toHaveLength(2)
    expect(hasDeck(root)).toBe(true)
    root.innerHTML = '<blockquote><dl><dt>x</dt><dd>y</dd></dl></blockquote>'
    expect(hasDeck(root)).toBe(false)
  })

  it('ignores definition lists inside a chat block', () => {
    // The body is rendered markdown, but in the source it sits inside a
    // ```chat fence, which src/flashcards.rs refuses to write to.
    const root = document.createElement('main')
    root.innerHTML =
      '<div class="mbr-chat" role="log"><div class="mbr-chat-msg"><div class="mbr-chat-body">' +
      '<dl><dt data-mbr-line="5">x</dt><dd data-mbr-line="6">y</dd></dl></div></div>' +
      '<div class="mbr-chat-md"><dl><dt data-mbr-line="8">z</dt><dd>w</dd></dl></div></div>'
    expect(deckLists(root)).toHaveLength(0)
    expect(hasDeck(root)).toBe(false)
  })

  it('groups terms with their answers and history', () => {
    const cards = deckCards(installDeckPage())
    expect(cards.map((c) => c.term.textContent)).toEqual([
      'Capital of France?',
      'Two answers?',
      'Loose?',
    ])
    expect(cards.map((c) => c.answers.length)).toEqual([1, 2, 1])
    expect(cards[0].history?.dataset.mbrLine).toBe('7')
    expect(cards[1].history).toBeNull()
    expect(cards[2].history?.dataset.mbrLine).toBe('20')
  })

  it('needs emphasis for a history label, in any case', () => {
    const dd = (html: string) => {
      const el = document.createElement('dd')
      el.innerHTML = html
      return el
    }
    expect(isHistoryDefinition(dd('<em><strong>Review History</strong></em><ul></ul>'))).toBe(true)
    expect(isHistoryDefinition(dd('<p><em>review history</em></p>'))).toBe(true)
    expect(isHistoryDefinition(dd('Review History'))).toBe(false)
    expect(isHistoryDefinition(dd('Answer with <em>Review History</em> inside'))).toBe(false)
    expect(isHistoryDefinition(dd('<code>Review History</code>'))).toBe(false)
  })

  it('skips a term with no answer', () => {
    const root = installDeckPage(
      '<main id="wrapper"><dl><dt>Only history</dt><dd><em>Review History</em></dd></dl></main>'
    )
    expect(deckCards(root)).toEqual([])
  })
})

describe('decorateHistory', () => {
  it('collapses a tight history into a summary, without its label', () => {
    const root = installDeckPage()
    decorateAllHistories(root)
    const dd = root.querySelector<HTMLElement>('dd[data-mbr-line="7"]')!
    expect(dd.classList.contains(HISTORY_CLASS)).toBe(true)
    const summary = dd.querySelector('details > summary')!
    expect(summary.textContent).toMatch(/^Reviewed 2× · last: Easy, /)
    expect(summary.getAttribute('tabindex')).toBe('0')
    expect(dd.querySelectorAll('details li')).toHaveLength(2)
    expect(dd.textContent).not.toContain('Review History')
  })

  it('drops a loose label paragraph and is idempotent', () => {
    const root = installDeckPage()
    const dd = root.querySelector<HTMLElement>('dd[data-mbr-line="20"]')!
    decorateHistory(dd)
    decorateHistory(dd)
    expect(dd.querySelectorAll('details')).toHaveLength(1)
    expect(dd.querySelector('p')).toBeNull()
    expect(dd.querySelector('summary')!.textContent).toMatch(/^Reviewed 1× · last: Good/)
    // Still recognised as a history once decorated.
    expect(deckCards(root)[2].history).toBe(dd)
    expect(historyEntriesOf(dd)).toHaveLength(1)
  })

  it('owns a class the deck stylesheet never targets', () => {
    // The deck's sheet is global and stays injected after it closes; a rule
    // on the page's history class once gave every collapsed history padding.
    expect(DECK_CSS).not.toMatch(new RegExp(`\\.${HISTORY_CLASS}[\\s{:,.]`))
  })

  it('leaves answers alone', () => {
    const root = installDeckPage()
    decorateAllHistories(root)
    expect(root.querySelector('dd[data-mbr-line="6"]')!.innerHTML).toBe('Paris.')
  })
})

describe('after a write', () => {
  it('shifts every line reference at or below the insert', () => {
    const root = installDeckPage()
    shiftSourceLines(root, 10, 2)
    expect(root.querySelector('dt')!.dataset.mbrLine).toBe('5')
    expect(
      Array.from(root.querySelectorAll<HTMLElement>('dt'), (dt) => dt.dataset.mbrLine)
    ).toEqual(['5', '13', '18', '27'])
    const task = root.querySelector<HTMLInputElement>('.mbr-task-check')!
    expect(task.id).toBe('mbr-task-32')
    expect(task.dataset.mbrTaskLine).toBe('32')
    expect(root.querySelector('.mbr-incomplete')!.id).toBe('mbr-marker-33')
    // An id that only looks like an anchor is left alone.
    expect(root.querySelector('#anchor')).not.toBeNull()
  })

  it('creates a history definition after the last answer', () => {
    const root = installDeckPage()
    const card = deckCards(root)[1]
    shiftSourceLines(root, 14, 2)
    appendHistoryEntry(card, '2026-10-20 10:00 - Hard', 14, 15)
    const dd = card.history!
    expect(dd.previousElementSibling).toBe(card.answers[1])
    expect(dd.dataset.mbrLine).toBe('14')
    expect(dd.querySelector('li')!.dataset.mbrLine).toBe('15')
    expect(dd.querySelector('summary')!.textContent).toMatch(/^Reviewed 1× · last: Hard/)
    expect(deckCards(root)[1].history).toBe(dd)
  })

  it('numbers nothing new on a terms-only page (--no-review with editing)', () => {
    // Only the terms carry a line; the server numbered no answer, item or
    // history, so neither may the page's own copy of the write.
    const root = installDeckPage(
      DECK_HTML.replace(/<(?!dt)(\w+) data-mbr-line="\d+"/g, '<$1')
    )
    expect(root.querySelectorAll('[data-mbr-line]')).toHaveLength(4)
    const [first, second] = deckCards(root)
    appendHistoryEntry(second, '2026-10-20 10:00 - Hard', 14, 15)
    appendHistoryEntry(first, '2026-10-20 10:05 - Good', 10, 10)
    expect(second.history!.hasAttribute('data-mbr-line')).toBe(false)
    expect(root.querySelectorAll('[data-mbr-line]')).toHaveLength(4)
    expect(Array.from(root.querySelectorAll('[data-mbr-line]'), (el) => el.tagName)).toEqual([
      'DT',
      'DT',
      'DT',
      'DT',
    ])
  })

  it('appends to an existing (decorated) history', () => {
    const root = installDeckPage()
    decorateAllHistories(root)
    const card = deckCards(root)[0]
    appendHistoryEntry(card, '2026-10-20 10:00 - Good', 10, 10)
    expect(card.history!.querySelectorAll('li')).toHaveLength(3)
    expect(card.history!.querySelector('summary')!.textContent).toMatch(/^Reviewed 3× · last: Good/)
  })
})

describe('cardSections', () => {
  const map = () => {
    const root = installDeckPage(SECTIONED_DECK_HTML)
    return cardSections(
      root,
      deckCards(root).map((c) => c.term)
    )
  }

  it('lists every heading with cards, in document order, across <section> wrappers', () => {
    const { sections } = map()
    expect(sections.map((s) => [s.id, s.level, s.cards])).toEqual([
      ['deck', 1, [0, 1, 2, 3, 4]],
      ['geo', 2, [0, 1, 2]],
      ['rivers', 3, [2]],
      ['math', 2, [3, 4]],
    ])
  })

  it('gives each card its nearest heading and that heading’s ancestors', () => {
    expect(map().cardAncestors).toEqual([
      ['deck', 'geo'],
      ['deck', 'geo'],
      ['deck', 'geo', 'rivers'],
      ['deck', 'math'],
      ['deck', 'math'],
    ])
  })

  it('reads heading text without enhancer decoration', () => {
    const { sections } = map()
    expect(sections.map((s) => s.text)).toEqual(['Deck', 'Geography', 'Rivers', 'Math'])
    const h = document.createElement('h2')
    h.innerHTML = 'A <code>b</code><span class="mbr-fc-pie" role="img" aria-label="x"></span>'
    expect(headingText(h)).toBe('A b')
  })

  it('ignores headings inside an answer and headings without an id', () => {
    const root = installDeckPage(
      '<main id="wrapper"><h2>No id</h2><dl><dt>Q1</dt><dd><h3 id="inner">Inner</h3>A</dd><dt>Q2</dt><dd>B</dd></dl></main>'
    )
    const result = cardSections(
      root,
      deckCards(root).map((c) => c.term)
    )
    expect(result.sections).toEqual([])
    expect(result.cardAncestors).toEqual([[], []])
  })

  it('ignores terms that are not cards', () => {
    const root = installDeckPage(SECTIONED_DECK_HTML)
    const cards = deckCards(root).slice(0, 2)
    const result = cardSections(
      root,
      cards.map((c) => c.term)
    )
    expect(result.sections.map((s) => s.id)).toEqual(['deck', 'geo'])
  })
})
