import { describe, it, expect } from 'vitest'
import {
  buildRegistry,
  buildRelationshipGraph,
  type RelationshipGraph,
  type SiteNote,
} from '../graph/relationship-graph.js'
import { GENEALOGY_TYPES, genealogyNotes, rel } from '../graph/test-fixtures.js'
import {
  familyChartGender,
  findParentChildCycle,
  toFamilyChartData,
  type FamilyChartDatum,
} from './family-chart-data.js'

function fixtureGraph(
  focus = '/people/john/',
  mutate?: (notes: Map<string, SiteNote>) => void
): RelationshipGraph {
  const notes = genealogyNotes()
  mutate?.(notes)
  return buildRelationshipGraph(focus, notes, buildRegistry(GENEALOGY_TYPES))
}

const datumOf = (data: FamilyChartDatum[], id: string): FamilyChartDatum => {
  const datum = data.find((d) => d.id === id)
  expect(datum, `datum ${id}`).toBeDefined()
  return datum!
}

describe('UNIT toFamilyChartData', () => {
  it('produces the family-chart v2 shape for the fixture', () => {
    const { data, mainId } = toFamilyChartData(fixtureGraph())
    expect(mainId).toBe('/people/john/')

    const john = datumOf(data, '/people/john/')
    expect(john).toEqual({
      id: '/people/john/',
      data: { label: 'John Doe', birthday: '1925', death: '1999' },
      rels: {
        parents: expect.arrayContaining(['/people/george/', '/people/martha/']),
        spouses: ['/people/mary/'],
        children: expect.arrayContaining(['/people/alice/', '/people/sam/']),
      },
    })
    expect(john.rels.parents).toHaveLength(2)
    expect(john.rels.children).toHaveLength(2)

    const robert = datumOf(data, '/people/robert/')
    expect(robert.data).toEqual({ label: 'Robert Doe', birthday: '1929' }) // no death year
    expect(robert.rels.parents).toEqual(
      expect.arrayContaining(['/people/george/', '/people/martha/'])
    )
  })

  it('builds symmetric rels: children↔parents and both spouses lists', () => {
    const { data } = toFamilyChartData(fixtureGraph())
    const byId = new Map(data.map((d) => [d.id, d]))
    for (const datum of data) {
      for (const child of datum.rels.children) {
        expect(byId.get(child)!.rels.parents).toContain(datum.id)
      }
      for (const parent of datum.rels.parents) {
        expect(byId.get(parent)!.rels.children).toContain(datum.id)
      }
      for (const spouse of datum.rels.spouses) {
        expect(byId.get(spouse)!.rels.spouses).toContain(datum.id)
      }
    }
  })

  it('never references a person outside the data set and never duplicates', () => {
    const { data } = toFamilyChartData(fixtureGraph())
    const ids = new Set(data.map((d) => d.id))
    for (const datum of data) {
      for (const list of [datum.rels.parents, datum.rels.spouses, datum.rels.children]) {
        for (const id of list) expect(ids.has(id)).toBe(true)
        expect(new Set(list).size).toBe(list.length)
      }
    }
  })

  it('excludes unresolved relationships and adds no sibling rels', () => {
    const { data } = toFamilyChartData(fixtureGraph())
    // Robert's spouse (Jane Ghost) is unresolved → no spouse entry, no ghost id.
    expect(datumOf(data, '/people/robert/').rels.spouses).toEqual([])
    expect(data.some((d) => d.id === '' || d.id.includes('Jane'))).toBe(false)
    // Siblings are represented only via shared parents: Alice and Sam appear in
    // each other's parents' children lists, never anywhere else.
    const alice = datumOf(data, '/people/alice/')
    expect(alice.rels.spouses).toEqual([])
    expect(alice.rels.children).toEqual([])
  })

  it('maps normalized genders to M/F and omits unknown ones', () => {
    expect(familyChartGender('male')).toBe('M')
    expect(familyChartGender('m')).toBe('M')
    expect(familyChartGender('female')).toBe('F')
    expect(familyChartGender('f')).toBe('F')
    expect(familyChartGender('nonbinary')).toBeUndefined()
    expect(familyChartGender(undefined)).toBeUndefined()

    const { data } = toFamilyChartData(
      fixtureGraph('/people/john/', (notes) => {
        const george = notes.get('/people/george/')!
        george.frontmatter = { ...george.frontmatter, gender: 'Male' }
        const martha = notes.get('/people/martha/')!
        martha.frontmatter = { ...martha.frontmatter, gender: 'female' }
      })
    )
    expect(datumOf(data, '/people/george/').data.gender).toBe('M')
    expect(datumOf(data, '/people/martha/').data.gender).toBe('F')
    // Nobody else declares a gender → the key is absent entirely.
    expect('gender' in datumOf(data, '/people/john/').data).toBe(false)
  })

  it('passes the raw frontmatter image path through as avatar', () => {
    const { data } = toFamilyChartData(
      fixtureGraph('/people/john/', (notes) => {
        const mary = notes.get('/people/mary/')!
        mary.frontmatter = { ...mary.frontmatter, image: 'images/mary.jpg' }
      })
    )
    expect(datumOf(data, '/people/mary/').data.avatar).toBe('images/mary.jpg')
    expect('avatar' in datumOf(data, '/people/john/').data).toBe(false)
  })

  it('uses the graph focus as mainId even when the focus is not first', () => {
    const { mainId, data } = toFamilyChartData(fixtureGraph('/people/alice/'))
    expect(mainId).toBe('/people/alice/')
    expect(data.some((d) => d.id === '/people/alice/')).toBe(true)
  })
})

