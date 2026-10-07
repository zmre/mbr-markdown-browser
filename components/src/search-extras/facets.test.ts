import { describe, it, expect } from 'vitest'
import {
  deriveSearchFacets,
  foldersFromUrlPaths,
  noteTypesFromSite,
  typeFacetToken,
  withTypeFacet,
} from './facets.js'

describe('foldersFromUrlPaths', () => {
  it('lists proper ancestors only, never a note page itself', () => {
    expect(foldersFromUrlPaths(['/people/john/', '/people/staff/amy/', '/README/'])).toEqual([
      '/people/',
      '/people/staff/',
    ])
  })

  it('sorts case-insensitively and de-duplicates', () => {
    expect(foldersFromUrlPaths(['/b/x/', '/A/y/', '/a/z/', '/b/w/'])).toEqual(['/A/', '/a/', '/b/'])
  })
})

describe('noteTypesFromSite', () => {
  it('counts distinct types case-insensitively under the first spelling', () => {
    const files = [
      { frontmatter: { type: 'person' } },
      { frontmatter: { type: 'Person' } },
      { frontmatter: { type: ' organization ' } },
      { frontmatter: { type: 42 } },
      { frontmatter: {} },
      {},
    ]
    expect(noteTypesFromSite(files)).toEqual([
      { type: 'organization', count: 1 },
      { type: 'person', count: 2 },
    ])
  })

  it('tolerates a missing file list', () => {
    expect(noteTypesFromSite(undefined)).toEqual([])
  })
})

describe('type facet tokens', () => {
  it('quotes values containing whitespace', () => {
    expect(typeFacetToken('person')).toBe('type:person')
    expect(typeFacetToken('Meeting Notes')).toBe('type:"Meeting Notes"')
    expect(typeFacetToken('say "hi" there')).toBe('type:"say hi there"')
  })

  it('appends the token, replacing any existing type facet', () => {
    expect(withTypeFacet('', 'person')).toBe('type:person')
    expect(withTypeFacet('jane', 'person')).toBe('jane type:person')
    expect(withTypeFacet('jane type:org tags:x', 'person')).toBe('jane tags:x type:person')
    expect(withTypeFacet('type:"Meeting Notes" plan', 'person')).toBe('plan type:person')
    expect(withTypeFacet('plan TYPE:x', 'Meeting Notes')).toBe('plan type:"Meeting Notes"')
  })
})

describe('deriveSearchFacets', () => {
  it('derives folders and types from one site.json payload', () => {
    const data = {
      markdown_files: [
        { url_path: '/people/jane/', frontmatter: { type: 'person' } },
        { url_path: '/orgs/acme/', frontmatter: { type: 'organization' } },
        { url_path: 42 },
      ],
    }
    expect(deriveSearchFacets(data)).toEqual({
      folders: ['/orgs/', '/people/'],
      types: [
        { type: 'organization', count: 1 },
        { type: 'person', count: 1 },
      ],
    })
    expect(deriveSearchFacets(null)).toEqual({ folders: [], types: [] })
  })
})
