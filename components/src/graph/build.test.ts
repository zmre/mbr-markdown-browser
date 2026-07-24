import { describe, it, expect } from 'vitest'
import { neighborsOf, edgesOf, mergeLevel, filterToDepth, type MiniGraph } from './build.ts'
import type { PageLinks } from './relationship-graph.ts'
import { rel } from './test-fixtures.ts'

function links(partial: Partial<PageLinks> = {}): PageLinks {
  return { inbound: [], outbound: [], ...partial }
}

function out(to: string, internal = true) {
  return { to, text: to, internal }
}

function inb(from: string) {
  return { from, text: from }
}

const allKnown = () => true

describe('neighborsOf', () => {
  it('unions internal outbound, inbound, and resolved relationships', () => {
    const page = links({
      outbound: [out('/b/'), out('https://example.com', false)],
      inbound: [inb('/c/')],
      relationships: [
        rel({ rel_type: 'parent', predicate: 'parent', neighbor: '/d/', direction: 'outgoing' }),
      ],
    })
    expect(neighborsOf('/a/', page)).toEqual(['/b/', '/c/', '/d/'])
  })

  it('excludes external outbound links', () => {
    const page = links({ outbound: [out('https://example.com/x/', false)] })
    expect(neighborsOf('/a/', page)).toEqual([])
  })

  it('removes self-loops, including slashless spellings', () => {
    const page = links({
      outbound: [out('/a/'), out('/a')],
      inbound: [inb('/a/')],
    })
    expect(neighborsOf('/a/', page)).toEqual([])
  })

  it('canonicalizes slashless paths and de-duplicates across sources', () => {
    const page = links({
      outbound: [out('/b')],
      inbound: [inb('/b/')],
      relationships: [
        rel({ rel_type: 'spouse', predicate: 'spouse', neighbor: '/b/', direction: 'outgoing' }),
      ],
    })
    expect(neighborsOf('/a/', page)).toEqual(['/b/'])
  })

  it('canonicalizes a slashless self before comparing', () => {
    const page = links({ outbound: [out('/b/')], inbound: [inb('/a/')] })
    expect(neighborsOf('/a', page)).toEqual(['/b/'])
  })

  it('skips unresolved and empty-neighbor relationships', () => {
    const page = links({
      relationships: [
        rel({
          rel_type: 'spouse',
          predicate: 'spouse',
          neighbor: '',
          neighbor_title: 'Ghost',
          resolved: false,
          direction: 'outgoing',
        }),
      ],
    })
    expect(neighborsOf('/a/', page)).toEqual([])
  })

  it('handles a payload without relationships', () => {
    expect(neighborsOf('/a/', links({ outbound: [out('/b/')] }))).toEqual(['/b/'])
  })
})

describe('edgesOf', () => {
  it('collapses an outbound link and a backlink to one undirected edge', () => {
    const page = links({ outbound: [out('/b/')], inbound: [inb('/b/')] })
    expect(edgesOf('/a/', page)).toEqual([{ source: '/a/', target: '/b/' }])
  })

  it('emits one edge per distinct neighbor, source-canonicalized', () => {
    const page = links({ outbound: [out('/b/'), out('/c')] })
    expect(edgesOf('/a', page)).toEqual([
      { source: '/a/', target: '/b/' },
      { source: '/a/', target: '/c/' },
    ])
  })
})

