import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { GENEALOGY_TYPES, genealogyNotes, rel } from './graph/test-fixtures.js'
import type {
  GraphEdge,
  RelationshipGraph,
  RelationTypeConfig,
  SiteNote,
} from './graph/relationship-graph.js'
import { setGenealogyModuleLoader, type MbrGenealogyElement } from './mbr-genealogy.js'

/**
 * The trigger reads site data through shared.ts, whose module-level site.json
 * fetch already ran with the test-setup stub. Mock the module so each test can
 * inject its own site data and canonical path. `getBasePath` is included
 * because dynamic-loader.ts (also imported by the trigger) uses it.
 */
const mocks = vi.hoisted(() => ({
  state: { isLoading: true, data: null as unknown, error: null as string | null },
  canonicalPath: { value: '/people/john/' },
}))

vi.mock('./shared.ts', () => ({
  subscribeSiteNav: (cb: (s: unknown) => void) => {
    cb({ ...mocks.state })
    return () => {}
  },
  getCanonicalPath: () => mocks.canonicalPath.value,
  // Prefixed so tests can tell a resolved href from a raw url_path.
  resolveUrl: (p: string) => `/base${p}`,
  getBasePath: () => '',
}))

/** Full genealogy site.json payload built from the shared fixture. */
function fixtureSiteData(): unknown {
  return {
    markdown_files: [...genealogyNotes().values()],
    relationship_types: GENEALOGY_TYPES,
  }
}

async function createElement(): Promise<MbrGenealogyElement> {
  const el = document.createElement('mbr-genealogy')
  document.body.appendChild(el)
  await flush(el)
  return el
}

/** Let waitForDom/subscription microtasks and Lit updates settle. */
async function flush(el: MbrGenealogyElement): Promise<void> {
  for (let i = 0; i < 4; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0))
    await el.updateComplete
  }
}

describe('UNIT MbrGenealogyElement', () => {
  let element: MbrGenealogyElement | null = null

  beforeEach(() => {
    // No IntersectionObserver → the element must load immediately.
    vi.stubGlobal('IntersectionObserver', undefined)
    mocks.state.isLoading = false
    mocks.state.data = fixtureSiteData()
    mocks.state.error = null
    mocks.canonicalPath.value = '/people/john/'
    window.frontmatter = { type: 'person' }
  })

  afterEach(() => {
    element?.remove()
    element = null
    setGenealogyModuleLoader(null)
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
    delete window.frontmatter
  })

  it('renders nothing on non-person pages', async () => {
    window.frontmatter = { type: 'project' }
    element = await createElement()
    expect(element.shadowRoot?.querySelector('figure')).toBeNull()
  })

  it('renders nothing when the person has no relationship edges', async () => {
    mocks.state.data = {
      markdown_files: [
        { url_path: '/people/john/', frontmatter: { type: 'person', title: 'John' } },
      ],
      relationship_types: GENEALOGY_TYPES,
    }
    const mountGenealogy = vi.fn()
    setGenealogyModuleLoader(async () => ({ mountGenealogy }))
    element = await createElement()
    expect(element.shadowRoot?.querySelector('figure')).toBeNull()
    expect(mountGenealogy).not.toHaveBeenCalled()
  })

  it('renders the placeholder and mounts the chunk with the focus graph', async () => {
    const controller = { destroy: vi.fn(), setChartType: vi.fn() }
    const mountGenealogy = vi.fn().mockReturnValue(controller)
    const loader = vi.fn().mockResolvedValue({ mountGenealogy })
    setGenealogyModuleLoader(loader)

    element = await createElement()

    // Fixed-height placeholder figure is rendered (no layout shift).
    const figure = element.shadowRoot?.querySelector('figure.gen-figure')
    expect(figure).not.toBeNull()

    // The chunk was imported from the .mbr components URL...
    expect(loader).toHaveBeenCalledTimes(1)
    expect(String(loader.mock.calls[0][0])).toContain('components/mbr-genealogy.min.js')

    // ...and mounted with the correct focus, graph, and services.
    expect(mountGenealogy).toHaveBeenCalledTimes(1)
    const [container, ctx] = mountGenealogy.mock.calls[0]
    expect(container).toBeInstanceOf(HTMLElement)
    expect((container as HTMLElement).classList.contains('gen-mount')).toBe(true)
    expect(ctx.focusPath).toBe('/people/john/')
    expect(ctx.graph.focus).toBe('/people/john/')
    expect(ctx.graph.edges.length).toBeGreaterThan(0)
    expect(ctx.graph.nodes.some((n: { urlPath: string }) => n.urlPath === '/people/mary/')).toBe(
      true
    )
    expect(ctx.notesByPath.get('/people/john/')).toBeDefined()
    expect(ctx.registry.isSymmetric('spouse')).toBe(true)
    expect(typeof ctx.resolveUrl).toBe('function')
    expect(typeof ctx.navigate).toBe('function')

    // Spinner is gone once the chart is mounted; the figure remains.
    await flush(element)
    expect(element.shadowRoot?.querySelector('.gen-loading')).toBeNull()
    expect(element.shadowRoot?.querySelector('figure.gen-figure')).not.toBeNull()
  })

  it('warns and renders nothing when the chunk fails to load', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    setGenealogyModuleLoader(() => Promise.reject(new Error('network down')))
    element = await createElement()
    await flush(element)
    expect(warn).toHaveBeenCalled()
    expect(element.shadowRoot?.querySelector('figure')).toBeNull()
  })

  it('destroys the chart controller on disconnect', async () => {
    const controller = { destroy: vi.fn(), setChartType: vi.fn() }
    setGenealogyModuleLoader(async () => ({
      mountGenealogy: vi.fn().mockReturnValue(controller),
    }))
    element = await createElement()
    await flush(element)
    element.remove()
    element = null
    expect(controller.destroy).toHaveBeenCalledTimes(1)
  })

  it('renders nothing when the focus note is unknown to site.json', async () => {
    mocks.canonicalPath.value = '/people/nobody/'
    const mountGenealogy = vi.fn()
    setGenealogyModuleLoader(async () => ({ mountGenealogy }))
    element = await createElement()
    expect(element.shadowRoot?.querySelector('figure')).toBeNull()
    expect(mountGenealogy).not.toHaveBeenCalled()
  })
})

