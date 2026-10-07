import { describe, it, expect, beforeEach, afterEach, vi, type MockInstance } from 'vitest'
import { buildRegistry, buildRelationshipGraph } from '../graph/relationship-graph.js'
import {
  CONTACT_TYPES,
  GENEALOGY_TYPES,
  LEGACY_CONTACT_TYPES,
  buildSiteNotes,
  companyNotes,
  genealogyNotes,
} from '../graph/test-fixtures.js'
import { CHART_TYPES, DEFAULT_CHART_ID, type GenealogyContext } from './chart-registry.js'
import {
  CHART_STORAGE_KEY,
  chooseChartId,
  createSelector,
  readStoredChartId,
  resolveChartId,
  storeChartId,
} from './selector.js'
import { mountGenealogy, type GenealogyMountInput } from './index.js'

/** A chart context that can also be handed to `mountGenealogy` as its input. */
type TestContext = GenealogyContext & GenealogyMountInput

function makeContext(): TestContext {
  const notesByPath = genealogyNotes()
  const registry = buildRegistry(GENEALOGY_TYPES)
  const graph = buildRelationshipGraph('/people/john/', notesByPath, registry)
  return {
    graph,
    notesByPath,
    registry,
    focusPath: graph.focus,
    resolveUrl: (p) => p,
    navigate: vi.fn(),
    graphDepth: 2,
    loadGraphChunk: () => Promise.resolve(false),
    fetchPageLinks: () => Promise.resolve(null),
    relationshipTypes: GENEALOGY_TYPES,
  }
}

