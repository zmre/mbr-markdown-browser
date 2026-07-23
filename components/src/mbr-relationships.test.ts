/**
 * Unit tests for the pure relationship-graph logic behind `<mbr-relationships>`:
 * registry classification, breadth-first traversal + edge de-duplication, and
 * mermaid-source generation. The fixture mirrors the resolved `relationships`
 * shape emitted in `site.json` for the genealogy test repo.
 */
import { describe, it, expect } from 'vitest'
import {
  buildRegistry,
  classifyRelationship,
  buildRelationshipGraph,
  isGraphRelationship,
  canonicalizeNotePath,
  computeGenerations,
  mermaidNodeId,
  generateMermaidSource,
  parseViewBox,
  formatViewBox,
  clampViewBoxScale,
  zoomViewBoxAtPoint,
  panViewBox,
  clientPointToSvg,
  type ViewBox,
  notesByPathFromSite,
  formatNodeLabel,
  formatLifespan,
  yearOf,
  normalizeGender,
  escapeMermaidLabel,
  sanitizeEdgeLabel,
  hasHierarchy,
  type SiteNote,
  type SiteRelationship,
  type RelationTypeConfig,
} from './mbr-relationships.ts'

// ---------------------------------------------------------------------------
// Fixtures (from the genealogy site.json neighbourhood)
// ---------------------------------------------------------------------------

const GENEALOGY_TYPES: RelationTypeConfig[] = [
  { name: 'child', symmetric: false, inverse: 'parent', label: 'Child', label_plural: 'Children' },
  { name: 'parent', symmetric: false, inverse: 'child', label: 'Parent', label_plural: 'Parents' },
  { name: 'sibling', symmetric: true, inverse: null, label: 'Sibling', label_plural: 'Siblings' },
  { name: 'spouse', symmetric: true, inverse: null, label: 'Spouse', label_plural: 'Spouses' },
]

function rel(partial: Partial<SiteRelationship> & Pick<SiteRelationship, 'rel_type' | 'predicate' | 'neighbor' | 'direction'>): SiteRelationship {
  return {
    neighbor_title: partial.neighbor_title ?? partial.neighbor,
    neighbor_raw: partial.neighbor_raw ?? partial.neighbor,
    resolved: partial.resolved ?? true,
    attributes: partial.attributes ?? {},
    derived: partial.derived ?? false,
    ...partial,
  }
}

