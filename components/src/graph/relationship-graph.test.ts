/**
 * Unit tests for the pure relationship-graph logic: registry classification,
 * breadth-first traversal + edge de-duplication, and generation numbering.
 * The shared fixture mirrors the resolved `relationships` shape emitted in
 * `site.json` for the genealogy test repo.
 */
import { describe, it, expect } from 'vitest'
import {
  buildRegistry,
  classifyRelationship,
  buildRelationshipGraph,
  isGraphRelationship,
  canonicalizeNotePath,
  computeGenerations,
  notesByPathFromSite,
  formatNodeLabel,
  formatLifespan,
  yearOf,
  normalizeGender,
  type SiteNote,
} from './relationship-graph.js'
import { GENEALOGY_TYPES, rel, genealogyNotes } from './test-fixtures.js'

const registry = buildRegistry(GENEALOGY_TYPES)

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

describe('helpers', () => {
  it('yearOf extracts a 4-digit year', () => {
    expect(yearOf('1925-06-02')).toBe('1925')
    expect(yearOf(1980)).toBe('1980')
    expect(yearOf(undefined)).toBeUndefined()
    expect(yearOf('no year here')).toBeUndefined()
  })

  it('normalizeGender lowercases strings and ignores other types', () => {
    expect(normalizeGender('Male')).toBe('male')
    expect(normalizeGender('  FEMALE ')).toBe('female')
    expect(normalizeGender('')).toBeUndefined()
    expect(normalizeGender('   ')).toBeUndefined()
    expect(normalizeGender(42)).toBeUndefined()
    expect(normalizeGender(undefined)).toBeUndefined()
  })

  it('formatLifespan handles all combinations', () => {
    expect(formatLifespan('1925', '1999')).toBe('(1925–1999)')
    expect(formatLifespan('1950', undefined)).toBe('(b. 1950)')
    expect(formatLifespan(undefined, '2010')).toBe('(d. 2010)')
    expect(formatLifespan(undefined, undefined)).toBe('')
  })

  it('formatNodeLabel appends lifespan when present', () => {
    expect(formatNodeLabel({ urlPath: '/x/', title: 'John Doe', born: '1925', died: '1999', isFocus: false }))
      .toBe('John Doe (1925–1999)')
    expect(formatNodeLabel({ urlPath: '/x/', title: 'Nobody', isFocus: false }))
      .toBe('Nobody')
  })

  it('canonicalizeNotePath appends a trailing slash only when needed', () => {
    expect(canonicalizeNotePath('/people/george')).toBe('/people/george/')
    expect(canonicalizeNotePath('/people/george/')).toBe('/people/george/')
    expect(canonicalizeNotePath('')).toBe('')
    expect(canonicalizeNotePath('/')).toBe('/')
  })

  it('notesByPathFromSite indexes markdown_files by url_path', () => {
    const map = notesByPathFromSite({ markdown_files: [{ url_path: '/a/' }, { url_path: '/b/' }] })
    expect(map.size).toBe(2)
    expect(map.get('/a/')).toBeDefined()
    expect(notesByPathFromSite(null).size).toBe(0)
  })
})

// ---------------------------------------------------------------------------
// classifyRelationship
// ---------------------------------------------------------------------------

