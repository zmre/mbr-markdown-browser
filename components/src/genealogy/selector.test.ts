import { describe, it, expect, beforeEach, afterEach, vi, type MockInstance } from 'vitest'
import { buildRegistry, buildRelationshipGraph } from '../graph/relationship-graph.js'
import { GENEALOGY_TYPES, genealogyNotes } from '../graph/test-fixtures.js'
import { CHART_TYPES, DEFAULT_CHART_ID, type GenealogyContext } from './chart-registry.js'
import {
  CHART_STORAGE_KEY,
  createSelector,
  readStoredChartId,
  resolveChartId,
  storeChartId,
} from './selector.js'
import { mountGenealogy } from './index.js'

function makeContext(): GenealogyContext {
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
    expect(mountCtx).toBe(ctx)
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
