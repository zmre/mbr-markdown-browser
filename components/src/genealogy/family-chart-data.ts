/**
 * Pure conversion from the shared `RelationshipGraph` model to the
 * family-chart v0.9 data format (verified against the package's shipped
 * `dist/types/types/data.d.ts`):
 *
 *   { id, data: { gender: 'M' | 'F', ...fields }, rels: { parents[], spouses[], children[] } }
 *
 * Differences we paper over:
 *  - family-chart types `gender` as required `'M' | 'F'`; the runtime renders a
 *    genderless card when it is absent, so unknown genders are OMITTED (our
 *    `FamilyChartDatum` marks it optional; the view casts at the boundary).
 *  - `avatar` stays the RAW frontmatter image path here — the view resolves it
 *    through `ctx.resolveUrl` so this module stays pure and testable.
 *
 * Rels are symmetric by construction: every hierarchical edge writes both the
 * parent's `children` and the child's `parents`, every spouse edge writes both
 * `spouses` lists. All edges in `RelationshipGraph` connect included nodes, so
 * no dangling ids are possible; sibling and unresolved relationships were
 * already excluded upstream in `buildRelationshipGraph`.
 *
 * The same upstream contract also guarantees ACYCLIC parent/child rels
 * (`buildRelationshipGraph` breaks hierarchical cycles), which family-chart
 * requires: its `calculateTree()` walks the rels with `d3.hierarchy()`, which
 * has no cycle detection and allocates forever on a loop. `findParentChildCycle`
 * below re-checks that here so a regression upstream surfaces as a message
 * instead of a frozen tab.
 */
import type { RelationshipGraph } from '../graph/relationship-graph.js'

/** One person in family-chart's data format (gender optional, see above). */
export interface FamilyChartDatum {
  id: string
  data: {
    label: string
    birthday?: string
    death?: string
    avatar?: string
    gender?: 'M' | 'F'
  }
  rels: {
    parents: string[]
    spouses: string[]
    children: string[]
  }
}

export interface FamilyChartData {
  data: FamilyChartDatum[]
  /** The focused person's id (family-chart's "main" person). */
  mainId: string
}

/** Map a normalized gender string to family-chart's 'M'/'F'; else undefined. */
export function familyChartGender(gender: string | undefined): 'M' | 'F' | undefined {
  if (gender === 'male' || gender === 'm' || gender === 'man') return 'M'
  if (gender === 'female' || gender === 'f' || gender === 'woman') return 'F'
  return undefined
}

/** Relationship types (of symmetric edges) that mean "spouse/partner". */
export const SPOUSE_REL_TYPES = new Set(['spouse', 'partner'])

/** Convert a relationship graph to family-chart data with `mainId = focus`. */
export function toFamilyChartData(graph: RelationshipGraph): FamilyChartData {
  const byId = new Map<string, FamilyChartDatum>()
  for (const node of graph.nodes) {
    const gender = familyChartGender(node.gender)
    byId.set(node.urlPath, {
      id: node.urlPath,
      data: {
        label: node.title,
        ...(node.born ? { birthday: node.born } : {}),
        ...(node.died ? { death: node.died } : {}),
        ...(node.image ? { avatar: node.image } : {}),
        ...(gender ? { gender } : {}),
      },
      rels: { parents: [], spouses: [], children: [] },
    })
  }

  const push = (list: string[], id: string) => {
    if (!list.includes(id)) list.push(id)
  }

  for (const edge of graph.edges) {
    const from = byId.get(edge.from)
    const to = byId.get(edge.to)
    if (!from || !to) continue
    if (edge.kind === 'hierarchical') {
      // Hierarchical edges are parent→child (`from` = parent).
      push(from.rels.children, to.id)
      push(to.rels.parents, from.id)
    } else if (edge.kind === 'symmetric' && SPOUSE_REL_TYPES.has(edge.relType)) {
      push(from.rels.spouses, to.id)
      push(to.rels.spouses, from.id)
    }
    // Other symmetric types and plain directed edges have no family-chart
    // equivalent and are skipped.
  }

  return { data: [...byId.values()], mainId: graph.focus }
}

/**
 * Find a parent/child cycle in family-chart data, or return `null` when there is
 * none.
 *
 * The returned array lists the ids taking part in the loop in traversal order;
 * the cycle closes from the LAST id back to the first (e.g. `['/a/', '/b/']`
 * means a → b → a).
 *
 * Both rel directions are checked independently, even though `toFamilyChartData`
 * writes them symmetrically: `calculateTree()` runs `d3.hierarchy()` over
 * `children` AND over `parents`, so a loop in either one is fatal.
 *
 * Iterative DFS (no recursion — a deep lineage could exceed the JS stack).
 * O(V + E).
 */
export function findParentChildCycle(data: FamilyChartDatum[]): string[] | null {
  const ids = data.map((d) => d.id)
  const children = new Map<string, string[]>()
  const parents = new Map<string, string[]>()
  for (const datum of data) {
    children.set(datum.id, datum.rels.children)
    parents.set(datum.id, datum.rels.parents)
  }
  return findCycle(children, ids) ?? findCycle(parents, ids)
}

/**
 * Depth-first cycle search over one adjacency map, returning the cycle's members
 * in traversal order. Ids absent from the map (dangling references) are skipped
 * rather than treated as nodes.
 */
function findCycle(adjacency: Map<string, string[]>, ids: string[]): string[] | null {
  const ON_STACK = 1
  const FINISHED = 2
  const state = new Map<string, typeof ON_STACK | typeof FINISHED>()

  for (const root of ids) {
    if (state.has(root)) continue
    state.set(root, ON_STACK)
    // `path` mirrors `stack`, so a back edge yields the cycle members directly.
    const path: string[] = [root]
    const stack: Array<{ id: string; next: number }> = [{ id: root, next: 0 }]
    while (stack.length > 0) {
      const frame = stack[stack.length - 1]
      const out = adjacency.get(frame.id)
      if (!out || frame.next >= out.length) {
        state.set(frame.id, FINISHED)
        stack.pop()
        path.pop()
        continue
      }
      const id = out[frame.next++]
      if (!adjacency.has(id)) continue
      const seen = state.get(id)
      if (seen === ON_STACK) return path.slice(path.indexOf(id))
      if (seen === FINISHED) continue
      state.set(id, ON_STACK)
      path.push(id)
      stack.push({ id, next: 0 })
    }
  }
  return null
}
