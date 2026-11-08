/**
 * Timeline layout for the day agenda (design/calendar.md §4.3): the visible
 * hour range, the side-by-side columns of overlapping entries, and the whole
 * drawn timeline ({@link timelineModel}: hour labels, block geometry and
 * labels, the screen-reader lines) that the companion renders with Lit and
 * the QuickLook preview with plain DOM.
 *
 * Pure, importing only `month-grid.ts`'s clock formatter.
 */
import { formatClock, formatSpan } from './month-grid.js'

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

/** Pixels per minute on a timeline (40px an hour). */
export const PX_PER_MINUTE = 40 / 60
/** Shortest drawn block, so a 5-minute entry still has a readable line. */
export const MIN_BLOCK_PX = 18

/** What a timeline draws per entry. */
export interface TimelineItem extends Timed {
  readonly title: string
  readonly details: readonly string[]
  readonly task?: string
}

/**
 * One block, placed: its `top` and `height` (px) and its column (`column` of
 * `columns`, which each renderer's CSS turns into `left` / `width` right of
 * its hour gutter, through {@link blockStyle}), plus its texts.
 */
export interface TimelineBlock<T extends TimelineItem> {
  readonly item: T
  readonly open: boolean
  readonly top: number
  readonly height: number
  readonly column: number
  readonly columns: number
  /** The start time shown in the block. */
  readonly time: string
  /** The full text: hover title and accessible name. */
  readonly label: string
}

/** A day's timeline, ready to draw. */
export interface Timeline<T extends TimelineItem> {
  /** The visible minutes ({@link visibleRange}). */
  readonly from: number
  readonly to: number
  readonly height: number
  readonly hours: readonly { readonly top: number; readonly label: string }[]
  readonly blocks: readonly TimelineBlock<T>[]
  /** One line per entry, in the entries' order, for the screen-reader list. */
  readonly spoken: readonly string[]
}

/**
 * A block's inline style: its `top` and `height`, and its column as `--col` /
 * `--cols`, which the renderer's stylesheet turns into `left` and `width`
 * (`left: calc(gutter + (100% - gutter) * var(--col) / var(--cols))`).
 */
export function blockStyle(b: TimelineBlock<TimelineItem>): string {
  return `top:${b.top}px;height:${b.height}px;--col:${b.column};--cols:${b.columns}`
}

/** Everything a timeline draws for `entries` in `locale`. */
export function timelineModel<T extends TimelineItem>(entries: readonly T[], locale: string): Timeline<T> {
  const { from, to } = visibleRange(entries)
  const y = (m: number): number => (m - from) * PX_PER_MINUTE
  const hours = []
  for (let h = from; h <= to; h += 60) {
    hours.push({ top: y(h), label: h < 24 * 60 ? formatClock(h, locale).replace(/:00(?=\D|$)/, '') : '' })
  }
  const blocks = layoutAgenda(entries).map((b): TimelineBlock<T> => ({
    item: b.item,
    open: b.open,
    top: y(b.start),
    height: Math.max(y(b.end) - y(b.start), MIN_BLOCK_PX),
    column: b.column,
    columns: b.columns,
    time: formatClock(b.item.start, locale),
    label: [formatSpan(b.item, locale), b.item.title, ...b.item.details].join(' · '),
  }))
  const spoken = entries.map(
    (e) => `${formatSpan(e, locale)} ${e.title}${e.task ? ` (${e.task})` : ''}${e.details.length ? `: ${e.details.join('; ')}` : ''}`
  )
  return { from, to, height: y(to), hours, blocks, spoken }
}

/** The current-time line's `top` on a timeline, or `null` when `minutes` is not on it. */
export function nowLineTop(timeline: { readonly from: number; readonly to: number }, minutes: number | null): number | null {
  return minutes === null || minutes < timeline.from || minutes > timeline.to ? null : (minutes - timeline.from) * PX_PER_MINUTE
}
