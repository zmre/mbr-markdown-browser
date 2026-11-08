/**
 * Puts the day calendar into a QuickLook preview, or does nothing.
 *
 * Reads the payload `quicklook.rs` wrote, runs the agenda grammar over the
 * rendered note (`main#wrapper`), and draws only when there is a date and at
 * least one entry. The page itself is enhanced by the calendar chunk's own
 * `enhancePage` (time chips when `agenda_chips` is on; on today's note the
 * now/next item marks), whose minute tick also moves the current-time line.
 */
import { agendaEntries } from '../calendar/agenda-parse.js'
import { enhancePage } from '../calendar/agenda-page.js'
import { todayKey } from '../calendar/note-date.js'
import { placementFor, QUICKLOOK_CALENDAR_GLOBAL, readPayload, type Placement } from './model.js'
import { renderDayCalendar, updateNowLine } from './render.js'
import { BODY_COLUMN_CLASS, QUICKLOOK_CALENDAR_CSS, STYLE_ID } from './styles.js'

export interface MountOptions {
  readonly doc?: Document
  readonly win?: Window
  /** The clock; tests pass a fixed one. */
  readonly now?: () => Date
}

export interface MountedCalendar {
  readonly calendar: HTMLElement
  readonly placement: () => Placement
  readonly dispose: () => void
}

/** Sets the calendar's placement for the panel's current width. */
function applyPlacement(doc: Document, calendar: HTMLElement, width: number): Placement {
  const placement = placementFor(width)
  calendar.dataset.placement = placement
  doc.body.classList.toggle(BODY_COLUMN_CLASS, placement === 'column')
  return placement
}

/** Mounts the calendar; `null` when this preview should not have one. */
export function mountQuickLookCalendar(options: MountOptions = {}): MountedCalendar | null {
  const doc = options.doc ?? document
  const win = options.win ?? window
  const clock = options.now ?? (() => new Date())

  const payload = readPayload((win as unknown as Record<string, unknown>)[QUICKLOOK_CALENDAR_GLOBAL])
  const main = doc.querySelector<HTMLElement>('main#wrapper')
  if (!payload || !main) return null
  const entries = agendaEntries(main)
  if (entries.length === 0) return null

  if (!doc.getElementById(STYLE_ID)) {
    const style = doc.createElement('style')
    style.id = STYLE_ID
    style.textContent = QUICKLOOK_CALENDAR_CSS
    doc.head.append(style)
  }

  const locale = win.navigator?.language || 'en-US'
  const calendar = renderDayCalendar(doc, { payload, entries, today: todayKey(clock()), locale })
  main.prepend(calendar)

  let placement = applyPlacement(doc, calendar, doc.documentElement.clientWidth || win.innerWidth)
  const onResize = (): void => {
    placement = applyPlacement(doc, calendar, doc.documentElement.clientWidth || win.innerWidth)
  }
  win.addEventListener('resize', onResize)

  // The current-time line and the now/next marks only while the note's day
  // is today: a preview left open past midnight drops them.
  const page = enhancePage({
    root: main,
    entries,
    agendaChips: payload.agendaChips,
    pageDay: payload.date,
    now: clock,
    onTick: (minutes) => updateNowLine(calendar, minutes),
  })

  return {
    calendar,
    placement: () => placement,
    dispose: () => {
      page.dispose()
      win.removeEventListener('resize', onResize)
      doc.body.classList.remove(BODY_COLUMN_CLASS)
      calendar.remove()
    },
  }
}
