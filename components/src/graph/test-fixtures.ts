/**
 * Shared test fixtures for the relationship-graph modules.
 *
 * The fixture mirrors the resolved `relationships` shape emitted in
 * `site.json` for the genealogy test repo, and is consumed by the pure graph
 * tests as well as the mermaid-rendering tests. Not imported by any production
 * entry point, so it never ships in a bundle.
 */
import type { RelationTypeConfig, SiteNote, SiteRelationship } from './relationship-graph.js'

export const GENEALOGY_TYPES: RelationTypeConfig[] = [
  { name: 'child', symmetric: false, inverse: 'parent', label: 'Child', label_plural: 'Children' },
  { name: 'parent', symmetric: false, inverse: 'child', label: 'Parent', label_plural: 'Parents' },
  { name: 'sibling', symmetric: true, inverse: null, label: 'Sibling', label_plural: 'Siblings' },
  { name: 'spouse', symmetric: true, inverse: null, label: 'Spouse', label_plural: 'Spouses' },
]

/** Build a full `SiteRelationship` from the interesting fields. */
export function rel(partial: Partial<SiteRelationship> & Pick<SiteRelationship, 'rel_type' | 'predicate' | 'neighbor' | 'direction'>): SiteRelationship {
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
export function genealogyNotes(): Map<string, SiteNote> {
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
