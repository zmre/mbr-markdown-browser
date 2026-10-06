/**
 * Flashcards in the rendered page: which definition lists form the deck, which
 * `<dd>` is a card's review history, and the in-place edits the page needs
 * after a review is written.
 *
 * Stateless DOM helpers shared by both bundles. The main bundle uses
 * {@link hasDeck} and {@link decorateHistory} (the reading view's collapsed
 * history line, which must work in static builds without the chunk); the lazy
 * chunk uses the rest. Exports the main bundle does not import are tree-shaken
 * out of it.
 */
import { isHistoryLabel, parseHistory, summarizeHistory, type HistoryEntry } from './history.js'

/**
 * A definition list inside any of these is not part of the deck.
 *
 * The writer (`src/flashcards.rs`) only edits *top-level* lists, because a
 * nested one would need every inserted line to repeat the container's `> ` or
 * indentation prefix. Collecting the same set here keeps "a card" meaning the
 * same thing on both sides, so a review can never be addressed to a term the
 * server will refuse. `<section>` is deliberately absent: it is the renderer's
 * own wrapper (`enable_sections`), not markdown nesting.
 */
const NESTING_SELECTOR = 'li, blockquote, dd, td, th, .footnote-definition'

/** Class a decorated history `<dd>` carries. */
export const HISTORY_CLASS = 'mbr-fc-history'

/** One card, as the rendered page holds it. */
export interface CardParts {
  readonly term: HTMLElement
  readonly answers: readonly HTMLElement[]
  /** The review-history `<dd>`, if the card has one. */
  history: HTMLElement | null
}

/** The top-level definition lists under `root`, in document order. */
export function deckLists(root: ParentNode): HTMLDListElement[] {
  return Array.from(root.querySelectorAll('dl')).filter(
    (dl) => dl.parentElement?.closest(NESTING_SELECTOR) == null
  )
}

/** True when `root` holds at least one deck term. */
export function hasDeck(root: ParentNode): boolean {
  return deckLists(root).some((dl) => dl.querySelector(':scope > dt') !== null)
}

/**
 * Every card under `root`: a `<dt>` with the `<dd>`s that follow it. The first
 * history `<dd>` is the card's history; any others are answers. A term with no
 * answer at all is skipped — there is nothing to show on its back.
 */
export function deckCards(root: ParentNode): CardParts[] {
  const cards: CardParts[] = []
  for (const dl of deckLists(root)) {
    let current: { term: HTMLElement; answers: HTMLElement[]; history: HTMLElement | null } | null =
      null
    for (const child of Array.from(dl.children)) {
      if (child.tagName === 'DT') {
        if (current) cards.push(current)
        current = { term: child as HTMLElement, answers: [], history: null }
      } else if (child.tagName === 'DD' && current) {
        const dd = child as HTMLElement
        if (!current.history && isHistoryDefinition(dd)) current.history = dd
        else current.answers.push(dd)
      }
    }
    if (current) cards.push(current)
  }
  return cards.filter((card) => card.answers.length > 0)
}

/** True for meaningful nodes: elements and non-blank text. */
function isMeaningful(node: Node): boolean {
  if (node.nodeType === Node.ELEMENT_NODE) return true
  return node.nodeType === Node.TEXT_NODE && (node.textContent ?? '').trim() !== ''
}

/**
 * The emphasis that labels `dd` as a history, and the node to remove with it:
 * the label itself, or its `<p>` when the label is all a loose definition's
 * first paragraph holds.
 */
function historyLabel(dd: Element): { label: Element; removable: Element } | null {
  const first = Array.from(dd.childNodes).find(isMeaningful)
  if (!(first instanceof Element)) return null
  let label: Element | undefined = first
  let removable: Element = first
  if (first.tagName === 'P') {
    const inner = Array.from(first.childNodes).filter(isMeaningful)
    label = inner[0] instanceof Element ? inner[0] : undefined
    if (label && inner.length > 1) removable = label
  }
  if (!label || (label.tagName !== 'EM' && label.tagName !== 'STRONG')) return null
  return isHistoryLabel(label.textContent ?? '') ? { label, removable } : null
}

/**
 * True for a card's review-history `<dd>`: its leading content is emphasis
 * reading "Review History" (any case; `___x___`, `**x**` and `*x*` all qualify).
 * Mirrors `is_history_label` in `src/flashcards.rs`.
 */
export function isHistoryDefinition(dd: Element): boolean {
  return dd.classList.contains(HISTORY_CLASS) || historyLabel(dd) !== null
}

