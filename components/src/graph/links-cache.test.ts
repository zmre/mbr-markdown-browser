import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { fetchPageLinks } from './links-cache.ts'
import type { PageLinks } from './relationship-graph.ts'

/**
 * NOTE: `src/test-setup.ts` installs a global fetch mock; these tests override
 * it per-test via `vi.stubGlobal` for precise control. The module-level cache
 * in links-cache persists across tests, so each test uses UNIQUE paths.
 */

function okResponse(payload: PageLinks): Partial<Response> {
  return { ok: true, status: 200, json: async () => payload }
}

function statusResponse(status: number): Partial<Response> {
  return { ok: false, status, json: async () => ({}) }
}

const EMPTY_LINKS: PageLinks = { inbound: [], outbound: [] }

describe('fetchPageLinks', () => {
  const originalConfig = window.__MBR_CONFIG__

  beforeEach(() => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    window.__MBR_CONFIG__ = originalConfig
  })

  it('fetches once per canonical path (slashless and slashed share an entry)', async () => {
    const fetchMock = vi.fn().mockResolvedValue(okResponse(EMPTY_LINKS))
    vi.stubGlobal('fetch', fetchMock)

    const [a, b] = await Promise.all([
      fetchPageLinks('/notes/one'),
      fetchPageLinks('/notes/one/'),
    ])
    await fetchPageLinks('/notes/one/')

    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(a).toEqual(EMPTY_LINKS)
    expect(b).toEqual(EMPTY_LINKS)
  })

  it('builds a server-mode URL with per-segment encoding', async () => {
    const fetchMock = vi.fn().mockResolvedValue(okResponse(EMPTY_LINKS))
    vi.stubGlobal('fetch', fetchMock)

    await fetchPageLinks('/Walsh/Patrick Joseph Walsh b.1977-10-01/')

    expect(fetchMock).toHaveBeenCalledWith(
      '/Walsh/Patrick%20Joseph%20Walsh%20b.1977-10-01/links.json'
    )
  })

  it('builds a static-mode URL using the base path', async () => {
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../../' }
    const fetchMock = vi.fn().mockResolvedValue(okResponse(EMPTY_LINKS))
    vi.stubGlobal('fetch', fetchMock)

    await fetchPageLinks('/docs/my guide/')

    expect(fetchMock).toHaveBeenCalledWith('../../docs/my%20guide/links.json')
  })

  it('caches a 404 as a permanent null (no refetch)', async () => {
    const fetchMock = vi.fn().mockResolvedValue(statusResponse(404))
    vi.stubGlobal('fetch', fetchMock)

    expect(await fetchPageLinks('/notes/two/')).toBeNull()
    expect(await fetchPageLinks('/notes/two/')).toBeNull()
    expect(fetchMock).toHaveBeenCalledTimes(1)
  })

  it('resolves null on a network error but allows a retry', async () => {
    const fetchMock = vi
      .fn()
      .mockRejectedValueOnce(new Error('network down'))
      .mockResolvedValueOnce(okResponse(EMPTY_LINKS))
    vi.stubGlobal('fetch', fetchMock)

    expect(await fetchPageLinks('/notes/three/')).toBeNull()
    // The failed entry was evicted: the next call fetches again and succeeds.
    expect(await fetchPageLinks('/notes/three/')).toEqual(EMPTY_LINKS)
    expect(fetchMock).toHaveBeenCalledTimes(2)
  })

  it('treats a server error (5xx) as retryable', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(statusResponse(500))
      .mockResolvedValueOnce(okResponse(EMPTY_LINKS))
    vi.stubGlobal('fetch', fetchMock)

    expect(await fetchPageLinks('/notes/four/')).toBeNull()
    expect(await fetchPageLinks('/notes/four/')).toEqual(EMPTY_LINKS)
    expect(fetchMock).toHaveBeenCalledTimes(2)
  })

  it('never rejects, even when JSON parsing fails', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => {
        throw new Error('bad json')
      },
    })
    vi.stubGlobal('fetch', fetchMock)

    await expect(fetchPageLinks('/notes/five/')).resolves.toBeNull()
  })
})
