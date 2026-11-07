/**
 * Entry of the lazy calendar chunk, `mbr-calendar.min.js`
 * (`vite.calendar.config.ts`), imported by `<mbr-calendar>` through
 * `calendar-chunk.ts`'s single-flight `loadCalendarChunk()`.
 *
 * Importing it registers `<mbr-calendar-month>`, `<mbr-calendar-companion>`,
 * the calendar overlay (`<mbr-calendar-overlay>`) and the large grids
 * (`<mbr-calendar-month-grid>`, `<mbr-calendar-week-grid>`); the named
 * exports are what the trigger calls. The tasks chunk (phase 6) creates the
 * grid elements by tag name after `loadCalendarChunk()` (it must not bundle
 * them: a second `customElements.define` throws), and may reuse the key map
 * (`overlayCommand`, `moveDay`) and `overflowSplit` re-exported here.
 *
 * **Nothing in this chunk may import `shared.ts`, `task-toggle.ts`, the
 * trigger or any other stateful main-bundle module**: everything stateful is
 * injected as element properties.
 */
import './mbr-calendar-month.js'
import './mbr-calendar-companion.js'
import './mbr-calendar-grid.js'
import './mbr-calendar-overlay.js'

export { enhancePage, type EnhancePageOptions } from './agenda-page.js'
export { moveDay, overflowSplit, overlayCommand } from './overlay-model.js'
