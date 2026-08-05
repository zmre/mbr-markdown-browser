/**
 * Entry point for the lazy `mbr-tasks.min.js` chunk (built by
 * vite.tasks.config.ts; loaded on demand by the `<mbr-tasks>` trigger the first
 * time the task browser is opened).
 *
 * Importing this module registers the `<mbr-tasks-panel>` custom element.
 *
 * IMPORTANT: nothing in this chunk may import stateful main-bundle modules
 * (`shared.ts`, `graph/links-cache.ts`, …) — those hold top-level fetches and
 * caches that would re-run inside the chunk. Pure modules (`safe-href.ts`) are
 * fine; everything stateful arrives through element properties set by the
 * trigger.
 */
export { MbrTasksPanelElement } from './mbr-tasks-panel.js'
export type {
  DueFilter,
  FolderFacet,
  TaskGroup,
  TaskHit,
  TaskMode,
  TaskPriority,
  TaskQueryRequest,
  TaskQueryResponse,
  TaskStatus,
} from './types.js'
