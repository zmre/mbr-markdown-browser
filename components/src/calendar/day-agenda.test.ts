import { afterEach, describe, expect, it, vi } from 'vitest'
import { DAY_AGENDA_CACHE_SIZE, agendaFromHtml, clearDayAgendaCache, loadDayAgenda } from './day-agenda.ts'

const PAGE = `<!doctype html><html><body>
<header><nav><ul><li>09:00 Not in main</li></ul></nav></header>
<main id="wrapper"><h2>Calendar</h2><ul><li>09:00 - 09:15 Standup<ul><li>Bob</li></ul></li><li>13:00 Lunch</li></ul></main>
</body></html>`

afterEach(() => clearDayAgendaCache())

describe('agendaFromHtml', () => {
  it('parses another page’s note body with the same parser', () => {
    expect(agendaFromHtml(PAGE).map((e) => [e.ordinal, e.title, e.details])).toEqual([
      [1, 'Standup', ['Bob']],
      [2, 'Lunch', []],
    ])
  })
})

describe('loadDayAgenda', () => {
  it('fetches each day once per page', async () => {
    const fetchPageHtml = vi.fn().mockResolvedValue(PAGE)
    const first = await loadDayAgenda('/daily/2026-10-30/', fetchPageHtml)
    const second = await loadDayAgenda('/daily/2026-10-30/', fetchPageHtml)
    expect(first).toBe(second)
    expect(fetchPageHtml).toHaveBeenCalledTimes(1)
    expect(fetchPageHtml).toHaveBeenCalledWith('/daily/2026-10-30/')
  })

  it('caches plain data, never the parsed document', async () => {
    const entries = await loadDayAgenda('/d/', vi.fn().mockResolvedValue(PAGE))
    expect(entries).toEqual([
      { start: 540, end: 555, title: 'Standup', details: ['Bob'], task: undefined, ordinal: 1 },
      { start: 780, end: undefined, title: 'Lunch', details: [], task: undefined, ordinal: 2 },
    ])
    // Nothing reachable from a cached entry is a DOM node: an `<li>` would
    // keep the whole fetched document alive for the page's life.
    const reachesNode = (v: unknown): boolean =>
      v instanceof Node || (typeof v === 'object' && v !== null && Object.values(v).some(reachesNode))
    expect(reachesNode(entries)).toBe(false)
  })

  it('keeps only the most recently used days', async () => {
    const fetchPageHtml = vi.fn().mockResolvedValue(PAGE)
    const url = (n: number) => `/daily/${n}/`
    for (let n = 0; n < DAY_AGENDA_CACHE_SIZE; n++) await loadDayAgenda(url(n), fetchPageHtml)
    expect(fetchPageHtml).toHaveBeenCalledTimes(DAY_AGENDA_CACHE_SIZE)
    // Touch the oldest, then one more day evicts the least recently used (1).
    await loadDayAgenda(url(0), fetchPageHtml)
    await loadDayAgenda(url(DAY_AGENDA_CACHE_SIZE), fetchPageHtml)
    expect(fetchPageHtml).toHaveBeenCalledTimes(DAY_AGENDA_CACHE_SIZE + 1)
    await loadDayAgenda(url(0), fetchPageHtml)
    expect(fetchPageHtml).toHaveBeenCalledTimes(DAY_AGENDA_CACHE_SIZE + 1)
    await loadDayAgenda(url(1), fetchPageHtml)
    expect(fetchPageHtml).toHaveBeenCalledTimes(DAY_AGENDA_CACHE_SIZE + 2)
  })

  it('retries a failed fetch next time', async () => {
    const fetchPageHtml = vi.fn().mockRejectedValueOnce(new Error('offline')).mockResolvedValue(PAGE)
    await expect(loadDayAgenda('/d/', fetchPageHtml)).rejects.toThrow('offline')
    await expect(loadDayAgenda('/d/', fetchPageHtml)).resolves.toHaveLength(2)
    expect(fetchPageHtml).toHaveBeenCalledTimes(2)
  })
})
