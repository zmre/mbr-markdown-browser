import { describe, it, expect } from 'vitest'
import {
  buildRegistry,
  buildRelationshipGraph,
  type GraphEdge,
  type RelationshipGraph,
  type SiteNote,
} from '../graph/relationship-graph.js'
import { GENEALOGY_TYPES, genealogyNotes } from '../graph/test-fixtures.js'
import {
  AXIS_W,
  CARD_W,
  LINEAGE_LEVEL_CAP,
  MARGIN,
  MIN_READABLE_CARD_PX,
  TARGET_READABLE_CARD_PX,
  assignYears,
  chooseTickStep,
  colorKeyForGender,
  computeInitialViewBox,
  computeTimelineLayout,
  extractLineage,
  type TimelineLayout,
} from './timeline-layout.js'

/** Build the genealogy fixture graph, optionally mutating notes first. */
function fixtureGraph(
  focus = '/people/john/',
  mutate?: (notes: Map<string, SiteNote>) => void
): RelationshipGraph {
  const notes = genealogyNotes()
  mutate?.(notes)
  return buildRelationshipGraph(focus, notes, buildRegistry(GENEALOGY_TYPES))
}

/** Hand-rolled graph for synthetic topologies (chains, cycles). */
function syntheticGraph(paths: string[], edges: GraphEdge[], focus: string): RelationshipGraph {
  return {
    focus,
    nodes: paths.map((p) => ({ urlPath: p, title: p, isFocus: p === focus })),
    edges,
  }
}

const hier = (from: string, to: string): GraphEdge => ({
  from,
  to,
  kind: 'hierarchical',
  relType: 'child',
  label: '',
})

const nodeOf = (layout: TimelineLayout, path: string) => {
  const node = layout.nodes.find((n) => n.path === path)
  expect(node, `node ${path}`).toBeDefined()
  return node!
}

describe('UNIT extractLineage', () => {
  it('assigns fixture lineage levels around the focus', () => {
    const lineage = extractLineage(fixtureGraph())
    expect(lineage.levels.get('/people/john/')).toBe(0)
    expect(lineage.levels.get('/people/george/')).toBe(-1)
    expect(lineage.levels.get('/people/martha/')).toBe(-1)
    // Spouse of the focus joins at the focus level.
    expect(lineage.levels.get('/people/mary/')).toBe(0)
    expect(lineage.levels.get('/people/alice/')).toBe(1)
    expect(lineage.levels.get('/people/sam/')).toBe(1)
    // The focus's brother is NOT part of the strict lineage.
    expect(lineage.levels.has('/people/robert/')).toBe(false)
  })

  it('honors the ancestor/descendant level caps', () => {
    const paths = ['/p0/', '/p1/', '/p2/', '/p3/', '/p4/', '/p5/', '/p6/']
    const edges = paths.slice(0, -1).map((p, i) => hier(p, paths[i + 1]))
    const lineage = extractLineage(syntheticGraph(paths, edges, '/p3/'))
    expect(LINEAGE_LEVEL_CAP).toBe(2)
    expect(lineage.levels.get('/p1/')).toBe(-2)
    expect(lineage.levels.get('/p2/')).toBe(-1)
    expect(lineage.levels.get('/p4/')).toBe(1)
    expect(lineage.levels.get('/p5/')).toBe(2)
    // Beyond ±2 generations is cut off.
    expect(lineage.levels.has('/p0/')).toBe(false)
    expect(lineage.levels.has('/p6/')).toBe(false)
  })

  it('terminates on cyclic parent data', () => {
    const cyclic = syntheticGraph(['/a/', '/b/'], [hier('/a/', '/b/'), hier('/b/', '/a/')], '/a/')
    const lineage = extractLineage(cyclic)
    expect(lineage.levels.get('/a/')).toBe(0)
    expect(lineage.levels.has('/b/')).toBe(true)
    // The whole layout also completes without hanging.
    const layout = computeTimelineLayout(cyclic)
    expect(layout.nodes).toHaveLength(2)
  })
})

