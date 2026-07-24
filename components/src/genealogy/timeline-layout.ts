/**
 * Pure layout for the custom "timeline tree" genealogy chart. No DOM, no d3.
 *
 * The chart plots a strict lineage (ancestors above, descendants below, plus
 * spouses of lineage members) on a vertical TIME axis: each person's card is
 * placed at their birth year, so generation gaps are visually proportional.
 * Horizontally, descendants get a tidy tree layout (couples in adjacent slots
 * joined by a marriage bar) and ancestors are pedigree-centered above their
 * children. Parent→child links are cubic beziers tinted by the PARENT's
 * gender; spouse pairs get a horizontal marriage bar.
 */
import type { GraphEdge, GraphNode, RelationshipGraph } from '../graph/relationship-graph.js'
import type { ViewBox } from '../graph/viewport.js'
import { SPOUSE_REL_TYPES } from './family-chart-data.js'

// ============================================================================
// Constants
// ============================================================================

/** Lineage depth cap in each direction (independent of `graph_depth`). */
export const LINEAGE_LEVEL_CAP = 2

export const CARD_W = 150
export const CARD_H = 34
/** Gap between spouse cards within a couple unit. */
const MEMBER_GAP = 14
/** Gap between adjacent units on the same level. */
const UNIT_GAP = 46
/** Outer margin around the drawing (exported for the view's axis rendering). */
export const MARGIN = 24
/**
 * Gutter reserved for year tick labels on BOTH the left and right edges of
 * the chart (only when `hasYears`). Exported for the view's axis rendering.
 */
export const AXIS_W = 64
/** Minimum vertical gap between consecutive generation bands (px). */
const MIN_LEVEL_GAP_PX = 60
/** Assumed years per generation for people with no usable birth year. */
const YEARS_PER_LEVEL = 28
/** Domain padding in years on each end of the axis. */
const YEAR_PAD = 5
/** Base vertical scale before the min-gap stretch pass. */
const BASE_PX_PER_YEAR = 2.2
/** Row height for the uniform (no-years) fallback layout. */
const ROW_HEIGHT = 110

/**
 * Smart-initial-zoom readability thresholds for the timeline chart.
 *
 * Deep/wide trees make the fit-all view render cards in a tiny band; when the
 * ON-SCREEN card height at fit-all falls below `MIN_READABLE_CARD_PX`, the
 * chart starts partly zoomed in (at `TARGET_READABLE_CARD_PX`) centered on the
 * focus person instead. Manual zoom-out to fit-all stays available.
 *
 * Values are keyed to on-screen TITLE text size so both genealogy charts feel
 * the same: the 34px timeline card carries a 12px title, so 24px ⇒ ≈8.5px
 * text (the legibility floor) and 32px ⇒ ≈11.3px text (comfortable).
 */
export const MIN_READABLE_CARD_PX = 24
export const TARGET_READABLE_CARD_PX = 32

// ============================================================================
// Output shapes
// ============================================================================

export interface TimelineNode {
  path: string
  title: string
  /** Card CENTER coordinates. */
  x: number
  y: number
  gender?: string
  born?: string
  died?: string
  bornPlace?: string
  isFocus: boolean
}

export type LinkColorKey = 'male' | 'female' | 'neutral'

export interface TimelineLink {
  /** SVG path data (cubic bezier from parent bottom to child top). */
  d: string
  /** The PARENT's gender bucket. */
  colorKey: LinkColorKey
  parent: string
  child: string
}

export interface MarriageBar {
  x1: number
  y1: number
  x2: number
  y2: number
  /** The spouse pair (left card first). */
  a: string
  b: string
}

export interface TimelineTick {
  y: number
  label: string
}

export interface TimelineLayout {
  nodes: TimelineNode[]
  links: TimelineLink[]
  marriageBars: MarriageBar[]
  ticks: TimelineTick[]
  width: number
  height: number
  hasYears: boolean
}

// ============================================================================
// Lineage extraction
// ============================================================================

export interface Lineage {
  /** url_path → level: negative = ancestors, 0 = focus, positive = descendants. */
  levels: Map<string, number>
  /** parent → children, both in the lineage. */
  childrenOf: Map<string, string[]>
  /** Deduped spouse pairs (sorted), both endpoints in the lineage. */
  spousePairs: Array<[string, string]>
}

