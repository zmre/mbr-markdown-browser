/**
 * Pure model + layout for the Org chart. No DOM, no d3.
 *
 * MODEL (`buildOrgTree`): a top-down tree assembled from work-hierarchy edges
 * (`category === 'work'`, `kind === 'hierarchical'`: reports_to/manages,
 * assistant/assists, employer/employee) read straight off site.json — it walks
 * the notes' own `relationships`, so it reaches the whole management chain no
 * matter how deep, without depending on the trigger's depth-limited graph.
 *
 * - Organization focus: the organization is the root; its employees hang
 *   beneath it, structured by reporting lines where known.
 * - Person focus: the management chain up to the top (plus the employer
 *   organization above it, if any), the focus's peers (same manager), and the
 *   focus's reports two levels down — all direct reports, including people
 *   whose first-listed manager is someone else.
 *
 * Every person has ONE tree parent — their first non-organization superior in
 * authored order, else their first organization. Other superiors are kept as
 * `also` (shown in the tooltip, and as a dashed link when on the chart). Visited
 * sets make every walk terminate on contradictory (cyclic) data, which also
 * means each note is placed at most once.
 *
 * Size is bounded by `maxNodes`; whatever does not fit collapses into a
 * "+N more" card that links to the person whose reports it stands for.
 *
 * LAYOUT (`computeOrgLayout`): a tidy top-down tree with orthogonal "bus"
 * connectors. Siblings are grouped by `department` into labelled boxes, and a
 * group made only of leaves is stacked as a compact column under a spine — the
 * shape that keeps a 30-person team from becoming a 6000px-wide row.
 */
import {
  classifyRelationship,
  isWorkHierarchyEdge,
  nodeTitle,
  type Registry,
  type SiteNote,
} from '../graph/relationship-graph.js'

// ============================================================================
// Model
// ============================================================================

export type OrgNodeKind = 'person' | 'organization' | 'more'

export interface OrgNode {
  /** url_path of the note; for a `more` card, a synthetic unique id. */
  id: string
  kind: OrgNodeKind
  title: string
  jobTitle?: string
  department?: string
  isFocus: boolean
  /** Canonical relType of the edge to the tree parent (`manages`, `assistant`, …). */
  relType?: string
  /** Other superiors (url_paths) beyond the tree parent. */
  also?: string[]
  /** `more` cards only: how many notes were left out. */
  moreCount?: number
  /** Where activating the card goes (the note itself; for `more`, the parent). */
  target: string
  children: OrgNode[]
}

export interface OrgTree {
  root: OrgNode
  /** Total notes collapsed into "+N more" cards. */
  hidden: number
}

export interface OrgTreeOptions {
  /** Card budget (excluding "+N more" cards). */
  maxNodes?: number
  /** Report levels shown below a person focus. */
  reportsDepth?: number
  /** Peers (besides the focus) shown beside a person focus. */
  maxPeers?: number
}

export const DEFAULT_ORG_MAX_NODES = 80
export const DEFAULT_REPORTS_DEPTH = 2
export const DEFAULT_MAX_PEERS = 12
/** Safety bound on the upward management walk. */
const MAX_CHAIN = 32

interface Superior {
  path: string
  relType: string
}

/** Locale-independent, case-insensitive ordering. */
function compareText(a: string, b: string): number {
  const x = a.toLowerCase()
  const y = b.toLowerCase()
  return x < y ? -1 : x > y ? 1 : a < b ? -1 : a > b ? 1 : 0
}

function stringField(fm: Record<string, unknown>, key: string): string | undefined {
  const value = fm[key]
  if (typeof value !== 'string') return undefined
  const trimmed = value.trim()
  return trimmed || undefined
}

/**
 * Lazily computed work-hierarchy adjacency over site.json notes, so building a
 * chart touches only the notes near the focus, never the whole repository.
 */
class WorkIndex {
  private readonly superiorsCache = new Map<string, Superior[]>()
  private readonly inferiorsCache = new Map<string, string[]>()

  constructor(
    private readonly notes: Map<string, SiteNote>,
    private readonly registry: Registry
  ) {}

  has(path: string): boolean {
    return this.notes.has(path)
  }

  frontmatter(path: string): Record<string, unknown> {
    return this.notes.get(path)?.frontmatter ?? {}
  }

  isOrganization(path: string): boolean {
    return stringField(this.frontmatter(path), 'type')?.toLowerCase() === 'organization'
  }

