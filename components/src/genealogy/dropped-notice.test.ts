import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import type { GraphEdge, RelationTypeConfig, SiteNote } from '../graph/relationship-graph.js'
import { GENEALOGY_TYPES, genealogyNotes, rel } from '../graph/test-fixtures.js'
import { CHART_TYPES } from './chart-registry.js'
import { mountGenealogy, type GenealogyController, type GenealogyMountInput } from './index.js'
import { renderDroppedNotice } from './dropped-notice.js'

/**
 * The contradictory-link notice is drawn by the chunk, inside the trigger's
 * fixed-height canvas. Every chart's mount is stubbed: only the notice and the
 * layout around it are under test.
 */
// ---------------------------------------------------------------------------
// Contradictory-link notice
// ---------------------------------------------------------------------------

/**
 * Site data in which `focus` and every path in `others` each declare the other
 * as their own child — a contradiction that `buildRelationshipGraph` resolves by
 * dropping one edge per pair.
 */
function contradictorySiteData(focus: string, others: string[]): SiteData {
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
function mentorLoopSiteData(): SiteData {
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


type SiteData = { markdown_files: SiteNote[]; relationship_types: RelationTypeConfig[] }

function input(data: SiteData, focusPath: string): GenealogyMountInput {
  return {
    notesByPath: new Map(data.markdown_files.map((n) => [n.url_path, n])),
    relationshipTypes: data.relationship_types,
    focusPath,
    // Prefixed so tests can tell a resolved href from a raw url_path.
    resolveUrl: (p) => `/base${p}`,
    navigate: vi.fn(),
    graphDepth: 2,
    loadGraphChunk: () => Promise.resolve(false),
    fetchPageLinks: () => Promise.resolve(null),
  }
}

describe('UNIT contradictory-link notice', () => {
  let container: HTMLElement
  let controller: GenealogyController | null = null

  beforeEach(() => {
    localStorage.clear()
    container = document.createElement('div')
    document.body.appendChild(container)
    for (const chart of CHART_TYPES) vi.spyOn(chart, 'mount').mockReturnValue({ destroy: vi.fn() })
  })

  afterEach(() => {
    controller?.destroy()
    controller = null
    container.remove()
    vi.restoreAllMocks()
  })

  const mount = (data: SiteData, focus: string) => {
    controller = mountGenealogy(container, input(data, focus))
  }
  const notice = () => container.querySelector('.gen-notice')
  const lines = () => [...(notice()?.querySelectorAll('li') ?? [])]
  const text = () => notice()?.textContent?.replace(/\s+/g, ' ') ?? ''
  /** Render the notice for a hand-made edge list, titles from the Ann/Kid data. */
  const renderFor = (edges: GraphEdge[] | undefined) => {
    const notes = new Map(contradictorySiteData('/p/ann/', ['/p/kid1/']).markdown_files.map((n) => [n.url_path, n]))
    const titleOf = (p: string) => (notes.get(p)?.frontmatter?.['title'] as string | undefined) ?? p
    return renderDroppedNotice(edges, titleOf, (p) => `/base${p}`)
  }

  it('renders no notice for a consistent family tree', () => {
    mount({ markdown_files: [...genealogyNotes().values()], relationship_types: GENEALOGY_TYPES }, '/people/john/')
    expect(notice()).toBeNull()
    expect(container.querySelector('.gen-chart-area')).not.toBeNull()
  })

  it('renders nothing for an undefined or empty edge list', () => {
    expect(renderFor(undefined)).toBeNull()
    expect(renderFor([])).toBeNull()
  })

  it('names both notes of the ignored claim, with resolved links', () => {
    mount(contradictorySiteData('/p/ann/', ['/p/kid1/']), '/p/ann/')
    const items = lines()
    expect(items).toHaveLength(1)

    // The dropped edge is "Kid 1 is the parent of Ann Doe" (the focus keeps its
    // own outgoing edge), and the wording spells that direction out.
    expect(items[0].textContent?.replace(/\s+/g, ' ').trim()).toBe('Ignored: Kid 1 as parent of Ann Doe')

    // Real anchors (middle-click/copy-link work), hrefs run through resolveUrl.
    const anchors = [...items[0].querySelectorAll('a')]
    expect(anchors.map((a) => a.getAttribute('href'))).toEqual(['/base/p/kid1/', '/base/p/ann/'])
    expect(anchors.map((a) => a.textContent)).toEqual(['Kid 1', 'Ann Doe'])
  })

  it('sits above the chart, outside the selector overlay', () => {
    mount(contradictorySiteData('/p/ann/', ['/p/kid1/']), '/p/ann/')
    const root = container.querySelector('.mbr-genealogy-root')!
    expect(root.firstElementChild?.classList.contains('gen-notice')).toBe(true)
    expect(notice()?.querySelector('.gen-chart-area, select')).toBeNull()
    expect(container.querySelector('.gen-chart-wrap select.gen-chart-select')).not.toBeNull()
  })

  it('falls back to the raw url_path when the note is unknown', () => {
    const el = renderFor([hier('/p/ghost/', '/p/ann/')])!
    const anchors = [...el.querySelectorAll('li')[0].querySelectorAll('a')]
    expect(anchors.map((a) => a.textContent)).toEqual(['/p/ghost/', 'Ann Doe'])
    expect(anchors[0].getAttribute('href')).toBe('/base/p/ghost/')
  })

  it('de-duplicates by unordered note pair', () => {
    const el = renderFor([hier('/p/ann/', '/p/kid1/'), hier('/p/kid1/', '/p/ann/')])!
    expect(el.querySelectorAll('li')).toHaveLength(1)
  })

  it('pins the parent/child wording when every ignored link is parent/child', () => {
    mount(contradictorySiteData('/p/ann/', ['/p/kid1/']), '/p/ann/')
    expect(text()).toContain('One contradictory parent/child link was ignored')
    expect(text()).toContain("the notes below each claim to be the other's ancestor")
  })

  it('uses type-neutral wording for a non-parent/child inverse pair', () => {
    // mentor/mentee cycles are hierarchical too, but nothing about them is
    // parental — the paragraph must not say "parent/child" or "each other's
    // ancestor".
    mount(mentorLoopSiteData(), '/p/ann/')
    expect(text()).toContain('One contradictory relationship link was ignored')
    expect(text()).toContain('each note below is listed as its own ancestor through a chain')
    expect(text()).not.toContain('parent/child')
    expect(text()).not.toContain("the other's ancestor")
    // Per-edge lines name the real relationship type.
    expect(lines().map((li) => li.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
      'Ignored: Ann Doe as mentee of Bob Roe',
    ])
  })

  it('uses type-neutral wording when the ignored links are mixed', () => {
    const el = renderFor([hier('/p/kid1/', '/p/ann/'), hier('/p/ann/', '/p/mentor/', 'mentee')])!
    const t = el.textContent?.replace(/\s+/g, ' ') ?? ''
    expect(t).toContain('2 contradictory relationship links were ignored')
    expect(t).not.toContain('parent/child')
    expect(el.querySelectorAll('li')).toHaveLength(2)
  })

  it('lists at most five pairs and points at the page problems panel', () => {
    const kids = [1, 2, 3, 4, 5, 6].map((n) => `/p/kid${n}/`)
    mount(contradictorySiteData('/p/ann/', kids), '/p/ann/')
    expect(lines()).toHaveLength(5)
    expect(text()).toContain('6 contradictory parent/child links were ignored')
    expect(text()).toContain('and 1 more')
    expect(text()).toContain('page problems panel')
  })
})
