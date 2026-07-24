import { describe, it, expect, vi } from 'vitest'
import { expandNeighborhood, mapWithConcurrency, type FetchPageLinks } from './bfs.ts'
import type { MiniGraph } from './build.ts'
import type { PageLinks } from './relationship-graph.ts'

/** Build a PageLinks with internal outbound links to the given paths. */
function outLinks(...to: string[]): PageLinks {
  return {
    inbound: [],
    outbound: to.map((t) => ({ to: t, text: t, internal: true })),
  }
}

/** A stub fetcher over a path → links map (missing paths resolve null). */
function stubFetch(map: Record<string, PageLinks>): FetchPageLinks & ReturnType<typeof vi.fn> {
  return vi.fn(async (path: string) => map[path] ?? null)
}

const allKnown = () => true

describe('mapWithConcurrency', () => {
  it('preserves item order in the results', async () => {
    const results = await mapWithConcurrency([3, 1, 2], 2, async (n) => {
      await new Promise((resolve) => setTimeout(resolve, n))
      return n * 10
    })
    expect(results).toEqual([30, 10, 20])
  })

  it('never exceeds the concurrency limit', async () => {
    let active = 0
    let maxActive = 0
    await mapWithConcurrency(Array.from({ length: 10 }, (_, i) => i), 3, async () => {
      active++
      maxActive = Math.max(maxActive, active)
      await new Promise((resolve) => setTimeout(resolve, 1))
      active--
    })
    expect(maxActive).toBeLessThanOrEqual(3)
    expect(maxActive).toBeGreaterThan(1)
  })

  it('stops dispatching new work once the signal aborts', async () => {
    const controller = new AbortController()
    const calls: number[] = []
    const results = await mapWithConcurrency(
      [1, 2, 3, 4, 5],
      1,
      async (n) => {
        calls.push(n)
        if (n === 2) controller.abort()
        return n
      },
      controller.signal
    )
    expect(calls).toEqual([1, 2])
    expect(results).toEqual([1, 2])
  })
})