  private scan(path: string): void {
    if (this.superiorsCache.has(path)) return
    const superiors: Superior[] = []
    const inferiors: string[] = []
    for (const rel of this.notes.get(path)?.relationships ?? []) {
      const classified = classifyRelationship(path, rel, this.registry)
      if (!classified || !isWorkHierarchyEdge(classified.edge)) continue
      const { from, to, relType } = classified.edge
      if (to === path && this.notes.has(from) && !superiors.some((s) => s.path === from)) {
        superiors.push({ path: from, relType })
      } else if (from === path && this.notes.has(to) && !inferiors.includes(to)) {
        inferiors.push(to)
      }
    }
    this.superiorsCache.set(path, superiors)
    this.inferiorsCache.set(path, inferiors)
  }

  superiors(path: string): Superior[] {
    this.scan(path)
    return this.superiorsCache.get(path) ?? []
  }

  inferiors(path: string): string[] {
    this.scan(path)
    return this.inferiorsCache.get(path) ?? []
  }

  /** The tree parent: first non-organization superior, else first organization. */
  primary(path: string): Superior | undefined {
    const all = this.superiors(path)
    return all.find((s) => !this.isOrganization(s.path)) ?? all[0]
  }

  /** The first non-organization superior (a person's manager), if any. */
  manager(path: string): Superior | undefined {
    return this.superiors(path).find((s) => !this.isOrganization(s.path))
  }

  /** Inferiors whose tree parent is `path` — the reports drawn beneath it. */
  childrenOf(path: string, parentOf: (p: string) => Superior | undefined): string[] {
    return this.inferiors(path).filter((c) => parentOf(c)?.path === path)
  }
}

/** True when the focus has any work-hierarchy relationship at all. */
export function hasWorkHierarchy(focus: string, notes: Map<string, SiteNote>, registry: Registry): boolean {
  const index = new WorkIndex(notes, registry)
  return index.superiors(focus).length > 0 || index.inferiors(focus).length > 0
}

function makeNode(index: WorkIndex, path: string, focus: string, parent?: Superior): OrgNode {
  const fm = index.frontmatter(path)
  const kind: OrgNodeKind = index.isOrganization(path) ? 'organization' : 'person'
  const others = index
    .superiors(path)
    .filter((s) => s.path !== parent?.path && !index.isOrganization(s.path))
    .map((s) => s.path)
  const jobTitle = stringField(fm, 'job_title')
  const department = stringField(fm, 'department')
  return {
    id: path,
    kind,
    title: nodeTitle(fm, path),
    ...(jobTitle ? { jobTitle } : {}),
    ...(department ? { department } : {}),
    isFocus: path === focus,
    ...(parent ? { relType: parent.relType } : {}),
    ...(others.length > 0 ? { also: others } : {}),
    target: path,
    children: [],
  }
}

function moreNode(parent: OrgNode, count: number, seq: number): OrgNode {
  return {
    id: `${parent.id}#more-${seq}`,
    kind: 'more',
    title: `+${count} more`,
    isFocus: false,
    moreCount: count,
    target: parent.id,
    children: [],
  }
}

/**
 * Sort sibling paths the way the layout groups them: by department (named
 * departments alphabetically, undepartmented last), then by title.
 */
function sortSiblings(index: WorkIndex, paths: string[]): string[] {
  const key = (p: string) => ({
    dept: stringField(index.frontmatter(p), 'department'),
    title: nodeTitle(index.frontmatter(p), p),
  })
  return [...paths].sort((a, b) => {
    const ka = key(a)
    const kb = key(b)
    if (ka.dept !== kb.dept) {
      if (!ka.dept) return 1
      if (!kb.dept) return -1
      return compareText(ka.dept, kb.dept)
    }
    return compareText(ka.title, kb.title) || compareText(a, b)
  })
}

/**
 * Build the org tree around `focus`, or `null` when the focus has no work
 * hierarchy (nothing to draw).
 */