describe('UNIT year scale', () => {
  it('positions cards proportionally to birth years', () => {
    const layout = computeTimelineLayout(fixtureGraph())
    expect(layout.hasYears).toBe(true)
    const george = nodeOf(layout, '/people/george/') // b. 1898
    const john = nodeOf(layout, '/people/john/') // b. 1925
    const alice = nodeOf(layout, '/people/alice/') // b. 1950
    expect(john.y).toBeGreaterThan(george.y)
    expect(alice.y).toBeGreaterThan(john.y)
    const ratio = (john.y - george.y) / (alice.y - george.y)
    expect(ratio).toBeCloseTo((1925 - 1898) / (1950 - 1898), 5)
  })

  it('falls back to the level median for a missing birth year', () => {
    const layout = computeTimelineLayout(
      fixtureGraph('/people/john/', (notes) => {
        const martha = notes.get('/people/martha/')!
        martha.frontmatter = { ...martha.frontmatter, born: undefined }
      })
    )
    // Martha has no year; her level's median (George's 1898) places her.
    expect(nodeOf(layout, '/people/martha/').y).toBeCloseTo(
      nodeOf(layout, '/people/george/').y,
      5
    )
  })

  it('renders uniform rows without an axis when nobody has a year', () => {
    const layout = computeTimelineLayout(
      fixtureGraph('/people/john/', (notes) => {
        for (const note of notes.values()) {
          note.frontmatter = { ...note.frontmatter, born: undefined, died: undefined }
        }
      })
    )
    expect(layout.hasYears).toBe(false)
    expect(layout.ticks).toHaveLength(0)
    const george = nodeOf(layout, '/people/george/')
    const martha = nodeOf(layout, '/people/martha/')
    const john = nodeOf(layout, '/people/john/')
    const alice = nodeOf(layout, '/people/alice/')
    // Same level → same row; consecutive levels evenly spaced.
    expect(martha.y).toBeCloseTo(george.y, 5)
    expect(alice.y - john.y).toBeCloseTo(john.y - george.y, 5)
  })

  it('assignYears returns null when no lineage member has a year', () => {
    const graph = syntheticGraph(['/a/', '/b/'], [hier('/a/', '/b/')], '/a/')
    const lineage = extractLineage(graph)
    const nodesByPath = new Map(graph.nodes.map((n) => [n.urlPath, n]))
    expect(assignYears(lineage, nodesByPath)).toBeNull()
  })

  it('reserves an AXIS_W label gutter on BOTH sides when the year axis shows', () => {
    const layout = computeTimelineLayout(fixtureGraph())
    expect(layout.hasYears).toBe(true)
    const minCardLeft = Math.min(...layout.nodes.map((n) => n.x - CARD_W / 2))
    const maxCardRight = Math.max(...layout.nodes.map((n) => n.x + CARD_W / 2))
    expect(minCardLeft).toBeCloseTo(MARGIN + AXIS_W, 5)
    expect(layout.width).toBeCloseTo(maxCardRight + AXIS_W + MARGIN, 5)
  })

  it('reserves no axis gutters when nobody has a year', () => {
    const layout = computeTimelineLayout(
      fixtureGraph('/people/john/', (notes) => {
        for (const note of notes.values()) {
          note.frontmatter = { ...note.frontmatter, born: undefined, died: undefined }
        }
      })
    )
    expect(layout.hasYears).toBe(false)
    const minCardLeft = Math.min(...layout.nodes.map((n) => n.x - CARD_W / 2))
    const maxCardRight = Math.max(...layout.nodes.map((n) => n.x + CARD_W / 2))
    expect(minCardLeft).toBeCloseTo(MARGIN, 5)
    expect(layout.width).toBeCloseTo(maxCardRight + MARGIN, 5)
  })

  it('produces year-axis ticks within the padded domain', () => {
    const layout = computeTimelineLayout(fixtureGraph())
    expect(layout.ticks.length).toBeGreaterThan(2)
    const labels = layout.ticks.map((t) => Number(t.label))
    // Fixture years span 1898–1953, padded ±5 → step 10.
    expect(labels[0]).toBeGreaterThanOrEqual(1893)
    expect(labels[labels.length - 1]).toBeLessThanOrEqual(1958)
    expect(labels[1] - labels[0]).toBe(10)
    // Tick y positions strictly increase with the year.
    for (let i = 1; i < layout.ticks.length; i++) {
      expect(layout.ticks[i].y).toBeGreaterThan(layout.ticks[i - 1].y)
    }
  })
})

