import { describe, expect, it } from 'vitest'

/**
 * Every source under `src/`, raw, so the chunk's import graph can be walked
 * without building it (the same `import.meta.glob` idea as edit-token.test.ts).
 */
const SOURCES = import.meta.glob('./**/*.ts', { query: '?raw', import: 'default', eager: true }) as Record<
  string,
  string
>

/** `./x.ts` keys, normalized to `src/`-relative paths. */
const byPath = new Map(Object.entries(SOURCES).map(([k, v]) => [k.replace(/^\.\//, ''), v]))

/**
 * Value (not `import type` / `export type`) imports and re-exports of a
 * module, resolved to `src/`-relative paths.
 */
function valueImports(path: string): string[] {
  const source = byPath.get(path) ?? ''
  const dir = path.split('/').slice(0, -1)
  const out: string[] = []
  for (const m of source.matchAll(/^(?:import|export)\s+(?!type\b)(?:[^'"]*?from\s+)?['"](\.[^'"]+)['"]/gm)) {
    const parts = [...dir]
    for (const seg of m[1].split('/')) {
      if (seg === '..') parts.pop()
      else if (seg !== '.') parts.push(seg)
    }
    out.push(parts.join('/').replace(/\.js$/, '.ts'))
  }
  return out
}

/** Every module an entry pulls in. */
function chunkModules(entry = 'calendar/index.ts'): Set<string> {
  const seen = new Set<string>()
  const queue = [entry]
  while (queue.length) {
    const next = queue.pop()!
    if (seen.has(next)) continue
    seen.add(next)
    queue.push(...valueImports(next))
  }
  return seen
}

describe('the calendar chunk (calendar/index.ts)', () => {
  it('bundles no stateful main-bundle module', () => {
    const modules = chunkModules()
    expect(modules.has('calendar/mbr-calendar-companion.ts')).toBe(true)
    expect(modules.has('calendar/agenda-parse.ts')).toBe(true)
    expect(modules.has('calendar/mbr-calendar-overlay.ts')).toBe(true)
    expect(modules.has('calendar/mbr-calendar-grid.ts')).toBe(true)
    for (const forbidden of [
      'shared.ts',
      'task-toggle.ts',
      'edit-token.ts',
      'mbr-calendar.ts',
      'calendar-chunk.ts',
      'calendar/periodic-nav.ts',
      'calendar/companion-pref.ts',
      'mbr-task-doc.ts',
      'mbr-keys.ts',
    ]) {
      expect(modules.has(forbidden), forbidden).toBe(false)
    }
  })

  it('stays out of the main bundle: the triggers reach the overlay only by tag name', () => {
    for (const entry of ['mbr-calendar.ts', 'mbr-browse.ts', 'mbr-browse-single.ts', 'mbr-search.ts']) {
      const modules = chunkModules(entry)
      for (const chunkOnly of [
        'calendar/mbr-calendar-overlay.ts',
        'calendar/mbr-calendar-grid.ts',
        'calendar/overlay-model.ts',
        'calendar/mbr-calendar-companion.ts',
      ]) {
        expect(modules.has(chunkOnly), `${entry} → ${chunkOnly}`).toBe(false)
      }
    }
  })
})

describe('the tasks chunk (tasks/index.ts)', () => {
  it('reaches the calendar grids only by tag name, through the injected loader', () => {
    const modules = chunkModules('tasks/index.ts')
    expect(modules.has('tasks/mbr-tasks-panel.ts')).toBe(true)
    expect(modules.has('tasks/task-grid.ts')).toBe(true)
    for (const calendarOnly of [
      'calendar/index.ts',
      'calendar/mbr-calendar-grid.ts',
      'calendar/mbr-calendar-month.ts',
      'calendar/mbr-calendar-companion.ts',
      'calendar/mbr-calendar-overlay.ts',
      'calendar/overlay-model.ts',
      'calendar-chunk.ts',
      'shared.ts',
      'task-toggle.ts',
    ]) {
      expect(modules.has(calendarOnly), calendarOnly).toBe(false)
    }
  })

  it('defines no calendar element of its own', () => {
    for (const path of chunkModules('tasks/index.ts')) {
      const source = byPath.get(path) ?? ''
      expect(source, path).not.toMatch(/customElements\.define\(\s*['"]mbr-calendar/)
      expect(source, path).not.toMatch(/@customElement\(\s*['"]mbr-calendar/)
    }
  })
})


describe('the search-extras chunk (search-extras/index.ts)', () => {
  it('reads queries with the pure date-facet tokenizer, and nothing stateful', () => {
    const modules = chunkModules('search-extras/index.ts')
    expect(modules.has('search-extras/facets.ts')).toBe(true)
    expect(modules.has('calendar/date-facets.ts')).toBe(true)
    for (const forbidden of ['shared.ts', 'task-toggle.ts', 'edit-token.ts', 'mbr-search.ts', 'calendar/periodic-nav.ts']) {
      expect(modules.has(forbidden), forbidden).toBe(false)
    }
  })
})