export function buildOrgTree(
  focus: string,
  notes: Map<string, SiteNote>,
  registry: Registry,
  options: OrgTreeOptions = {}
): OrgTree | null {
  const maxNodes = Math.max(1, Math.floor(options.maxNodes ?? DEFAULT_ORG_MAX_NODES))
  const reportsDepth = Math.max(0, Math.floor(options.reportsDepth ?? DEFAULT_REPORTS_DEPTH))
  const maxPeers = Math.max(0, Math.floor(options.maxPeers ?? DEFAULT_MAX_PEERS))
  const index = new WorkIndex(notes, registry)
  if (!index.has(focus)) return null
  if (index.superiors(focus).length === 0 && index.inferiors(focus).length === 0) return null

  const placed = new Set<string>()
  let budget = maxNodes
  let hidden = 0
  let moreSeq = 0
  const place = (path: string, parent?: Superior): OrgNode => {
    placed.add(path)
    budget -= 1
    return makeNode(index, path, focus, parent)
  }

  /**
   * Breadth-first expansion beneath already-placed nodes, so a capped chart is
   * shallow-and-complete rather than one deep branch. `depthLeft` counts the
   * levels still allowed below each queued node.
   */
  const expand = (
    seeds: Array<{ node: OrgNode; depthLeft: number }>,
    childrenOf: (path: string) => string[],
    parentOf: (path: string) => Superior | undefined
  ): void => {
    const queue = [...seeds]
    for (let i = 0; i < queue.length; i++) {
      const { node, depthLeft } = queue[i]
      const kids = sortSiblings(
        index,
        childrenOf(node.id).filter((c) => !placed.has(c))
      )
      if (kids.length === 0) continue
      if (depthLeft <= 0) {
        // Depth limit: one card says how many reports sit below.
        node.children.push(moreNode(node, kids.length, moreSeq++))
        hidden += kids.length
        continue
      }
      const shown = kids.slice(0, Math.max(0, budget))
      for (const kid of shown) {
        // The edge actually drawn, so `also` lists the OTHER superiors.
        const edge = index.superiors(kid).find((s) => s.path === node.id) ?? parentOf(kid)
        const child = place(kid, edge)
        node.children.push(child)
        queue.push({ node: child, depthLeft: depthLeft - 1 })
      }
      const rest = kids.length - shown.length
      if (rest > 0) {
        node.children.push(moreNode(node, rest, moreSeq++))
        hidden += rest
      }
    }
  }

  if (index.isOrganization(focus)) {
    // Organization: the root, with each employee's reporting chain resolved
    // upward to its top so a report whose manager declares no employer still
    // lands under that manager instead of vanishing.
    const root = place(focus)
    const tops: string[] = []
    for (const employee of index.inferiors(focus)) {
      const seen = new Set<string>()
      let top = employee
      for (let m = index.manager(top); m && !seen.has(m.path) && seen.size < MAX_CHAIN; m = index.manager(top)) {
        seen.add(top)
        top = m.path
      }
      if (top !== focus && !tops.includes(top)) tops.push(top)
    }
    const managerParent = (p: string) => index.manager(p)
    const orgChildren = (p: string) => (p === focus ? tops : index.childrenOf(p, managerParent))
    const parentOf = (p: string) =>
      tops.includes(p) ? index.superiors(p).find((s) => s.path === focus) ?? { path: focus, relType: 'employee' } : managerParent(p)
    expand([{ node: root, depthLeft: Number.POSITIVE_INFINITY }], orgChildren, parentOf)
    return { root, hidden }
  }

  // Person: the management chain, top first.
  const chain: string[] = [focus]
  const seen = new Set<string>([focus])
  for (let up = index.primary(focus); up && !seen.has(up.path) && chain.length < MAX_CHAIN; up = index.primary(up.path)) {
    chain.push(up.path)
    seen.add(up.path)
  }
  // The employer organization above the top person, if the chain did not
  // already end at one: the nearest organization any chain member works for.
  const topPath = chain[chain.length - 1]
  if (!index.isOrganization(topPath)) {
    for (const member of chain) {
      const org = index.superiors(member).find((s) => index.isOrganization(s.path) && !seen.has(s.path))
      if (org) {
        chain.push(org.path)
        seen.add(org.path)
        break
      }
    }
  }
  chain.reverse()

  const parentOf = (p: string) => index.primary(p)
  const nodes = new Map<string, OrgNode>()
  for (const [i, path] of chain.entries()) {
    const parentPath = i > 0 ? chain[i - 1] : undefined
    const edge = parentPath
      ? (index.superiors(path).find((s) => s.path === parentPath) ?? { path: parentPath, relType: 'employee' })
      : undefined
    const node = place(path, edge)
    nodes.set(path, node)
    if (parentPath) nodes.get(parentPath)?.children.push(node)
  }

  // Peers: the focus's siblings under its tree parent (capped; focus kept).
  const focusIndex = chain.indexOf(focus)
  const parentPath = focusIndex > 0 ? chain[focusIndex - 1] : undefined
  if (parentPath) {
    const parentNode = nodes.get(parentPath)!
    const siblings = index.childrenOf(parentPath, parentOf).filter((c) => !placed.has(c))
    const shown = sortSiblings(index, siblings).slice(0, Math.min(maxPeers, Math.max(0, budget)))
    for (const sibling of shown) {
      const node = place(sibling, parentOf(sibling))
      parentNode.children.push(node)
    }
    // Keep the siblings in grouped order with the focus among them.
    parentNode.children = sortNodes(parentNode.children)
    const rest = siblings.length - shown.length
    if (rest > 0) {
      parentNode.children.push(moreNode(parentNode, rest, moreSeq++))
      hidden += rest
    }
  }

  // Reports: EVERY direct report, not just those whose first-listed manager
  // is this person — on Eve's page, someone reporting to Eve and Carol is
  // Eve's report too. The placed set still draws each note once.
  const focusNode = nodes.get(focus)!
  expand([{ node: focusNode, depthLeft: reportsDepth }], (p) => index.inferiors(p), parentOf)
  return { root: nodes.get(chain[0])!, hidden }
}