describe('UNIT chooseTickStep', () => {
  it.each([
    [30, 5],
    [80, 10],
    [150, 20],
    [220, 25],
    [400, 50],
    [900, 50], // capped at the largest step
  ])('span %d years → step %d', (span, step) => {
    expect(chooseTickStep(span)).toBe(step)
  })
})

describe('UNIT link colors', () => {
  it('maps genders to color keys with a neutral fallback', () => {
    expect(colorKeyForGender('male')).toBe('male')
    expect(colorKeyForGender('m')).toBe('male')
    expect(colorKeyForGender('female')).toBe('female')
    expect(colorKeyForGender('f')).toBe('female')
    expect(colorKeyForGender(undefined)).toBe('neutral')
    expect(colorKeyForGender('other')).toBe('neutral')
  })

  it("colors each parent→child link by the PARENT's gender", () => {
    const layout = computeTimelineLayout(
      fixtureGraph('/people/john/', (notes) => {
        const george = notes.get('/people/george/')!
        george.frontmatter = { ...george.frontmatter, gender: 'male' }
        const martha = notes.get('/people/martha/')!
        martha.frontmatter = { ...martha.frontmatter, gender: 'Female' }
      })
    )
    const link = (parent: string, child: string) =>
      layout.links.find((l) => l.parent === parent && l.child === child)
    expect(link('/people/george/', '/people/john/')?.colorKey).toBe('male')
    expect(link('/people/martha/', '/people/john/')?.colorKey).toBe('female')
    // John has no gender in the fixture → neutral.
    expect(link('/people/john/', '/people/alice/')?.colorKey).toBe('neutral')
  })
})

