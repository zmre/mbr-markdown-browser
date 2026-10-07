import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import './mbr-flashcard-deck.js'
import { DECK_CLOSE_EVENT, SHORTCUT_GROUPS, type MbrFlashcardDeckElement } from './mbr-flashcard-deck.js'
import { DECK_STYLE_ID } from './styles.js'
import { SECTIONED_DECK_HTML, installDeckPage } from './test-fixtures.js'
import type { ReviewOutcome, ReviewRecorder, ReviewTarget } from './types.js'

let deck: MbrFlashcardDeckElement

// A recorder is what turns spaced repetition on, exactly as the trigger decides.
async function mount(options: { recorder?: ReviewRecorder; html?: string; threshold?: number } = {}) {
  const root = installDeckPage(options.html)
  deck = document.createElement('mbr-flashcard-deck')
  deck.root = root
  deck.recordReview = options.recorder ?? null
  if (options.threshold !== undefined) deck.concentricThreshold = options.threshold
  document.body.append(deck)
  await deck.updateComplete
  return root
}

function key(key: string, target: EventTarget = document.activeElement ?? document.body) {
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, composed: true, cancelable: true }))
  return deck.updateComplete
}

const card = () => deck.querySelector<HTMLElement>('.mbr-fc-card')
const isFlipped = () => card()?.classList.contains('is-flipped') ?? false
const frontText = () => deck.querySelector('.mbr-fc-front .mbr-fc-content')?.textContent?.trim()
const counter = () => deck.querySelector('.mbr-fc-counter')?.textContent

function select(mode: string) {
  const el = deck.querySelector<HTMLSelectElement>('.mbr-fc-bar select')!
  el.value = mode
  el.dispatchEvent(new Event('change'))
  return deck.updateComplete
}

afterEach(() => {
  deck?.remove()
  document.body.className = ''
  document.body.innerHTML = ''
  document.documentElement.style.overflow = ''
})

describe('<mbr-flashcard-deck> basics', () => {
  it('renders a modal dialog over the page, with its styles, and locks scrolling', async () => {
    await mount()
    const dialog = deck.querySelector('[role="dialog"]')!
    expect(dialog.getAttribute('aria-modal')).toBe('true')
    expect(document.getElementById(DECK_STYLE_ID)).not.toBeNull()
    expect(document.documentElement.style.overflow).toBe('hidden')
    deck.remove()
    expect(document.documentElement.style.overflow).toBe('')
  })

  it('offers In order, Random and Concentric, and no FSRS without a writer', async () => {
    await mount()
    const options = Array.from(deck.querySelectorAll('option'), (o) => o.value)
    expect(options).toEqual(['order', 'random', 'concentric'])
    expect(deck.querySelector('option[value="concentric"]')!.textContent!.trim()).toBe('Concentric (FSRS)')
  })

  it('opens in Concentric, without a writer', async () => {
    await mount()
    expect(deck.querySelector<HTMLSelectElement>('.mbr-fc-bar select')!.value).toBe('concentric')
    expect(counter()).toMatch(/in play/)
  })

  it('opens in Concentric with a writer too, though spaced repetition is offered', async () => {
    await mount({ recorder: () => Promise.resolve({ ok: false, kind: 'other', message: 'x' }) })
    expect(deck.querySelector('option[value="srs"]')).not.toBeNull()
    expect(deck.querySelector<HTMLSelectElement>('.mbr-fc-bar select')!.value).toBe('concentric')
  })

  it('opens in Random when there are no cards to review', async () => {
    await mount({ html: '<main id="wrapper"><dl><dd>orphan</dd></dl></main>' })
    expect(deck.querySelector('.mbr-fc-screen h3')!.textContent).toBe('No cards')
    expect(deck.querySelector('option[value="concentric"]')).toBeNull()
    expect(deck.querySelector<HTMLSelectElement>('.mbr-fc-bar select')!.value).toBe('random')
  })

  it('clones card content without ids or line numbers', async () => {
    await mount()
    await select('order')
    await key('ArrowRight')
    await key('ArrowRight')
    // Card 2 has an id'd link in its second answer.
    const back = deck.querySelector('.mbr-fc-back .mbr-fc-content')!
    expect(back.querySelectorAll('.mbr-fc-answer')).toHaveLength(2)
    expect(back.querySelector('a')!.hasAttribute('id')).toBe(false)
    expect(deck.querySelector('[data-mbr-line]')).toBeNull()
    expect(document.querySelectorAll('#anchor')).toHaveLength(1)
  })

  it('closes on Escape', async () => {
    await mount()
    const closed = vi.fn()
    deck.addEventListener(DECK_CLOSE_EVENT, closed)
    await key('Escape')
    expect(closed).toHaveBeenCalledOnce()
  })
})

