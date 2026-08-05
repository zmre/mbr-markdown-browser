/**
 * Pure formatting and date arithmetic for the task panel.
 *
 * Kept free of DOM and of Lit so it can be unit-tested directly, and free of
 * main-bundle imports so the chunk carries no shared state.
 *
 * # Why the dates are parsed by hand
 *
 * The server sends naive local values with no timezone: `2026-08-05T00:00:00`
 * for a datetime, `2026-08-05` for a date. `new Date()` treats those two forms
 * *differently* — ECMA-262 reads a date-time form as local but a date-only form
 * as UTC — so `new Date('2026-08-05')` is the 4th of August at 17:00 in
 * California. Every date in this module therefore goes through {@link parseNaive},
 * which always builds a local `Date`.
 */

/** `YYYY-MM-DD` with an optional `THH:MM[:SS]` tail, anchored. */
const NAIVE_PATTERN = /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2}))?)?$/

/**
 * Parse a naive server date or datetime into a **local** `Date`.
 *
 * Returns `null` for `null`, for a non-string, and for anything that does not
 * match the grammar — a malformed value must not become an `Invalid Date` that
 * silently renders as "NaN".
 */
export function parseNaive(value: string | null | undefined): Date | null {
  if (typeof value !== 'string') return null
  const m = NAIVE_PATTERN.exec(value)
  if (!m) return null
  const date = new Date(
    Number(m[1]),
    Number(m[2]) - 1,
    Number(m[3]),
    Number(m[4] ?? 0),
    Number(m[5] ?? 0),
    Number(m[6] ?? 0)
  )
  // Reject impossible calendar dates (`2026-02-30`), which the Date constructor
  // silently rolls forward instead of rejecting.
  return date.getMonth() === Number(m[2]) - 1 && date.getDate() === Number(m[3]) ? date : null
}

/** Midnight local on the same calendar day as `date`. */
export function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate())
}

/**
 * Whether a due value has already passed, by **calendar day**.
 *
 * Mirrors `task_query::due_bucket` exactly: a task due today at 09:00 is still
 * "today" at 17:00 and only becomes overdue once the day has ended. Comparing
 * the instant instead would make every all-day task overdue the moment it was
 * written.
 *
 * The server deliberately does not mark overdue-ness in the rendered document
 * (a cached page would go stale overnight); the panel knows the real today, so
 * it does it here.
 */
export function isOverdue(due: string | null | undefined, today: Date): boolean {
  const parsed = parseNaive(due)
  if (!parsed) return false
  return startOfDay(parsed).getTime() < startOfDay(today).getTime()
}

/**
 * Format a due/done chip the way the document renderer does
 * (`html.rs::push_task_time`): `Aug 5`, or `Aug 5, 3:00 PM` with a time.
 *
 * The month name is localized rather than hard-coded English — the panel knows
 * the user's locale and the server deliberately does not (see the ISO labels in
 * `task_query::bucket_label`).
 */
export function formatChipLabel(
  value: string | null | undefined,
  hasTime: boolean,
  locale?: string
): string {
  const date = parseNaive(value)
  if (!date) return ''
  const day = date.toLocaleDateString(locale, { month: 'short', day: 'numeric' })
  if (!hasTime) return day
  const time = date.toLocaleTimeString(locale, { hour: 'numeric', minute: '2-digit' })
  return `${day}, ${time}`
}

/** The `datetime` attribute for a chip: `2026-08-05` or `2026-08-05T15:00`. */
export function chipDatetime(value: string | null | undefined, hasTime: boolean): string {
  if (typeof value !== 'string') return ''
  const m = NAIVE_PATTERN.exec(value)
  if (!m) return ''
  const day = `${m[1]}-${m[2]}-${m[3]}`
  return hasTime && m[4] ? `${day}T${m[4]}:${m[5]}` : day
}

/**
 * Heading for one date in the calendar mode's Upcoming section, e.g.
 * `Thu, Aug 20`. The year is added only when it is not the current one, so the
 * common case stays short.
 */
export function formatDateHeading(value: string | null | undefined, today: Date, locale?: string): string {
  const date = parseNaive(value)
  if (!date) return ''
  const options: Intl.DateTimeFormatOptions = { weekday: 'short', month: 'short', day: 'numeric' }
  if (date.getFullYear() !== today.getFullYear()) {
    options.year = 'numeric'
  }
  return date.toLocaleDateString(locale, options)
}

/**
 * Completion percentage for a progress bar, clamped to 0–100.
 *
 * A zero total yields 0 rather than `NaN`: the calendar mode's Overdue bucket
 * is sent as `0/0` on purpose (a backlog of missed deadlines has no meaningful
 * denominator), and a `width: NaN%` silently paints a full bar.
 */
export function progressPercent(done: number, total: number): number {
  if (!Number.isFinite(done) || !Number.isFinite(total) || total <= 0) return 0
  return Math.max(0, Math.min(100, Math.round((done / total) * 100)))
}

/** Accessible label and modifier class for a priority dot, or `null` for normal. */
export function priorityDot(priority: string): { className: string; label: string } | null {
  switch (priority) {
    case 'high':
      return { className: 'mbr-task-pri-high', label: 'High priority' }
    case 'urgent':
      return { className: 'mbr-task-pri-urgent', label: 'Urgent priority' }
    default:
      return null
  }
}
