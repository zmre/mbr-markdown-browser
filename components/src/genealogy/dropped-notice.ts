/**
 * The muted notice listing contradictory hierarchical links that
 * `buildRelationshipGraph` had to drop to keep the hierarchy acyclic.
 *
 * Chunk-side, and drawn INSIDE the trigger's fixed-height canvas (above the
 * chart, which shrinks to fit), so it can appear after the lazy load without
 * shifting the page. It used to be rendered by the main-bundle trigger, which
 * meant every person page paid for it — and for building the graph at load.
 */
import { html, render, nothing } from 'lit'
import type { GraphEdge } from '../graph/relationship-graph.js'
import { safeHref } from '../safe-href.js'

/**
 * How many contradictory links the notice names before collapsing into "and N
 * more". Keeps a badly broken repository from filling the panel with text; the
 * complete list lives in the page-problems panel.
 */
const MAX_LISTED_DROPPED_EDGES = 5

/**
 * Canonical `relType` of the parent/child pair (the `down` half). NOT every
 * hierarchical edge is parent/child — `manages`, `mentee`, … can form cycles
 * too — so wording about parents and ancestors is gated on this.
 */
const PARENT_CHILD_REL_TYPE = 'child'

/**
 * Render the notice into a new `.gen-notice` element, or return `null` when
 * nothing was dropped.
 *
 * A hierarchical edge reads "`to` is the `relType` of `from`", so the
 * parent/child pair reads naturally as "<from> as parent of <to>"; any other
 * pair falls back to the neutral "<to> as <relType> of <from>".
 */
export function renderDroppedNotice(
  dropped: GraphEdge[] | undefined,
  titleOf: (path: string) => string,
  resolveUrl: (path: string) => string
): HTMLElement | null {
  if (!dropped || dropped.length === 0) return null

  // One line per note PAIR, so the notice stays honest if both directions of
  // the same contradiction are ever dropped.
  const seen = new Set<string>()
  const pairs: GraphEdge[] = []
  for (const edge of dropped) {
    const key = [edge.from, edge.to].sort().join('\0')
    if (seen.has(key)) continue
    seen.add(key)
    pairs.push(edge)
  }
  const listed = pairs.slice(0, MAX_LISTED_DROPPED_EDGES)
  const remaining = pairs.length - listed.length
  const one = pairs.length === 1

  // Only claim "parent/child" and "each other's ancestor" when EVERY ignored
  // edge really is the parent/child pair; a mixed set gets the neutral wording.
  const parentChildOnly = dropped.every((edge) => edge.relType === PARENT_CHILD_REL_TYPE)
  const kind = parentChildOnly ? 'parent/child' : 'relationship'
  const contradiction = parentChildOnly
    ? "the notes below each claim to be the other's ancestor, which cannot both be true"
    : 'each note below is listed as its own ancestor through a chain of these links, which cannot be true'

  const link = (path: string) => html`<a href=${safeHref(resolveUrl(path))}>${titleOf(path)}</a>`
  const line = (edge: GraphEdge) => {
    const parentChild = edge.relType === PARENT_CHILD_REL_TYPE
    const subject = parentChild ? edge.from : edge.to
    const object = parentChild ? edge.to : edge.from
    const role = parentChild ? 'parent' : edge.relType
    return html`<li>Ignored: ${link(subject)} as ${role} of ${link(object)}</li>`
  }

  const container = document.createElement('div')
  container.className = 'gen-notice'
  render(
    html`
      <p>
        ${one ? 'One' : pairs.length} contradictory ${kind} link${one ? '' : 's'}
        ${one ? 'was' : 'were'} ignored so the chart could be drawn: ${contradiction}. Fix the
        relationships in one note of each pair.
      </p>
      <ul>
        ${listed.map(line)}
      </ul>
      ${remaining > 0
        ? html`<p>
            …and ${remaining} more. The full list is in the page problems panel (the ⚠ button in
            the header).
          </p>`
        : nothing}
    `,
    container
  )
  return container
}