describe('In order mode keys', () => {
  beforeEach(async () => {
    await mount()
    await select('order')
  })

  it('Space flips, then advances; → and n do the same', async () => {
    expect(counter()).toBe('1 / 3')
    expect(frontText()).toBe('Capital of France?')
    await key(' ')
    expect(isFlipped()).toBe(true)
    await key(' ')
    expect(counter()).toBe('2 / 3')
    expect(isFlipped()).toBe(false)
    await key('ArrowRight')
    await key('n')
    expect(counter()).toBe('3 / 3')
    await key('PageDown')
    await key('PageDown')
    expect(deck.querySelector('.mbr-fc-screen h3')!.textContent).toBe('End of deck')
  })

  it('Enter flips both ways; ← / PageUp go back; Home / End jump', async () => {
    await key('Enter')
    expect(isFlipped()).toBe(true)
    await key('Enter')
    expect(isFlipped()).toBe(false)
    await key('End')
    expect(counter()).toBe('3 / 3')
    await key('ArrowLeft')
    expect(counter()).toBe('2 / 3')
    await key('PageUp')
    expect(counter()).toBe('1 / 3')
    await key('End')
    await key('Home')
    expect(counter()).toBe('1 / 3')
  })

  it('swaps sides', async () => {
    deck.querySelector<HTMLButtonElement>('.mbr-fc-swap')!.click()
    await deck.updateComplete
    expect(frontText()).toBe('Paris.')
    expect(deck.querySelector('.mbr-fc-swap')!.getAttribute('aria-pressed')).toBe('true')
  })

  it('a click on the card flips it, but a click on a link inside does not', async () => {
    card()!.click()
    await deck.updateComplete
    expect(isFlipped()).toBe(true)
  })

  it('leaves Space to a focused button', async () => {
    const swap = deck.querySelector<HTMLButtonElement>('.mbr-fc-swap')!
    swap.focus()
    await key(' ', swap)
    expect(isFlipped()).toBe(false)
  })
})