function isSpouseEdge(e: GraphEdge): boolean {
  return e.kind === 'symmetric' && SPOUSE_REL_TYPES.has(e.relType)
}

/**
 * Extract the strict lineage around the focus: BFS up through parents (levels
 * −1..−`LINEAGE_LEVEL_CAP`) and down through children (+1..+cap), then add
 * spouses of every included person at their partner's level. A visited set
 * guards against cycles in malformed data (each person is levelled at most
 * once, so traversal always terminates). Spouses do not expand further.
 */
export function extractLineage(graph: RelationshipGraph): Lineage {
  const parentsOf = new Map<string, string[]>()
  const childrenOfAll = new Map<string, string[]>()
  const spousesOf = new Map<string, string[]>()
  const append = (map: Map<string, string[]>, key: string, value: string) => {
    const list = map.get(key)
    if (list) {
      if (!list.includes(value)) list.push(value)
    } else {
      map.set(key, [value])
    }
  }
  for (const e of graph.edges) {
    if (e.kind === 'hierarchical') {
      append(childrenOfAll, e.from, e.to)
      append(parentsOf, e.to, e.from)
    } else if (isSpouseEdge(e)) {
      append(spousesOf, e.from, e.to)
      append(spousesOf, e.to, e.from)
    }
  }

  const levels = new Map<string, number>()
  levels.set(graph.focus, 0)

  // Up (ancestors) and down (descendants), each a plain level-by-level BFS.
  for (const [adjacency, direction] of [
    [parentsOf, -1],
    [childrenOfAll, +1],
  ] as const) {
    let frontier = [graph.focus]
    for (let step = 1; step <= LINEAGE_LEVEL_CAP && frontier.length > 0; step++) {
      const next: string[] = []
      for (const person of frontier) {
        for (const neighbor of adjacency.get(person) ?? []) {
          if (levels.has(neighbor)) continue
          levels.set(neighbor, direction * step)
          next.push(neighbor)
        }
      }
      frontier = next
    }
  }

  // Spouses of included people join at the same level (single pass; a spouse's
  // own relatives are intentionally NOT expanded).
  for (const [person, level] of [...levels]) {
    for (const spouse of spousesOf.get(person) ?? []) {
      if (!levels.has(spouse)) levels.set(spouse, level)
    }
  }

  // Restrict adjacency to the lineage.
  const childrenOf = new Map<string, string[]>()
  for (const [parent, children] of childrenOfAll) {
    if (!levels.has(parent)) continue
    const kept = children.filter((c) => levels.has(c))
    if (kept.length > 0) childrenOf.set(parent, kept)
  }
  const pairKeys = new Set<string>()
  const spousePairs: Array<[string, string]> = []
  for (const [person, spouses] of spousesOf) {
    if (!levels.has(person)) continue
    for (const spouse of spouses) {
      if (!levels.has(spouse)) continue
      const [a, b] = person < spouse ? [person, spouse] : [spouse, person]
      const key = `${a}|${b}`
      if (pairKeys.has(key)) continue
      pairKeys.add(key)
      spousePairs.push([a, b])
    }
  }

  return { levels, childrenOf, spousePairs }
}

// ============================================================================
// Year assignment and scale
// ============================================================================

/** Median of a non-empty sorted-or-not numeric array. */
function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b)
  const mid = Math.floor(sorted.length / 2)
  return sorted.length % 2 === 1 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2
}

/**
 * Assign every lineage member an effective year: their own birth year, else
 * the median birth year of their level, else `anchor + 28·level` where the
 * anchor generation-0 year is derived from any dated person. Returns `null`
 * when nobody has a usable year (caller falls back to uniform rows).
 */
export function assignYears(
  lineage: Lineage,
  nodesByPath: Map<string, GraphNode>
): Map<string, number> | null {
  const bornOf = (path: string): number | undefined => {
    const born = nodesByPath.get(path)?.born
    const year = born ? Number.parseInt(born, 10) : Number.NaN
    return Number.isFinite(year) ? year : undefined
  }

  const knownByLevel = new Map<number, number[]>()
  const anchorEstimates: number[] = []
  for (const [path, level] of lineage.levels) {
    const year = bornOf(path)
    if (year === undefined) continue
    const list = knownByLevel.get(level)
    if (list) list.push(year)
    else knownByLevel.set(level, [year])
    anchorEstimates.push(year - YEARS_PER_LEVEL * level)
  }
  if (anchorEstimates.length === 0) return null
  const anchor = median(anchorEstimates)

  const years = new Map<string, number>()
  for (const [path, level] of lineage.levels) {
    const own = bornOf(path)
    if (own !== undefined) {
      years.set(path, own)
      continue
    }
    const levelYears = knownByLevel.get(level)
    years.set(path, levelYears ? median(levelYears) : anchor + YEARS_PER_LEVEL * level)
  }
  return years
}

