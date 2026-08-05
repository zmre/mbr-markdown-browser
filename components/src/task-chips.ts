/**
 * Redrawing a task's `@done(...)` chip in the rendered document.
 *
 * A task write used to be followed by a live reload, and that reload was the
 * only thing that could draw a freshly stamped `@done(...)`. Writes now
 * suppress their own reload — the page must survive one, because the edit token
 * lives in memory and a reload would take it with it (`edit-token.ts`) — so the
 * writer has to render that one chip itself.
 *
 * It really is only that one chip. `tasks::set_status` changes exactly two
 * things about a line: the status marker, and the `@done(...)` stamp it adds or
 * removes. Priority, tags, `@due(...)` and the trailing move marker are
 * untouched, and the display text is unaffected because the stamp is stripped
 * out of it either way. So the whole difference between "patched in place" and
 * "re-rendered by the server" is this file plus `applyCheckboxStatus`.
 *
 * # Why the parsing and formatting are duplicated here
 *
 * The input is the **source line** the server just wrote back
 * (`server.rs::TaskToggleResponse::text`), not the naive datetimes
 * `POST /.mbr/tasks` sends, so `tasks/task-format.ts` — which parses the wire
 * form for the panel — cannot read it. The grammar mirrored below is
 * `tasks.rs`'s `DATE_ANNOTATION` + `DATETIME`, and the output mirrors
 * `html.rs::push_task_time`.
 *
 * The label is deliberately built from a hard-coded English month table rather
 * than `toLocaleDateString`, which is the opposite of the choice
 * `task-format.ts` makes for the panel. The panel renders its own view and can
 * use the reader's locale; this chip is dropped in among sibling chips the
 * *server* rendered in English, and has to match them or the same list shows
 * two date formats until the next render. Hand-formatting also avoids ICU's
 * narrow no-break space before AM/PM, which `%-I:%M %p` does not emit.
 */

/** `@done(...)` payloads, mirroring the `done` half of `tasks::DATE_ANNOTATION`. */
const DONE_ANNOTATION = /@done\(([^)]*)\)/g

/** `tasks::DATETIME`: `YYYY-MM-DD`, optionally ` HH:MM`, optionally ` AM/PM`. */
const SOURCE_DATETIME =
  /^(\d{4})-(\d{2})-(\d{2})(?:[ \t]+(\d{1,2}):(\d{2})(?:[ \t]*([AaPp][Mm]))?)?$/

/** Month abbreviations as chrono's `%b` writes them. */
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

/** The rendered `<time>` chip for a completion stamp. */
export interface TaskChip {
  /** The `datetime` attribute: `2026-08-04` or `2026-08-04T22:16`. */
  datetime: string
  /** The visible label: `Aug 4` or `Aug 4, 10:16 PM`. */
  label: string
}

/** A parsed `@done(...)` payload, in the fields `html.rs` formats from. */
interface Stamp {
  year: number
  month: number
  day: number
  hour: number
  minute: number
  hasTime: boolean
}

/** Whether `day` exists in `month` of `year`, so `2026-02-30` is rejected. */
function isRealDate(year: number, month: number, day: number): boolean {
  if (month < 1 || month > 12 || day < 1) return false
  return day <= new Date(year, month, 0).getDate()
}

/**
 * Parse one `@done(...)` payload, mirroring `tasks::parse_datetime`.
 *
 * Returns `null` for anything that is not a datetime — which is not merely a
 * formatting question: an unparseable payload is not an annotation at all on
 * the Rust side, so it stays in the task's display text and never becomes a
 * chip.
 */
function parseStamp(value: string): Stamp | null {
  const match = SOURCE_DATETIME.exec(value.trim())
  if (!match) return null

  const year = Number(match[1])
  const month = Number(match[2])
  const day = Number(match[3])
  if (!isRealDate(year, month, day)) return null

  if (match[4] === undefined) {
    return { year, month, day, hour: 0, minute: 0, hasTime: false }
  }
  const hour = Number(match[4])
  const minute = Number(match[5])
  if (minute > 59) return null

  const meridiem = match[6]?.[0]?.toLowerCase()
  let hour24 = hour
  if (meridiem !== undefined) {
    // A 12-hour clock only: `14:00 PM` is not a time anybody meant.
    if (hour < 1 || hour > 12) return null
    hour24 = meridiem === 'a' ? hour % 12 : (hour % 12) + 12
  }
  if (hour24 > 23) return null

  return { year, month, day, hour: hour24, minute, hasTime: true }
}

/** Two-digit zero-padded, for the `datetime` attribute. */
function pad2(value: number): string {
  return String(value).padStart(2, '0')
}