describe('classifyRelationship', () => {
  it('produces the same canonical key for an inverse pair from either side', () => {
    // George: "John is my child" (outgoing child).
    const fromGeorge = classifyRelationship(
      '/people/george/',
      rel({ rel_type: 'child', predicate: 'child', neighbor: '/people/john/', direction: 'outgoing' }),
      registry
    )
    // John: "George is my parent" (incoming child / predicate parent).
    const fromJohn = classifyRelationship(
      '/people/john/',
      rel({ rel_type: 'child', predicate: 'parent', neighbor: '/people/george/', direction: 'incoming' }),
      registry
    )
    expect(fromGeorge).not.toBeNull()
    expect(fromJohn).not.toBeNull()
    expect(fromGeorge!.key).toBe(fromJohn!.key)
    // Oriented parent → child so a top-down layout puts the ancestor on top.
    expect(fromGeorge!.edge.kind).toBe('hierarchical')
    expect(fromGeorge!.edge.from).toBe('/people/george/')
    expect(fromGeorge!.edge.to).toBe('/people/john/')
  })

  it('canonicalizes symmetric edges to a sorted unordered pair', () => {
    const a = classifyRelationship(
      '/people/george/',
      rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/people/martha/', direction: 'outgoing' }),
      registry
    )
    const b = classifyRelationship(
      '/people/martha/',
      rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/people/george/', direction: 'incoming' }),
      registry
    )
    expect(a!.key).toBe(b!.key)
    expect(a!.edge.kind).toBe('symmetric')
    expect([a!.edge.from, a!.edge.to]).toEqual(['/people/george/', '/people/martha/'])
    expect(a!.edge.label).toBe('Spouse')
  })

  it('treats unknown types as labelled directed edges', () => {
    const reg = buildRegistry([]) // empty registry → everything is unknown/directed
    const out = classifyRelationship(
      '/notes/a/',
      rel({ rel_type: 'depends_on', predicate: 'depends_on', neighbor: '/notes/b/', direction: 'outgoing', label: 'needs' }),
      reg
    )
    expect(out!.edge.kind).toBe('directed')
    expect(out!.edge.from).toBe('/notes/a/')
    expect(out!.edge.to).toBe('/notes/b/')
    expect(out!.edge.label).toBe('needs')
  })

  it('skips unresolved, empty, and self-loop edges', () => {
    expect(classifyRelationship('/a/', rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '', resolved: false, direction: 'outgoing' }), registry)).toBeNull()
    expect(classifyRelationship('/a/', rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/a/', direction: 'outgoing' }), registry)).toBeNull()
  })
})

// ---------------------------------------------------------------------------
// buildRelationshipGraph
// ---------------------------------------------------------------------------

