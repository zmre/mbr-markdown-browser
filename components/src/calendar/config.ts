/**
 * The `[calendar]` configuration as the browser sees it
 * (`window.__MBR_CONFIG__.calendar`, from `crates/mbr-core/src/calendar.rs`'s
 * `CalendarConfig`).
 *
 * Pure, no imports: `shared.ts`'s `getCalendarConfig()` reads the
 * page's value through `calendarConfigFrom`, and a lazy chunk receives the
 * result as a property. A missing or malformed value (a custom `_head.html`
 * predating the option, an error page) yields the defaults, key by key.
 */

export type WeekStart = 'locale' | 'monday' | 'sunday' | 'saturday'
export type CompanionMode = 'auto' | 'on-demand' | 'off'

export interface CalendarConfig {
  /** Master switch for periodic navigation, the keys and the calendar UI. */
  readonly enabled: boolean
  /** Frontmatter fields that date a note (already applied). */
  readonly dateFields: readonly string[]
  /** First column of every grid. */
  readonly weekStart: WeekStart
  /** ISO week-number column; `null` = on when the repository has week notes. */
  readonly weekNumbers: boolean | null
  /** When the daily-note companion shows. */
  readonly companion: CompanionMode
  /** Inline time chips on time-prefixed list items. */
  readonly agendaChips: boolean
}

export const DEFAULT_CALENDAR_CONFIG: CalendarConfig = Object.freeze({
  enabled: true,
  dateFields: ['date', 'created'],
  weekStart: 'locale',
  weekNumbers: null,
  companion: 'auto',
  agendaChips: true,
})

export const WEEK_STARTS: readonly WeekStart[] = ['locale', 'monday', 'sunday', 'saturday']
const COMPANION_MODES: readonly CompanionMode[] = ['auto', 'on-demand', 'off']

const isStringArray = (v: unknown): v is string[] => Array.isArray(v) && v.every((s) => typeof s === 'string')

/** `v` when it is one of `allowed`, else `fallback`. */
export const oneOf = <T extends string>(v: unknown, allowed: readonly T[], fallback: T): T =>
  typeof v === 'string' && (allowed as readonly string[]).includes(v) ? (v as T) : fallback

/** Validates the page's raw snake_case object, defaulting key by key. */
export function calendarConfigFrom(raw: unknown): CalendarConfig {
  const r = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>
  const d = DEFAULT_CALENDAR_CONFIG
  return {
    enabled: typeof r.enabled === 'boolean' ? r.enabled : d.enabled,
    dateFields: isStringArray(r.date_fields) ? r.date_fields : d.dateFields,
    weekStart: oneOf(r.week_start, WEEK_STARTS, d.weekStart),
    weekNumbers: typeof r.week_numbers === 'boolean' ? r.week_numbers : null,
    companion: oneOf(r.companion, COMPANION_MODES, d.companion),
    agendaChips: typeof r.agenda_chips === 'boolean' ? r.agenda_chips : d.agendaChips,
  }
}
