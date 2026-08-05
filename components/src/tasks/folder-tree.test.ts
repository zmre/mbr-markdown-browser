import { describe, it, expect } from 'vitest'
import { buildTaskFolderTree, folderScopeValue, type FolderTreeNode } from './folder-tree.js'
import type { FolderFacet } from './types.js'

/** `path(count)` pairs, depth-first, for compact structural assertions. */
function flatten(node: FolderTreeNode, depth = 0): string[] {
  return [
    `${'  '.repeat(depth)}${node.path} ${node.name} ${node.count}`,
    ...node.children.flatMap((child) => flatten(child, depth + 1)),
  ]
}

describe('buildTaskFolderTree', () => {
  it('rebuilds the tree from the flat cumulative facet list', () => {
    // Exactly the facets `task_query::folder_facets` produces for a repo with
    // tasks in /top, /docs/guide, /docs/notes/weekly and /other/thing.
    const facets: FolderFacet[] = [
      { path: '/', count: 4 },
      { path: '/docs/', count: 2 },
      { path: '/docs/notes/', count: 1 },
      { path: '/other/', count: 1 },
    ]

    const tree = buildTaskFolderTree(facets)
    expect(tree).not.toBeNull()
    expect(flatten(tree!)).toEqual([
      '/ Home 4',
      '  /docs/ docs 2',
      '    /docs/notes/ notes 1',
      '  /other/ other 1',
    ])
  })

  it('names the root Home and keeps the repository total on it', () => {
    const tree = buildTaskFolderTree([{ path: '/', count: 12 }])
    expect(tree?.name).toBe('Home')
    expect(tree?.path).toBe('/')
    expect(tree?.count).toBe(12)
    expect(tree?.children).toEqual([])
  })

  it('sorts siblings alphabetically at every level', () => {
    const tree = buildTaskFolderTree([
      { path: '/', count: 3 },
      { path: '/zeta/', count: 1 },
      { path: '/alpha/', count: 2 },
      { path: '/alpha/zulu/', count: 1 },
      { path: '/alpha/bravo/', count: 1 },
    ])
    expect(tree!.children.map((c) => c.name)).toEqual(['alpha', 'zeta'])
    expect(tree!.children[0].children.map((c) => c.name)).toEqual(['bravo', 'zulu'])
  })

  it('synthesizes a missing intermediate folder rather than orphaning a subtree', () => {
    // The server increments every ancestor, so this should not happen today —
    // but a deep node must never fall off the tree if that rule ever changes.
    const tree = buildTaskFolderTree([
      { path: '/', count: 1 },
      { path: '/a/b/c/', count: 1 },
    ])
    expect(flatten(tree!)).toEqual(['/ Home 1', '  /a/ a 0', '    /a/b/ b 0', '      /a/b/c/ c 1'])
  })

  it('tolerates facet paths written without slashes', () => {
    const tree = buildTaskFolderTree([{ path: 'docs', count: 2 }])
    expect(tree!.children[0].path).toBe('/docs/')
  })

  it('returns null when there is nothing to show', () => {
    expect(buildTaskFolderTree([])).toBeNull()
    expect(buildTaskFolderTree(undefined as unknown as FolderFacet[])).toBeNull()
  })

  it('always produces a root even when the facets omit one', () => {
    const tree = buildTaskFolderTree([{ path: '/docs/', count: 2 }])
    expect(tree!.path).toBe('/')
    expect(tree!.count).toBe(0)
    expect(tree!.children.map((c) => c.path)).toEqual(['/docs/'])
  })
})

describe('folderScopeValue', () => {
  it('mirrors task_query::normalize_folder', () => {
    for (const input of ['docs', '/docs', 'docs/', '/docs/', '  /docs/  ']) {
      expect(folderScopeValue(input)).toBe('/docs/')
    }
  })

  it('treats the repository root as no scope at all', () => {
    for (const input of ['', '/', '   ', '//']) {
      expect(folderScopeValue(input)).toBeNull()
    }
    expect(folderScopeValue(null)).toBeNull()
  })
})
