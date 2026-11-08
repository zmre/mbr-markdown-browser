/**
 * Which list items can be agenda entries, and the cheap "might this page have
 * any?" test the main-bundle trigger runs before deciding to load the calendar
 * chunk.
 *
 * Pure DOM, imports only `../top-level.ts`: shared by the main bundle
 * (`mbr-calendar.ts`'s prefilter) and the chunk (`agenda-parse.ts`), so the two
 * cannot disagree about which lists count. Kept apart from the grammar so the
 * main bundle carries a regex and a selector, not the parser.
 */
import { MARKDOWN_NESTING_SELECTOR, isTopLevel } from '../top-level.js'

/**
 * A list inside any of these is not an agenda (design/calendar.md §4.4): the
 * markdown nesting flashcards share, plus definition lists, tables, `<details>`
 * and `<aside>` (raw-HTML containers whose lists are asides, not the day).
 */
export const AGENDA_NESTING_SELECTOR = `${MARKDOWN_NESTING_SELECTOR}, dl, table, details, aside`

/**
 * A cheap superset of the agenda grammar (`agenda-parse.ts`): anything this
 * rejects, the grammar rejects too, so the trigger may skip loading the chunk.
 *
 * Deliberately looser than the spec's draft (`…|midnight)\b`, `\s?`): a
 * trailing `\b` rejected `9:00am` (no boundary between `0` and `a`), and a
 * single optional space rejected `9  am`, both of which the grammar accepts.
 * A superset may only err towards loading the chunk.
 */
export const AGENDA_PREFILTER = /^\s*(?:(?:at|from)\s+)?(?:\d{1,2}[:.]\d{2}|\d{1,2}\s*[ap]\.?m|noon|midnight)/i

/** Every `<li>` of every top-level `ul` / `ol` under `root`, in document order, lazily. */
export function* eachTopLevelListItem(root: ParentNode): Generator<HTMLLIElement> {
  const lists = root.querySelectorAll('ul, ol')
  for (let i = 0; i < lists.length; i++) {
    const list = lists[i]
    if (!isTopLevel(list, AGENDA_NESTING_SELECTOR)) continue
    for (let child = list.firstElementChild; child; child = child.nextElementSibling) {
      if (child.tagName === 'LI') yield child as HTMLLIElement
    }
  }
}

/** {@link eachTopLevelListItem}, collected. */
export function topLevelListItems(root: ParentNode): HTMLLIElement[] {
  return [...eachTopLevelListItem(root)]
}

/** How much of an item's start the prefilter needs to see. */
const PREFILTER_CHARS = 24

/**
 * The first few characters of an item's own text, stopping at a nested list.
 * Reads child by child, so an item holding a long outline costs no more than
 * a short one.
 */
function itemStart(li: Element): string {
  let text = ''
  for (let node = li.firstChild; node && text.length < PREFILTER_CHARS; node = node.nextSibling) {
    if (node.nodeType === 1 && /^(UL|OL)$/.test((node as Element).tagName)) break
    text += node.textContent ?? ''
  }
  return text
}

/**
 * True when some top-level item under `root` starts with something time-like.
 * Stops at the first one: a long note with an agenda near the top costs a
 * few items, not the whole document.
 */
export function mayHaveAgenda(root: ParentNode): boolean {
  for (const li of eachTopLevelListItem(root)) {
    if (AGENDA_PREFILTER.test(itemStart(li))) return true
  }
  return false
}