describe('buildRelationshipGraph', () => {
  const notes = genealogyNotes()

  it('collects the whole family and de-duplicates every reciprocal edge', () => {
    const graph = buildRelationshipGraph('/people/john/', notes, registry, 3)
    // All seven people are reachable within three hops of John (Robert via
    // George's child edge, not the — now excluded — John↔Robert sibling link).
    expect(graph.nodes).toHaveLength(7)
    // 8 parent→child edges + 2 spouse; sibling edges are excluded.
    expect(graph.edges).toHaveLength(10)
    expect(graph.edges.filter((e) => e.kind === 'hierarchical')).toHaveLength(8)
    expect(graph.edges.filter((e) => e.kind === 'symmetric')).toHaveLength(2)
    // No sibling edge is ever produced.
    expect(graph.edges.some((e) => e.relType === 'sibling')).toBe(false)
    // A sibling that is a co-child of an in-graph parent still appears as a node.
    expect(graph.nodes.some((n) => n.urlPath === '/people/robert/')).toBe(true)
    // The unresolved Jane Ghost spouse edge must not appear.
    expect(graph.nodes.some((n) => n.title === 'Jane Ghost')).toBe(false)
    // Focus flag is set exactly once.
    expect(graph.nodes.filter((n) => n.isFocus)).toHaveLength(1)
    expect(graph.nodes.find((n) => n.isFocus)!.urlPath).toBe('/people/john/')
  })

  it('honours the depth bound (nodes within N hops of the focus)', () => {
    // Sam's own edges are his two parents (the sibling link to Alice is excluded),
    // so at depth 1 only John and Mary join him.
    const depth1 = buildRelationshipGraph('/people/sam/', notes, registry, 1)
    expect(new Set(depth1.nodes.map((n) => n.urlPath))).toEqual(
      new Set(['/people/sam/', '/people/john/', '/people/mary/'])
    )
    // John→Sam, Mary→Sam (hierarchical) + John↔Mary (spouse) = 3 edges.
    expect(depth1.edges).toHaveLength(3)

    // Depth 2 reaches Alice and the grandparents (but not Robert, who is only
    // within reach at depth 3 via George's child edge).
    const depth2 = buildRelationshipGraph('/people/sam/', notes, registry, 2)
    expect(new Set(depth2.nodes.map((n) => n.urlPath))).toEqual(
      new Set(['/people/sam/', '/people/john/', '/people/mary/', '/people/alice/', '/people/george/', '/people/martha/'])
    )
    // 6 parent→child edges + 2 spouse (George↔Martha, John↔Mary) = 8.
    expect(depth2.edges).toHaveLength(8)
    expect(depth2.edges.some((e) => e.relType === 'sibling')).toBe(false)
  })

  it('derives edges purely from other notes for a note with no declarations', () => {
    // Sam's own note declares only incoming/derived edges; they still render.
    const graph = buildRelationshipGraph('/people/sam/', notes, registry, 2)
    const focus = graph.nodes.find((n) => n.isFocus)!
    expect(focus.title).toBe('Sam Doe')
    // Sam has two parents (John, Mary) drawn as hierarchical edges into Sam.
    const intoSam = graph.edges.filter((e) => e.kind === 'hierarchical' && e.to === '/people/sam/')
    expect(intoSam.map((e) => e.from).sort()).toEqual(['/people/john/', '/people/mary/'])
  })

  it('returns an empty graph for an unknown focus', () => {
    const graph = buildRelationshipGraph('/people/nobody/', notes, registry, 3)
    expect(graph.nodes).toHaveLength(0)
    expect(graph.edges).toHaveLength(0)
  })

  it('respects the maxNodes cap', () => {
    const graph = buildRelationshipGraph('/people/john/', notes, registry, 3, 3)
    expect(graph.nodes.length).toBeLessThanOrEqual(3)
  })

  it('normalizes a slashless focus path to the canonical trailing-slash form', () => {
    // Server mode serves markdown at non-trailing-slash URLs in place, so the
    // focus path can arrive without the trailing slash that every site.json
    // `url_path` key carries. The graph must be identical either way.
    const withSlash = buildRelationshipGraph('/people/george/', notes, registry, 3)
    const withoutSlash = buildRelationshipGraph('/people/george', notes, registry, 3)

    // The slashless call must not fall through to the empty-graph guard.
    expect(withoutSlash.nodes.length).toBeGreaterThan(0)
    expect(withoutSlash.edges.length).toBeGreaterThan(0)

    // Identical graphs (same focus, nodes, and edges) regardless of the slash.
    expect(withoutSlash.focus).toBe('/people/george/')
    expect(withoutSlash.focus).toBe(withSlash.focus)
    expect(withoutSlash.nodes).toEqual(withSlash.nodes)
    expect(withoutSlash.edges).toEqual(withSlash.edges)
  })

  it('sets the focus flag on the canonical node for a slashless focus path', () => {
    const graph = buildRelationshipGraph('/people/george', notes, registry, 3)
    const focusNodes = graph.nodes.filter((n) => n.isFocus)
    expect(focusNodes).toHaveLength(1)
    expect(focusNodes[0].urlPath).toBe('/people/george/')
    expect(focusNodes[0].title).toBe('George Doe')
  })

  it('matches a decoded focus path with spaces against decoded site.json keys', () => {
    // End-to-end contract: getCanonicalPath() now returns DECODED paths (literal
    // spaces), matching site.json's decoded url_path/neighbor keys. No
    // percent-encoding reaches buildRelationshipGraph.
    const focusKey = '/Walsh/Patrick Joseph Walsh b.1977-10-01/'
    const spaced = new Map<string, SiteNote>([
      [
        focusKey,
        {
          url_path: focusKey,
          frontmatter: { type: 'person', title: 'Patrick Joseph Walsh' },
          relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/Walsh/Jane Walsh/', direction: 'outgoing' })],
        },
      ],
      [
        '/Walsh/Jane Walsh/',
        {
          url_path: '/Walsh/Jane Walsh/',
          frontmatter: { type: 'person', title: 'Jane Walsh' },
          relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: focusKey, direction: 'incoming' })],
        },
      ],
    ])
    const graph = buildRelationshipGraph(focusKey, spaced, registry, 2)
    expect(graph.nodes.map((n) => n.urlPath).sort()).toEqual(['/Walsh/Jane Walsh/', focusKey])
    expect(graph.edges).toHaveLength(1)
    expect(graph.nodes.find((n) => n.isFocus)!.urlPath).toBe(focusKey)
  })

  it('populates image and bornPlace from frontmatter', () => {
    const withMedia = new Map<string, SiteNote>([
      ['/p/a/', {
        url_path: '/p/a/',
        frontmatter: { title: 'A', image: '/images/a.jpg', born_place: 'Denver, CO' },
        relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/p/b/', direction: 'outgoing' })],
      }],
      ['/p/b/', {
        url_path: '/p/b/',
        frontmatter: { title: 'B' },
        relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/p/a/', direction: 'incoming' })],
      }],
    ])
    const graph = buildRelationshipGraph('/p/a/', withMedia, registry, 1)
    const a = graph.nodes.find((n) => n.urlPath === '/p/a/')!
    expect(a.image).toBe('/images/a.jpg')
    expect(a.bornPlace).toBe('Denver, CO')
    // Absent frontmatter keys yield undefined.
    const b = graph.nodes.find((n) => n.urlPath === '/p/b/')!
    expect(b.image).toBeUndefined()
    expect(b.bornPlace).toBeUndefined()
  })

  it('ignores non-string or blank image/born_place values', () => {
    const bad = new Map<string, SiteNote>([
      ['/p/a/', {
        url_path: '/p/a/',
        frontmatter: { title: 'A', image: 42, born_place: '   ' },
        relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/p/b/', direction: 'outgoing' })],
      }],
      ['/p/b/', {
        url_path: '/p/b/',
        frontmatter: { title: 'B', image: ' /images/b.png ', born_place: 'Boulder' },
        relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/p/a/', direction: 'incoming' })],
      }],
    ])
    const graph = buildRelationshipGraph('/p/a/', bad, registry, 1)
    const a = graph.nodes.find((n) => n.urlPath === '/p/a/')!
    expect(a.image).toBeUndefined()
    expect(a.bornPlace).toBeUndefined()
    // Values are trimmed.
    const b = graph.nodes.find((n) => n.urlPath === '/p/b/')!
    expect(b.image).toBe('/images/b.png')
    expect(b.bornPlace).toBe('Boulder')
  })
})

