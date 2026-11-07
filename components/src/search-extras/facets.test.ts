import { describe, it, expect } from 'vitest'
import {
  deriveSearchFacets,
  foldersFromUrlPaths,
  noteTypesFromSite,
  facetValue,
  typeFacetToken,
  WHEN_ANY,
  WHEN_CUSTOM,
  WHEN_PRESETS,
  whenCustomRange,
  whenCustomValue,
  whenSelectValue,
  withFacet,
  withTypeFacet,
} from './facets.js'
import { evaluateDateExpr } from '../calendar/date-expr.js'

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

describe('withFacet', () => {
  it('appends, replaces and removes a facet of any key', () => {
    expect(withFacet('', 'when', '-7d')).toBe('when:-7d')
    expect(withFacet('plan when:today', 'when', '-30d')).toBe('plan when:-30d')
    expect(withFacet('WHEN:x plan When:y', 'when', 'this-year')).toBe('plan when:this-year')
    expect(withFacet('plan when:-7d type:x', 'when', null)).toBe('plan type:x')
    expect(withFacet('when:-7d', 'when', '')).toBe('')
    // Only the named key: `whenever:` and `type:` are untouched.
    expect(withFacet('whenever:x type:y', 'when', 'today')).toBe('whenever:x type:y when:today')
  })

  it('reads the last value of a key, unquoted', () => {
    expect(facetValue('a when:today b', 'when')).toBe('today')
    expect(facetValue('when:a when:b', 'when')).toBe('b')
    expect(facetValue('type:"Meeting Notes"', 'type')).toBe('Meeting Notes')
    expect(facetValue('plan', 'when')).toBeNull()
  })
})

describe('the When select', () => {
  it('every preset is a valid date expression', () => {
    for (const preset of WHEN_PRESETS) {
      expect(evaluateDateExpr(preset.value, '2026-11-06'), preset.value).not.toBeNull()
    }
    expect(evaluateDateExpr(WHEN_CUSTOM, '2026-11-06')).toBeNull()
  })

  it('round-trips every preset through the query', () => {
    for (const preset of WHEN_PRESETS) {
      const query = withFacet('standup', 'when', preset.value)
      expect(query).toBe(`standup when:${preset.value}`)
      expect(whenSelectValue(query)).toBe(preset.value)
    }
    expect(whenSelectValue(withFacet('standup when:-7d', 'when', null))).toBe(WHEN_ANY)
  })

  it('reflects a hand-typed token: a preset when it equals one, else custom', () => {
    expect(whenSelectValue('when:-30D plan')).toBe('-30d')
    expect(whenSelectValue('when:Today')).toBe('today')
    expect(whenSelectValue('when:2026-W44')).toBe(WHEN_CUSTOM)
    expect(whenSelectValue('when:-14d')).toBe(WHEN_CUSTOM)
    expect(whenSelectValue('date:-7d')).toBe(WHEN_ANY)
  })

  it('round-trips a custom range through the two date inputs', () => {
    for (const [from, to] of [
      ['2026-10-01', '2026-10-15'],
      ['2026-10-01', ''],
      ['', '2026-10-15'],
    ]) {
      const value = whenCustomValue(from, to)!
      const query = withFacet('plan', 'when', value)
      expect(whenSelectValue(query)).toBe(WHEN_CUSTOM)
      expect(whenCustomRange(query)).toEqual({ from, to })
      expect(evaluateDateExpr(value, '2026-11-06')).toEqual({ from: from || null, to: to || null })
    }
    expect(whenCustomValue('', '')).toBeNull()
    expect(whenCustomRange('when:2026-10-31')).toEqual({ from: '2026-10-31', to: '2026-10-31' })
    expect(whenCustomRange('when:2026-W44')).toEqual({ from: '', to: '' })
  })
})