// ---------------------------------------------------------------------------
// Contradictory-link notice
// ---------------------------------------------------------------------------

/**
 * Site data in which `focus` and every path in `others` each declare the other
 * as their own child — a contradiction that `buildRelationshipGraph` resolves by
 * dropping one edge per pair.
 */
function contradictorySiteData(focus: string, others: string[]): unknown {
  const notes: SiteNote[] = [
    {
      url_path: focus,
      frontmatter: { type: 'person', title: 'Ann Doe' },
      relationships: others.map((other) =>
        rel({ rel_type: 'child', predicate: 'child', neighbor: other, direction: 'outgoing' })
      ),
    },
    ...others.map((other, i) => ({
      url_path: other,
      frontmatter: { type: 'person', title: `Kid ${i + 1}` },
      relationships: [
        rel({ rel_type: 'child', predicate: 'child', neighbor: focus, direction: 'outgoing' }),
      ],
    })),
  ]
  return { markdown_files: notes, relationship_types: GENEALOGY_TYPES }
}

const hier = (from: string, to: string, relType = 'child'): GraphEdge => ({
  from,
  to,
  kind: 'hierarchical',
  relType,
  label: '',
})

/**
 * A configured inverse pair that is NOT parent/child. It still produces
 * `hierarchical` edges (and so can still cycle), but nothing about it is
 * parental — the notice wording must not pretend otherwise.
 */
const MENTOR_TYPES: RelationTypeConfig[] = [
  { name: 'mentor', symmetric: false, inverse: 'mentee', label: 'Mentor', label_plural: 'Mentors' },
  { name: 'mentee', symmetric: false, inverse: 'mentor', label: 'Mentee', label_plural: 'Mentees' },
]

/** Two notes that each declare the other as their own mentor. */
function mentorLoopSiteData(): unknown {
  const notes: SiteNote[] = [
    {
      url_path: '/p/ann/',
      frontmatter: { type: 'person', title: 'Ann Doe' },
      relationships: [
        rel({ rel_type: 'mentor', predicate: 'mentor', neighbor: '/p/bob/', direction: 'outgoing' }),
      ],
    },
    {
      url_path: '/p/bob/',
      frontmatter: { type: 'person', title: 'Bob Roe' },
      relationships: [
        rel({ rel_type: 'mentor', predicate: 'mentor', neighbor: '/p/ann/', direction: 'outgoing' }),
      ],
    },
  ]
  return { markdown_files: notes, relationship_types: MENTOR_TYPES }
}