// ---------------------------------------------------------------------------
// findParentChildCycle
// ---------------------------------------------------------------------------

/** A minimal datum with only the rels under test populated. */
const datum = (id: string, rels: Partial<FamilyChartDatum['rels']> = {}): FamilyChartDatum => ({
  id,
  data: { label: id },
  rels: { parents: [], spouses: [], children: [], ...rels },
})

describe('UNIT findParentChildCycle', () => {
  it('returns null for the acyclic fixture', () => {
    expect(findParentChildCycle(toFamilyChartData(fixtureGraph()).data)).toBeNull()
  })

  it('returns null for a lone person and for empty data', () => {
    expect(findParentChildCycle([])).toBeNull()
    expect(findParentChildCycle([datum('/a/')])).toBeNull()
  })

  it('finds a 2-cycle in the children rels', () => {
    const cycle = findParentChildCycle([
      datum('/a/', { children: ['/b/'] }),
      datum('/b/', { children: ['/a/'] }),
    ])
    expect(cycle).toEqual(['/a/', '/b/'])
  })

  it('finds a cycle declared only in the parents rels', () => {
    // `toFamilyChartData` writes both directions, but `calculateTree` walks
    // `parents` through d3.hierarchy() too, so a one-sided loop still hangs.
    const cycle = findParentChildCycle([
      datum('/a/', { parents: ['/b/'] }),
      datum('/b/', { parents: ['/a/'] }),
    ])
    expect(cycle).toEqual(['/a/', '/b/'])
  })

  it('returns the members of a 3-cycle in traversal order', () => {
    // The loop closes from the LAST id back to the first: a → b → c → a.
    const cycle = findParentChildCycle([
      datum('/a/', { children: ['/b/'] }),
      datum('/b/', { children: ['/c/'] }),
      datum('/c/', { children: ['/a/'] }),
    ])
    expect(cycle).toEqual(['/a/', '/b/', '/c/'])
  })

  it('reports only the looping members, not the tail that led into them', () => {
    const cycle = findParentChildCycle([
      datum('/root/', { children: ['/a/'] }),
      datum('/a/', { children: ['/b/'] }),
      datum('/b/', { children: ['/a/'] }),
    ])
    expect(cycle).toEqual(['/a/', '/b/'])
  })

  it('is not fooled by a diamond, shared children, or dangling ids', () => {
    // Two parents of one child, plus a reference to a person not in the data.
    expect(
      findParentChildCycle([
        datum('/mum/', { children: ['/kid/', '/ghost/'] }),
        datum('/dad/', { children: ['/kid/'] }),
        datum('/kid/', { parents: ['/mum/', '/dad/'] }),
      ])
    ).toBeNull()
  })
})

// ---------------------------------------------------------------------------
// Regression: a contradictory family tree must still render
// ---------------------------------------------------------------------------

describe('REGRESSION cyclic relationships never reach family-chart', () => {
  /** Two notes that each declare the other as their own parent. */
  function contradictoryPair(): Map<string, SiteNote> {
    const notes: SiteNote[] = [
      {
        url_path: '/p/a/',
        frontmatter: { type: 'person', title: 'Ann' },
        relationships: [
          rel({ rel_type: 'child', predicate: 'parent', neighbor: '/p/b/', direction: 'incoming' }),
        ],
      },
      {
        url_path: '/p/b/',
        frontmatter: { type: 'person', title: 'Bob' },
        relationships: [
          rel({ rel_type: 'child', predicate: 'parent', neighbor: '/p/a/', direction: 'incoming' }),
        ],
      },
    ]
    return new Map(notes.map((n) => [n.url_path, n]))
  }

  it('lays out a tree for mutually-declared parents', async () => {
    const graph = buildRelationshipGraph(
      '/p/a/',
      contradictoryPair(),
      buildRegistry(GENEALOGY_TYPES)
    )
    const { data, mainId } = toFamilyChartData(graph)

    // ORDER MATTERS — assert acyclicity FIRST and only call `calculateTree` if
    // it holds. family-chart hands this data straight to `d3.hierarchy()`, which
    // has NO cycle detection: on a regression `calculateTree` would allocate
    // until the vitest worker is OOM-killed instead of failing cleanly.
    expect(graph.droppedEdges).toHaveLength(1)
    expect(findParentChildCycle(data)).toBeNull()

    const { calculateTree } = await import('family-chart')
    const tree = calculateTree(data as unknown as Parameters<typeof calculateTree>[0], {
      main_id: mainId,
      // Matches family-chart-view.ts's setSingleParentEmptyCard(false).
      single_parent_empty_card: false,
    })
    expect(tree.main_id).toBe(mainId)
    expect(tree.data.length).toBeGreaterThan(0)
    expect(tree.data.map((node) => node.data.id)).toContain('/p/b/')
  })
})