describe('UNIT computeInitialViewBox', () => {
  // 34px cards, 24px trigger, 32px target — the timeline's real constants.
  const common = {
    cardH: 34,
    minReadablePx: MIN_READABLE_CARD_PX,
    targetPx: TARGET_READABLE_CARD_PX,
  }

  it('returns the fit-all box unchanged when cards stay readable', () => {
    // fitScale = min(800/1000, 600/700) = 0.8 → 27.2px cards ≥ 24px.
    const vb = computeInitialViewBox({
      ...common,
      contentWidth: 1000,
      contentHeight: 700,
      canvasWidth: 800,
      canvasHeight: 600,
      focusX: 500,
      focusY: 350,
    })
    expect(vb).toEqual({ x: 0, y: 0, w: 1000, h: 700 })
  })

  it('returns fit-all for tiny content that fits fine (scale > 1)', () => {
    const vb = computeInitialViewBox({
      ...common,
      contentWidth: 300,
      contentHeight: 200,
      canvasWidth: 800,
      canvasHeight: 600,
      focusX: 150,
      focusY: 100,
    })
    expect(vb).toEqual({ x: 0, y: 0, w: 300, h: 200 })
  })

  it('zooms to target readability centered on the focus for degenerate wide content', () => {
    // fitScale = min(800/8000, 560/500) = 0.1 → 3.4px cards → unreadable.
    const vb = computeInitialViewBox({
      ...common,
      contentWidth: 8000,
      contentHeight: 500,
      canvasWidth: 800,
      canvasHeight: 560,
      focusX: 4000,
      focusY: 250,
    })
    const s = 32 / 34
    expect(vb.w).toBeCloseTo(800 / s, 5)
    expect(vb.h).toBeCloseTo(560 / s, 5)
    // On-screen card height at the achieved scale is exactly the target.
    expect(34 * (800 / vb.w)).toBeCloseTo(32, 5)
    // Horizontally centered on the focus; vertically the view is taller than
    // the content, so the content is centered instead.
    expect(vb.x).toBeCloseTo(4000 - vb.w / 2, 5)
    expect(vb.y).toBeCloseTo((500 - vb.h) / 2, 5)
  })

  it('clamps to the left/right content edges when the focus is near an edge', () => {
    const wide = {
      ...common,
      contentWidth: 8000,
      contentHeight: 500,
      canvasWidth: 800,
      canvasHeight: 560,
      focusY: 250,
    }
    const left = computeInitialViewBox({ ...wide, focusX: 100 })
    expect(left.x).toBe(0)
    const right = computeInitialViewBox({ ...wide, focusX: 7950 })
    expect(right.x).toBeCloseTo(8000 - right.w, 5)
  })

  it('falls back to fit-all on degenerate canvas or content sizes', () => {
    const degenerate = {
      ...common,
      contentWidth: 8000,
      contentHeight: 500,
      focusX: 4000,
      focusY: 250,
    }
    expect(
      computeInitialViewBox({ ...degenerate, canvasWidth: 0, canvasHeight: 560 })
    ).toEqual({ x: 0, y: 0, w: 8000, h: 500 })
    expect(
      computeInitialViewBox({ ...degenerate, canvasWidth: 800, canvasHeight: 0 })
    ).toEqual({ x: 0, y: 0, w: 8000, h: 500 })
    expect(
      computeInitialViewBox({
        ...common,
        contentWidth: 0,
        contentHeight: 0,
        canvasWidth: 800,
        canvasHeight: 560,
        focusX: 0,
        focusY: 0,
      })
    ).toEqual({ x: 0, y: 0, w: 0, h: 0 })
  })
})

describe('UNIT horizontal layout', () => {
  it('centers the focus couple over its children and parents over the focus', () => {
    const layout = computeTimelineLayout(fixtureGraph())
    const john = nodeOf(layout, '/people/john/')
    const mary = nodeOf(layout, '/people/mary/')
    const alice = nodeOf(layout, '/people/alice/')
    const sam = nodeOf(layout, '/people/sam/')
    const george = nodeOf(layout, '/people/george/')
    const martha = nodeOf(layout, '/people/martha/')
    // Focus couple centered over the children row.
    expect((john.x + mary.x) / 2).toBeCloseTo((alice.x + sam.x) / 2, 5)
    // Parent couple pedigree-centered over the focus card.
    expect((george.x + martha.x) / 2).toBeCloseTo(john.x, 5)
    // Everything fits inside the reported bounds.
    for (const node of layout.nodes) {
      expect(node.x).toBeGreaterThan(0)
      expect(node.x).toBeLessThan(layout.width)
      expect(node.y).toBeGreaterThan(0)
      expect(node.y).toBeLessThan(layout.height)
    }
  })

  it('emits a marriage bar between each spouse pair', () => {
    const layout = computeTimelineLayout(fixtureGraph())
    const pairs = layout.marriageBars.map((bar) => [bar.a, bar.b].sort().join('|'))
    expect(pairs).toContain('/people/john/|/people/mary/')
    expect(pairs).toContain('/people/george/|/people/martha/')
    for (const bar of layout.marriageBars) {
      expect(bar.x2).toBeGreaterThan(bar.x1)
    }
  })

  it('marks the focus node', () => {
    const layout = computeTimelineLayout(fixtureGraph())
    const focused = layout.nodes.filter((n) => n.isFocus)
    expect(focused).toHaveLength(1)
    expect(focused[0].path).toBe('/people/john/')
  })
})
