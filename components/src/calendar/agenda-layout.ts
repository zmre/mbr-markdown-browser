/**
 * Timeline layout for the day agenda (design/calendar.md §4.3): the visible
 * hour range and the side-by-side columns of overlapping entries.
 *
 * Pure, no imports: the companion's timeline today, and any later day or week
 * view that draws timed blocks.
 */

/** Something placed on the timeline: minutes after midnight. */
export interface Timed {
  readonly start: number
  readonly end?: number
}

/** One block as laid out. */
export interface LaidOut<T extends Timed> {
  readonly item: T
  readonly start: number
  /** The drawn end: the item's own, else `start + DEFAULT_DURATION`. */
  readonly end: number
  /** True when the item had no end (drawn with a dashed bottom edge). */
  readonly open: boolean
  /** 0-based column within its overlap cluster. */
  readonly column: number
  /** Columns in its overlap cluster (the block is `1 / columns` wide). */
  readonly columns: number
}

/** How long an entry without an end is drawn. */
export const DEFAULT_DURATION = 30

const DAY = 24 * 60

/** The drawn end of an item. */
export function drawnEnd(item: Timed): number {
  return Math.min(item.end ?? item.start + DEFAULT_DURATION, DAY)
}

/**
 * The minutes the timeline shows: `[min(start, 08:00) − 30 min,
 * max(end, 18:00) + 30 min]`, snapped outwards to whole hours and clamped to
 * the day. An empty agenda shows the working day.
 */
export function visibleRange(items: readonly Timed[]): { from: number; to: number } {
  const first = Math.min(8 * 60, ...items.map((i) => i.start))
  const last = Math.max(18 * 60, ...items.map(drawnEnd))
  return {
    from: Math.max(0, Math.floor((first - 30) / 60) * 60),
    to: Math.min(DAY, Math.ceil((last + 30) / 60) * 60),
  }
}

/**
 * The standard interval-partition layout: blocks sorted by start, then end;
 * transitively overlapping blocks form a cluster; within a cluster each block
 * takes the first column whose last block has ended (greedy), and every block
 * in the cluster is as wide as the cluster has columns. Touching blocks
 * (`10:00–10:30`, `10:30–11:00`) do not overlap.
 */
export function layoutAgenda<T extends Timed>(items: readonly T[]): LaidOut<T>[] {
  const blocks = items
    .map((item, index) => ({ item, index, start: item.start, end: Math.max(drawnEnd(item), item.start + 1) }))
    .sort((a, b) => a.start - b.start || a.end - b.end || a.index - b.index)

  const out: LaidOut<T>[] = []
  let cluster: { block: (typeof blocks)[number]; column: number }[] = []
  let columnEnds: number[] = []
  let clusterEnd = -1

  const flush = (): void => {
    for (const { block, column } of cluster) {
      out.push({
        item: block.item,
        start: block.start,
        end: drawnEnd(block.item),
        open: block.item.end === undefined,
        column,
        columns: columnEnds.length,
      })
    }
    cluster = []
    columnEnds = []
  }

  for (const block of blocks) {
    if (block.start >= clusterEnd) flush()
    let column = columnEnds.findIndex((end) => end <= block.start)
    if (column < 0) {
      column = columnEnds.length
      columnEnds.push(block.end)
    } else {
      columnEnds[column] = block.end
    }
    cluster.push({ block, column })
    clusterEnd = Math.max(clusterEnd, block.end)
  }
  flush()
  return out
}
