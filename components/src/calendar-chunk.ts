/**
 * Single-flight loader for the lazy `mbr-calendar.min.js` chunk, which
 * registers `<mbr-calendar-companion>`, `<mbr-calendar-month>`, the calendar
 * overlay and the large month / week grids.
 *
 * MAIN BUNDLE. `graph-chunk.ts`'s pattern: `<mbr-calendar>` loads it, and
 * the task panel (design/calendar.md §8.2) receives `loadCalendarChunk` as an
 * injected service and creates the grid elements by tag name with type-only
 * imports — a chunk that bundled its own copy of an element would hit a
 * double `customElements.define`. The grids' pure helpers (ranges, titles,
 * the key map) are `calendar/month-grid.ts`, which any bundle may import.
 *
 * The URL is computed against the asset base, so it works in server mode and
 * in static builds deployed at any depth.
 */
import { getMbrAssetBase } from './dynamic-loader.js'
import type { EnhancePageOptions } from './calendar/agenda-page.js'
import type { AgendaEntry } from './calendar/agenda-parse.js'

/** What the chunk exports (`calendar/index.ts`) besides its elements. */
export interface CalendarChunk {
  enhancePage(options: EnhancePageOptions): { entries: AgendaEntry[]; dispose: () => void }
}

const defaultImporter = (): Promise<unknown> => {
  const url = new URL(getMbrAssetBase() + 'components/mbr-calendar.min.js', document.baseURI).href
  return import(/* @vite-ignore */ url)
}

let importCalendarChunk: () => Promise<unknown> = defaultImporter

/** Shared once-per-page promise; `null` when the import failed. */
let calendarChunkPromise: Promise<CalendarChunk | null> | null = null

/** Test hook: replace the chunk importer (module-level seam). */
export function setCalendarChunkImporter(importer: () => Promise<unknown>): void {
  importCalendarChunk = importer
  calendarChunkPromise = null
}

/**
 * Loads the calendar chunk once per page. Never rejects: resolves `null` when
 * the import failed, so callers simply leave the calendar out.
 */
export function loadCalendarChunk(): Promise<CalendarChunk | null> {
  if (!calendarChunkPromise) {
    calendarChunkPromise = importCalendarChunk()
      .then((module) => module as CalendarChunk)
      .catch((err: unknown) => {
        console.warn('Failed to load the calendar chunk:', err)
        return null
      })
  }
  return calendarChunkPromise
}
