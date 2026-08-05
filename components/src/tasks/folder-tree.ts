/**
 * Folder pane model, built from the response's `folders` facet.
 *
 * The server already does the hard part: `task_query::folder_facets` walks each
 * matching task up its ancestor chain, so `/docs/notes/` contributes to `/`,
 * `/docs/` and `/docs/notes/` alike. Every count is therefore already
 * cumulative — "what selecting this folder would show" — and `/` always carries
 * the repository total. This module only has to turn that flat, sorted list of
 * paths back into a tree.
 *
 * The facets are computed **ignoring** the folder filter, so the tree is stable
 * as the user clicks around it and never empties out a sibling.
 */
import type { FolderFacet } from './types.js'

export interface FolderTreeNode {
  /** Facet path, with leading and trailing slashes: `/`, `/docs/`, `/docs/notes/`. */
  path: string
  /** Display name: the last segment, or `Home` for the root. */
  name: string
  /** Matching tasks in this folder and its subfolders. */
  count: number
  children: FolderTreeNode[]
}

/** Label used for the repository root, which has no segment name of its own. */
export const ROOT_LABEL = 'Home'

/**
 * Build the folder tree for the left pane.
 *
 * Returns `null` when there is nothing to show (no facets at all).
 *
 * Intermediate nodes are synthesized when missing. In practice the server never
 * omits one — it increments every ancestor — but a folder whose only tasks were
 * filtered out is dropped from the facet list, and a future change to that rule
 * must not produce an orphaned subtree here.
 */
export function buildTaskFolderTree(facets: FolderFacet[]): FolderTreeNode | null {
  if (!Array.isArray(facets) || facets.length === 0) return null

  const nodes = new Map<string, FolderTreeNode>()
  const root: FolderTreeNode = { path: '/', name: ROOT_LABEL, count: 0, children: [] }
  nodes.set('/', root)

  /** Get (creating if needed) the node for a normalized folder path. */
  const nodeFor = (path: string): FolderTreeNode => {
    const existing = nodes.get(path)
    if (existing) return existing

    const segments = path.split('/').filter((s) => s.length > 0)
    const node: FolderTreeNode = {
      path,
      name: segments[segments.length - 1] ?? ROOT_LABEL,
      count: 0,
      children: [],
    }
    nodes.set(path, node)
    // `/docs/notes/` -> parent `/docs/`; a single-segment path's parent is `/`.
    const parentPath = segments.length > 1 ? `/${segments.slice(0, -1).join('/')}/` : '/'
    nodeFor(parentPath).children.push(node)
    return node
  }

  for (const facet of facets) {
    if (!facet || typeof facet.path !== 'string') continue
    const trimmed = facet.path.trim().replace(/^\/+|\/+$/g, '')
    const path = trimmed.length === 0 ? '/' : `/${trimmed}/`
    nodeFor(path).count = Number(facet.count) || 0
  }

  sortChildren(root)
  return root
}

/** Alphabetical, case-insensitive, depth-first — the order the pane renders. */
function sortChildren(node: FolderTreeNode): void {
  node.children.sort((a, b) => a.name.localeCompare(b.name))
  for (const child of node.children) {
    sortChildren(child)
  }
}

/**
 * Normalize a folder path for the request body: `/` (the whole repo) becomes
 * `null`, everything else keeps both slashes.
 *
 * Mirrors `task_query::normalize_folder`, so the client and the server agree on
 * what "no scope" means without a round trip.
 */
export function folderScopeValue(path: string | null): string | null {
  if (typeof path !== 'string') return null
  const trimmed = path.trim().replace(/^\/+|\/+$/g, '')
  return trimmed.length === 0 ? null : `/${trimmed}/`
}
