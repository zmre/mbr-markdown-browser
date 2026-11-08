/**
 * The QuickLook day calendar's stylesheet, injected once into `<head>`.
 *
 * Every colour is a theme token (`--mbr-cal-*` and Pico's own, inlined into the
 * preview with theme.css), with a fallback for a repository's older custom
 * theme, so light and dark follow the preview like the rest of the page.
 * Placement is `data-placement` on the calendar (`model.ts`'s `placementFor`),
 * plus `mbr-ql-cal-column` on `<body>` to make room for the column.
 */
import { COLUMN_WIDTH } from './model.js'

/** `id` of the injected `<style>`, so a second run does not add another. */
export const STYLE_ID = 'mbr-ql-cal-style'

/** `<body>` class while the calendar is a right column. */
export const BODY_COLUMN_CLASS = 'mbr-ql-cal-column'

export const QUICKLOOK_CALENDAR_CSS = `
body.${BODY_COLUMN_CLASS} {
  padding-right: calc(${COLUMN_WIDTH}px + 2rem);
}
/* Pico's .container has no padding above 576px, relying on its max-width
   leaving slack; beside the column there may be none. */
body.${BODY_COLUMN_CLASS} > header,
body.${BODY_COLUMN_CLASS} > main {
  padding-inline: var(--pico-spacing, 1rem);
}
.mbr-ql-cal {
  --mbr-ql-cal-gutter: 3.2rem;
  box-sizing: border-box;
  font-size: 0.875rem;
  line-height: 1.3;
  color: var(--pico-color);
  background: var(--pico-card-background-color, var(--pico-background-color, transparent));
  border: 1px solid var(--pico-muted-border-color, #e0e0e0);
  border-radius: var(--pico-border-radius, 0.5rem);
  padding: 0.75rem 0.7rem 0.9rem;
}
.mbr-ql-cal[data-placement='column'] {
  position: fixed;
  top: 1rem;
  right: 1rem;
  width: ${COLUMN_WIDTH}px;
  max-height: calc(100vh - 2rem);
  overflow-y: auto;
  overscroll-behavior: contain;
  z-index: 10;
}
.mbr-ql-cal[data-placement='top'] {
  max-width: 26rem;
  margin: 0 0 var(--pico-block-spacing-vertical, 1.5rem);
}
.mbr-ql-cal-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.5rem;
  margin-bottom: 0.65rem;
}
.mbr-ql-cal-head time {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.mbr-ql-cal-weekday {
  font-size: 1.05rem;
  font-weight: 700;
}
.mbr-ql-cal-date {
  color: var(--pico-muted-color);
}
.mbr-ql-cal-today-badge {
  flex: none;
  padding: 0.05rem 0.45rem;
  border-radius: 999px;
  font-size: 0.72rem;
  font-weight: 600;
  color: var(--pico-primary-inverse, #fff);
  background: var(--mbr-cal-today, var(--pico-primary, #0172ad));
}
.mbr-ql-cal-month-title {
  font-weight: 600;
  font-size: 0.82rem;
  margin-bottom: 0.25rem;
}
.mbr-ql-cal-row {
  display: grid;
  grid-template-columns: repeat(7, minmax(0, 1fr));
}
.mbr-ql-cal-grid.with-weeks .mbr-ql-cal-row {
  grid-template-columns: 1.6rem repeat(7, minmax(0, 1fr));
}
.mbr-ql-cal-dow {
  text-align: center;
  font-size: 0.68rem;
  color: var(--pico-muted-color);
  padding-bottom: 0.15rem;
}
.mbr-ql-cal-wk {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.66rem;
  color: var(--pico-muted-color);
  opacity: 0.7;
}
.mbr-ql-cal-day {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 1.85rem;
  margin: 1px;
  border-radius: 6px;
  font-variant-numeric: tabular-nums;
  font-size: 0.78rem;
}
.mbr-ql-cal-day.outside {
  color: var(--pico-muted-color);
  opacity: 0.5;
}
.mbr-ql-cal-day.today {
  box-shadow: inset 0 0 0 2px var(--mbr-cal-today, var(--pico-primary, #0172ad));
}
.mbr-ql-cal-day.selected {
  background: var(--mbr-cal-selected, var(--pico-primary, #0172ad));
  color: var(--pico-primary-inverse, #fff);
  font-weight: 700;
}
.mbr-ql-cal-agenda {
  margin-top: 0.8rem;
}
.mbr-ql-cal-scroll {
  max-height: 70vh;
  overflow-y: auto;
  padding-top: 0.6em;
}
.mbr-ql-cal[data-placement='top'] .mbr-ql-cal-scroll {
  max-height: 22rem;
}
.mbr-ql-cal-timeline {
  position: relative;
}
.mbr-ql-cal-hour {
  position: absolute;
  left: 0;
  right: 0;
  border-top: 1px solid var(--pico-muted-border-color, #e0e0e0);
}
.mbr-ql-cal-hour span {
  position: absolute;
  top: -0.55em;
  left: 0;
  width: calc(var(--mbr-ql-cal-gutter) - 0.3rem);
  padding-right: 0.3rem;
  background: var(--pico-card-background-color, var(--pico-background-color, #fff));
  font-size: 0.66rem;
  line-height: 1.1;
  white-space: nowrap;
  color: var(--pico-muted-color);
  font-variant-numeric: tabular-nums;
}
.mbr-ql-cal-block {
  position: absolute;
  left: calc(var(--mbr-ql-cal-gutter) + (100% - var(--mbr-ql-cal-gutter)) * var(--col) / var(--cols));
  width: calc((100% - var(--mbr-ql-cal-gutter)) / var(--cols) - 2px);
  box-sizing: border-box;
  overflow: hidden;
  padding: 1px 4px;
  border-left: 3px solid var(--mbr-cal-today, var(--pico-primary, #0172ad));
  border-radius: 4px;
  background: var(--mbr-cal-daily-bg, rgba(1, 114, 173, 0.12));
  font-size: 0.74rem;
  line-height: 1.25;
}
.mbr-ql-cal-block.no-end {
  border-bottom: 1px dashed var(--pico-muted-color);
}
.mbr-ql-cal-span {
  font-variant-numeric: tabular-nums;
  color: var(--pico-muted-color);
}
.mbr-ql-cal-title {
  font-weight: 600;
}
.mbr-ql-cal-detail {
  display: block;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  color: var(--pico-muted-color);
}
.mbr-ql-cal-check {
  display: inline-block;
  width: 0.62rem;
  height: 0.62rem;
  margin-right: 0.25rem;
  vertical-align: -1px;
  border: 1.5px solid currentColor;
  border-radius: 2px;
}
.mbr-ql-cal-block.done .mbr-ql-cal-check {
  background: currentColor;
}
.mbr-ql-cal-block.done .mbr-ql-cal-title,
.mbr-ql-cal-block.canceled .mbr-ql-cal-title {
  text-decoration: line-through;
}
.mbr-ql-cal-block.canceled {
  opacity: 0.55;
}
.mbr-ql-cal-now {
  position: absolute;
  left: var(--mbr-ql-cal-gutter);
  right: 0;
  height: 0;
  border-top: 2px solid var(--mbr-cal-overdue, #d32f2f);
  z-index: 3;
}
.mbr-ql-cal-now::before {
  content: '';
  position: absolute;
  left: -4px;
  top: -5px;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--mbr-cal-overdue, #d32f2f);
}
.mbr-ql-cal-sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}
@media print {
  body.${BODY_COLUMN_CLASS} {
    padding-right: 0;
  }
  .mbr-ql-cal[data-placement='column'] {
    position: static;
    width: auto;
    max-height: none;
  }
}
`