describe('expandNeighborhood', () => {
  it('returns null when the focus has no links.json', async () => {
    const graph = await expandNeighborhood({
      focus: '/a/',
      depth: 2,
      fetchLinks: stubFetch({}),
      isKnownNote: allKnown,
    })
    expect(graph).toBeNull()
  })

  it('builds levels breadth-first with minimum degrees', async () => {
    const fetchLinks = stubFetch({
      '/a/': outLinks('/b/', '/c/'),
      '/b/': outLinks('/a/', '/c/', '/d/'),
      '/c/': outLinks('/d/'),
    })
    const graph = (await expandNeighborhood({
      focus: '/a/',
      depth: 2,
      fetchLinks,
      isKnownNote: allKnown,
    })) as MiniGraph
    const degrees = new Map(graph.nodes.map((n) => [n.id, n.degree]))
    expect(degrees.get('/a/')).toBe(0)
    expect(degrees.get('/b/')).toBe(1)
    // /c/ is reachable at level 1 AND via /b/ at level 2: level 1 wins.
    expect(degrees.get('/c/')).toBe(1)
    expect(degrees.get('/d/')).toBe(2)
  })

  it('canonicalizes a slashless focus path', async () => {
    const fetchLinks = stubFetch({ '/a/': outLinks('/b/') })
    const graph = (await expandNeighborhood({
      focus: '/a',
      depth: 1,
      fetchLinks,
      isKnownNote: allKnown,
    })) as MiniGraph
    expect(graph.focus).toBe('/a/')
    expect(fetchLinks).toHaveBeenCalledWith('/a/')
  })

  it('calls onUpdate with a snapshot after each settled level', async () => {
    const fetchLinks = stubFetch({
      '/a/': outLinks('/b/'),
      '/b/': outLinks('/c/'),
    })
    const snapshots: MiniGraph[] = []
    await expandNeighborhood({
      focus: '/a/',
      depth: 2,
      fetchLinks,
      isKnownNote: allKnown,
      onUpdate: (graph) => snapshots.push(graph),
    })
    expect(snapshots).toHaveLength(2)
    expect(snapshots[0].nodes.map((n) => n.id)).toEqual(['/a/', '/b/'])
    expect(snapshots[1].nodes.map((n) => n.id)).toEqual(['/a/', '/b/', '/c/'])
  })

  it('does not fetch the final level frontier (leaves)', async () => {
    const fetchLinks = stubFetch({
      '/a/': outLinks('/b/', '/c/'),
      '/b/': outLinks('/d/'),
      '/c/': outLinks('/e/'),
    })
    await expandNeighborhood({ focus: '/a/', depth: 1, fetchLinks, isKnownNote: allKnown })
    // Depth 1: only the focus itself is fetched; /b/ and /c/ are leaves.
    expect(fetchLinks.mock.calls.map((c) => c[0])).toEqual(['/a/'])
  })

  it('keeps a neighbor whose own fetch resolves null as a leaf', async () => {
    const fetchLinks = stubFetch({
      '/a/': outLinks('/b/', '/c/'),
      '/c/': outLinks('/d/'),
      // '/b/' intentionally missing: its links.json fetch resolves null.
    })
    const graph = (await expandNeighborhood({
      focus: '/a/',
      depth: 2,
      fetchLinks,
      isKnownNote: allKnown,
    })) as MiniGraph
    const ids = graph.nodes.map((n) => n.id)
    expect(ids).toContain('/b/')
    expect(ids).toContain('/d/')
    // /b/ contributed no expansions but remains in the graph.
    expect(graph.links).toContainEqual({ source: '/a/', target: '/b/' })
  })

  it('respects the per-level concurrency ceiling', async () => {
    const neighbors = Array.from({ length: 10 }, (_, i) => `/n${i}/`)
    const map: Record<string, PageLinks> = { '/a/': outLinks(...neighbors) }
    for (const n of neighbors) map[n] = outLinks('/a/')

    let active = 0
    let maxActive = 0
    const fetchLinks: FetchPageLinks = async (path) => {
      active++
      maxActive = Math.max(maxActive, active)
      await new Promise((resolve) => setTimeout(resolve, 1))
      active--
      return map[path] ?? null
    }
    await expandNeighborhood({ focus: '/a/', depth: 2, fetchLinks, isKnownNote: allKnown })
    expect(maxActive).toBeLessThanOrEqual(4)
    expect(maxActive).toBeGreaterThan(1)
  })

  it('stops fetching when aborted mid-level', async () => {
    const neighbors = ['/b/', '/c/', '/d/', '/e/', '/f/', '/g/']
    const map: Record<string, PageLinks> = { '/a/': outLinks(...neighbors) }
    for (const n of neighbors) map[n] = outLinks('/z/')
    map['/z/'] = outLinks()

    const controller = new AbortController()
    let frontierFetches = 0
    const fetchLinks: FetchPageLinks = async (path) => {
      if (path !== '/a/') {
        frontierFetches++
        if (frontierFetches === 2) controller.abort()
      }
      return map[path] ?? null
    }
    const graph = (await expandNeighborhood({
      focus: '/a/',
      depth: 3,
      concurrency: 1,
      fetchLinks,
      isKnownNote: allKnown,
      signal: controller.signal,
    })) as MiniGraph
    // Only the two frontier fetches before the abort ran; /z/ never appears.
    expect(frontierFetches).toBe(2)
    expect(graph.nodes.map((n) => n.id)).not.toContain('/z/')
    expect(Math.max(...graph.nodes.map((n) => n.degree))).toBe(1)
  })

  it('halts expansion when the node cap is reached', async () => {
    const neighbors = Array.from({ length: 10 }, (_, i) => `/n${i}/`)
    const map: Record<string, PageLinks> = { '/a/': outLinks(...neighbors) }
    for (const n of neighbors) map[n] = outLinks('/deep/')
    const fetchLinks = stubFetch(map)

    const graph = (await expandNeighborhood({
      focus: '/a/',
      depth: 3,
      maxNodes: 4,
      fetchLinks,
      isKnownNote: allKnown,
    })) as MiniGraph
    expect(graph.nodes).toHaveLength(4)
    expect(graph.truncated).toBe(true)
    // Cap already hit after level 1: no second-level fetches at all.
    expect(fetchLinks.mock.calls.map((c) => c[0])).toEqual(['/a/'])
  })

  it('applies the isKnownNote gate during expansion', async () => {
    const fetchLinks = stubFetch({
      '/a/': outLinks('/b/', '/media/clip.mp4'),
      '/b/': outLinks(),
    })
    const known = new Set(['/a/', '/b/'])
    const graph = (await expandNeighborhood({
      focus: '/a/',
      depth: 2,
      fetchLinks,
      isKnownNote: (p) => known.has(p),
    })) as MiniGraph
    expect(graph.nodes.map((n) => n.id)).toEqual(['/a/', '/b/'])
  })
})
