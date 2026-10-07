/**
 * Contacts contract (spec §0.3/0.4): registry-driven orientation of
 * hierarchical edges, categories, the family-only subgraph, and the partial
 * `dates.*` readers — each exercised against BOTH registry shapes: the new one
 * (`hierarchy` + `category` on every type) and the legacy one without them.
 */
import { describe, it, expect } from 'vitest'
import {
  buildRegistry,
  buildRelationshipGraph,
  classifyRelationship,
  familyGraph,
  hasFamilyEdges,
  isFamilyEdge,
  isWorkHierarchyEdge,
  lifeYears,
  yearOf,
  type GraphEdge,
  type RelationTypeConfig,
} from './relationship-graph.js'
import {
  CONTACT_TYPES,
  GENEALOGY_TYPES,
  LEGACY_CONTACT_TYPES,
  buildSiteNotes,
  companyNotes,
  rel,
} from './test-fixtures.js'

const modern = buildRegistry(CONTACT_TYPES)
const legacy = buildRegistry(LEGACY_CONTACT_TYPES)

/** Edges as `superior>subordinate:relType` strings, sorted, for compact asserts. */
function hierarchy(edges: GraphEdge[]): string[] {
  return edges
    .filter((e) => e.kind === 'hierarchical')
    .map((e) => `${e.from}>${e.to}:${e.relType}`)
    .sort()
}

describe('Registry hierarchy/category lookups', () => {
  it('reads hierarchy and category from the registry', () => {
    expect(modern.hierarchyOf('reports_to')).toBe('up')
    expect(modern.hierarchyOf('MANAGES')).toBe('down')
    expect(modern.hierarchyOf('spouse')).toBeUndefined()
    expect(modern.categoryOf('assistant')).toBe('work')
    expect(modern.categoryOf('sibling')).toBe('family')
    expect(modern.hasCategories).toBe(true)
  })

  it('derives a missing hierarchy from the inverse, flipped', () => {
    const types: RelationTypeConfig[] = [
      { name: 'mentor', inverse: 'mentee', symmetric: false, label: 'Mentor', label_plural: 'Mentors', hierarchy: 'up' },
      { name: 'mentee', inverse: 'mentor', symmetric: false, label: 'Mentee', label_plural: 'Mentees' },
    ]
    expect(buildRegistry(types).hierarchyOf('mentee')).toBe('down')
  })

  it('inherits a missing category from the inverse', () => {
    const types: RelationTypeConfig[] = [
      { name: 'mentor', inverse: 'mentee', symmetric: false, label: 'Mentor', label_plural: 'Mentors', category: 'work' },
      { name: 'mentee', inverse: 'mentor', symmetric: false, label: 'Mentee', label_plural: 'Mentees' },
    ]
    expect(buildRegistry(types).categoryOf('mentee')).toBe('work')
  })

  it('falls back to the built-in categories only when no type has one', () => {
    expect(legacy.hasCategories).toBe(false)
    expect(legacy.categoryOf('reports_to')).toBe('work')
    expect(legacy.categoryOf('child')).toBe('family')
    expect(legacy.categoryOf('mentor')).toBeUndefined()
    // A modern registry never borrows a built-in category for a type it left blank.
    const mixed = buildRegistry([
      ...CONTACT_TYPES,
      { name: 'friend', inverse: null, symmetric: true, label: 'Friend', label_plural: 'Friends' },
    ])
    expect(mixed.categoryOf('friend')).toBeUndefined()
  })

  it('reports no hierarchy for a legacy registry', () => {
    expect(legacy.hierarchyOf('reports_to')).toBeUndefined()
  })
})