/**
 * Choose a "nice" year-axis tick step so the axis stays readable: the smallest
 * of 5/10/20/25/50 that yields at most ~10 ticks over the domain span.
 */
export function chooseTickStep(spanYears: number): number {
  const steps = [5, 10, 20, 25, 50]
  for (const step of steps) {
    if (spanYears / step <= 10) return step
  }
  return steps[steps.length - 1]
}

interface YearScale {
  y: (year: number) => number
  domainMin: number
  domainMax: number
}

/**
 * Linear year→y scale. The px-per-year factor is stretched (a pure linear
 * stretch, so year positions stay proportional) until consecutive generation
 * bands — represented by each level's median effective year — are at least
 * `MIN_LEVEL_GAP_PX` apart. Returns `null` when adjacent generation bands have
 * non-increasing years (pathological data); the caller then falls back to
 * uniform rows.
 */
export function computeYearScale(lineage: Lineage, years: Map<string, number>): YearScale | null {
  let domainMin = Infinity
  let domainMax = -Infinity
  for (const year of years.values()) {
    if (year < domainMin) domainMin = year
    if (year > domainMax) domainMax = year
  }
  domainMin -= YEAR_PAD
  domainMax += YEAR_PAD

  // Representative (median) year per level, in level order.
  const byLevel = new Map<number, number[]>()
  for (const [path, level] of lineage.levels) {
    const year = years.get(path)!
    const list = byLevel.get(level)
    if (list) list.push(year)
    else byLevel.set(level, [year])
  }
  const orderedLevels = [...byLevel.keys()].sort((a, b) => a - b)
  let minGapYears = Infinity
  for (let i = 1; i < orderedLevels.length; i++) {
    const gap = median(byLevel.get(orderedLevels[i])!) - median(byLevel.get(orderedLevels[i - 1])!)
    if (gap < minGapYears) minGapYears = gap
  }
  if (orderedLevels.length > 1 && minGapYears <= 0) return null

  let pxPerYear = BASE_PX_PER_YEAR
  if (Number.isFinite(minGapYears) && minGapYears > 0) {
    pxPerYear = Math.max(pxPerYear, MIN_LEVEL_GAP_PX / minGapYears)
  }

  return {
    y: (year: number) => MARGIN + CARD_H / 2 + (year - domainMin) * pxPerYear,
    domainMin,
    domainMax,
  }
}

// ============================================================================
// Horizontal (tidy) layout
// ============================================================================

interface Unit {
  members: string[]
  level: number
}

/**
 * Group each level's members into "units": couples (via spouse pairs) share a
 * unit, everyone else is a singleton. Members with multiple spouses sit in the
 * middle of their unit so every marriage bar spans adjacent cards.
 */
