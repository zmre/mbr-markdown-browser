import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { GENEALOGY_TYPES, genealogyNotes } from './graph/test-fixtures.js'
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
  resolveUrl: (p: string) => p,
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