describe('classifyRelationship orientation (contract 0.4)', () => {
  // Regression: lexicographic canonicalization put whichever half sorted first
  // on the subordinate side. For `manager`/`report` that is "manager", so every
  // manager was drawn BELOW their report.
  const managerReport: RelationTypeConfig[] = [
    { name: 'manager', inverse: 'report', symmetric: false, label: 'Manager', label_plural: 'Managers', hierarchy: 'up', category: 'work' },
    { name: 'report', inverse: 'manager', symmetric: false, label: 'Report', label_plural: 'Reports', hierarchy: 'down', category: 'work' },
  ]

  it('puts a manager above their report from both viewpoints', () => {
    const registry = buildRegistry(managerReport)
    // On Sam's note: "Mia is Sam's manager".
    const fromReport = classifyRelationship(
      '/sam/',
      rel({ rel_type: 'manager', predicate: 'manager', neighbor: '/mia/', direction: 'outgoing' }),
      registry
    )
    // On Mia's note, the derived reverse: "Sam is Mia's report".
    const fromManager = classifyRelationship(
      '/mia/',
      rel({ rel_type: 'manager', predicate: 'report', neighbor: '/sam/', direction: 'incoming', derived: true }),
      registry
    )
    expect(fromReport?.edge).toMatchObject({ from: '/mia/', to: '/sam/', relType: 'report', category: 'work' })
    expect(fromManager?.key).toBe(fromReport?.key)
  })

  it('without hierarchy keeps the legacy (upside-down) lexicographic orientation', () => {
    const registry = buildRegistry(managerReport.map(({ hierarchy: _h, ...t }) => t))
    const classified = classifyRelationship(
      '/sam/',
      rel({ rel_type: 'manager', predicate: 'manager', neighbor: '/mia/', direction: 'outgoing' }),
      registry
    )
    expect(classified?.edge).toMatchObject({ from: '/sam/', to: '/mia/', relType: 'manager' })
  })

  it.each([
    ['reports_to', '/boss/', '/me/', 'manages'],
    ['manages', '/me/', '/boss/', 'manages'],
    ['assists', '/boss/', '/me/', 'assistant'],
    ['assistant', '/me/', '/boss/', 'assistant'],
    ['employer', '/boss/', '/me/', 'employee'],
    ['employee', '/me/', '/boss/', 'employee'],
    ['parent', '/boss/', '/me/', 'child'],
    ['child', '/me/', '/boss/', 'child'],
  ])('%s on /me/ toward /boss/ → %s above %s as %s', (type, superior, subordinate, canonical) => {
    const classified = classifyRelationship(
      '/me/',
      rel({ rel_type: type, predicate: type, neighbor: '/boss/', direction: 'outgoing' }),
      modern
    )
    expect(classified?.edge).toMatchObject({ from: superior, to: subordinate, relType: canonical, kind: 'hierarchical' })
  })

  it('tags symmetric and directed edges with their category', () => {
    const colleague = classifyRelationship(
      '/a/',
      rel({ rel_type: 'colleague', predicate: 'colleague', neighbor: '/b/', direction: 'outgoing' }),
      modern
    )
    expect(colleague?.edge).toMatchObject({ kind: 'symmetric', category: 'work' })
    const unknown = classifyRelationship(
      '/a/',
      rel({ rel_type: 'likes', predicate: 'likes', neighbor: '/b/', direction: 'outgoing' }),
      modern
    )
    expect(unknown?.edge.category).toBeUndefined()
  })
})

describe('family/work split', () => {
  for (const [name, types] of [
    ['modern registry', CONTACT_TYPES],
    ['legacy registry', LEGACY_CONTACT_TYPES],
  ] as const) {
    describe(name, () => {
      const registry = buildRegistry(types)
      const notes = companyNotes(types)

      it('orients the whole company top-down', () => {
        const graph = buildRelationshipGraph('/people/bob/', notes, registry, 3)
        expect(hierarchy(graph.edges)).toEqual(
          expect.arrayContaining([
            '/people/ada/>/people/bob/:manages',
            '/people/bob/>/people/carol/:manages',
            '/people/ada/>/people/dan/:assistant',
            '/orgs/acme/>/people/bob/:employee',
            '/people/george/>/people/bob/:child',
          ])
        )
      })

      it('keeps only family edges, and the nodes they reach, on family charts', () => {
        const graph = buildRelationshipGraph('/people/bob/', notes, registry, 3)
        const family = familyGraph(graph, registry)
        expect(hierarchy(family.edges)).toEqual(['/people/george/>/people/bob/:child'])
        expect(family.nodes.map((n) => n.urlPath).sort()).toEqual(['/people/bob/', '/people/george/'])
        expect(hasFamilyEdges(graph, registry)).toBe(true)
        expect(graph.edges.filter(isWorkHierarchyEdge).length).toBeGreaterThan(0)
      })

      it('finds no family edges for a work-only person', () => {
        const graph = buildRelationshipGraph('/people/carol/', notes, registry, 1)
        expect(hasFamilyEdges(graph, registry)).toBe(false)
        expect(familyGraph(graph, registry).nodes.map((n) => n.urlPath)).toEqual(['/people/carol/'])
      })
    })
  }

  it('legacy: a custom uncategorised hierarchy still counts as family', () => {
    const registry = buildRegistry(GENEALOGY_TYPES)
    const edge: GraphEdge = { from: '/a/', to: '/b/', kind: 'hierarchical', relType: 'mentee', label: '' }
    expect(isFamilyEdge(edge, registry)).toBe(true)
  })

  it('modern: an uncategorised type is not family', () => {
    const edge: GraphEdge = { from: '/a/', to: '/b/', kind: 'hierarchical', relType: 'mentee', label: '' }
    expect(isFamilyEdge(edge, modern)).toBe(false)
  })
})

