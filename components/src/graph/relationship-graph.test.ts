/**
 * Unit tests for the pure relationship-graph logic: registry classification,
 * breadth-first traversal + edge de-duplication, and generation numbering.
 * The shared fixture mirrors the resolved `relationships` shape emitted in
 * `site.json` for the genealogy test repo.
 */
import { describe, it, expect } from 'vitest'
import {
  breakHierarchicalCycles,
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
  type GraphEdge,
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
// Hierarchical cycle breaking
// ---------------------------------------------------------------------------

const hier = (from: string, to: string, relType = 'child'): GraphEdge => ({
  from,
  to,
  kind: 'hierarchical',
  relType,
  label: '',
})

const directed = (from: string, to: string): GraphEdge => ({
  from,
  to,
  kind: 'directed',
  relType: 'depends_on',
  label: 'Depends on',
})

/**
 * Independent acyclicity check over the hierarchical edges, by Kahn's algorithm:
 * a topological sort consumes every node exactly when the graph is acyclic.
 * Deliberately NOT the DFS the implementation uses, so the same mistake cannot
 * hide in both.
 */
function hierarchyIsAcyclic(edges: GraphEdge[]): boolean {
  const inDegree = new Map<string, number>()
  const outgoing = new Map<string, string[]>()
  for (const edge of edges) {
    if (edge.kind !== 'hierarchical') continue
    inDegree.set(edge.from, inDegree.get(edge.from) ?? 0)
    inDegree.set(edge.to, (inDegree.get(edge.to) ?? 0) + 1)
    outgoing.set(edge.from, [...(outgoing.get(edge.from) ?? []), edge.to])
  }
  const queue = [...inDegree].filter(([, degree]) => degree === 0).map(([node]) => node)
  let sorted = 0
  while (queue.length > 0) {
    const node = queue.shift()!
    sorted++
    for (const next of outgoing.get(node) ?? []) {
      const degree = (inDegree.get(next) ?? 0) - 1
      inDegree.set(next, degree)
      if (degree === 0) queue.push(next)
    }
  }
  return sorted === inDegree.size
}

/**
 * Notes with contradictory frontmatter: each declares the NEXT note in `chain`
 * as its own parent, so the last one closes the loop back to the first.
 * `classifyRelationship` canonicalizes each declaration onto `hier|child|…` with
 * swapped endpoints, so every one of them survives de-duplication as a distinct
 * edge — the exact shape that hangs family-chart's `d3.hierarchy()` walk.
 */
function parentLoop(chain: string[]): Map<string, SiteNote> {
  const notes: SiteNote[] = chain.map((path, i) => ({
    url_path: path,
    frontmatter: { type: 'person', title: path },
    relationships: [
      rel({
        rel_type: 'child',
        predicate: 'parent',
        neighbor: chain[(i + 1) % chain.length],
        direction: 'incoming',
      }),
    ],
  }))
  return new Map(notes.map((n) => [n.url_path, n]))
}

describe('breakHierarchicalCycles', () => {
  it('drops the back edge of a 2-cycle and keeps the rest', () => {
    const { edges, droppedEdges } = breakHierarchicalCycles(
      [hier('/a/', '/b/'), hier('/b/', '/a/')],
      '/a/'
    )
    expect(droppedEdges).toEqual([hier('/b/', '/a/')])
    expect(edges).toEqual([hier('/a/', '/b/')])
    expect(hierarchyIsAcyclic(edges)).toBe(true)
  })

  it('leaves non-hierarchical cycles alone', () => {
    // Directed/symmetric cycles are harmless (and meaningful): never touched.
    const input = [directed('/a/', '/b/'), directed('/b/', '/a/')]
    const { edges, droppedEdges } = breakHierarchicalCycles(input, '/a/')
    expect(droppedEdges).toEqual([])
    expect(edges).toEqual(input)
  })

  it('drops a self-loop', () => {
    const { edges, droppedEdges } = breakHierarchicalCycles([hier('/a/', '/a/')], '/a/')
    expect(droppedEdges).toEqual([hier('/a/', '/a/')])
    expect(edges).toEqual([])
  })

  it('keeps forward and cross edges (a diamond is not a cycle)', () => {
    const input = [hier('/a/', '/b/'), hier('/a/', '/c/'), hier('/b/', '/d/'), hier('/c/', '/d/')]
    const { edges, droppedEdges } = breakHierarchicalCycles(input, '/a/')
    expect(droppedEdges).toEqual([])
    expect(edges).toEqual(input)
    expect(hierarchyIsAcyclic(edges)).toBe(true)
  })

  it('is deterministic regardless of the input edge order', () => {
    const cycle = [hier('/a/', '/b/'), hier('/b/', '/c/'), hier('/c/', '/a/')]
    const forward = breakHierarchicalCycles(cycle, '/a/')
    const reversed = breakHierarchicalCycles([...cycle].reverse(), '/a/')
    expect(reversed.droppedEdges).toEqual(forward.droppedEdges)
    // The focus is the first DFS root, so its own subtree survives and the edge
    // closing back onto it is the one dropped.
    expect(forward.droppedEdges).toEqual([hier('/c/', '/a/')])
  })

  it('handles a lineage deeper than the JS call stack (no recursion)', () => {
    // A recursive DFS would blow the stack well before 20k frames.
    const depth = 20_000
    const chain: GraphEdge[] = []
    for (let i = 0; i < depth; i++) chain.push(hier(`/n/${i}/`, `/n/${i + 1}/`))
    chain.push(hier(`/n/${depth}/`, '/n/0/')) // closes the whole chain into a loop
    const { edges, droppedEdges } = breakHierarchicalCycles(chain, '/n/0/')
    expect(droppedEdges).toHaveLength(1)
    expect(edges).toHaveLength(depth)
    expect(hierarchyIsAcyclic(edges)).toBe(true)
  })
})

describe('buildRelationshipGraph acyclic-hierarchy invariant', () => {
  it('breaks a 2-cycle of mutually-declared parents', () => {
    const graph = buildRelationshipGraph('/p/a/', parentLoop(['/p/a/', '/p/b/']), registry, 3)
    expect(graph.nodes).toHaveLength(2)
    expect(hierarchyIsAcyclic(graph.edges)).toBe(true)
    expect(graph.edges.filter((e) => e.kind === 'hierarchical')).toHaveLength(1)
    // The focus is the first DFS root, so the edge out of the focus survives.
    expect(graph.droppedEdges).toEqual([hier('/p/b/', '/p/a/')])
    expect(graph.edges).toEqual([hier('/p/a/', '/p/b/')])
  })

  it('breaks a 3-cycle of mutually-declared parents', () => {
    const graph = buildRelationshipGraph(
      '/p/a/',
      parentLoop(['/p/a/', '/p/b/', '/p/c/']),
      registry,
      3
    )
    expect(graph.nodes).toHaveLength(3)
    expect(hierarchyIsAcyclic(graph.edges)).toBe(true)
    expect(graph.edges.filter((e) => e.kind === 'hierarchical')).toHaveLength(2)
    expect(graph.droppedEdges).toEqual([hier('/p/b/', '/p/a/')])
  })

  it('drops the same edges on every build, whatever the note order', () => {
    const chain = ['/p/a/', '/p/b/', '/p/c/']
    const first = buildRelationshipGraph('/p/a/', parentLoop(chain), registry, 3)
    const second = buildRelationshipGraph('/p/a/', parentLoop(chain), registry, 3)
    // Reversed insertion order changes Map iteration order, which must not
    // change which edge is dropped.
    const shuffled = new Map([...parentLoop(chain)].reverse())
    const third = buildRelationshipGraph('/p/a/', shuffled, registry, 3)

    expect(second.droppedEdges).toEqual(first.droppedEdges)
    expect(third.droppedEdges).toEqual(first.droppedEdges)
    expect(third.edges.filter((e) => e.kind === 'hierarchical')).toEqual(
      first.edges.filter((e) => e.kind === 'hierarchical')
    )
  })

  it('leaves an acyclic family tree untouched', () => {
    const graph = buildRelationshipGraph('/people/john/', genealogyNotes(), registry, 3)
    expect(graph.droppedEdges).toEqual([])
    expect(graph.edges).toHaveLength(10)
    expect(hierarchyIsAcyclic(graph.edges)).toBe(true)
  })

  it('does not flag a properly reciprocal parent/child declaration', () => {
    // The parent says "C is my child" and the child says "P is my parent". Both
    // canonicalize to the SAME key, so they collapse into one edge — a correctly
    // authored pair must never look like a contradiction.
    const notes = new Map<string, SiteNote>([
      [
        '/p/parent/',
        {
          url_path: '/p/parent/',
          frontmatter: { title: 'Parent' },
          relationships: [
            rel({
              rel_type: 'child',
              predicate: 'child',
              neighbor: '/p/child/',
              direction: 'outgoing',
            }),
          ],
        },
      ],
      [
        '/p/child/',
        {
          url_path: '/p/child/',
          frontmatter: { title: 'Child' },
          relationships: [
            rel({
              rel_type: 'child',
              predicate: 'parent',
              neighbor: '/p/parent/',
              direction: 'incoming',
            }),
          ],
        },
      ],
    ])
    const graph = buildRelationshipGraph('/p/parent/', notes, registry, 2)
    expect(graph.edges).toEqual([hier('/p/parent/', '/p/child/')])
    expect(graph.droppedEdges).toEqual([])
  })

  it('reports no dropped edges for an unknown focus', () => {
    expect(buildRelationshipGraph('/people/nobody/', genealogyNotes(), registry, 3).droppedEdges)
      .toEqual([])
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
