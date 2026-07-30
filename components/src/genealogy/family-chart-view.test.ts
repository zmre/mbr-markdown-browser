/**
 * Tests for the family-chart view's inconsistent-data guard.
 *
 * The happy path is deliberately NOT exercised here: the library needs real
 * layout metrics, and `selector.test.ts` already stubs `chart.mount` for the
 * orchestration tests. What matters is that a cyclic hierarchy never reaches
 * `createChart()`.
 */
import { describe, it, expect, vi, afterEach } from 'vitest'
import {
  buildRegistry,
  type GraphEdge,
  type RelationshipGraph,
} from '../graph/relationship-graph.js'
import { GENEALOGY_TYPES, genealogyNotes } from '../graph/test-fixtures.js'
import type { GenealogyContext } from './chart-registry.js'
import { findParentChildCycle, toFamilyChartData } from './family-chart-data.js'
import { familyChartType } from './family-chart-view.js'

const hier = (from: string, to: string): GraphEdge => ({
  from,
  to,
  kind: 'hierarchical',
  relType: 'child',
  label: '',
})

/**
 * A graph built by hand so it violates `buildRelationshipGraph`'s acyclic
 * invariant — i.e. exactly the upstream regression the view guards against.
 */
function cyclicContext(): GenealogyContext {
  const graph: RelationshipGraph = {
    focus: '/p/a/',
    nodes: [
      { urlPath: '/p/a/', title: 'Ann', isFocus: true },
      { urlPath: '/p/b/', title: 'Bob', isFocus: false },
    ],
    edges: [hier('/p/a/', '/p/b/'), hier('/p/b/', '/p/a/')],
  }
  return {
    graph,
    notesByPath: genealogyNotes(),
    registry: buildRegistry(GENEALOGY_TYPES),
    focusPath: graph.focus,
    resolveUrl: (p) => p,
    navigate: vi.fn(),
  }
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('UNIT familyChartType cycle guard', () => {
  it('renders an error card instead of calling createChart', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const container = document.createElement('div')
    document.body.appendChild(container)

    // Assert the detector fires BEFORE mounting. If `findParentChildCycle` ever
    // regressed, `mount()` would reach `createChart` → `d3.hierarchy()`, which
    // has no cycle detection and would OOM the test worker instead of failing.
    expect(findParentChildCycle(toFamilyChartData(cyclicContext().graph).data)).not.toBeNull()

    const instance = familyChartType.mount(container, cyclicContext())

    const card = container.querySelector('.mbr-f3-error')
    expect(card).not.toBeNull()
    expect(card!.getAttribute('role')).toBe('alert')
    // Both looping notes are named so the author knows where to look.
    expect([...card!.querySelectorAll('li')].map((li) => li.textContent)).toEqual([
      '/p/a/',
      '/p/b/',
    ])
    // No chart was built.
    expect(container.querySelector('svg')).toBeNull()
    expect(warn).toHaveBeenCalledOnce()

    instance.destroy()
    expect(container.querySelector('.mbr-f3-error')).toBeNull()
    container.remove()
  })
})
