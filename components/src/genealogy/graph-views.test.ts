import { describe, it, expect, vi, afterEach } from 'vitest'
import { buildRegistry, buildRelationshipGraph, type PageLinks } from '../graph/relationship-graph.js'
import { CONTACT_TYPES, companyNotes } from '../graph/test-fixtures.js'
import type { MbrMiniGraphElement } from '../graph/mbr-mini-graph.js'
import type { GenealogyContext } from './chart-registry.js'
import {
  allChartType,
  allPeopleChartType,
  areRelated,
  chartFetchLinks,
  graphNodeClass,
  relationshipLinks,
  relationshipReach,
} from './graph-views.js'

const registry = buildRegistry(CONTACT_TYPES)

function makeContext(overrides: Partial<GenealogyContext> = {}): GenealogyContext {
  const notesByPath = companyNotes()
  const graph = buildRelationshipGraph('/people/bob/', notesByPath, registry)
  return {
    graph,
    notesByPath,
    registry,
    focusPath: '/people/bob/',
    resolveUrl: (p) => p,
    navigate: vi.fn(),
    graphDepth: 2,
    // The real chunk entry: registers <mbr-mini-graph> exactly as the lazy load does.
    loadGraphChunk: async () => {
      await import('../graph/index.js')
      return true
    },
    fetchPageLinks: async () => null,
    ...overrides,
  }
}

/** Let the chunk import, the BFS and Lit settle. */
async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) await new Promise((resolve) => setTimeout(resolve, 0))
}

/** Wait until the chart has swapped its loading message for the graph. */
async function mounted(host: HTMLElement): Promise<MbrMiniGraphElement> {
  await vi.waitFor(() => expect(host.querySelector('mbr-mini-graph')).not.toBeNull(), { timeout: 3000 })
  await settle()
  return host.querySelector('mbr-mini-graph') as MbrMiniGraphElement
}

const containers: HTMLElement[] = []
function container(): HTMLElement {
  const el = document.createElement('div')
  document.body.appendChild(el)
  containers.push(el)
  return el
}

afterEach(() => {
  for (const el of containers.splice(0)) el.remove()
})

describe('pure helpers', () => {
  const notes = companyNotes()

  it('synthesizes a links.json from relationships only', () => {
    const links = relationshipLinks(notes.get('/people/carol/'))
    expect(links?.inbound).toEqual([])
    expect(links?.outbound).toEqual([])
    expect(links?.relationships?.length).toBeGreaterThan(0)
    expect(relationshipLinks(undefined)).toBeNull()
  })

  it('computes relationship reach by depth', () => {
    expect([...relationshipReach('/people/carol/', notes, 1)].sort()).toEqual([
      '/orgs/acme/',
      '/people/bob/',
      '/people/carol/',
    ])
    expect(relationshipReach('/people/carol/', notes, 2).has('/people/ada/')).toBe(true)
  })

  it('knows which pairs are related', () => {
    expect(areRelated('/people/carol/', '/people/bob/', notes)).toBe(true)
    expect(areRelated('/people/bob/', '/people/carol/', notes)).toBe(true)
    expect(areRelated('/people/carol/', '/people/eve/', notes)).toBe(false)
  })

  it('classes plain-link and organization nodes, never the focus', () => {
    const related = new Set(['/people/bob/', '/orgs/acme/'])
    expect(graphNodeClass('/people/bob/', '/people/bob/', notes, related)).toBeUndefined()
    expect(graphNodeClass('/orgs/acme/', '/people/bob/', notes, related)).toBe('node-org')
    expect(graphNodeClass('/people/zed/', '/people/bob/', notes, related)).toBe('node-plain')
    expect(graphNodeClass('/people/carol/', '/people/bob/', notes, null)).toBeUndefined()
  })

  it('merges the focus’s real links.json only for the All chart', async () => {
    const real: PageLinks = {
      inbound: [{ from: '/notes/standup/', text: 'x' }],
      outbound: [{ to: '/notes/plan/', text: 'y', internal: true }],
    }
    const fetchPageLinks = vi.fn(async () => real)
    const ctx = makeContext({ fetchPageLinks })
    const people = chartFetchLinks(ctx, false)
    expect((await people('/people/bob/'))?.outbound).toEqual([])
    expect(fetchPageLinks).not.toHaveBeenCalled()

    const all = chartFetchLinks(ctx, true)
    const focus = await all('/people/bob/')
    expect(focus?.outbound).toEqual(real.outbound)
    expect(focus?.relationships?.length).toBeGreaterThan(0)
    // Other notes are never fetched: plain links stay at depth 1.
    expect((await all('/people/ada/'))?.inbound).toEqual([])
    expect(fetchPageLinks).toHaveBeenCalledTimes(1)
  })

  it('falls back to relationships and reports it when links.json is unavailable', async () => {
    const onUnavailable = vi.fn()
    const all = chartFetchLinks(makeContext(), true, onUnavailable)
    expect((await all('/people/bob/'))?.relationships?.length).toBeGreaterThan(0)
    expect(onUnavailable).toHaveBeenCalledTimes(1)
  })
})

describe('All people / All charts', () => {
  it('mounts an inline <mbr-mini-graph> fed from site.json', async () => {
    const host = container()
    const instance = allPeopleChartType.mount(host, makeContext())
    const graph = await mounted(host)
    expect(graph!.inline).toBe(true)
    expect(graph!.depth).toBe(2)
    expect(graph!.focusPath).toBe('/people/bob/')
    expect(graph!.nodeClass?.('/orgs/acme/')).toBe('node-org')
    expect(graph!.linkClass).toBeUndefined()
    // An organization is reachable, so the legend explains its color.
    expect(host.querySelector('.gen-legend')?.textContent).toContain('Organization')
    instance.destroy()
    expect(host.querySelector('mbr-mini-graph')).toBeNull()
  })

  it('All adds plain-link styling and a two-kind legend', async () => {
    const host = container()
    allChartType.mount(host, makeContext())
    const graph = await mounted(host)
    expect(graph.linkClass?.('/people/bob/', '/notes/plan/')).toBe('link-plain')
    expect(graph.linkClass?.('/people/bob/', '/people/ada/')).toBeUndefined()
    expect(graph.nodeClass?.('/notes/plan/')).toBe('node-plain')
    const legend = host.querySelector('.gen-legend')?.textContent ?? ''
    expect(legend).toContain('Relationship')
    expect(legend).toContain('Linked note')
  })

  it('says so when the graph chunk cannot be loaded', async () => {
    const host = container()
    allPeopleChartType.mount(host, makeContext({ loadGraphChunk: async () => false }))
    await settle()
    expect(host.querySelector('mbr-mini-graph')).toBeNull()
    expect(host.querySelector('.gen-empty')?.textContent).toMatch(/could not be loaded/)
  })

  it('does nothing after destroy, even if the chunk resolves later', async () => {
    let resolve!: (v: boolean) => void
    const host = container()
    const instance = allPeopleChartType.mount(
      host,
      makeContext({ loadGraphChunk: () => new Promise<boolean>((r) => (resolve = r)) })
    )
    instance.destroy()
    resolve(true)
    await settle()
    expect(host.querySelector('mbr-mini-graph')).toBeNull()
  })
})