function buildUnits(lineage: Lineage, nodesByPath: Map<string, GraphNode>): Map<string, Unit> {
  // Union-find over same-level spouse pairs.
  const parent = new Map<string, string>()
  const find = (x: string): string => {
    let root = x
    while (parent.get(root) !== undefined && parent.get(root) !== root) root = parent.get(root)!
    return root
  }
  for (const path of lineage.levels.keys()) parent.set(path, path)
  for (const [a, b] of lineage.spousePairs) {
    if (lineage.levels.get(a) !== lineage.levels.get(b)) continue
    const [ra, rb] = [find(a), find(b)]
    if (ra !== rb) parent.set(ra, rb)
  }

  const groups = new Map<string, string[]>()
  for (const path of lineage.levels.keys()) {
    const root = find(path)
    const list = groups.get(root)
    if (list) list.push(path)
    else groups.set(root, [path])
  }

  const spouseCount = new Map<string, number>()
  for (const [a, b] of lineage.spousePairs) {
    spouseCount.set(a, (spouseCount.get(a) ?? 0) + 1)
    spouseCount.set(b, (spouseCount.get(b) ?? 0) + 1)
  }
  const sortKey = (path: string): [number, string] => {
    const born = nodesByPath.get(path)?.born
    const year = born ? Number.parseInt(born, 10) : Number.NaN
    return [Number.isFinite(year) ? year : Number.MAX_SAFE_INTEGER, path]
  }

  const unitOf = new Map<string, Unit>()
  for (const members of groups.values()) {
    const ordered = [...members].sort((a, b) => {
      const [ya, pa] = sortKey(a)
      const [yb, pb] = sortKey(b)
      return ya - yb || (pa < pb ? -1 : 1)
    })
    if (ordered.length > 2) {
      // Put the most-married member in the middle so bars span adjacent cards.
      const primary = [...ordered].sort(
        (a, b) => (spouseCount.get(b) ?? 0) - (spouseCount.get(a) ?? 0)
      )[0]
      const others = ordered.filter((m) => m !== primary)
      const mid = Math.floor(others.length / 2)
      ordered.splice(0, ordered.length, ...others.slice(0, mid), primary, ...others.slice(mid))
    }
    const unit: Unit = { members: ordered, level: lineage.levels.get(members[0])! }
    for (const m of members) unitOf.set(m, unit)
  }
  return unitOf
}

const unitWidth = (unit: Unit): number =>
  unit.members.length * CARD_W + (unit.members.length - 1) * MEMBER_GAP

/** Member center x positions for a unit centered at `centerX`. */
function placeMembers(unit: Unit, centerX: number, x: Map<string, number>): void {
  const n = unit.members.length
  unit.members.forEach((member, i) => {
    x.set(member, centerX + (i - (n - 1) / 2) * (CARD_W + MEMBER_GAP))
  })
}

/**
 * Assign card-center x coordinates: a tidy tree over descendant units (each
 * unit centered over its children), then ancestors pedigree-centered over the
 * children they parent (with a left-to-right overlap-resolution sweep).
 */
function assignX(lineage: Lineage, nodesByPath: Map<string, GraphNode>): Map<string, number> {
  const x = new Map<string, number>()
  const unitOf = buildUnits(lineage, nodesByPath)
  const focusUnit = unitOf.get(
    [...lineage.levels.entries()].find(([, level]) => level === 0)![0]
  )!

  const sortKey = (unit: Unit): string => unit.members[0]
  // Children units of a descendant unit: units at level+1 containing a child of
  // any member. Each child unit attaches to exactly one parent unit.
  const claimed = new Set<Unit>()
  const childUnits = (unit: Unit): Unit[] => {
    const out: Unit[] = []
    for (const member of unit.members) {
      for (const child of lineage.childrenOf.get(member) ?? []) {
        const cu = unitOf.get(child)
        if (!cu || cu.level !== unit.level + 1 || claimed.has(cu)) continue
        claimed.add(cu)
        out.push(cu)
      }
    }
    return out.sort((a, b) => (sortKey(a) < sortKey(b) ? -1 : 1))
  }

  // Pass 1 (post-order): subtree widths for the descendant tree.
  const subtreeWidth = new Map<Unit, number>()
  const childrenByUnit = new Map<Unit, Unit[]>()
  const measure = (unit: Unit): number => {
    const children = childUnits(unit)
    childrenByUnit.set(unit, children)
    let childrenWidth = 0
    for (const child of children) childrenWidth += measure(child)
    if (children.length > 0) childrenWidth += (children.length - 1) * UNIT_GAP
    const width = Math.max(unitWidth(unit), childrenWidth)
    subtreeWidth.set(unit, width)
    return width
  }
  measure(focusUnit)

  // Pass 2 (pre-order): place descendants; each unit centered over children.
  const placeDescendants = (unit: Unit, left: number): void => {
    const width = subtreeWidth.get(unit)!
    const children = childrenByUnit.get(unit)!
    if (children.length === 0) {
      placeMembers(unit, left + width / 2, x)
      return
    }
    let childrenWidth = (children.length - 1) * UNIT_GAP
    for (const child of children) childrenWidth += subtreeWidth.get(child)!
    let cursor = left + (width - childrenWidth) / 2
    let first = Infinity
    let last = -Infinity
    for (const child of children) {
      const childWidth = subtreeWidth.get(child)!
      placeDescendants(child, cursor)
      const childCenter = cursor + childWidth / 2
      if (childCenter < first) first = childCenter
      if (childCenter > last) last = childCenter
      cursor += childWidth + UNIT_GAP
    }
    placeMembers(unit, (first + last) / 2, x)
  }
  placeDescendants(focusUnit, 0)

  // Ancestors: level −1 upward, each unit centered over the (already placed)
  // children its members parent, then a sweep resolves overlaps.
  let minLevel = 0
  for (const level of lineage.levels.values()) if (level < minLevel) minLevel = level
  for (let level = -1; level >= minLevel; level--) {
    const units = new Set<Unit>()
    for (const [path, l] of lineage.levels) {
      if (l === level) units.add(unitOf.get(path)!)
    }
    const anchored = [...units].map((unit) => {
      const childXs: number[] = []
      for (const member of unit.members) {
        for (const child of lineage.childrenOf.get(member) ?? []) {
          const cx = x.get(child)
          if (cx !== undefined) childXs.push(cx)
        }
      }
      const anchor =
        childXs.length > 0 ? childXs.reduce((a, b) => a + b, 0) / childXs.length : 0
      return { unit, anchor }
    })
    anchored.sort((a, b) => a.anchor - b.anchor || (sortKey(a.unit) < sortKey(b.unit) ? -1 : 1))
    let minLeft = -Infinity
    for (const { unit, anchor } of anchored) {
      const width = unitWidth(unit)
      const left = Math.max(anchor - width / 2, minLeft)
      placeMembers(unit, left + width / 2, x)
      minLeft = left + width + UNIT_GAP
    }
  }

  return x
}