/** The full 7-person genealogy neighbourhood keyed by url_path. */
function genealogyNotes(): Map<string, SiteNote> {
  const notes: SiteNote[] = [
    {
      url_path: '/people/george/',
      frontmatter: { type: 'person', title: 'George Doe', born: '1898-02-11', died: '1972-09-30' },
      relationships: [
        rel({ rel_type: 'child', predicate: 'child', neighbor: '/people/john/', direction: 'outgoing' }),
        rel({ rel_type: 'child', predicate: 'child', neighbor: '/people/robert/', direction: 'outgoing' }),
        rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/people/martha/', direction: 'outgoing', attributes: { married: '1920-04-10' } }),
      ],
    },
    {
      url_path: '/people/martha/',
      frontmatter: { type: 'person', title: 'Martha Doe', born: '1901-07-22', died: '1985-01-15' },
      relationships: [
        rel({ rel_type: 'parent', predicate: 'child', neighbor: '/people/john/', direction: 'incoming', derived: true }),
        rel({ rel_type: 'parent', predicate: 'child', neighbor: '/people/robert/', direction: 'incoming', derived: true }),
        rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/people/george/', direction: 'incoming', attributes: { married: '1920-04-10' } }),
      ],
    },
    {
      url_path: '/people/john/',
      frontmatter: { type: 'person', title: 'John Doe', born: '1925-06-02', died: '1999-11-20' },
      relationships: [
        rel({ rel_type: 'child', predicate: 'child', neighbor: '/people/alice/', direction: 'outgoing' }),
        rel({ rel_type: 'child', predicate: 'child', neighbor: '/people/sam/', direction: 'outgoing' }),
        rel({ rel_type: 'child', predicate: 'parent', neighbor: '/people/george/', direction: 'incoming' }),
        rel({ rel_type: 'parent', predicate: 'parent', neighbor: '/people/martha/', direction: 'outgoing' }),
        rel({ rel_type: 'sibling', predicate: 'sibling', neighbor: '/people/robert/', direction: 'outgoing' }),
        rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/people/mary/', direction: 'outgoing', attributes: { married: '1948-06-01', place: 'Denver, CO' } }),
      ],
    },
    {
      url_path: '/people/mary/',
      frontmatter: { type: 'person', title: 'Mary Smith', born: '1927-03-19', died: '2010-08-05' },
      relationships: [
        rel({ rel_type: 'child', predicate: 'child', neighbor: '/people/alice/', direction: 'outgoing' }),
        rel({ rel_type: 'child', predicate: 'child', neighbor: '/people/sam/', direction: 'outgoing' }),
        rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/people/john/', direction: 'incoming', derived: true, attributes: { married: '1948-06-01', place: 'Denver, CO' } }),
      ],
    },
    {
      url_path: '/people/robert/',
      frontmatter: { type: 'person', title: 'Robert Doe', born: '1929-12-01' },
      relationships: [
        rel({ rel_type: 'child', predicate: 'parent', neighbor: '/people/george/', direction: 'incoming' }),
        rel({ rel_type: 'parent', predicate: 'parent', neighbor: '/people/martha/', direction: 'outgoing' }),
        rel({ rel_type: 'sibling', predicate: 'sibling', neighbor: '/people/john/', direction: 'incoming', derived: true }),
        // Deliberately unresolved endpoint (Jane Ghost): must be skipped.
        rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '', neighbor_title: 'Jane Ghost', neighbor_raw: '[[Jane Ghost]]', resolved: false, direction: 'outgoing', attributes: { married: '1955-05-05' } }),
      ],
    },
    {
      url_path: '/people/alice/',
      frontmatter: { type: 'person', title: 'Alice Doe', born: '1950-10-08' },
      relationships: [
        rel({ rel_type: 'child', predicate: 'parent', neighbor: '/people/john/', direction: 'incoming', derived: true }),
        rel({ rel_type: 'child', predicate: 'parent', neighbor: '/people/mary/', direction: 'incoming', derived: true }),
        rel({ rel_type: 'sibling', predicate: 'sibling', neighbor: '/people/sam/', direction: 'outgoing' }),
      ],
    },
    {
      url_path: '/people/sam/',
      frontmatter: { type: 'person', title: 'Sam Doe', born: '1953-04-27' },
      relationships: [
        rel({ rel_type: 'child', predicate: 'parent', neighbor: '/people/john/', direction: 'incoming', derived: true }),
        rel({ rel_type: 'child', predicate: 'parent', neighbor: '/people/mary/', direction: 'incoming', derived: true }),
        rel({ rel_type: 'sibling', predicate: 'sibling', neighbor: '/people/alice/', direction: 'incoming', derived: true }),
      ],
    },
  ]
  return new Map(notes.map((n) => [n.url_path, n]))
}

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

  it('escapeMermaidLabel neutralizes quotes and newlines', () => {
    expect(escapeMermaidLabel('A "quoted" name')).toBe('A #quot;quoted#quot; name')
    expect(escapeMermaidLabel('line1\nline2')).toBe('line1 line2')
  })

  it('sanitizeEdgeLabel keeps only a safe subset', () => {
    expect(sanitizeEdgeLabel('married | 1948')).toBe('married 1948')
    expect(sanitizeEdgeLabel('Spouse')).toBe('Spouse')
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
// generateMermaidSource
// ---------------------------------------------------------------------------

describe('generateMermaidSource', () => {
  const notes = genealogyNotes()

  it('renders a top-down family tree grouped into per-generation LR subgraphs', () => {
    const graph = buildRelationshipGraph('/people/john/', notes, registry, 3)
    const src = generateMermaidSource(graph)

    expect(src.startsWith('graph TD')).toBe(true)
    expect(hasHierarchy(graph)).toBe(true)

    // Node label includes the lifespan.
    expect(src).toContain('John Doe (1925–1999)')

    // Three generations (grandparents / John's row / grandchildren), each in a
    // `subgraph … direction LR` block.
    const subgraphs = src.split('\n').filter((l) => /^ {2}subgraph gen\d+ \[" "\]$/.test(l))
    expect(subgraphs).toHaveLength(3)
    const lrDirectives = src.split('\n').filter((l) => /^ {4}direction LR$/.test(l))
    expect(lrDirectives).toHaveLength(3)

    // One node declaration per node, indented inside its generation subgraph.
    const nodeDecls = src.split('\n').filter((l) => /^ {4}n\d+\["/.test(l))
    expect(nodeDecls).toHaveLength(7)

    // Edges are emitted after the subgraphs (at top-level indent), one per edge.
    const arrowEdges = src.split('\n').filter((l) => l.includes('-->'))
    expect(arrowEdges).toHaveLength(8) // hierarchical arrows
    const dottedEdges = src.split('\n').filter((l) => /-\.-|-\. /.test(l))
    expect(dottedEdges).toHaveLength(2) // symmetric dotted links (spouses only)
    // Sibling links are excluded entirely.
    expect(src).not.toContain('Sibling')

    // Generations are emitted ancestors-first (gen0 before gen1 before gen2).
    const genOrder = src
      .split('\n')
      .filter((l) => /^ {2}subgraph gen\d+ /.test(l))
      .map((l) => l.match(/gen(\d+)/)![1])
    expect(genOrder).toEqual(['0', '1', '2'])

    // Focus highlighting present.
    expect(src).toContain('classDef focus')
    expect(/class n\d+ focus;/.test(src)).toBe(true)
  })

  it('uses left-to-right orientation when there is no hierarchy', () => {
    // A note with only a symmetric spouse link → no hierarchical edges.
    const flat = new Map<string, SiteNote>([
      ['/p/a/', { url_path: '/p/a/', frontmatter: { title: 'A' }, relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/p/b/', direction: 'outgoing' })] }],
      ['/p/b/', { url_path: '/p/b/', frontmatter: { title: 'B' }, relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/p/a/', direction: 'incoming' })] }],
    ])
    const graph = buildRelationshipGraph('/p/a/', flat, registry, 2)
    const src = generateMermaidSource(graph)
    expect(src.startsWith('graph LR')).toBe(true)
    expect(hasHierarchy(graph)).toBe(false)
  })

  it('escapes quotes in node labels so the diagram stays valid', () => {
    const tricky = new Map<string, SiteNote>([
      ['/p/a/', { url_path: '/p/a/', frontmatter: { title: 'A "The Great"' }, relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/p/b/', direction: 'outgoing' })] }],
      ['/p/b/', { url_path: '/p/b/', frontmatter: { title: 'B' }, relationships: [] }],
    ])
    const graph = buildRelationshipGraph('/p/a/', tricky, registry, 1)
    const src = generateMermaidSource(graph)
    expect(src).toContain('A #quot;The Great#quot;')
    expect(src).not.toContain('A "The Great"')
  })
})

// ---------------------------------------------------------------------------
// Gender tinting
// ---------------------------------------------------------------------------

describe('gender tinting', () => {
  /** A gendered couple: John (male) — Mary (female). */
  function genderedCouple(): Map<string, SiteNote> {
    const notes: SiteNote[] = [
      {
        url_path: '/p/john/',
        frontmatter: { type: 'person', title: 'John', gender: 'Male' },
        relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/p/mary/', direction: 'outgoing' })],
      },
      {
        url_path: '/p/mary/',
        frontmatter: { type: 'person', title: 'Mary', gender: 'FEMALE' },
        relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/p/john/', direction: 'incoming' })],
      },
    ]
    return new Map(notes.map((n) => [n.url_path, n]))
  }

  it('buildRelationshipGraph populates node.gender (lowercased) from frontmatter', () => {
    const graph = buildRelationshipGraph('/p/john/', genderedCouple(), registry, 1)
    expect(graph.nodes.find((n) => n.urlPath === '/p/john/')!.gender).toBe('male')
    expect(graph.nodes.find((n) => n.urlPath === '/p/mary/')!.gender).toBe('female')
  })

  it('generateMermaidSource emits per-gender classDefs and assigns them', () => {
    const graph = buildRelationshipGraph('/p/john/', genderedCouple(), registry, 1)
    const src = generateMermaidSource(graph)

    expect(src).toContain('classDef genderFemale')
    expect(src).toContain('classDef genderMale')
    expect(/class n\d+ genderFemale;/.test(src)).toBe(true)
    expect(/class n\d+ genderMale;/.test(src)).toBe(true)

    // The focus assignment must remain the LAST class assignment (focus wins).
    const classLines = src.split('\n').filter((l) => /^ {2}class n\d+ /.test(l))
    expect(classLines[classLines.length - 1]).toMatch(/class n\d+ focus;/)
  })

  it('emits no gender classDef when no node declares a gender', () => {
    const plain = new Map<string, SiteNote>([
      ['/p/a/', { url_path: '/p/a/', frontmatter: { title: 'A' }, relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/p/b/', direction: 'outgoing' })] }],
      ['/p/b/', { url_path: '/p/b/', frontmatter: { title: 'B' }, relationships: [rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/p/a/', direction: 'incoming' })] }],
    ])
    const graph = buildRelationshipGraph('/p/a/', plain, registry, 1)
    const src = generateMermaidSource(graph)
    expect(src).not.toContain('classDef genderFemale')
    expect(src).not.toContain('classDef genderMale')
    // Focus highlight still emitted.
    expect(/class n\d+ focus;/.test(src)).toBe(true)
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

// ---------------------------------------------------------------------------
// mermaidNodeId
// ---------------------------------------------------------------------------

describe('mermaidNodeId', () => {
  it('extracts the node id from a mermaid v11 flowchart element id', () => {
    expect(mermaidNodeId('flowchart-n0-1')).toBe('n0')
    expect(mermaidNodeId('flowchart-n12-3')).toBe('n12')
    expect(mermaidNodeId('flowchart-n7-10')).toBe('n7')
  })

  it('falls back to a trailing n<number>', () => {
    expect(mermaidNodeId('n5')).toBe('n5')
    expect(mermaidNodeId('flowchart-n2')).toBe('n2')
  })

  it('returns null when there is no node id', () => {
    expect(mermaidNodeId('')).toBeNull()
    expect(mermaidNodeId('flowchart-node-0')).toBeNull()
    expect(mermaidNodeId('random-thing')).toBeNull()
  })
})

// ---------------------------------------------------------------------------
// Viewport (zoom / pan) math
// ---------------------------------------------------------------------------

describe('parseViewBox / formatViewBox', () => {
  it('parses a space-separated viewBox', () => {
    expect(parseViewBox('0 0 100 50')).toEqual({ x: 0, y: 0, w: 100, h: 50 })
  })

  it('parses a comma/whitespace-separated viewBox', () => {
    expect(parseViewBox(' 1, 2, 30 , 40 ')).toEqual({ x: 1, y: 2, w: 30, h: 40 })
  })

  it('rejects malformed, wrong-arity, or non-positive-size boxes', () => {
    expect(parseViewBox(null)).toBeNull()
    expect(parseViewBox('')).toBeNull()
    expect(parseViewBox('0 0 100')).toBeNull()
    expect(parseViewBox('0 0 100 nan')).toBeNull()
    expect(parseViewBox('0 0 0 50')).toBeNull()
    expect(parseViewBox('0 0 100 -5')).toBeNull()
  })

  it('round-trips through formatViewBox', () => {
    const vb: ViewBox = { x: 5, y: -3, w: 200, h: 120 }
    expect(formatViewBox(vb)).toBe('5 -3 200 120')
    expect(parseViewBox(formatViewBox(vb))).toEqual(vb)
  })
})

describe('clampViewBoxScale', () => {
  const baseW = 100
  it('clamps zoom-in to maxScale (smallest width)', () => {
    // Requesting an absurdly small width clamps to baseW / maxScale.
    expect(clampViewBoxScale(1, baseW, 1, 8)).toBeCloseTo(100 / 8)
  })

  it('clamps zoom-out to minScale (largest width)', () => {
    // Requesting a huge width clamps to baseW / minScale.
    expect(clampViewBoxScale(10000, baseW, 1, 8)).toBeCloseTo(100 / 1)
  })

  it('passes through a width within bounds', () => {
    expect(clampViewBoxScale(40, baseW, 1, 8)).toBe(40)
  })
})

describe('zoomViewBoxAtPoint', () => {
  const base: ViewBox = { x: 0, y: 0, w: 100, h: 80 }
  const opts = { minScale: 1, maxScale: 8, baseW: 100 }

  it('keeps the zoom point fixed (same fractional position)', () => {
    const point = { x: 25, y: 20 }
    const before = { fx: (point.x - base.x) / base.w, fy: (point.y - base.y) / base.h }
    const out = zoomViewBoxAtPoint(base, 2, point, opts)
    const after = { fx: (point.x - out.x) / out.w, fy: (point.y - out.y) / out.h }
    expect(after.fx).toBeCloseTo(before.fx)
    expect(after.fy).toBeCloseTo(before.fy)
  })

  it('zooms in: a factor > 1 shrinks the viewBox uniformly', () => {
    const out = zoomViewBoxAtPoint(base, 2, { x: 50, y: 40 }, opts)
    expect(out.w).toBeCloseTo(50)
    expect(out.h).toBeCloseTo(40) // aspect preserved
  })

  it('zooms out: a factor < 1 grows the viewBox (bounded by minScale = fit)', () => {
    // Already at fit (scale 1); zooming out cannot exceed the base width.
    const out = zoomViewBoxAtPoint(base, 0.5, { x: 50, y: 40 }, opts)
    expect(out.w).toBeCloseTo(100)
  })

  it('does not zoom in past maxScale', () => {
    const out = zoomViewBoxAtPoint(base, 1000, { x: 50, y: 40 }, opts)
    expect(out.w).toBeCloseTo(100 / 8)
  })

  it('returns the input unchanged for a non-positive factor or empty box', () => {
    expect(zoomViewBoxAtPoint(base, 0, { x: 0, y: 0 }, opts)).toEqual(base)
    expect(zoomViewBoxAtPoint({ x: 0, y: 0, w: 0, h: 0 }, 2, { x: 0, y: 0 }, opts)).toEqual({ x: 0, y: 0, w: 0, h: 0 })
  })
})

describe('panViewBox', () => {
  it('translates the origin by the negated user delta', () => {
    const vb: ViewBox = { x: 10, y: 20, w: 100, h: 80 }
    // Dragging content right (positive dx) moves the viewBox origin left.
    expect(panViewBox(vb, 5, -3)).toEqual({ x: 5, y: 23, w: 100, h: 80 })
  })

  it('preserves width and height', () => {
    const vb: ViewBox = { x: 0, y: 0, w: 100, h: 80 }
    const out = panViewBox(vb, 40, 40)
    expect(out.w).toBe(100)
    expect(out.h).toBe(80)
  })
})

describe('clientPointToSvg', () => {
  const rect = { left: 0, top: 0, width: 200, height: 160 }
  const vb: ViewBox = { x: 0, y: 0, w: 100, h: 80 }

  it('maps the canvas center to the viewBox center', () => {
    expect(clientPointToSvg(100, 80, rect, vb)).toEqual({ x: 50, y: 40 })
  })

  it('maps a corner accounting for the rect offset', () => {
    const offset = { left: 20, top: 10, width: 200, height: 160 }
    expect(clientPointToSvg(20, 10, offset, vb)).toEqual({ x: 0, y: 0 })
    expect(clientPointToSvg(220, 170, offset, vb)).toEqual({ x: 100, y: 80 })
  })

  it('accounts for a non-zero viewBox origin', () => {
    const shifted: ViewBox = { x: 10, y: 5, w: 100, h: 80 }
    expect(clientPointToSvg(0, 0, rect, shifted)).toEqual({ x: 10, y: 5 })
  })
})
