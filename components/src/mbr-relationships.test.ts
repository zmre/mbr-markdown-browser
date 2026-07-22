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
  canonicalizeNotePath,
  generateMermaidSource,
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
    // All seven people are reachable within one hop of John.
    expect(graph.nodes).toHaveLength(7)
    // 8 parent→child edges + 4 symmetric (2 spouse, 2 sibling) = 12 unique.
    expect(graph.edges).toHaveLength(12)
    expect(graph.edges.filter((e) => e.kind === 'hierarchical')).toHaveLength(8)
    expect(graph.edges.filter((e) => e.kind === 'symmetric')).toHaveLength(4)
    // The unresolved Jane Ghost spouse edge must not appear.
    expect(graph.nodes.some((n) => n.title === 'Jane Ghost')).toBe(false)
    // Focus flag is set exactly once.
    expect(graph.nodes.filter((n) => n.isFocus)).toHaveLength(1)
    expect(graph.nodes.find((n) => n.isFocus)!.urlPath).toBe('/people/john/')
  })

  it('honours the depth bound (nodes within N hops of the focus)', () => {
    // Sam declares no edges of his own; at depth 1 only his direct neighbours appear.
    const depth1 = buildRelationshipGraph('/people/sam/', notes, registry, 1)
    expect(new Set(depth1.nodes.map((n) => n.urlPath))).toEqual(
      new Set(['/people/sam/', '/people/john/', '/people/mary/', '/people/alice/'])
    )
    // Edges among the collected set (Sam's parents/sibling + the parents' own links).
    expect(depth1.edges).toHaveLength(6)

    // Depth 2 reaches the grandparents and uncle: the full family.
    const depth2 = buildRelationshipGraph('/people/sam/', notes, registry, 2)
    expect(depth2.nodes).toHaveLength(7)
    expect(depth2.edges).toHaveLength(12)
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
// generateMermaidSource
// ---------------------------------------------------------------------------

describe('generateMermaidSource', () => {
  const notes = genealogyNotes()

  it('renders a top-down family tree with a highlighted focus', () => {
    const graph = buildRelationshipGraph('/people/john/', notes, registry, 3)
    const src = generateMermaidSource(graph)

    expect(src.startsWith('graph TD')).toBe(true)
    expect(hasHierarchy(graph)).toBe(true)

    // Node label includes the lifespan.
    expect(src).toContain('John Doe (1925–1999)')

    // One node declaration per node, and one edge statement per unique edge.
    const nodeDecls = src.split('\n').filter((l) => /^ {2}n\d+\["/.test(l))
    expect(nodeDecls).toHaveLength(7)
    const arrowEdges = src.split('\n').filter((l) => l.includes('-->'))
    expect(arrowEdges).toHaveLength(8) // hierarchical arrows
    const dottedEdges = src.split('\n').filter((l) => /-\.-|-\. /.test(l))
    expect(dottedEdges).toHaveLength(4) // symmetric dotted links

    // Focus highlighting present.
    expect(src).toContain('classDef focus')
    expect(/class n\d+ focus;/.test(src)).toBe(true)
  })

  it('uses left-to-right orientation when there is no hierarchy', () => {
    // A note with only symmetric siblings → no hierarchical edges.
    const flat = new Map<string, SiteNote>([
      ['/p/a/', { url_path: '/p/a/', frontmatter: { title: 'A' }, relationships: [rel({ rel_type: 'sibling', predicate: 'sibling', neighbor: '/p/b/', direction: 'outgoing' })] }],
      ['/p/b/', { url_path: '/p/b/', frontmatter: { title: 'B' }, relationships: [rel({ rel_type: 'sibling', predicate: 'sibling', neighbor: '/p/a/', direction: 'incoming' })] }],
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