// ============================================================================
// Full layout
// ============================================================================

/** Map a normalized gender to a link color bucket. */
export function colorKeyForGender(gender: string | undefined): LinkColorKey {
  if (gender === 'male' || gender === 'm' || gender === 'man') return 'male'
  if (gender === 'female' || gender === 'f' || gender === 'woman') return 'female'
  return 'neutral'
}

/** Cubic bezier from a parent card's bottom edge to a child card's top edge. */
function linkPath(px: number, py: number, cx: number, cy: number): string {
  const fromY = py + CARD_H / 2
  const toY = cy - CARD_H / 2
  const bend = Math.max((toY - fromY) * 0.5, 10)
  return `M ${px} ${fromY} C ${px} ${fromY + bend}, ${cx} ${toY - bend}, ${cx} ${toY}`
}

/**
 * Compute the full timeline-tree layout for a relationship graph. Coordinates
 * are in SVG user units with the origin at the top-left of the drawing.
 */
export function computeTimelineLayout(graph: RelationshipGraph): TimelineLayout {
  const nodesByPath = new Map(graph.nodes.map((n) => [n.urlPath, n]))
  const lineage = extractLineage(graph)

  // Vertical: year scale, or uniform rows when years are unusable.
  const years = assignYears(lineage, nodesByPath)
  const scale = years ? computeYearScale(lineage, years) : null
  const hasYears = scale !== null
  let minLevel = 0
  for (const level of lineage.levels.values()) if (level < minLevel) minLevel = level
  const yOf = (path: string): number =>
    scale
      ? scale.y(years!.get(path)!)
      : MARGIN + CARD_H / 2 + (lineage.levels.get(path)! - minLevel) * ROW_HEIGHT

  // Horizontal, then shift everything right of the axis gutter and margin.
  const rawX = assignX(lineage, nodesByPath)
  let minX = Infinity
  for (const value of rawX.values()) if (value < minX) minX = value
  const xShift = MARGIN + (hasYears ? AXIS_W : 0) + CARD_W / 2 - minX
  const xOf = (path: string): number => rawX.get(path)! + xShift

  const nodes: TimelineNode[] = [...lineage.levels.keys()].map((path) => {
    const meta = nodesByPath.get(path)
    return {
      path,
      title: meta?.title ?? path,
      x: xOf(path),
      y: yOf(path),
      gender: meta?.gender,
      born: meta?.born,
      died: meta?.died,
      bornPlace: meta?.bornPlace,
      isFocus: path === graph.focus,
    }
  })

  const links: TimelineLink[] = []
  for (const [parent, children] of lineage.childrenOf) {
    for (const child of children) {
      links.push({
        d: linkPath(xOf(parent), yOf(parent), xOf(child), yOf(child)),
        colorKey: colorKeyForGender(nodesByPath.get(parent)?.gender),
        parent,
        child,
      })
    }
  }

  const marriageBars: MarriageBar[] = []
  for (const [a, b] of lineage.spousePairs) {
    const [leftPath, rightPath] = xOf(a) <= xOf(b) ? [a, b] : [b, a]
    const x1 = xOf(leftPath) + CARD_W / 2
    const x2 = xOf(rightPath) - CARD_W / 2
    if (x2 <= x1) continue
    marriageBars.push({ x1, y1: yOf(leftPath), x2, y2: yOf(rightPath), a: leftPath, b: rightPath })
  }

  let maxX = 0
  let maxY = 0
  for (const node of nodes) {
    if (node.x + CARD_W / 2 > maxX) maxX = node.x + CARD_W / 2
    if (node.y + CARD_H / 2 > maxY) maxY = node.y + CARD_H / 2
  }
  // With a year axis, mirror the left label gutter on the right so tick labels
  // can be rendered on both sides without clipping.
  const width = maxX + (hasYears ? AXIS_W : 0) + MARGIN
  const height = maxY + MARGIN

  const ticks: TimelineTick[] = []
  if (scale) {
    const step = chooseTickStep(scale.domainMax - scale.domainMin)
    for (
      let year = Math.ceil(scale.domainMin / step) * step;
      year <= scale.domainMax;
      year += step
    ) {
      ticks.push({ y: scale.y(year), label: String(year) })
    }
  }

  return { nodes, links, marriageBars, ticks, width, height, hasYears }
}