/** Sort already-built sibling nodes into layout order ("more" cards last). */
function sortNodes(nodes: OrgNode[]): OrgNode[] {
  return [...nodes].sort((a, b) => {
    if ((a.kind === 'more') !== (b.kind === 'more')) return a.kind === 'more' ? 1 : -1
    if (a.department !== b.department) {
      if (!a.department) return 1
      if (!b.department) return -1
      return compareText(a.department, b.department)
    }
    return compareText(a.title, b.title) || compareText(a.id, b.id)
  })
}

// ============================================================================
// Layout
// ============================================================================

export const ORG_CARD_W = 184
export const ORG_CARD_H = 52
export const ORG_MORE_H = 30
const H_GAP = 18
/** Vertical gap between a card's bottom and its children's top. */
const V_GAP = 46
const GROUP_GAP = 22
const GROUP_PAD = 10
const GROUP_LABEL_H = 24
const STACK_GAP = 10
/** Horizontal room left of a stacked column for its spine and stubs. */
const STACK_INDENT = 22
/** A leaf-only group larger than this stacks into a column. */
const STACK_THRESHOLD = 3
export const ORG_MARGIN = 24
/** Rough px per character of the 11px group label (for box sizing). */
const LABEL_CHAR_W = 6.6

export interface OrgCard {
  id: string
  kind: OrgNodeKind
  title: string
  jobTitle?: string
  isFocus: boolean
  also?: string[]
  moreCount?: number
  target: string
  /** Card CENTER. */
  x: number
  y: number
  w: number
  h: number
}

export type OrgLinkKind = 'primary' | 'assistant' | 'secondary'

export interface OrgLink {
  d: string
  kind: OrgLinkKind
}

export interface OrgGroupBox {
  x: number
  y: number
  w: number
  h: number
  label: string
}

export interface OrgLayout {
  cards: OrgCard[]
  links: OrgLink[]
  groups: OrgGroupBox[]
  width: number
  height: number
}

function cardHeight(node: OrgNode): number {
  return node.kind === 'more' ? ORG_MORE_H : ORG_CARD_H
}

interface Group {
  label?: string
  members: OrgNode[]
  stacked: boolean
  w: number
  h: number
}

interface Measured {
  w: number
  h: number
  groups: Group[]
  childrenW: number
}

/**
 * Partition children into department groups, preserving their order. A
 * department matching the parent's own goes unlabelled: the parent already sits
 * in that department's box, and a box inside a box of the same name is noise.
 */
function groupChildren(children: OrgNode[], parentDepartment?: string): Array<{ label?: string; members: OrgNode[] }> {
  const inherited = parentDepartment?.toLowerCase()
  const labelOf = (child: OrgNode) =>
    child.department && child.department.toLowerCase() !== inherited ? child.department : undefined
  if (!children.some(labelOf)) return children.length > 0 ? [{ members: children }] : []
  const groups: Array<{ label?: string; members: OrgNode[] }> = []
  for (const child of children) {
    const label = labelOf(child)
    const last = groups[groups.length - 1]
    if (last && last.label === label) last.members.push(child)
    else groups.push({ label, members: [child] })
  }
  return groups
}