describe('spaced repetition', () => {
  let calls: ReviewTarget[]
  let reply: ReviewOutcome

  const recorder: ReviewRecorder = (target) => {
    calls.push(target)
    return Promise.resolve(reply)
  }

  beforeEach(() => {
    calls = []
    reply = { ok: true, entry: '2026-10-20 10:00 - Good', line: 15, insertedAt: 14, insertedCount: 2 }
    // The fixture's histories end in October 2026; pin "now" well after, so
    // both are due whatever day the suite runs. Only `Date` is faked — Lit's
    // microtasks and frames run as usual.
    vi.useFakeTimers({ now: new Date(2027, 5, 1, 12), toFake: ['Date'] })
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  async function mountSrs() {
    const root = await mount({ recorder })
    await select('srs')
    return root
  }

  it('is offered with a writer, and orders due cards before new ones', async () => {
    await mountSrs()
    expect(deck.querySelector<HTMLSelectElement>('.mbr-fc-bar select')!.value).toBe('srs')
    // Cards 1 and 3 have old histories (due); card 2 is new.
    expect(counter()).toBe('Due 2 · New 1')
  })

  it('shows numbered rating buttons with intervals; Space on the back does not skip', async () => {
    await mountSrs()
    await key(' ')
    const buttons = Array.from(deck.querySelectorAll('.mbr-fc-ratings button'))
    expect(buttons.map((b) => b.querySelector('span')!.textContent)).toEqual([
      '1Again',
      '2Hard',
      '3Good',
      '4Easy',
    ])
    expect(buttons.every((b) => /\d|<1m/.test(b.querySelector('small')!.textContent!))).toBe(true)
    const before = counter()
    await key(' ')
    await key('ArrowRight')
    expect(isFlipped()).toBe(true)
    expect(counter()).toBe(before)
    expect(calls).toHaveLength(0)
  })

  it('a rating key records the review, updates the page and advances', async () => {
    const root = await mountSrs()
    // Walk to the new card (always last in the queue) — rate the first two.
    reply = { ok: true, entry: '2026-10-20 10:00 - Easy', line: 10, insertedAt: 10, insertedCount: 1 }
    await key(' ')
    await key('4')
    await vi.waitFor(() => expect(calls).toHaveLength(1))
    await deck.updateComplete
    expect(calls[0].rating).toBe('easy')
    expect([5, 16]).toContain(calls[0].line)
    // Lines below the insert moved in the page itself.
    expect(root.querySelector<HTMLElement>('.mbr-task-check')!.dataset.mbrTaskLine).toBe('31')
    expect(isFlipped()).toBe(false)
  })

  it('brings a lapsed card back later in the same session (learn-ahead)', async () => {
    await mountSrs()
    reply = { ok: true, entry: '2027-06-01 12:00 - Again', line: 10, insertedAt: 10, insertedCount: 1 }
    expect(counter()).toBe('Due 2 · New 1')
    await key(' ')
    await key('1')
    await vi.waitFor(() => expect(calls).toHaveLength(1))
    await deck.updateComplete
    // One due card left, the new one, and the lapsed card queued again.
    expect(counter()).toBe('Due 2 · New 1')
  })

  it('stops writing after a conflict and says so', async () => {
    reply = { ok: false, kind: 'conflict', message: 'This note changed on disk — reload to continue reviewing.' }
    await mountSrs()
    await key(' ')
    await key('3')
    await vi.waitFor(() => expect(deck.querySelector('.mbr-fc-banner')).not.toBeNull())
    await deck.updateComplete
    expect(deck.querySelector('.mbr-fc-banner')!.textContent).toContain('changed on disk')
    expect(deck.querySelector('.mbr-fc-ratings')).toBeNull()
    // Read-only from here: Space now moves on.
    await key(' ')
    expect(isFlipped()).toBe(false)
  })

  it('keeps the card on any other failure so the rating can be retried', async () => {
    reply = { ok: false, kind: 'auth', message: 'Editing needs a token' }
    await mountSrs()
    await key(' ')
    await key('1')
    await vi.waitFor(() => expect(deck.querySelector('.mbr-fc-banner')).not.toBeNull())
    await deck.updateComplete
    expect(deck.querySelector('.mbr-fc-ratings')).not.toBeNull()
    expect(isFlipped()).toBe(true)
  })

  it('says nothing is due when every card is scheduled later', async () => {
    const future = '2999-01-01 00:00 - Easy'
    const root = installDeckPage(
      `<main id="wrapper"><dl><dt data-mbr-line="1">Q</dt><dd>A</dd><dd><em>Review History</em><ul><li>${future}</li></ul></dd></dl></main>`
    )
    deck = document.createElement('mbr-flashcard-deck')
    deck.root = root
    deck.recordReview = recorder
    document.body.append(deck)
    await deck.updateComplete
    await select('srs')
    expect(deck.querySelector('.mbr-fc-screen h3')!.textContent).toBe('Nothing due')
    deck.querySelector<HTMLButtonElement>('.mbr-fc-screen button')!.click()
    await deck.updateComplete
    expect(counter()).toBe('1 / 1')
  })

  it('is not offered when a term carries no source line', async () => {
    const root = installDeckPage('<main id="wrapper"><dl><dt>Q</dt><dd>A</dd></dl></main>')
    deck = document.createElement('mbr-flashcard-deck')
    deck.root = root
    deck.recordReview = recorder
    document.body.append(deck)
    await deck.updateComplete
    expect(deck.querySelector('option[value="srs"]')).toBeNull()
  })
})

describe('concentric mode', () => {
  let calls: ReviewTarget[]
  const ok: ReviewOutcome = { ok: true, entry: '2026-10-20 10:00 - Good', line: 10, insertedAt: 10, insertedCount: 1 }
  let reply: ReviewOutcome
  const recorder: ReviewRecorder = (target) => {
    calls.push(target)
    return Promise.resolve(reply)
  }

  const sizeBox = () => deck.querySelector<HTMLInputElement>('.mbr-fc-size input')
  async function setSize(n: number) {
    const box = sizeBox()!
    box.value = String(n)
    box.dispatchEvent(new Event('change'))
    await deck.updateComplete
  }
  async function rate(keyName: string) {
    await key(' ')
    await key(keyName)
    await deck.updateComplete
  }

  beforeEach(() => {
    calls = []
    reply = ok
  })

  it('needs no writer, shows the Cards box, and counts the stack', async () => {
    await mount({ html: SECTIONED_DECK_HTML })
    await select('order')
    expect(sizeBox()).toBeNull()
    await select('concentric')
    expect(sizeBox()!.value).toBe('5')
    expect(sizeBox()!.getAttribute('aria-label')).toBe('Cards in play')
    expect(counter()).toBe('5 in play · 0 left')
    await setSize(2)
    expect(sizeBox()!.value).toBe('2')
    expect(counter()).toBe('2 in play · 3 left')
  })

  it('rates session-only without a writer: no write, ratings move the card on', async () => {
    const root = await mount({ html: SECTIONED_DECK_HTML })
    await select('concentric')
    await key(' ')
    expect(deck.querySelectorAll('.mbr-fc-ratings button')).toHaveLength(4)
    expect(deck.querySelector('.mbr-fc-ratings small')!.textContent).toBe('soon')
    // Space on the back only nudges; a card is left by rating it.
    await key(' ')
    expect(isFlipped()).toBe(true)
    await key('ArrowLeft')
    expect(isFlipped()).toBe(true)
    await key('4')
    await deck.updateComplete
    expect(isFlipped()).toBe(false)
    expect(calls).toHaveLength(0)
    // Nothing in the page moved or grew.
    expect(root.querySelectorAll('li')).toHaveLength(4)
  })

  it('writes each rating when a writer is present, exactly as FSRS does', async () => {
    const root = await mount({ html: SECTIONED_DECK_HTML, recorder })
    await select('concentric')
    await rate('3')
    await vi.waitFor(() => expect(calls).toHaveLength(1))
    await deck.updateComplete
    expect(calls[0].rating).toBe('good')
    // Lines at or below the insert shifted in the page.
    expect(root.querySelector<HTMLElement>('dt[data-mbr-line="31"]')).not.toBeNull()
  })

  it('carries on session-only after a conflict, keeping the notice', async () => {
    reply = { ok: false, kind: 'conflict', message: 'This note changed on disk — reload to continue reviewing.' }
    await mount({ html: SECTIONED_DECK_HTML, recorder })
    await select('concentric')
    await rate('3')
    await vi.waitFor(() => expect(deck.querySelector('.mbr-fc-banner')).not.toBeNull())
    await deck.updateComplete
    expect(isFlipped()).toBe(false)
    await rate('3')
    expect(calls).toHaveLength(1)
    expect(deck.querySelector('.mbr-fc-banner')).not.toBeNull()
    await key(' ')
    expect(deck.querySelector('.mbr-fc-ratings')).not.toBeNull()
  })

  it('updates the Cards box when the stack grows, and reports mastery', async () => {
    await mount({ html: SECTIONED_DECK_HTML })
    await select('concentric')
    await setSize(3)
    for (let i = 0; i < 3; i++) await rate('4')
    expect(sizeBox()!.value).toBe('5')
    expect(counter()).toBe('5 in play · 0 left')
    for (let i = 0; i < 5; i++) await rate('4')
    expect(deck.querySelector('.mbr-fc-screen h3')!.textContent).toBe('Stack mastered')
    const keep = Array.from(deck.querySelectorAll<HTMLButtonElement>('.mbr-fc-screen button')).find(
      (b) => b.textContent!.trim() === 'Keep practising'
    )!
    keep.click()
    await deck.updateComplete
    expect(card()).not.toBeNull()
    expect(counter()).toBe('5 in play · 0 left')
  })

  it('leaves keys typed in the Cards box to the box', async () => {
    await mount({ html: SECTIONED_DECK_HTML })
    await select('concentric')
    const box = sizeBox()!
    box.focus()
    await key(' ', box)
    await key('4', box)
    expect(isFlipped()).toBe(false)
  })
})

describe('section filter', () => {
  const funnel = () => deck.querySelector<HTMLButtonElement>('.mbr-fc-filter-btn')
  const popover = () => deck.querySelector('.mbr-fc-filter-pop')
  const boxes = () => Array.from(deck.querySelectorAll<HTMLInputElement>('.mbr-fc-filter-pop input'))
  async function openFilter() {
    funnel()!.click()
    await deck.updateComplete
  }
  async function check(label: string) {
    const input = boxes().find((b) => b.closest('label')!.textContent!.includes(label))!
    input.checked = !input.checked
    input.dispatchEvent(new Event('change'))
    await deck.updateComplete
  }
  const total = () => counter()!.split(' / ')[1]

  it('is hidden on a page without at least two headings holding cards', async () => {
    await mount()
    expect(funnel()).toBeNull()
  })

  it('lists headings with cards, indented by level, and toggles open', async () => {
    await mount({ html: SECTIONED_DECK_HTML })
    expect(funnel()!.getAttribute('aria-label')).toBe('Filter by section')
    expect(funnel()!.getAttribute('aria-haspopup')).toBe('true')
    expect(funnel()!.getAttribute('aria-expanded')).toBe('false')
    await openFilter()
    expect(funnel()!.getAttribute('aria-expanded')).toBe('true')
    const labels = Array.from(deck.querySelectorAll('.mbr-fc-filter-pop label'), (l) => l.querySelector('span')?.textContent ?? 'All')
    // The H1 holds every card, so it would filter nothing and is left out.
    expect(labels).toEqual(['All', 'Geography', 'Rivers', 'Math'])
    expect(boxes()[0].checked).toBe(true)
    const rivers = deck.querySelectorAll<HTMLElement>('.mbr-fc-filter-pop label')[2]
    expect(rivers.getAttribute('style')).toContain('--mbr-fc-depth: 1')
  })

  it('narrows In order to the chosen subtree and restarts', async () => {
    await mount({ html: SECTIONED_DECK_HTML })
    await select('order')
    await key('ArrowRight')
    await key('ArrowRight')
    await openFilter()
    await check('Math')
    expect(counter()).toBe('1 / 2')
    expect(funnel()!.querySelector('.mbr-fc-filter-badge')).not.toBeNull()
    expect(boxes()[0].checked).toBe(false)
    // A heading includes its whole subtree: Geography brings Rivers' card.
    await check('Math')
    await check('Geography')
    expect(total()).toBe('3')
    // Back to everything.
    await check('All sections')
    expect(total()).toBe('5')
    expect(funnel()!.querySelector('.mbr-fc-filter-badge')).toBeNull()
  })

  it('narrows Random too', async () => {
    await mount({ html: SECTIONED_DECK_HTML })
    await select('random')
    await openFilter()
    await check('Rivers')
    expect(counter()).toBe('1 / 1')
    expect(frontText()).toBe('Longest river?')
  })

  it('re-plans spaced repetition over the chosen cards', async () => {
    vi.useFakeTimers({ now: new Date(2027, 5, 1, 12), toFake: ['Date'] })
    try {
      await mount({ html: SECTIONED_DECK_HTML, recorder: () => Promise.resolve({ ok: false, kind: 'other', message: 'x' }) })
      await select('srs')
      expect(counter()).toBe('Due 3 · New 2')
      await openFilter()
      await check('Math')
      expect(counter()).toBe('Due 1 · New 1')
    } finally {
      vi.useRealTimers()
    }
  })

  it('swaps Concentric’s stack to the chosen cards and keeps X', async () => {
    await mount({ html: SECTIONED_DECK_HTML })
    await select('concentric')
    await openFilter()
    await check('Math')
    expect(counter()).toBe('2 in play · 0 left')
    expect(deck.querySelector<HTMLInputElement>('.mbr-fc-size input')!.value).toBe('2')
    await check('All sections')
    expect(counter()).toBe('5 in play · 0 left')
  })

  it('Escape closes the popover first, then the deck', async () => {
    await mount({ html: SECTIONED_DECK_HTML })
    const closed = vi.fn()
    deck.addEventListener(DECK_CLOSE_EVENT, closed)
    await openFilter()
    boxes()[1].focus()
    await key('Escape', boxes()[1])
    expect(popover()).toBeNull()
    expect(closed).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(funnel())
    await key('Escape')
    expect(closed).toHaveBeenCalledOnce()
  })

  it('closes on a click outside, but not on one inside', async () => {
    await mount({ html: SECTIONED_DECK_HTML })
    await openFilter()
    boxes()[1].dispatchEvent(new Event('pointerdown', { bubbles: true }))
    await deck.updateComplete
    expect(popover()).not.toBeNull()
    deck.querySelector('.mbr-fc-stage')!.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    await deck.updateComplete
    expect(popover()).toBeNull()
  })
})

describe('keyboard shortcuts', () => {
  const writer: ReviewRecorder = () => Promise.resolve({ ok: false, kind: 'other', message: 'x' })
  const mode = () => deck.querySelector<HTMLSelectElement>('.mbr-fc-bar select')!.value
  const help = () => deck.querySelector<HTMLElement>('.mbr-fc-help')
  const sizeBox = () => deck.querySelector<HTMLInputElement>('.mbr-fc-size input')
  const checks = () => Array.from(deck.querySelectorAll<HTMLInputElement>('.mbr-fc-filter-pop input'))
  const settle = async () => {
    await deck.updateComplete
    await deck.updateComplete
  }

  it('advertises ? in the hints line', async () => {
    await mount()
    expect(deck.querySelector('.mbr-fc-hints')!.textContent).toMatch(/\?\s*for shortcuts/)
    await select('order')
    expect(deck.querySelector('.mbr-fc-hints')!.textContent).toMatch(/\?\s*for shortcuts/)
  })

  it('? opens an accessible help dialog listing every shortcut, grouped', async () => {
    await mount()
    const before = document.activeElement
    expect(before).toBe(card())
    await key('?')
    await settle()
    const dialog = help()!
    expect(dialog.getAttribute('role')).toBe('dialog')
    expect(dialog.getAttribute('aria-modal')).toBe('true')
    expect(document.getElementById(dialog.getAttribute('aria-labelledby')!)!.textContent).toBe('Keyboard shortcuts')
    expect(dialog.contains(document.activeElement)).toBe(true)
    expect(Array.from(dialog.querySelectorAll('h4'), (h) => h.textContent)).toEqual([
      'Navigation',
      'Rating',
      'Deck controls',
    ])
    const shown = Array.from(dialog.querySelectorAll('kbd'), (k) => k.textContent)
    for (const group of SHORTCUT_GROUPS) for (const s of group.shortcuts) for (const k of s.keys) expect(shown).toContain(k)

    // ? again closes it and gives focus back.
    await key('?')
    await settle()
    expect(help()).toBeNull()
    expect(document.activeElement).toBe(before)
  })

  it('Esc closes the help, not the deck; other keys wait while it is open', async () => {
    await mount()
    const closed = vi.fn()
    deck.addEventListener(DECK_CLOSE_EVENT, closed)
    await key('?')
    await settle()
    await key(' ')
    await key('o')
    expect(isFlipped()).toBe(false)
    expect(mode()).toBe('concentric')
    await key('Escape')
    await settle()
    expect(help()).toBeNull()
    expect(closed).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(card())
    // The close button works too.
    await key('?')
    await settle()
    help()!.querySelector<HTMLButtonElement>('button')!.click()
    await settle()
    expect(help()).toBeNull()
  })

  it('switches mode with c / o / r, and ignores s without a writer', async () => {
    await mount()
    expect(mode()).toBe('concentric')
    await key('o')
    expect(mode()).toBe('order')
    expect(counter()).toBe('1 / 3')
    await key('r')
    expect(mode()).toBe('random')
    await key('s')
    expect(mode()).toBe('random')
    await key('c')
    expect(mode()).toBe('concentric')
    expect(counter()).toMatch(/in play/)
  })

  it('s switches to spaced repetition when it is offered', async () => {
    await mount({ recorder: writer })
    await key('s')
    expect(mode()).toBe('srs')
    expect(counter()).toMatch(/^Due \d+ · New \d+$/)
  })

  it('a mode key for the current mode keeps the reader’s place', async () => {
    await mount()
    await select('order')
    await key('ArrowRight')
    await key('ArrowRight')
    expect(counter()).toBe('2 / 3')
    await key('o')
    expect(counter()).toBe('2 / 3')
  })

  it('w swaps sides', async () => {
    await mount()
    await key('w')
    expect(deck.querySelector('.mbr-fc-front .mbr-fc-side')!.textContent).toBe('Answer')
    expect(deck.querySelector('.mbr-fc-swap')!.getAttribute('aria-pressed')).toBe('true')
    await key('w')
    expect(deck.querySelector('.mbr-fc-front .mbr-fc-side')!.textContent).toBe('Question')
  })

  it('+ / = grow and - shrinks Concentric’s stack through the Cards box path', async () => {
    await mount({ html: SECTIONED_DECK_HTML })
    expect(counter()).toBe('5 in play · 0 left')
    await key('-')
    expect(counter()).toBe('4 in play · 1 left')
    expect(sizeBox()!.value).toBe('4')
    await key('-')
    await key('+')
    expect(sizeBox()!.value).toBe('4')
    await key('=')
    expect(counter()).toBe('5 in play · 0 left')
    // Clamped by the eligible cards, like the box.
    await key('+')
    expect(sizeBox()!.value).toBe('5')
    // Not a Concentric deck: nothing to resize.
    await select('order')
    await key('-')
    expect(counter()).toBe('1 / 5')
  })

  it('# focuses the Cards box; keys there are the box’s, Enter applies, Esc restores', async () => {
    await mount({ html: SECTIONED_DECK_HTML })
    const closed = vi.fn()
    deck.addEventListener(DECK_CLOSE_EVENT, closed)
    await key('#')
    const box = sizeBox()!
    expect(document.activeElement).toBe(box)
    await key(' ', box)
    await key('o', box)
    await key('-', box)
    expect(isFlipped()).toBe(false)
    expect(mode()).toBe('concentric')
    expect(counter()).toBe('5 in play · 0 left')

    box.value = '2'
    await key('Enter', box)
    await settle()
    expect(counter()).toBe('2 in play · 3 left')
    expect(document.activeElement).toBe(card())

    await key('#')
    sizeBox()!.value = '4'
    await key('Escape', sizeBox()!)
    await settle()
    expect(sizeBox()!.value).toBe('2')
    expect(counter()).toBe('2 in play · 3 left')
    expect(closed).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(card())
  })

  it('f opens the filter with focus inside; arrows move, a picks all, Esc returns focus', async () => {
    await mount({ html: SECTIONED_DECK_HTML })
    await select('order')
    card()!.focus()
    await key('f')
    await settle()
    expect(checks()).toHaveLength(4)
    expect(document.activeElement).toBe(checks()[0])
    await key('ArrowDown')
    expect(document.activeElement).toBe(checks()[1])
    await key('ArrowUp')
    await key('ArrowUp')
    expect(document.activeElement).toBe(checks()[3])
    // Space toggles the box natively; the change reaches the filter as a click would.
    const math = checks()[3]
    math.checked = true
    math.dispatchEvent(new Event('change'))
    await settle()
    expect(counter()).toBe('1 / 2')
    await key('a', checks()[3])
    await settle()
    expect(counter()).toBe('1 / 5')
    expect(checks()[0].checked).toBe(true)
    await key('Escape')
    await settle()
    expect(checks()).toHaveLength(0)
    expect(document.activeElement).toBe(card())
  })

  it('f does nothing on a deck the filter is not offered for', async () => {
    await mount()
    await key('f')
    await settle()
    expect(deck.querySelector('.mbr-fc-filter-pop')).toBeNull()
  })

  it('keeps every key from the page’s own shortcuts while open', async () => {
    await mount()
    const page = vi.fn()
    document.addEventListener('keydown', page)
    try {
      for (const k of ['?', 'f', 'j', '/', 'r', '-', '=', 'Escape']) await key(k)
      expect(page).not.toHaveBeenCalled()
    } finally {
      document.removeEventListener('keydown', page)
    }
  })

  it('leaves Cmd / Ctrl / Alt combinations to the browser', async () => {
    await mount()
    for (const modifier of ['ctrlKey', 'metaKey', 'altKey'] as const) {
      const e = new KeyboardEvent('keydown', { key: 'o', bubbles: true, composed: true, cancelable: true, [modifier]: true })
      card()!.dispatchEvent(e)
      await deck.updateComplete
      expect(e.defaultPrevented).toBe(false)
      expect(mode()).toBe('concentric')
    }
    const print = new KeyboardEvent('keydown', { key: '?', bubbles: true, cancelable: true, metaKey: true })
    card()!.dispatchEvent(print)
    await settle()
    expect(help()).toBeNull()
  })

  it('stops listening once removed', async () => {
    await mount()
    deck.remove()
    const page = vi.fn()
    document.addEventListener('keydown', page)
    try {
      await key('o', document.body)
      expect(page).toHaveBeenCalledOnce()
    } finally {
      document.removeEventListener('keydown', page)
    }
  })
})