// ============================================================================
// Smart initial viewport
// ============================================================================

export interface InitialViewBoxParams {
  /** Drawing (SVG user unit) size of the whole chart content. */
  contentWidth: number
  contentHeight: number
  /** On-screen canvas size in CSS px. */
  canvasWidth: number
  canvasHeight: number
  /** Card height in SVG user units (readability is judged on cards). */
  cardH: number
  /** Focus card center, in content coordinates. */
  focusX: number
  focusY: number
  /** On-screen card height below which fit-all is considered unreadable. */
  minReadablePx: number
  /** On-screen card height the zoomed initial view aims for. */
  targetPx: number
}

/** Clamp a range start into [lo, hi]; center when the range doesn't fit. */
function clampOrCenter(start: number, lo: number, hi: number): number {
  return hi < lo ? (lo + hi) / 2 : Math.min(Math.max(start, lo), hi)
}

/**
 * Choose the initial viewBox for a chart: the fit-all box (`0 0 w h`) when the
 * on-screen card height at fit-all scale is readable, else a box zoomed so
 * cards render at `targetPx`, centered on the focus card and clamped to the
 * content bounds (centered on the content in any dimension where the zoomed
 * view is larger than the content). Pure; used by the timeline view and, via
 * coordinate conversion, by the family-chart view.
 *
 * The fit-all scale mirrors `preserveAspectRatio="xMidYMid meet"`:
 * `min(canvasW/contentW, canvasH/contentH)`. The zoomed box uses the canvas
 * aspect exactly (`w = canvasW/s`, `h = canvasH/s` with `s = targetPx/cardH`),
 * so no letterboxing occurs and the achieved scale is exactly `s`.
 */
export function computeInitialViewBox(p: InitialViewBoxParams): ViewBox {
  const fitAll: ViewBox = { x: 0, y: 0, w: p.contentWidth, h: p.contentHeight }
  if (
    !(p.contentWidth > 0) ||
    !(p.contentHeight > 0) ||
    !(p.canvasWidth > 0) ||
    !(p.canvasHeight > 0) ||
    !(p.cardH > 0) ||
    !(p.targetPx > 0)
  ) {
    return fitAll
  }

  const fitScale = Math.min(p.canvasWidth / p.contentWidth, p.canvasHeight / p.contentHeight)
  if (p.cardH * fitScale >= p.minReadablePx) return fitAll

  const s = p.targetPx / p.cardH
  const w = p.canvasWidth / s
  const h = p.canvasHeight / s
  return {
    x: clampOrCenter(p.focusX - w / 2, 0, p.contentWidth - w),
    y: clampOrCenter(p.focusY - h / 2, 0, p.contentHeight - h),
    w,
    h,
  }
}
