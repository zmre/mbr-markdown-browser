/**
 * Chart-type selector: a compact native `<select>` overlaid top-left of the
 * genealogy chart, persisting the chosen chart id in localStorage (same
 * `mbr_*` key convention and try/catch guards as mbr-browse's recent-files).
 */
import { CHART_TYPES, DEFAULT_CHART_ID } from './chart-registry.js'

export const CHART_STORAGE_KEY = 'mbr_genealogy_chart'

/** Map a stored (possibly stale/unknown) chart id to a valid one. */
export function resolveChartId(stored: string | null | undefined): string {
  if (stored && CHART_TYPES.some((chart) => chart.id === stored)) return stored
  return DEFAULT_CHART_ID
}

/** Read the persisted chart id, tolerating unavailable/broken localStorage. */
export function readStoredChartId(): string {
  try {
    return resolveChartId(localStorage.getItem(CHART_STORAGE_KEY))
  } catch {
    return DEFAULT_CHART_ID
  }
}

/** Persist the chosen chart id; storage failures are ignored. */
export function storeChartId(id: string): void {
  try {
    localStorage.setItem(CHART_STORAGE_KEY, id)
  } catch {
    // Ignore localStorage errors (private mode, quota, disabled storage).
  }
}

/** Build the selector element with `active` selected. */
export function createSelector(active: string, onChange: (id: string) => void): HTMLSelectElement {
  const select = document.createElement('select')
  select.className = 'gen-chart-select'
  select.setAttribute('aria-label', 'Chart type')
  for (const chart of CHART_TYPES) {
    const option = document.createElement('option')
    option.value = chart.id
    option.textContent = chart.label
    option.selected = chart.id === active
    select.appendChild(option)
  }
  select.addEventListener('change', () => onChange(resolveChartId(select.value)))
  return select
}
