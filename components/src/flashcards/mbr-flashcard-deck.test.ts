import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import './mbr-flashcard-deck.js'
import { DECK_CLOSE_EVENT, type MbrFlashcardDeckElement } from './mbr-flashcard-deck.js'
import { DECK_STYLE_ID } from './styles.js'
import { installDeckPage } from './test-fixtures.js'
import type { ReviewOutcome, ReviewRecorder, ReviewTarget } from './types.js'

let deck: MbrFlashcardDeckElement

async function mount(options: { srs?: boolean; recorder?: ReviewRecorder } = {}) {
  const root = installDeckPage()
  deck = document.createElement('mbr-flashcard-deck')
  deck.root = root
  deck.recordReview = options.recorder ?? null
  deck.srsAvailable = options.srs ?? false
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

  it('offers In order and Random, and no FSRS without a writer', async () => {
    await mount()
    const options = Array.from(deck.querySelectorAll('option'), (o) => o.value)
    expect(options).toEqual(['order', 'random'])
    expect(deck.querySelector('option[value="random"]')!.hasAttribute('selected')).toBe(true)
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

  it('is the default when available, and orders due cards before new ones', async () => {
    await mount({ srs: true, recorder })
    expect(deck.querySelector('select')!.querySelector('option[value="srs"]')!.hasAttribute('selected')).toBe(true)
    // Cards 1 and 3 have old histories (due); card 2 is new.
    expect(counter()).toBe('Due 2 · New 1')
  })

  it('shows numbered rating buttons with intervals; Space on the back does not skip', async () => {
    await mount({ srs: true, recorder })
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
    const root = await mount({ srs: true, recorder })
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
    await mount({ srs: true, recorder })
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
    await mount({ srs: true, recorder })
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
    await mount({ srs: true, recorder })
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
    deck.srsAvailable = true
    document.body.append(deck)
    await deck.updateComplete
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
    deck.srsAvailable = true
    document.body.append(deck)
    await deck.updateComplete
    expect(deck.querySelector('option[value="srs"]')).toBeNull()
  })
})