beforeEach(() => {
  localStorage.clear()
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('UNIT selector persistence', () => {
  it('resolveChartId falls back to the default for unknown or missing ids', () => {
    expect(resolveChartId(null)).toBe(DEFAULT_CHART_ID)
    expect(resolveChartId(undefined)).toBe(DEFAULT_CHART_ID)
    expect(resolveChartId('bogus-chart')).toBe(DEFAULT_CHART_ID)
    expect(resolveChartId('timeline')).toBe('timeline')
    expect(resolveChartId('family-chart')).toBe('family-chart')
  })

  it('readStoredChartId reads localStorage and survives storage errors', () => {
    expect(readStoredChartId()).toBe(DEFAULT_CHART_ID)
    localStorage.setItem(CHART_STORAGE_KEY, 'timeline')
    expect(readStoredChartId()).toBe('timeline')
    localStorage.setItem(CHART_STORAGE_KEY, 'stale-id-from-old-version')
    expect(readStoredChartId()).toBe(DEFAULT_CHART_ID)
    vi.mocked(localStorage.getItem).mockImplementationOnce(() => {
      throw new Error('storage disabled')
    })
    expect(readStoredChartId()).toBe(DEFAULT_CHART_ID)
  })

  it('storeChartId writes and ignores storage errors', () => {
    storeChartId('timeline')
    expect(localStorage.getItem(CHART_STORAGE_KEY)).toBe('timeline')
    vi.mocked(localStorage.setItem).mockImplementationOnce(() => {
      throw new Error('quota exceeded')
    })
    expect(() => storeChartId('family-chart')).not.toThrow()
  })
})

describe('UNIT createSelector', () => {
  it('renders one labelled option per chart type with the active one selected', () => {
    const select = createSelector('timeline', () => {})
    expect(select.getAttribute('aria-label')).toBe('Chart type')
    const options = Array.from(select.querySelectorAll('option'))
    expect(options.map((o) => o.value)).toEqual(CHART_TYPES.map((c) => c.id))
    expect(options.map((o) => o.textContent)).toEqual(CHART_TYPES.map((c) => c.label))
    expect(select.value).toBe('timeline')
  })

  it('fires onChange with the newly selected id', () => {
    const onChange = vi.fn()
    const select = createSelector(DEFAULT_CHART_ID, onChange)
    document.body.appendChild(select)
    select.value = 'timeline'
    select.dispatchEvent(new Event('change'))
    expect(onChange).toHaveBeenCalledWith('timeline')
    select.remove()
  })
})

describe('UNIT mountGenealogy', () => {
  let container: HTMLElement
  let mountSpies: Map<string, MockInstance>
  let destroySpies: Map<string, ReturnType<typeof vi.fn>>

  beforeEach(() => {
    container = document.createElement('div')
    document.body.appendChild(container)
    // Stub every chart's mount so neither family-chart nor the timeline
    // renderer actually runs; we only assert on orchestration.
    mountSpies = new Map()
    destroySpies = new Map()
    for (const chart of CHART_TYPES) {
      const destroy = vi.fn()
      destroySpies.set(chart.id, destroy)
      mountSpies.set(chart.id, vi.spyOn(chart, 'mount').mockReturnValue({ destroy }))
    }
  })

  afterEach(() => {
    container.remove()
  })

  it('mounts the default chart (family-chart) and renders the selector', () => {
    const ctx = makeContext()
    const controller = mountGenealogy(container, ctx)
    expect(mountSpies.get('family-chart')).toHaveBeenCalledTimes(1)
    expect(mountSpies.get('timeline')).not.toHaveBeenCalled()
    const [mountContainer, mountCtx] = mountSpies.get('family-chart')!.mock.calls[0]
    expect(mountContainer).toBeInstanceOf(HTMLElement)
    // The chunk builds the graph and registry itself from the input.
    expect(mountCtx).toBe(controller.context)
    expect(mountCtx.focusPath).toBe('/people/john/')
    expect(mountCtx.graph.edges).toEqual(ctx.graph.edges)
    expect(mountCtx.registry.isSymmetric('spouse')).toBe(true)
    const select = container.querySelector<HTMLSelectElement>('select.gen-chart-select')
    expect(select).not.toBeNull()
    expect(select!.value).toBe(DEFAULT_CHART_ID)
    controller.destroy()
  })

  it('honors a persisted chart choice', () => {
    localStorage.setItem(CHART_STORAGE_KEY, 'timeline')
    const controller = mountGenealogy(container, makeContext())
    expect(mountSpies.get('timeline')).toHaveBeenCalledTimes(1)
    expect(mountSpies.get('family-chart')).not.toHaveBeenCalled()
    expect(container.querySelector<HTMLSelectElement>('select')!.value).toBe('timeline')
    controller.destroy()
  })

  it('re-mounts on selection change (destroying the old chart) and persists', () => {
    const controller = mountGenealogy(container, makeContext())
    const select = container.querySelector<HTMLSelectElement>('select')!
    select.value = 'timeline'
    select.dispatchEvent(new Event('change'))
    expect(destroySpies.get('family-chart')).toHaveBeenCalledTimes(1)
    expect(mountSpies.get('timeline')).toHaveBeenCalledTimes(1)
    expect(localStorage.getItem(CHART_STORAGE_KEY)).toBe('timeline')
    controller.destroy()
  })

  it('setChartType switches charts programmatically and syncs the selector', () => {
    const controller = mountGenealogy(container, makeContext())
    controller.setChartType('timeline')
    expect(destroySpies.get('family-chart')).toHaveBeenCalledTimes(1)
    expect(mountSpies.get('timeline')).toHaveBeenCalledTimes(1)
    expect(container.querySelector<HTMLSelectElement>('select')!.value).toBe('timeline')
    // Unknown ids resolve to the default instead of blowing up.
    controller.setChartType('bogus')
    expect(mountSpies.get('family-chart')).toHaveBeenCalledTimes(2)
    controller.destroy()
  })

  it('destroy tears down the active chart and removes all DOM', () => {
    const controller = mountGenealogy(container, makeContext())
    controller.destroy()
    expect(destroySpies.get('family-chart')).toHaveBeenCalledTimes(1)
    expect(container.querySelector('.mbr-genealogy-root')).toBeNull()
  })
})

// ---------------------------------------------------------------------------
// Default chart choice (B3)
// ---------------------------------------------------------------------------

function companyContext(focus: string, types = CONTACT_TYPES): TestContext {
  const notesByPath = companyNotes(types)
  const registry = buildRegistry(types)
  const graph = buildRelationshipGraph(focus, notesByPath, registry)
  return {
    graph,
    notesByPath,
    registry,
    focusPath: graph.focus,
    resolveUrl: (p) => p,
    navigate: vi.fn(),
    graphDepth: 2,
    loadGraphChunk: () => Promise.resolve(false),
    fetchPageLinks: () => Promise.resolve(null),
    relationshipTypes: types,
  }
}

describe('UNIT chooseChartId', () => {
  it('keeps an applicable persisted choice', () => {
    expect(chooseChartId('timeline', makeContext())).toBe('timeline')
    expect(chooseChartId('all', companyContext('/people/carol/'))).toBe('all')
  })

  it('family edges → Family chart', () => {
    expect(chooseChartId(null, makeContext())).toBe('family-chart')
    // Bob has a parent AND a manager: family wins.
    expect(chooseChartId(null, companyContext('/people/bob/'))).toBe('family-chart')
  })

  it('work edges only → Org chart, even over a persisted family choice', () => {
    expect(chooseChartId(null, companyContext('/people/carol/'))).toBe('org-chart')
    expect(chooseChartId('family-chart', companyContext('/orgs/acme/'))).toBe('org-chart')
  })

  it('works the same against a legacy registry (no hierarchy/category)', () => {
    expect(chooseChartId(null, companyContext('/people/carol/', LEGACY_CONTACT_TYPES))).toBe('org-chart')
    expect(chooseChartId(null, companyContext('/people/bob/', LEGACY_CONTACT_TYPES))).toBe('family-chart')
  })

  it('neither → All people', () => {
    const notesByPath = buildSiteNotes(
      [
        { path: '/a/', fm: { type: 'person' } },
        { path: '/b/', fm: { type: 'person' } },
      ],
      [['/a/', 'colleague', '/b/']]
    )
    const registry = buildRegistry(CONTACT_TYPES)
    const ctx = { ...companyContext('/people/carol/'), notesByPath, registry, focusPath: '/a/', graph: buildRelationshipGraph('/a/', notesByPath, registry) }
    expect(chooseChartId('family-chart', ctx)).toBe('all-people')
  })
})

describe('UNIT selector applicability', () => {
  it('disables charts that cannot draw this note, but never the active one', () => {
    const select = createSelector('org-chart', () => {}, companyContext('/people/carol/'))
    const disabled = Array.from(select.options)
      .filter((o) => o.disabled)
      .map((o) => o.value)
    expect(disabled).toEqual(['family-chart', 'timeline'])
    expect(Array.from(select.options).map((o) => o.textContent)).toEqual([
      'Family chart',
      'Timeline tree',
      'Org chart',
      'All people',
      'All',
    ])
  })

  it('mountGenealogy opens the org chart on a work-only page without overwriting the stored choice', () => {
    localStorage.setItem(CHART_STORAGE_KEY, 'family-chart')
    const spies = CHART_TYPES.map((chart) => vi.spyOn(chart, 'mount').mockReturnValue({ destroy: vi.fn() }))
    const host = document.createElement('div')
    document.body.appendChild(host)
    const controller = mountGenealogy(host, companyContext('/people/carol/'))
    const mounted = CHART_TYPES.filter((_, i) => spies[i].mock.calls.length > 0).map((c) => c.id)
    expect(mounted).toEqual(['org-chart'])
    expect(localStorage.getItem(CHART_STORAGE_KEY)).toBe('family-chart')
    controller.destroy()
    host.remove()
  })
})
