/**
 * Deterministic post-pass that pushes apart overlapping node LABELS.
 *
 * The force simulation's collision is circular, and a label is a wide, short
 * rectangle under its node: no circle radius both separates side-by-side labels
 * and leaves the layout compact, and the simulation is stochastic, so some
 * layouts still end up with "Dan Mos|Hana Ito". This pass fixes what is left,
 * in two phases:
 *
 * 1. A few gentle passes that move each overlapping pair apart (half each)
 *    along the axis of SMALLER penetration — the least visible correction,
 *    and all a typical force layout needs.
 * 2. If anything still overlaps (a jam, where pairwise pushes oscillate), a
 *    greedy top-to-bottom sweep: each label, in order of height, drops just
 *    below every already-placed label it overlaps horizontally. One pass, and
 *    no overlap by construction — a placed label never moves again, and each
 *    new one is placed clear of all of them.
 *
 * O(n²) with n ≤ the graph's node cap (80). Pure apart from mutating `x`/`y`.
 */

/** A node with its label's box size and offset from the node center. */
export interface LabelledPoint {
  x: number
  y: number
  /** Label width (user units). */
  w: number
  /** Label height (user units). */
  h: number
  /** Label center's vertical offset from the node center. */
  dy: number
}

/** Extra gap kept between separated labels. */
const GAP = 2
/** Gentle pairwise passes before falling back to the sweep. */
const GENTLE_PASSES = 8
/**
 * Overlaps at or below this are treated as none. Load-bearing for the sweep:
 * placing a label exactly below another leaves a rounding residue (~1e-14)
 * that would otherwise count as an overlap and re-trigger the same move
 * forever.
 */
const EPSILON = 1e-6

function overlapX(a: LabelledPoint, b: LabelledPoint): number {
  return (a.w + b.w) / 2 + GAP - Math.abs(b.x - a.x)
}

function overlapY(a: LabelledPoint, b: LabelledPoint): number {
  return (a.h + b.h) / 2 + GAP - Math.abs(b.y + b.dy - (a.y + a.dy))
}

/** Count overlapping pairs. */
function countOverlaps(points: LabelledPoint[]): number {
  let n = 0
  for (let i = 0; i < points.length; i++) {
    for (let j = i + 1; j < points.length; j++) {
      if (overlapX(points[i], points[j]) > EPSILON && overlapY(points[i], points[j]) > EPSILON) n++
    }
  }
  return n
}

/** Phase 1: one pairwise pass along the cheaper axis. Returns overlaps seen. */
function gentlePass(points: LabelledPoint[]): number {
  let seen = 0
  for (let i = 0; i < points.length; i++) {
    for (let j = i + 1; j < points.length; j++) {
      const a = points[i]
      const b = points[j]
      const ox = overlapX(a, b)
      const oy = overlapY(a, b)
      if (ox <= EPSILON || oy <= EPSILON) continue
      seen++
      // Ties (identical positions) break by index, so the result is stable.
      if (oy <= ox) {
        const dir = b.y + b.dy >= a.y + a.dy ? 1 : -1
        a.y -= (oy / 2) * dir
        b.y += (oy / 2) * dir
      } else {
        const dir = b.x >= a.x ? 1 : -1
        a.x -= (ox / 2) * dir
        b.x += (ox / 2) * dir
      }
    }
  }
  return seen
}

/** Phase 2: greedy downward sweep; leaves no overlap. */
function sweep(points: LabelledPoint[]): void {
  const order = points
    .map((p, i) => ({ p, i }))
    .sort((a, b) => a.p.y + a.p.dy - (b.p.y + b.p.dy) || a.i - b.i)
    .map(({ p }) => p)
  const placed: LabelledPoint[] = []
  for (const p of order) {
    // Re-check after every move: dropping below one label can land on another.
    let moved = true
    while (moved) {
      moved = false
      for (const q of placed) {
        if (overlapX(q, p) > EPSILON && overlapY(q, p) > EPSILON) {
          // Clear by a little more than the epsilon, so this q can never
          // trigger again; p only ever moves down, so the loop terminates.
          p.y = q.y + q.dy + (q.h + p.h) / 2 + GAP + 2 * EPSILON - p.dy
          moved = true
        }
      }
    }
    placed.push(p)
  }
}

/**
 * Separate overlapping labels in place. Returns the number of overlapping
 * pairs remaining, which is always 0: the sweep guarantees it.
 */
export function separateLabels(points: LabelledPoint[]): number {
  for (let pass = 0; pass < GENTLE_PASSES; pass++) {
    if (gentlePass(points) === 0) return 0
  }
  if (countOverlaps(points) === 0) return 0
  sweep(points)
  return countOverlaps(points)
}
