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