describe('graph nodes carry contact fields', () => {
  it('reads job title, department, type and contract dates', () => {
    const graph = buildRelationshipGraph('/people/bob/', companyNotes(), modern, 2)
    const byPath = new Map(graph.nodes.map((n) => [n.urlPath, n]))
    expect(byPath.get('/people/bob/')).toMatchObject({
      type: 'person',
      jobTitle: 'VP Engineering',
      department: 'Engineering',
      born: '1970', // legacy `born` fallback
    })
    expect(byPath.get('/people/ada/')?.born).toBe('1960')
    // `--05-06` has no year: no death year rather than a misread.
    expect(byPath.get('/people/george/')).toMatchObject({ born: '1940' })
    expect(byPath.get('/people/george/')?.died).toBeUndefined()
    expect(byPath.get('/orgs/acme/')?.type).toBe('organization')
  })
})

describe('partial dates', () => {
  it.each([
    ['1927-03-19', '1927'],
    ['1927-03', '1927'],
    ['1898', '1898'],
    [1898, '1898'],
    ['03-19', undefined],
    ['--03-19', undefined],
    ['3-9', undefined],
    ['c. 1898', '1898'],
    ['', undefined],
    [null, undefined],
  ])('yearOf(%j) → %j', (input, expected) => {
    expect(yearOf(input)).toBe(expected)
  })

  it('prefers dates.birthday/death over the legacy born/died', () => {
    expect(lifeYears({ 'dates.birthday': '1927-03-19', born: '1900', died: '1980' })).toEqual({
      born: '1927',
      died: '1980',
    })
  })

  it('reads a nested dates map (a page’s own frontmatter)', () => {
    expect(lifeYears({ dates: { birthday: '1950-06', death: '2001' } })).toEqual({ born: '1950', died: '2001' })
  })

  it('matches date labels in any case (older site.json kept the authored case)', () => {
    expect(lifeYears({ 'dates.Birthday': '1960-01-02', born: '1950' }).born).toBe('1960')
    expect(lifeYears({ 'dates.Birthday': '1960-01-02' }).born).toBe('1960')
    expect(lifeYears({ 'dates.DEATH': '2001' }).died).toBe('2001')
    expect(lifeYears({ dates: { Birthday: '1960-01-02' }, born: '1950' }).born).toBe('1960')
    // An exact key still wins over a case variant.
    expect(lifeYears({ 'dates.Birthday': '1900', 'dates.birthday': '1960' }).born).toBe('1960')
  })

  it('falls back to born when the birthday has no year', () => {
    expect(lifeYears({ 'dates.birthday': '--03-19', born: '1927' }).born).toBe('1927')
  })
})

describe('buildSiteNotes fixture', () => {
  it('mirrors the server: derived reverse entries carry the inverse predicate', () => {
    const notes = buildSiteNotes(
      [
        { path: '/a/', fm: {} },
        { path: '/b/', fm: {} },
      ],
      [['/a/', 'reports_to', '/b/']]
    )
    expect(notes.get('/b/')?.relationships?.[0]).toMatchObject({ predicate: 'manages', direction: 'incoming' })
  })
})