/** Render a stamp the way `html.rs::push_task_time` does. */
function chipFor(stamp: Stamp): TaskChip {
  const date = `${stamp.year}-${pad2(stamp.month)}-${pad2(stamp.day)}`
  const day = `${MONTHS[stamp.month - 1]} ${stamp.day}`
  if (!stamp.hasTime) return { datetime: date, label: day }

  // `%-I:%M %p`: a 12-hour clock where midnight and noon are both 12.
  const hour12 = stamp.hour % 12 === 0 ? 12 : stamp.hour % 12
  const suffix = stamp.hour < 12 ? 'AM' : 'PM'
  return {
    datetime: `${date}T${pad2(stamp.hour)}:${pad2(stamp.minute)}`,
    label: `${day}, ${hour12}:${pad2(stamp.minute)} ${suffix}`,
  }
}

/**
 * The completion chip a task's **source line** should carry, or `null` when it
 * carries none.
 *
 * The first parseable `@done(...)` wins, matching `tasks::strip_annotations`.
 */
export function doneChipFor(sourceLine: string): TaskChip | null {
  DONE_ANNOTATION.lastIndex = 0
  for (
    let match = DONE_ANNOTATION.exec(sourceLine);
    match !== null;
    match = DONE_ANNOTATION.exec(sourceLine)
  ) {
    const stamp = parseStamp(match[1] ?? '')
    if (stamp) {
      DONE_ANNOTATION.lastIndex = 0
      return chipFor(stamp)
    }
  }
  return null
}

/** Chip classes in the order `html.rs::task_annotations_html` emits them. */
const CHIP_CLASSES = [
  'mbr-task-pri',
  'mbr-task-tag',
  'mbr-task-due',
  'mbr-task-completed',
  'mbr-task-moved',
] as const

/** Class of the completion chip, and its rank in {@link CHIP_CLASSES}. */
const DONE_CLASS = 'mbr-task-completed'
const DONE_RANK = CHIP_CLASSES.indexOf(DONE_CLASS)

/** Where `element` sits in the chip run, or `null` when it is not a chip. */
function chipRank(element: Element): number | null {
  for (let rank = 0; rank < CHIP_CLASSES.length; rank++) {
    if (element.classList.contains(CHIP_CLASSES[rank])) return rank
  }
  return null
}

/**
 * The `<span class="mbr-task-text">` belonging to `input`.
 *
 * The renderer emits the span immediately after the checkbox, so the first
 * match under the shared parent is this task's own — a nested subtask list
 * comes later in the item.
 */
function taskTextSpan(input: HTMLInputElement): HTMLElement | null {
  return input.parentElement?.querySelector<HTMLElement>('.mbr-task-text') ?? null
}

/** Drop a chip along with the single space the renderer put in front of it. */
function removeChip(chip: Element): void {
  const before = chip.previousSibling
  chip.remove()
  if (before?.nodeType === Node.TEXT_NODE && (before.textContent ?? '').trim() === '') {
    before.parentNode?.removeChild(before)
  }
}

/**
 * Make the document's completion chip agree with `sourceLine`.
 *
 * Adds, updates or removes the `<time class="mbr-task-completed">` next to
 * `input`, keeping the chip order and the single separating space that
 * `task_annotations_html` produces. A no-op when the markup has no text span to
 * hang chips off — a raw-HTML render, or a checkbox some other template drew.
 */
export function syncDoneChip(input: HTMLInputElement, sourceLine: string): void {
  const span = taskTextSpan(input)
  if (!span) return

  const wanted = doneChipFor(sourceLine)

  // Walk the run of chips following the text span: find any existing
  // completion chip, and the last chip that should stay in front of it.
  let existing: Element | null = null
  let anchor: Element = span
  for (let element = span.nextElementSibling; element; element = element.nextElementSibling) {
    const rank = chipRank(element)
    if (rank === null) break
    if (rank === DONE_RANK) existing = element
    if (rank < DONE_RANK) anchor = element
  }

  if (!wanted) {
    if (existing) removeChip(existing)
    return
  }
  if (existing) {
    existing.setAttribute('datetime', wanted.datetime)
    existing.textContent = wanted.label
    return
  }

  const chip = document.createElement('time')
  chip.className = DONE_CLASS
  chip.setAttribute('datetime', wanted.datetime)
  chip.textContent = wanted.label
  // A text node for the space, so the chip does not butt up against whatever
  // precedes it — `task_annotations_html` writes `" <time …>"`.
  anchor.after(' ', chip)
}