// ---------------------------------------------------------------------------
// Sibling exclusion
// ---------------------------------------------------------------------------

describe('sibling exclusion', () => {
  it('isGraphRelationship rejects only sibling relationships', () => {
    expect(isGraphRelationship(rel({ rel_type: 'sibling', predicate: 'sibling', neighbor: '/x/', direction: 'outgoing' }))).toBe(false)
    // Matches on either field, case-insensitively.
    expect(isGraphRelationship(rel({ rel_type: 'Sibling', predicate: 'child', neighbor: '/x/', direction: 'outgoing' }))).toBe(false)
    expect(isGraphRelationship(rel({ rel_type: 'child', predicate: 'child', neighbor: '/x/', direction: 'outgoing' }))).toBe(true)
    expect(isGraphRelationship(rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/x/', direction: 'outgoing' }))).toBe(true)
  })

  it('excludes a node reachable ONLY through a sibling link', () => {
    // X has a spouse (Z) and a sibling (Y). Y has no other connection, so it is
    // reachable only via the excluded sibling link and must not appear.
    const notes = new Map<string, SiteNote>([
      ['/x/', { url_path: '/x/', frontmatter: { title: 'X' }, relationships: [
        rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/z/', direction: 'outgoing' }),
        rel({ rel_type: 'sibling', predicate: 'sibling', neighbor: '/y/', direction: 'outgoing' }),
      ] }],
      ['/z/', { url_path: '/z/', frontmatter: { title: 'Z' }, relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/x/', direction: 'incoming' })] }],
      ['/y/', { url_path: '/y/', frontmatter: { title: 'Y' }, relationships: [rel({ rel_type: 'sibling', predicate: 'sibling', neighbor: '/x/', direction: 'incoming' })] }],
    ])
    const graph = buildRelationshipGraph('/x/', notes, registry, 3)
    expect(graph.nodes.map((n) => n.urlPath).sort()).toEqual(['/x/', '/z/'])
    expect(graph.nodes.some((n) => n.urlPath === '/y/')).toBe(false)
    // Only the spouse edge remains.
    expect(graph.edges).toHaveLength(1)
    expect(graph.edges[0].kind).toBe('symmetric')
    expect(graph.edges.some((e) => e.relType === 'sibling')).toBe(false)
  })
})

// ---------------------------------------------------------------------------
// computeGenerations
// ---------------------------------------------------------------------------

describe('computeGenerations', () => {
  /**
   * GP → (P + spouse SP) → (C + SB). C and SB are BOTH children of P (co-children)
   * and also declare a sibling link to each other. The sibling link is excluded
   * from the graph, but SB still appears at the child generation via P→SB.
   */
  function threeGenFamily(): Map<string, SiteNote> {
    const notes: SiteNote[] = [
      {
        url_path: '/gp/',
        frontmatter: { type: 'person', title: 'GP' },
        relationships: [rel({ rel_type: 'child', predicate: 'child', neighbor: '/p/', direction: 'outgoing' })],
      },
      {
        url_path: '/p/',
        frontmatter: { type: 'person', title: 'P' },
        relationships: [
          rel({ rel_type: 'child', predicate: 'parent', neighbor: '/gp/', direction: 'incoming' }),
          rel({ rel_type: 'child', predicate: 'child', neighbor: '/c/', direction: 'outgoing' }),
          rel({ rel_type: 'child', predicate: 'child', neighbor: '/sb/', direction: 'outgoing' }),
          rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/sp/', direction: 'outgoing' }),
        ],
      },
      {
        url_path: '/sp/',
        frontmatter: { type: 'person', title: 'SP' },
        relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/p/', direction: 'incoming' })],
      },
      {
        url_path: '/c/',
        frontmatter: { type: 'person', title: 'C' },
        relationships: [
          rel({ rel_type: 'child', predicate: 'parent', neighbor: '/p/', direction: 'incoming' }),
          rel({ rel_type: 'sibling', predicate: 'sibling', neighbor: '/sb/', direction: 'outgoing' }),
        ],
      },
      {
        url_path: '/sb/',
        frontmatter: { type: 'person', title: 'SB' },
        relationships: [
          rel({ rel_type: 'child', predicate: 'parent', neighbor: '/p/', direction: 'incoming' }),
          rel({ rel_type: 'sibling', predicate: 'sibling', neighbor: '/c/', direction: 'incoming' }),
        ],
      },
    ]
    return new Map(notes.map((n) => [n.url_path, n]))
  }

  it('numbers generations ancestors-first, co-children on the same row', () => {
    // Focus on the middle generation (P): ancestors go negative then normalize
    // so the grandparent row is 0. SB lands on the child row via P→SB (its
    // sibling link to C is excluded).
    const graph = buildRelationshipGraph('/p/', threeGenFamily(), registry, 3)
    const gens = computeGenerations(graph)
    expect(gens.get('/gp/')).toBe(0) // grandparent
    expect(gens.get('/p/')).toBe(1) // parent (focus)
    expect(gens.get('/sp/')).toBe(1) // spouse: same generation as parent
    expect(gens.get('/c/')).toBe(2) // child
    expect(gens.get('/sb/')).toBe(2) // co-child (not via the sibling link)
  })

  it('normalizes the minimum generation to 0', () => {
    const graph = buildRelationshipGraph('/p/', threeGenFamily(), registry, 3)
    const values = [...computeGenerations(graph).values()]
    expect(Math.min(...values)).toBe(0)
  })

  it('returns an empty map for an empty graph', () => {
    expect(computeGenerations({ focus: '/x/', nodes: [], edges: [] }).size).toBe(0)
  })
})
