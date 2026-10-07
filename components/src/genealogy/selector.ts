/**
 * Chart-type selector: a compact native `<select>` overlaid top-left of the
 * relationship charts, persisting the chosen chart id in localStorage (same
 * `mbr_*` key convention and try/catch guards as mbr-browse's recent-files).
 */
import {
  CHART_TYPES,
  DEFAULT_CHART_ID,
  isChartApplicable,
  type GenealogyContext,
} from './chart-registry.js'

/** Key kept from the genealogy-only days so existing choices survive. */
export const CHART_STORAGE_KEY = 'mbr_genealogy_chart'

/**
 * Default chart order when there is no applicable persisted choice: family
 * first (the original purpose), then the org chart, then the graph that can
 * always draw something.
 */
const FALLBACK_ORDER = ['family-chart', 'org-chart', 'all-people']

/** Map a stored (possibly stale/unknown) chart id to a valid one. */
export function resolveChartId(stored: string | null | undefined): string {
  if (stored && CHART_TYPES.some((chart) => chart.id === stored)) return stored
  return DEFAULT_CHART_ID
}

/** The persisted chart id if it names a known chart, else `null`. */
export function readStoredChartChoice(): string | null {
  try {
    const stored = localStorage.getItem(CHART_STORAGE_KEY)
    return stored && CHART_TYPES.some((chart) => chart.id === stored) ? stored : null
  } catch {
    return null
  }
}

/** Read the persisted chart id, tolerating unavailable/broken localStorage. */
export function readStoredChartId(): string {
  return readStoredChartChoice() ?? DEFAULT_CHART_ID
}

/** Persist the chosen chart id; storage failures are ignored. */
export function storeChartId(id: string): void {
  try {
    localStorage.setItem(CHART_STORAGE_KEY, id)
  } catch {
    // Ignore localStorage errors (private mode, quota, disabled storage).
  }
}

/**
 * The chart to open with: the persisted choice when it can draw this note
 * (a Family-chart fan opening a company page should not get an empty chart),
 * else family edges → Family chart, work edges → Org chart, else All people.
 * Only a user's explicit selection is ever persisted, so falling back here
 * never overwrites their preference.
 */
export function chooseChartId(stored: string | null, ctx: GenealogyContext): string {
  const applicable = (id: string) => {
    const chart = CHART_TYPES.find((c) => c.id === id)
    return chart !== undefined && isChartApplicable(chart, ctx)
  }
  if (stored && applicable(stored)) return stored
  return FALLBACK_ORDER.find(applicable) ?? 'all-people'
}

/**
 * Build the selector element with `active` selected. With a context, charts
 * that have nothing to draw for this note are listed but disabled, so the menu
 * keeps one stable shape across pages.
 */
export function createSelector(
  active: string,
  onChange: (id: string) => void,
  ctx?: GenealogyContext
): HTMLSelectElement {
  const select = document.createElement('select')
  select.className = 'gen-chart-select'
  select.setAttribute('aria-label', 'Chart type')
  for (const chart of CHART_TYPES) {
    const option = document.createElement('option')
    option.value = chart.id
    option.textContent = chart.label
    option.selected = chart.id === active
    if (ctx && chart.id !== active && !isChartApplicable(chart, ctx)) option.disabled = true
    select.appendChild(option)
  }
  select.addEventListener('change', () => onChange(resolveChartId(select.value)))
  return select
}
