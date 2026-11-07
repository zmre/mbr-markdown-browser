import { afterEach, describe, expect, it, vi } from 'vitest'
import { agendaFromHtml, clearDayAgendaCache, loadDayAgenda } from './day-agenda.ts'

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

  it('retries a failed fetch next time', async () => {
    const fetchPageHtml = vi.fn().mockRejectedValueOnce(new Error('offline')).mockResolvedValue(PAGE)
    await expect(loadDayAgenda('/d/', fetchPageHtml)).rejects.toThrow('offline')
    await expect(loadDayAgenda('/d/', fetchPageHtml)).resolves.toHaveLength(2)
    expect(fetchPageHtml).toHaveBeenCalledTimes(2)
  })
})
