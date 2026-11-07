/**
 * Entry of `mbr-quicklook-calendar.min.js` (`vite.quicklook-calendar.config.ts`),
 * a self-contained IIFE that `crates/mbr-ffi/src/quicklook.rs` inlines into a
 * QuickLook preview after `window.__MBR_QUICKLOOK_CALENDAR__` (design/calendar.md
 * §4.7). QuickLook only: never served, never in a static build, never in a
 * thumbnail.
 */
import { mountQuickLookCalendar } from './mount.js'

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', () => mountQuickLookCalendar(), { once: true })
} else {
  mountQuickLookCalendar()
}