describe('UNIT MbrGenealogyElement contradictory-link notice', () => {
  let element: MbrGenealogyElement | null = null

  beforeEach(() => {
    vi.stubGlobal('IntersectionObserver', undefined)
    mocks.state.isLoading = false
    mocks.state.error = null
    mocks.canonicalPath.value = '/p/ann/'
    mocks.state.data = contradictorySiteData('/p/ann/', ['/p/kid1/'])
    window.frontmatter = { type: 'person' }
    setGenealogyModuleLoader(async () => ({
      mountGenealogy: vi.fn(() => ({ destroy: vi.fn(), setChartType: vi.fn() })),
    }))
  })

  afterEach(() => {
    element?.remove()
    element = null
    setGenealogyModuleLoader(null)
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
    delete window.frontmatter
  })

  /** Overwrite just the dropped-edge list and re-render, keeping notesByPath. */
  async function setDroppedEdges(
    el: MbrGenealogyElement,
    droppedEdges: GraphEdge[] | undefined
  ): Promise<void> {
    const internals = el as unknown as { _graph: RelationshipGraph | null }
    expect(internals._graph).not.toBeNull()
    internals._graph = { ...internals._graph!, droppedEdges }
    el.requestUpdate()
    await el.updateComplete
  }

  const notice = (el: MbrGenealogyElement) => el.shadowRoot?.querySelector('.gen-notice')
  const lines = (el: MbrGenealogyElement) => [...(notice(el)?.querySelectorAll('li') ?? [])]

  it('renders no notice for a consistent family tree', async () => {
    mocks.state.data = fixtureSiteData()
    mocks.canonicalPath.value = '/people/john/'
    element = await createElement()
    expect(element.shadowRoot?.querySelector('figure.gen-figure')).not.toBeNull()
    expect(notice(element)).toBeNull()
  })

  it('renders no notice when droppedEdges is undefined', async () => {
    element = await createElement()
    expect(notice(element)).not.toBeNull()
    await setDroppedEdges(element, undefined)
    expect(notice(element)).toBeNull()
    await setDroppedEdges(element, [])
    expect(notice(element)).toBeNull()
  })

  it('names both notes of the ignored claim, with resolved links', async () => {
    element = await createElement()
    const items = lines(element)
    expect(items).toHaveLength(1)

    // The dropped edge is "Kid 1 is the parent of Ann Doe" (the focus keeps its
    // own outgoing edge), and the wording spells that direction out.
    expect(items[0].textContent?.replace(/\s+/g, ' ').trim()).toBe(
      'Ignored: Kid 1 as parent of Ann Doe'
    )

    // Real anchors (middle-click/copy-link work), hrefs run through resolveUrl.
    const anchors = [...items[0].querySelectorAll('a')]
    expect(anchors.map((a) => a.getAttribute('href'))).toEqual(['/base/p/kid1/', '/base/p/ann/'])
    expect(anchors.map((a) => a.textContent)).toEqual(['Kid 1', 'Ann Doe'])
  })

  it('leaves the fixed-height canvas outside the notice', async () => {
    element = await createElement()
    const figure = element.shadowRoot?.querySelector('figure.gen-figure')
    expect(notice(element)?.querySelector('.gen-canvas')).toBeNull()
    expect(figure?.querySelector('.gen-canvas')).not.toBeNull()
  })

  it('falls back to the raw url_path when the note is unknown', async () => {
    element = await createElement()
    await setDroppedEdges(element, [hier('/p/ghost/', '/p/ann/')])
    const anchors = [...lines(element)[0].querySelectorAll('a')]
    expect(anchors.map((a) => a.textContent)).toEqual(['/p/ghost/', 'Ann Doe'])
    expect(anchors[0].getAttribute('href')).toBe('/base/p/ghost/')
  })

  it('de-duplicates by unordered note pair', async () => {
    element = await createElement()
    await setDroppedEdges(element, [hier('/p/ann/', '/p/kid1/'), hier('/p/kid1/', '/p/ann/')])
    expect(lines(element)).toHaveLength(1)
  })

  it('pins the parent/child wording when every ignored link is parent/child', async () => {
    element = await createElement()
    const text = notice(element)?.textContent?.replace(/\s+/g, ' ') ?? ''
    expect(text).toContain('One contradictory parent/child link was ignored')
    expect(text).toContain("the notes below each claim to be the other's ancestor")
  })

  it('uses type-neutral wording for a non-parent/child inverse pair', async () => {
    // mentor/mentee cycles are hierarchical too, but nothing about them is
    // parental — the paragraph must not say "parent/child" or "each other's
    // ancestor".
    mocks.state.data = mentorLoopSiteData()
    element = await createElement()

    const text = notice(element)?.textContent?.replace(/\s+/g, ' ') ?? ''
    expect(text).toContain('One contradictory relationship link was ignored')
    expect(text).toContain('each note below is listed as its own ancestor through a chain')
    expect(text).not.toContain('parent/child')
    expect(text).not.toContain("the other's ancestor")

    // Per-edge lines are unchanged: they already name the real relationship type.
    expect(lines(element).map((li) => li.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
      'Ignored: Ann Doe as mentee of Bob Roe',
    ])
  })

  it('uses type-neutral wording when the ignored links are mixed', async () => {
    element = await createElement()
    await setDroppedEdges(element, [
      hier('/p/kid1/', '/p/ann/'),
      hier('/p/ann/', '/p/mentor/', 'mentee'),
    ])
    const text = notice(element)?.textContent?.replace(/\s+/g, ' ') ?? ''
    expect(text).toContain('2 contradictory relationship links were ignored')
    expect(text).not.toContain('parent/child')
    expect(lines(element)).toHaveLength(2)
  })

  it('lists at most five pairs and points at the page problems panel', async () => {
    const kids = [1, 2, 3, 4, 5, 6].map((n) => `/p/kid${n}/`)
    mocks.state.data = contradictorySiteData('/p/ann/', kids)
    element = await createElement()

    expect(lines(element)).toHaveLength(5)
    const text = notice(element)?.textContent?.replace(/\s+/g, ' ') ?? ''
    expect(text).toContain('6 contradictory parent/child links were ignored')
    expect(text).toContain('and 1 more')
    expect(text).toContain('page problems panel')
  })
})
