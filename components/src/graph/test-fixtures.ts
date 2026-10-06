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

// ============================================================================
// Contacts fixtures (contract 0.3: hierarchy + category on every type)
// ============================================================================

type TypeRow = [name: string, inverse: string | null, symmetric: boolean, hierarchy: 'up' | 'down' | null, category: string, label: string, plural: string]

const CONTACT_TYPE_ROWS: TypeRow[] = [
  ['parent', 'child', false, 'up', 'family', 'Parent', 'Parents'],
  ['child', 'parent', false, 'down', 'family', 'Child', 'Children'],
  ['spouse', null, true, null, 'family', 'Spouse', 'Spouses'],
  ['sibling', null, true, null, 'family', 'Sibling', 'Siblings'],
  ['reports_to', 'manages', false, 'up', 'work', 'Reports to', 'Reports to'],
  ['manages', 'reports_to', false, 'down', 'work', 'Manages', 'Manages'],
  ['assistant', 'assists', false, 'down', 'work', 'Assistant', 'Assistants'],
  ['assists', 'assistant', false, 'up', 'work', 'Assists', 'Assists'],
  ['employer', 'employee', false, 'up', 'work', 'Employer', 'Employers'],
  ['employee', 'employer', false, 'down', 'work', 'Employee', 'Employees'],
  ['colleague', null, true, null, 'work', 'Colleague', 'Colleagues'],
]

/** The contract-0.3 built-in registry, as Part A's site.json emits it. */
export const CONTACT_TYPES: RelationTypeConfig[] = CONTACT_TYPE_ROWS.map(
  ([name, inverse, symmetric, hierarchy, category, label, label_plural]) => ({
    name,
    inverse,
    symmetric,
    label,
    label_plural,
    ...(hierarchy ? { hierarchy } : {}),
    category,
  })
)

/** The same registry as an older server emits it: no `hierarchy`, no `category`. */
export const LEGACY_CONTACT_TYPES: RelationTypeConfig[] = CONTACT_TYPES.map(
  ({ hierarchy: _h, category: _c, ...rest }) => rest
)

/**
 * Build site.json notes from declarations, adding the derived reverse entry on
 * the object exactly as the server does: on S, `{type: T, to: O}` becomes an
 * outgoing `predicate: T` edge; on O, an incoming edge whose predicate is T's
 * inverse (or T itself when symmetric).
 */
export function buildSiteNotes(
  notes: Array<{ path: string; fm: Record<string, unknown> }>,
  declarations: Array<[subject: string, type: string, object: string]>,
  types: RelationTypeConfig[] = CONTACT_TYPES
): Map<string, SiteNote> {
  const byName = new Map(types.map((t) => [t.name, t]))
  const map = new Map<string, SiteNote>(
    notes.map(({ path, fm }) => [path, { url_path: path, frontmatter: fm, relationships: [] }])
  )
  for (const [subject, type, object] of declarations) {
    const config = byName.get(type)
    map.get(subject)?.relationships?.push(rel({ rel_type: type, predicate: type, neighbor: object, direction: 'outgoing' }))
    const reverse = config?.symmetric ? type : (config?.inverse ?? type)
    map.get(object)?.relationships?.push(
      rel({ rel_type: type, predicate: reverse, neighbor: subject, direction: 'incoming', derived: true })
    )
  }
  return map
}

/**
 * A small company: Acme (organization) employs Ada (CEO), Bob (VP Eng),
 * Carol and Eve (engineers under Bob, Engineering), Fay (designer under Bob,
 * Design), Dan (Ada's assistant). Bob is also George's child — one family edge
 * so the family/work split has something to separate. Dates use the contract
 * `dates.birthday` keys, plus one legacy `born`.
 */
export function companyNotes(types: RelationTypeConfig[] = CONTACT_TYPES): Map<string, SiteNote> {
  return buildSiteNotes(
    [
      { path: '/orgs/acme/', fm: { type: 'organization', title: 'Acme Corp' } },
      { path: '/people/ada/', fm: { type: 'person', title: 'Ada King', job_title: 'CEO', 'dates.birthday': '1960-01-02' } },
      { path: '/people/bob/', fm: { type: 'person', title: 'Bob Stone', job_title: 'VP Engineering', department: 'Engineering', born: '1970' } },
      { path: '/people/carol/', fm: { type: 'person', title: 'Carol Diaz', job_title: 'Engineer', department: 'Engineering' } },
      { path: '/people/eve/', fm: { type: 'person', title: 'Eve Park', job_title: 'Engineer', department: 'Engineering' } },
      { path: '/people/fay/', fm: { type: 'person', title: 'Fay Wu', job_title: 'Designer', department: 'Design' } },
      { path: '/people/dan/', fm: { type: 'person', title: 'Dan Moss', job_title: 'Executive Assistant' } },
      { path: '/people/george/', fm: { type: 'person', title: 'George Stone', 'dates.birthday': '1940', 'dates.death': '--05-06' } },
    ],
    [
      ['/people/ada/', 'employer', '/orgs/acme/'],
      ['/people/bob/', 'employer', '/orgs/acme/'],
      ['/people/carol/', 'employer', '/orgs/acme/'],
      ['/people/eve/', 'employer', '/orgs/acme/'],
      ['/people/fay/', 'employer', '/orgs/acme/'],
      ['/people/dan/', 'employer', '/orgs/acme/'],
      ['/people/bob/', 'reports_to', '/people/ada/'],
      ['/people/carol/', 'reports_to', '/people/bob/'],
      ['/people/eve/', 'reports_to', '/people/bob/'],
      ['/people/fay/', 'reports_to', '/people/bob/'],
      ['/people/ada/', 'assistant', '/people/dan/'],
      ['/people/bob/', 'parent', '/people/george/'],
    ],
    types
  )
}
