import { describe, it, expect } from 'vitest'
import { buildRegistry, type RelationTypeConfig, type SiteNote } from '../graph/relationship-graph.js'
import { buildOrgTree, hasWorkHierarchy, type OrgNode } from './org-layout.js'
import { SERVER_SITE_JSON } from './site-json-snapshot.js'

// Round-trip through JSON so the readonly `as const` literal becomes the plain
// mutable shape `fetch().json()` hands the charts at runtime.
const site = JSON.parse(JSON.stringify(SERVER_SITE_JSON)) as {
  relationship_types: RelationTypeConfig[]
  markdown_files: SiteNote[]
}
const registry = buildRegistry(site.relationship_types)
const notes = new Map(site.markdown_files.map((n) => [n.url_path, n]))

function outline(node: OrgNode, depth = 0): string[] {
  return [`${'  '.repeat(depth)}${node.id}${node.isFocus ? ' *' : ''}`, ...node.children.flatMap((c) => outline(c, depth + 1))]
}

describe('charts against real server site.json', () => {
  it('reads hierarchy and category from the server registry, not the built-in fallback', () => {
    expect(registry.hasCategories).toBe(true)
    expect(registry.hierarchyOf('employer')).toBe('up')
    expect(registry.hierarchyOf('employee')).toBe('down')
    expect(registry.hierarchyOf('spouse')).toBeUndefined()
    expect(registry.categoryOf('reports_to')).toBe('work')
    expect(registry.categoryOf('parent')).toBe('family')
  })

  it('places a company-only employee on the organization page', () => {
    expect(hasWorkHierarchy('/orgs/acme/', notes, registry)).toBe(true)
    const tree = buildOrgTree('/orgs/acme/', notes, registry)
    expect(outline(tree!.root)).toEqual([
      '/orgs/acme/ *',
      '  /people/ada/',
      '    /people/bob/',
      '      /people/zoe/',
    ])
  })

  it("puts the employer above a company-only employee's management chain", () => {
    expect(hasWorkHierarchy('/people/zoe/', notes, registry)).toBe(true)
    const tree = buildOrgTree('/people/zoe/', notes, registry)
    expect(outline(tree!.root)).toEqual([
      '/orgs/acme/',
      '  /people/ada/',
      '    /people/bob/',
      '      /people/zoe/ *',
    ])
  })
})