function measure(node: OrgNode, cache: Map<OrgNode, Measured>): Measured {
  const cached = cache.get(node)
  if (cached) return cached
  const groups: Group[] = groupChildren(node.children, node.department).map(({ label, members }) => {
    const sizes = members.map((m) => measure(m, cache))
    const stacked = members.length > STACK_THRESHOLD && members.every((m) => m.children.length === 0)
    let w: number
    let h: number
    if (stacked) {
      w = STACK_INDENT + ORG_CARD_W
      h = members.reduce((sum, m) => sum + cardHeight(m), 0) + STACK_GAP * (members.length - 1)
    } else {
      w = sizes.reduce((sum, s) => sum + s.w, 0) + H_GAP * (members.length - 1)
      h = Math.max(...sizes.map((s) => s.h))
    }
    if (label) {
      w = Math.max(w + 2 * GROUP_PAD, Math.ceil(label.length * LABEL_CHAR_W) + 40)
      h += GROUP_LABEL_H + GROUP_PAD
    }
    return { label, members, stacked, w, h }
  })
  const childrenW = groups.reduce((sum, g) => sum + g.w, 0) + GROUP_GAP * Math.max(0, groups.length - 1)
  const childrenH = groups.length > 0 ? Math.max(...groups.map((g) => g.h)) : 0
  const own = cardHeight(node)
  const result: Measured = {
    w: Math.max(ORG_CARD_W, childrenW),
    h: own + (groups.length > 0 ? V_GAP + childrenH : 0),
    groups,
    childrenW,
  }
  cache.set(node, result)
  return result
}

/** Lay out an org tree; coordinates include `ORG_MARGIN` on every side. */
export function computeOrgLayout(tree: OrgTree): OrgLayout {
  const cache = new Map<OrgNode, Measured>()
  const cards: OrgCard[] = []
  const links: OrgLink[] = []
  const groups: OrgGroupBox[] = []

  const linkKind = (child: OrgNode): OrgLinkKind => (child.relType === 'assistant' ? 'assistant' : 'primary')

  const position = (node: OrgNode, left: number, top: number): void => {
    const m = measure(node, cache)
    const h = cardHeight(node)
    const cx = left + m.w / 2
    cards.push({
      id: node.id,
      kind: node.kind,
      title: node.title,
      ...(node.jobTitle ? { jobTitle: node.jobTitle } : {}),
      isFocus: node.isFocus,
      ...(node.also ? { also: node.also } : {}),
      ...(node.moreCount !== undefined ? { moreCount: node.moreCount } : {}),
      target: node.target,
      x: cx,
      y: top + h / 2,
      w: ORG_CARD_W,
      h,
    })
    if (m.groups.length === 0) return

    const bottom = top + h
    const busY = bottom + V_GAP / 2
    const childTop = bottom + V_GAP
    let gx = left + (m.w - m.childrenW) / 2
    for (const group of m.groups) {
      let innerX = gx
      let innerY = childTop
      if (group.label) {
        groups.push({ x: gx, y: childTop, w: group.w, h: group.h, label: group.label })
        innerX = gx + GROUP_PAD
        innerY = childTop + GROUP_LABEL_H
      }
      if (group.stacked) {
        const spineX = innerX + STACK_INDENT / 2
        let y = innerY
        for (const member of group.members) {
          const mh = cardHeight(member)
          position(member, innerX + STACK_INDENT, y)
          links.push({
            d: `M${cx},${bottom} V${busY} H${spineX} V${y + mh / 2} H${innerX + STACK_INDENT}`,
            kind: linkKind(member),
          })
          y += mh + STACK_GAP
        }
      } else {
        // Center the row inside a box made wider by its label.
        const rowW = group.members.reduce((sum, mem) => sum + measure(mem, cache).w, 0) + H_GAP * (group.members.length - 1)
        const innerW = group.label ? group.w - 2 * GROUP_PAD : group.w
        let x = innerX + (innerW - rowW) / 2
        for (const member of group.members) {
          const mw = measure(member, cache).w
          position(member, x, innerY)
          const mx = x + mw / 2
          links.push({ d: `M${cx},${bottom} V${busY} H${mx} V${innerY}`, kind: linkKind(member) })
          x += mw + H_GAP
        }
      }
      gx += group.w + GROUP_GAP
    }
  }

  const rootSize = measure(tree.root, cache)
  position(tree.root, ORG_MARGIN, ORG_MARGIN)

  // Secondary superiors that made it onto the chart: a dashed curve from the
  // other manager to the person, so dual reporting lines are visible.
  const byId = new Map(cards.map((c) => [c.id, c]))
  for (const card of cards) {
    for (const other of card.also ?? []) {
      const from = byId.get(other)
      if (!from) continue
      const x1 = from.x
      const y1 = from.y + from.h / 2
      const x2 = card.x
      const y2 = card.y - card.h / 2
      const midY = (y1 + y2) / 2
      links.push({ d: `M${x1},${y1} C${x1},${midY} ${x2},${midY} ${x2},${y2}`, kind: 'secondary' })
    }
  }

  return {
    cards,
    links,
    groups,
    width: rootSize.w + 2 * ORG_MARGIN,
    height: rootSize.h + 2 * ORG_MARGIN,
  }
}
