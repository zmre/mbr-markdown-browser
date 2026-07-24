/**
 * Chart registry for the person-page genealogy chunk.
 *
 * Each chart is a small `GenealogyChart` descriptor with a `mount()` factory;
 * `mountGenealogy()` (index.ts) renders a selector over `CHART_TYPES` and
 * mounts the active chart. EXTENSIBILITY: a future chart (ancestors/descendants
 * sunburst, hierarchical edge bundling, birth-place bubble map, …) is one new
 * module exporting a `GenealogyChart` plus one entry appended to `CHART_TYPES`
 * below — nothing else changes. If a future chart is heavyweight, add an
 * optional `load(): Promise<GenealogyChart>` thunk here and resolve it in
 * `mountGenealogy` before mounting.
 *
 * NOTE on the import cycle: this module imports the chart view modules (values)
 * while those views import the style helper below (value) and types from here.
 * That ESM cycle is safe: the views only *call* `injectStylesOnce` at mount
 * time (long after module evaluation), and `CHART_TYPES` is built after the
 * view modules finish evaluating.
 */
import type { RelationshipGraph, Registry, SiteNote } from '../graph/relationship-graph.js'
import { familyChartType } from './family-chart-view.js'
import { timelineChartType } from './timeline-view.js'

/** Everything a chart needs, passed down from the `<mbr-genealogy>` trigger. */
export interface GenealogyContext {
  /** De-duplicated relationship graph centred on the current person. */
  graph: RelationshipGraph
  /** `url_path` → site.json note (frontmatter etc.) for every known note. */
  notesByPath: Map<string, SiteNote>
  /** Relationship-type registry from site.json's `relationship_types`. */
  registry: Registry
  /** Canonical url_path of the focused person (same as `graph.focus`). */
  focusPath: string
  /** Resolve a root-relative url_path to an href valid for the current page. */
  resolveUrl: (path: string) => string
  /** Navigate to a note by url_path (same tab). */
  navigate: (path: string) => void
}

/** A mounted chart; `destroy()` must remove all DOM and listeners it added. */
export interface GenealogyChartInstance {
  destroy(): void
}

/** A selectable chart type. */
export interface GenealogyChart {
  /** Stable id, persisted in localStorage. */
  id: string
  /** Human-readable label shown in the selector. */
  label: string
  mount(container: HTMLElement, ctx: GenealogyContext): GenealogyChartInstance
}

/** All available charts, in selector order. family-chart is the default. */
export const CHART_TYPES: GenealogyChart[] = [familyChartType, timelineChartType]

export const DEFAULT_CHART_ID = 'family-chart'

// ============================================================================
// Style injection (shared by the chunk's views)
// ============================================================================

/** Roots that already received a given stylesheet id. */
const injectedStyles = new WeakMap<Node, Set<string>>()

/**
 * Inject a stylesheet into a document or shadow root exactly once per root.
 * Prefers constructable stylesheets (`adoptedStyleSheets`); falls back to an
 * appended `<style>` element when they are unavailable.
 */
export function injectStylesOnce(root: Node, id: string, cssText: string): void {
  const target = root instanceof Document || root instanceof ShadowRoot ? root : document
  let ids = injectedStyles.get(target)
  if (ids?.has(id)) return
  if (!ids) {
    ids = new Set()
    injectedStyles.set(target, ids)
  }
  ids.add(id)

  try {
    const sheet = new CSSStyleSheet()
    sheet.replaceSync(cssText)
    target.adoptedStyleSheets = [...target.adoptedStyleSheets, sheet]
    return
  } catch {
    // Constructable stylesheets unsupported; fall through to a <style> tag.
  }
  const style = document.createElement('style')
  style.dataset.mbrStyle = id
  style.textContent = cssText
  if (target instanceof Document) target.head.appendChild(style)
  else target.appendChild(style)
}
