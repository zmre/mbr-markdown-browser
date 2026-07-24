import { describe, it, expect } from 'vitest'
import {
  buildRegistry,
  buildRelationshipGraph,
  type RelationshipGraph,
  type SiteNote,
} from '../graph/relationship-graph.js'
import { GENEALOGY_TYPES, genealogyNotes } from '../graph/test-fixtures.js'
import { familyChartGender, toFamilyChartData, type FamilyChartDatum } from './family-chart-data.js'

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