/** The parsed entries of a history `<dd>`, oldest first. */
export function historyEntriesOf(dd: Element): HistoryEntry[] {
  return parseHistory(
    Array.from(dd.querySelectorAll('li'), (li) => (li.textContent ?? '').split('\n')[0])
  )
}

/**
 * Collapse a history `<dd>` into a `<details>` whose `<summary>` reads
 * `Reviewed 4× · last: Easy, Oct 9`; idempotent, and re-run after an entry is
 * added to refresh the summary.
 *
 * The `<summary>` gets an explicit `tabindex`: the FAQ disclosure keeps a
 * definition open only while focus is inside it, and Safari does not focus a
 * clicked `<summary>` (or button) unless it is explicitly tabbable — without
 * this, expanding the history in Safari would collapse the whole card.
 */
export function decorateHistory(dd: HTMLElement, now: Date = new Date()): void {
  let summary = dd.querySelector<HTMLElement>(`:scope > details > summary`)
  if (!dd.classList.contains(HISTORY_CLASS) || !summary) {
    const found = historyLabel(dd)
    if (!found) return
    const { removable } = found
    // Drop the whitespace a tight label leaves behind before its list.
    const next = removable.nextSibling
    if (next && next.nodeType === Node.TEXT_NODE && (next.textContent ?? '').trim() === '') {
      next.remove()
    }
    removable.remove()
    const details = document.createElement('details')
    summary = document.createElement('summary')
    summary.tabIndex = 0
    details.append(summary, ...Array.from(dd.childNodes))
    dd.append(details)
    dd.classList.add(HISTORY_CLASS)
  }
  summary.textContent = summarizeHistory(historyEntriesOf(dd), now)
}

/** Decorate every history `<dd>` of the deck under `root`. */
export function decorateAllHistories(root: ParentNode): void {
  for (const card of deckCards(root)) {
    if (card.history) decorateHistory(card.history)
  }
}

/**
 * Shift every source-line reference in the page at or after line `from` by
 * `by`, after a write inserted `by` lines there.
 *
 * Nothing re-renders the page after a review (the write suppresses its own
 * live reload, exactly like a task toggle), yet the lines below the insert have
 * moved on disk. Left alone, the next review — or task toggle, or review note —
 * would address the line *above* the one it means, and `expected` might even
 * match it. Covers every attribute the renderer emits a line in:
 * `data-mbr-line`, `data-mbr-task-line`, and the `mbr-task-N` / `mbr-marker-N`
 * anchor ids.
 */
export function shiftSourceLines(root: ParentNode, from: number, by: number): void {
  const shift = (value: string | null | undefined): string | null => {
    const line = Number(value)
    return Number.isInteger(line) && line >= from ? String(line + by) : null
  }
  for (const el of Array.from(root.querySelectorAll<HTMLElement>('[data-mbr-line]'))) {
    const next = shift(el.dataset.mbrLine)
    if (next) el.dataset.mbrLine = next
  }
  for (const el of Array.from(root.querySelectorAll<HTMLElement>('[data-mbr-task-line]'))) {
    const next = shift(el.dataset.mbrTaskLine)
    if (next) el.dataset.mbrTaskLine = next
  }
  for (const el of Array.from(root.querySelectorAll<HTMLElement>('[id^="mbr-task-"], [id^="mbr-marker-"]'))) {
    const match = /^(mbr-(?:task|marker)-)(\d+)$/.exec(el.id)
    const next = match ? shift(match[2]) : null
    if (match && next) el.id = match[1] + next
  }
}

/**
 * Reflect a written review into the page: add the entry to the card's history
 * `<dd>` (creating it, as the server did, when the card had none) and refresh
 * its summary. Line attributes on new elements match the server's insert, so a
 * later write addressed through them stays correct.
 *
 * Call {@link shiftSourceLines} first — the new elements are already numbered.
 */
export function appendHistoryEntry(
  card: CardParts,
  entry: string,
  insertedAt: number,
  entryLine: number
): void {
  const li = document.createElement('li')
  li.textContent = entry
  li.dataset.mbrLine = String(entryLine)

  let dd = card.history
  if (!dd) {
    dd = document.createElement('dd')
    dd.dataset.mbrLine = String(insertedAt)
    const label = document.createElement('em')
    label.append(document.createElement('strong'))
    label.firstElementChild!.textContent = 'Review History'
    dd.append(label, document.createElement('ul'))
    const last = card.answers[card.answers.length - 1]
    last.after(dd)
    card.history = dd
  }
  let list = dd.querySelector('ul, ol')
  if (!list) {
    list = document.createElement('ul')
    ;(dd.querySelector(':scope > details') ?? dd).append(list)
  }
  list.append(li)
  decorateHistory(dd)
}