describe('mergeLevel', () => {
  function seed(focus = '/a/'): MiniGraph {
    return { focus, nodes: [{ id: focus, degree: 0 }], links: [], truncated: false }
  }

  it('adds neighbors as nodes at the given level with edges', () => {
    const fetched = [{ path: '/a/', links: links({ outbound: [out('/b/'), out('/c/')] }) }]
    const { graph, frontier } = mergeLevel(seed(), fetched, 1, 80, allKnown)
    expect(graph.nodes).toEqual([
      { id: '/a/', degree: 0 },
      { id: '/b/', degree: 1 },
      { id: '/c/', degree: 1 },
    ])
    expect(graph.links).toEqual([
      { source: '/a/', target: '/b/' },
      { source: '/a/', target: '/c/' },
    ])
    expect(graph.truncated).toBe(false)
    expect(frontier).toEqual(['/b/', '/c/'])
  })

  it('assigns the BFS level as degree and keeps earlier degrees (min wins)', () => {
    const level1 = mergeLevel(
      seed(),
      [{ path: '/a/', links: links({ outbound: [out('/b/')] }) }],
      1,
      80,
      allKnown
    )
    // /b/ links back to /a/ and on to /c/: /a/ keeps degree 0, /c/ gets 2.
    const level2 = mergeLevel(
      level1.graph,
      [{ path: '/b/', links: links({ outbound: [out('/a/'), out('/c/')] }) }],
      2,
      80,
      allKnown
    )
    const degrees = new Map(level2.graph.nodes.map((n) => [n.id, n.degree]))
    expect(degrees.get('/a/')).toBe(0)
    expect(degrees.get('/c/')).toBe(2)
    expect(level2.frontier).toEqual(['/c/'])
  })

  it('de-duplicates reciprocal edges across pages (sorted-pair key)', () => {
    const level1 = mergeLevel(
      seed(),
      [{ path: '/a/', links: links({ outbound: [out('/b/')] }) }],
      1,
      80,
      allKnown
    )
    const level2 = mergeLevel(
      level1.graph,
      [{ path: '/b/', links: links({ outbound: [out('/a/')] }) }],
      2,
      80,
      allKnown
    )
    expect(level2.graph.links).toEqual([{ source: '/a/', target: '/b/' }])
  })

  it('caps nodes at maxNodes, sets truncated, and drops edges to dropped nodes', () => {
    const fetched = [
      { path: '/a/', links: links({ outbound: [out('/b/'), out('/c/'), out('/d/')] }) },
    ]
    const { graph, frontier } = mergeLevel(seed(), fetched, 1, 2, allKnown)
    expect(graph.nodes.map((n) => n.id)).toEqual(['/a/', '/b/'])
    expect(graph.links).toEqual([{ source: '/a/', target: '/b/' }])
    expect(graph.truncated).toBe(true)
    expect(frontier).toEqual(['/b/'])
  })

  it('excludes targets rejected by the isKnownNote gate', () => {
    const fetched = [
      { path: '/a/', links: links({ outbound: [out('/b/'), out('/images/pic.jpg')] }) },
    ]
    const known = new Set(['/a/', '/b/'])
    const { graph } = mergeLevel(seed(), fetched, 1, 80, (p) => known.has(p))
    expect(graph.nodes.map((n) => n.id)).toEqual(['/a/', '/b/'])
    expect(graph.links).toEqual([{ source: '/a/', target: '/b/' }])
  })

  it('adds edges between already-included nodes without re-adding them', () => {
    const level1 = mergeLevel(
      seed(),
      [{ path: '/a/', links: links({ outbound: [out('/b/'), out('/c/')] }) }],
      1,
      80,
      allKnown
    )
    // /b/ ↔ /c/ cross-link at level 2: edge added, no new nodes.
    const level2 = mergeLevel(
      level1.graph,
      [{ path: '/b/', links: links({ outbound: [out('/c/')] }) }],
      2,
      80,
      allKnown
    )
    expect(level2.graph.nodes).toHaveLength(3)
    expect(level2.graph.links).toContainEqual({ source: '/b/', target: '/c/' })
    expect(level2.frontier).toEqual([])
  })

  it('ignores fetched pages that are not part of the graph', () => {
    const { graph } = mergeLevel(
      seed(),
      [{ path: '/stranger/', links: links({ outbound: [out('/x/')] }) }],
      1,
      80,
      allKnown
    )
    expect(graph.nodes).toHaveLength(1)
    expect(graph.links).toHaveLength(0)
  })

  it('does not mutate the input graph', () => {
    const input = seed()
    mergeLevel(input, [{ path: '/a/', links: links({ outbound: [out('/b/')] }) }], 1, 80, allKnown)
    expect(input.nodes).toHaveLength(1)
    expect(input.links).toHaveLength(0)
  })
})

describe('filterToDepth', () => {
  const graph: MiniGraph = {
    focus: '/a/',
    nodes: [
      { id: '/a/', degree: 0 },
      { id: '/b/', degree: 1 },
      { id: '/c/', degree: 2 },
    ],
    links: [
      { source: '/a/', target: '/b/' },
      { source: '/b/', target: '/c/' },
      { source: '/a/', target: '/c/' },
    ],
    truncated: true,
  }

  it('keeps only nodes within depth and links among survivors', () => {
    const filtered = filterToDepth(graph, 1)
    expect(filtered.nodes.map((n) => n.id)).toEqual(['/a/', '/b/'])
    expect(filtered.links).toEqual([{ source: '/a/', target: '/b/' }])
    expect(filtered.focus).toBe('/a/')
  })

  it('is the identity when depth covers every node', () => {
    const filtered = filterToDepth(graph, 2)
    expect(filtered.nodes).toEqual(graph.nodes)
    expect(filtered.links).toEqual(graph.links)
  })

  it('preserves the truncated flag', () => {
    expect(filterToDepth(graph, 1).truncated).toBe(true)
  })
})
