import { describe, expect, it, vi } from 'vitest'
import { createTaskMarkers, dayTaskCounts } from './task-markers.ts'

const NOW = () => new Date(2026, 9, 31, 12, 0)

function respond(body: unknown, ok = true) {
  return vi.fn((_input: RequestInfo | URL, _init?: RequestInit) =>
    Promise.resolve({ ok, status: ok ? 200 : 500, json: () => Promise.resolve(body) } as Response)
  )
}

describe('dayTaskCounts', () => {
  it('adds a day note’s undated open tasks to the placed ones (D9: markers only)', () => {
    const counts = dayTaskCounts(
      {
        days: { '2026-10-30': { open: 2, overdue: true }, '2026-11-02': { open: 1, overdue: false } },
        undated_in_day_notes: { '2026-11-02': 3, '2026-10-29': 1, '2026-11-05': 2 },
      },
      '2026-10-31'
    )
    expect(Object.fromEntries(counts)).toEqual({
      '2026-10-30': { open: 2, overdue: true },
      '2026-11-02': { open: 4, overdue: false },
      // Undated only, in the past: red, like any past day with open tasks.
      '2026-10-29': { open: 1, overdue: true },
      '2026-11-05': { open: 2, overdue: false },
    })
  })

  it('draws nothing for a day whose placed tasks are all done', () => {
    expect(dayTaskCounts({ days: { '2026-10-30': { open: 0, overdue: false } } }, '2026-10-31').size).toBe(0)
  })
})

describe('createTaskMarkers', () => {
  it('asks for counts only, over exactly the range, once per range', async () => {
    const fetchImpl = respond({ days: { '2026-10-30': { open: 1, overdue: true } }, undated_in_day_notes: {} })
    const markers = createTaskMarkers('/.mbr/tasks', fetchImpl as unknown as typeof fetch, NOW)
    const counts = await markers.source('2026-09-28', '2026-11-08')
    expect(counts.get('2026-10-30')).toEqual({ open: 1, overdue: true })
    await markers.source('2026-09-28', '2026-11-08')
    expect(fetchImpl).toHaveBeenCalledTimes(1)
    const [url, init] = fetchImpl.mock.calls[0]
    expect(url).toBe('/.mbr/tasks')
    expect(JSON.parse(init!.body as string)).toEqual({
      mode: 'grid',
      from: '2026-09-28',
      to: '2026-11-08',
      statuses: ['open'],
      limit: 0,
    })
    await markers.source('2026-10-26', '2026-12-06')
    expect(fetchImpl).toHaveBeenCalledTimes(2)
  })

  it('forgets everything on invalidate, under a new source identity', async () => {
    const fetchImpl = respond({ days: {} })
    const markers = createTaskMarkers('/.mbr/tasks', fetchImpl as unknown as typeof fetch, NOW)
    const first = markers.source
    await first('2026-09-28', '2026-11-08')
    const second = markers.invalidate()
    expect(second).not.toBe(first)
    expect(markers.source).toBe(second)
    await second('2026-09-28', '2026-11-08')
    expect(fetchImpl).toHaveBeenCalledTimes(2)
  })

  it('does not cache a failure', async () => {
    const fetchImpl = respond({}, false)
    const markers = createTaskMarkers('/.mbr/tasks', fetchImpl as unknown as typeof fetch, NOW)
    await expect(markers.source('2026-09-28', '2026-11-08')).rejects.toThrow('HTTP 500')
    await expect(markers.source('2026-09-28', '2026-11-08')).rejects.toThrow()
    expect(fetchImpl).toHaveBeenCalledTimes(2)
  })
})
