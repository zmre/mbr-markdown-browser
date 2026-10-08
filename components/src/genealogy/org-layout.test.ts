import { describe, it, expect } from 'vitest'
import { buildRegistry } from '../graph/relationship-graph.js'
import { CONTACT_TYPES, LEGACY_CONTACT_TYPES, buildSiteNotes, companyNotes } from '../graph/test-fixtures.js'
import {
  ORG_CARD_W,
  ORG_MAX_INITIAL_SCALE,
  ORG_MIN_TITLE_PX,
  ORG_EXPAND_STEP,
  ORG_TITLE_PX,
  anchoredOrgView,
  buildOrgTree,
  computeOrgInitialView,
  computeOrgLayout,
  hasWorkHierarchy,
  nextOrgBudget,
  type OrgCard,
  type OrgNode,
} from './org-layout.js'

const registry = buildRegistry(CONTACT_TYPES)

/** Indented `id` outline of a tree, for readable whole-shape assertions. */
function outline(node: OrgNode, depth = 0): string[] {
  const label = node.kind === 'more' ? `${node.title}→${node.target}` : node.id
  return [`${'  '.repeat(depth)}${label}${node.isFocus ? ' *' : ''}`, ...node.children.flatMap((c) => outline(c, depth + 1))]
}

function overlaps(a: OrgCard, b: OrgCard): boolean {
  return (
    Math.abs(a.x - b.x) < (a.w + b.w) / 2 && Math.abs(a.y - b.y) < (a.h + b.h) / 2
  )
}

describe('buildOrgTree', () => {
  it('roots an organization page at the organization, structured by reporting lines', () => {
    const tree = buildOrgTree('/orgs/acme/', companyNotes(), registry)
    expect(outline(tree!.root)).toEqual([
      '/orgs/acme/ *',
      '  /people/ada/',
      '    /people/bob/',
      // Departments alphabetically (Design, Engineering), then by title.
      '      /people/fay/',
      '      /people/carol/',
      '      /people/eve/',
      '    /people/dan/',
    ])
    expect(tree!.hidden).toBe(0)
  })

  it('shows a person their chain, their peers and their reports', () => {
    const tree = buildOrgTree('/people/bob/', companyNotes(), registry)
    expect(outline(tree!.root)).toEqual([
      '/orgs/acme/',
      '  /people/ada/',
      '    /people/bob/ *',
      '      /people/fay/',
      '      /people/carol/',
      '      /people/eve/',
      '    /people/dan/',
    ])
  })

  it('shows a leaf person their chain and peers', () => {
    const tree = buildOrgTree('/people/carol/', companyNotes(), registry)
    expect(outline(tree!.root)).toEqual([
      '/orgs/acme/',
      '  /people/ada/',
      '    /people/bob/',
      '      /people/fay/',
      '      /people/carol/ *',
      '      /people/eve/',
    ])
  })

  it('records the edge type to the tree parent', () => {
    const tree = buildOrgTree('/orgs/acme/', companyNotes(), registry)!
    const ada = tree.root.children[0]
    expect(ada.relType).toBe('employee')
    expect(ada.children.find((c) => c.id === '/people/dan/')?.relType).toBe('assistant')
    expect(ada.children.find((c) => c.id === '/people/bob/')?.relType).toBe('manages')
  })

  it('builds the same tree from a legacy registry (no hierarchy/category)', () => {
    const legacy = buildRegistry(LEGACY_CONTACT_TYPES)
    const modern = buildOrgTree('/people/bob/', companyNotes(), registry)
    const old = buildOrgTree('/people/bob/', companyNotes(LEGACY_CONTACT_TYPES), legacy)
    expect(outline(old!.root)).toEqual(outline(modern!.root))
  })

  it('returns null when the focus has no work hierarchy', () => {
    expect(buildOrgTree('/people/george/', companyNotes(), registry)).toBeNull()
    expect(hasWorkHierarchy('/people/george/', companyNotes(), registry)).toBe(false)
    expect(hasWorkHierarchy('/people/carol/', companyNotes(), registry)).toBe(true)
    expect(buildOrgTree('/nowhere/', companyNotes(), registry)).toBeNull()
  })

  it('picks the first manager and notes the others', () => {
    const notes = buildSiteNotes(
      [
        { path: '/a/', fm: { title: 'A' } },
        { path: '/b/', fm: { title: 'B' } },
        { path: '/c/', fm: { title: 'C' } },
      ],
      [
        ['/c/', 'reports_to', '/a/'],
        ['/c/', 'reports_to', '/b/'],
      ]
    )
    const tree = buildOrgTree('/c/', notes, registry)!
    expect(tree.root.id).toBe('/a/')
    expect(tree.root.children[0]).toMatchObject({ id: '/c/', also: ['/b/'] })
  })

  it('shows a dual report under the second manager too, on that manager’s page', () => {
    const notes = buildSiteNotes(
      [
        { path: '/a/', fm: { title: 'A' } },
        { path: '/b/', fm: { title: 'B' } },
        { path: '/c/', fm: { title: 'C' } },
      ],
      [
        ['/c/', 'reports_to', '/a/'],
        ['/c/', 'reports_to', '/b/'],
      ]
    )
    const tree = buildOrgTree('/b/', notes, registry)!
    expect(outline(tree.root)).toEqual(['/b/ *', '  /c/'])
    // `also` names the manager NOT drawn as the parent here.
    expect(tree.root.children[0].also).toEqual(['/a/'])
  })

  it('terminates on a reporting cycle and places each note once', () => {
    const notes = buildSiteNotes(
      [
        { path: '/a/', fm: { title: 'A' } },
        { path: '/b/', fm: { title: 'B' } },
        { path: '/c/', fm: { title: 'C' } },
      ],
      [
        ['/a/', 'reports_to', '/b/'],
        ['/b/', 'reports_to', '/c/'],
        ['/c/', 'reports_to', '/a/'],
      ]
    )
    for (const focus of ['/a/', '/b/', '/c/']) {
      const tree = buildOrgTree(focus, notes, registry)!
      const ids = outline(tree.root).map((line) => line.trim().replace(' *', ''))
      expect(new Set(ids).size).toBe(ids.length)
      expect(ids.sort()).toEqual(['/a/', '/b/', '/c/'])
    }
  })

  it('caps a huge organization and summarises the rest', () => {
    const people = Array.from({ length: 120 }, (_, i) => ({
      path: `/staff/p${String(i).padStart(3, '0')}/`,
      fm: { title: `Person ${String(i).padStart(3, '0')}` },
    }))
    const notes = buildSiteNotes(
      [{ path: '/org/', fm: { type: 'organization', title: 'Big' } }, ...people],
      people.map((p) => [p.path, 'employer', '/org/'] as [string, string, string])
    )
    const tree = buildOrgTree('/org/', notes, registry, { maxNodes: 20 })!
    const shown = tree.root.children.filter((c) => c.kind !== 'more')
    const more = tree.root.children.filter((c) => c.kind === 'more')
    expect(shown).toHaveLength(19) // the organization itself is one of the 20
    expect(more).toEqual([expect.objectContaining({ moreCount: 101, target: '/org/' })])
    expect(tree.hidden).toBe(101)
  })

  it('limits reports below a person and says how many are hidden', () => {
    const notes = buildSiteNotes(
      [
        { path: '/a/', fm: { title: 'A' } },
        { path: '/b/', fm: { title: 'B' } },
        { path: '/c/', fm: { title: 'C' } },
        { path: '/d/', fm: { title: 'D' } },
        { path: '/e/', fm: { title: 'E' } },
      ],
      [
        ['/b/', 'reports_to', '/a/'],
        ['/c/', 'reports_to', '/b/'],
        ['/d/', 'reports_to', '/c/'],
        ['/e/', 'reports_to', '/c/'],
      ]
    )
    const tree = buildOrgTree('/a/', notes, registry, { reportsDepth: 2 })!
    expect(outline(tree.root)).toEqual(['/a/ *', '  /b/', '    /c/', '      +2 more→/c/'])
  })

  it('caps peers but always keeps the focus', () => {
    const peers = Array.from({ length: 20 }, (_, i) => ({ path: `/p${i}/`, fm: { title: `P${String(i).padStart(2, '0')}` } }))
    const notes = buildSiteNotes(
      [{ path: '/boss/', fm: { title: 'Boss' } }, ...peers],
      peers.map((p) => [p.path, 'reports_to', '/boss/'] as [string, string, string])
    )
    const tree = buildOrgTree('/p19/', notes, registry, { maxPeers: 5 })!
    const children = tree.root.children
    expect(children.filter((c) => c.kind === 'person')).toHaveLength(6)
    expect(children.some((c) => c.isFocus)).toBe(true)
    expect(children[children.length - 1]).toMatchObject({ kind: 'more', moreCount: 14 })
  })
})

/** An organization employing `n` people directly (no reporting lines). */
function flatOrg(n: number) {
  const people = Array.from({ length: n }, (_, i) => ({
    path: `/staff/p${String(i).padStart(4, '0')}/`,
    fm: { title: `Person ${String(i).padStart(4, '0')}` },
  }))
  return buildSiteNotes(
    [{ path: '/org/', fm: { type: 'organization', title: 'Big' } }, ...people],
    people.map((p) => [p.path, 'employer', '/org/'] as [string, string, string])
  )
}

describe('expanding a focus-targeted "+N more" card', () => {
  it('grows the budget by exactly the hidden count, up to one step', () => {
    expect(nextOrgBudget(80, 21)).toBe(101)
    expect(nextOrgBudget(80, 5000)).toBe(80 + ORG_EXPAND_STEP)
    expect(nextOrgBudget(80, 0)).toBe(80)
    expect(nextOrgBudget(80, -3)).toBe(80)
  })

  it('reveals every hidden employee of an organization at the next budget', () => {
    const notes = flatOrg(100)
    const capped = buildOrgTree('/org/', notes, registry)!
    const more = capped.root.children.find((c) => c.kind === 'more')!
    // The repro: 79 cards plus "+21 more" pointing back at the page itself.
    expect(more).toMatchObject({ moreCount: 21, target: '/org/' })

    const grown = buildOrgTree('/org/', notes, registry, { maxNodes: nextOrgBudget(80, more.moreCount!) })!
    expect(grown.root.children.filter((c) => c.kind === 'person')).toHaveLength(100)
    expect(grown.root.children.some((c) => c.kind === 'more')).toBe(false)
    expect(grown.hidden).toBe(0)
  })

  it('reveals a large organization one step at a time', () => {
    const notes = flatOrg(1000)
    const first = buildOrgTree('/org/', notes, registry)!
    const budget = nextOrgBudget(80, first.hidden)
    const next = buildOrgTree('/org/', notes, registry, { maxNodes: budget })!
    expect(next.root.children.filter((c) => c.kind === 'person')).toHaveLength(budget - 1)
    expect(next.root.children[next.root.children.length - 1]).toMatchObject({ kind: 'more', moreCount: 1000 - (budget - 1), target: '/org/' })
  })

  it("reveals a person's direct reports without opening deeper levels", () => {
    // Boss has 100 reports; the first report has two of their own.
    const reports = Array.from({ length: 100 }, (_, i) => ({ path: `/r${String(i).padStart(3, '0')}/`, fm: { title: `R${String(i).padStart(3, '0')}` } }))
    const notes = buildSiteNotes(
      [{ path: '/boss/', fm: { title: 'Boss' } }, ...reports, { path: '/x/', fm: { title: 'X' } }, { path: '/y/', fm: { title: 'Y' } }],
      [
        ...reports.map((p) => [p.path, 'reports_to', '/boss/'] as [string, string, string]),
        ['/x/', 'reports_to', '/r000/'],
        ['/y/', 'reports_to', '/r000/'],
      ]
    )
    const capped = buildOrgTree('/boss/', notes, registry)!
    const more = capped.root.children.find((c) => c.kind === 'more')!
    expect(more).toMatchObject({ moreCount: 21, target: '/boss/' })

    const grown = buildOrgTree('/boss/', notes, registry, { maxNodes: nextOrgBudget(80, more.moreCount!) })!
    expect(grown.root.children.filter((c) => c.kind === 'person')).toHaveLength(100)
    expect(grown.root.children.some((c) => c.kind === 'more')).toBe(false)
    // The extra budget went to the focus's own row; R000's reports stay collapsed.
    const r000 = grown.root.children.find((c) => c.id === '/r000/')!
    expect(r000.children).toEqual([expect.objectContaining({ kind: 'more', moreCount: 2, target: '/r000/' })])
  })

  it('anchors the view on the slot the more card occupied', () => {
    const view = { x: 100, y: 50, w: 800, h: 600 }
    const moreCard = { x: 500, y: 300, w: 184, h: 30 }
    const revealed = { x: 700, y: 311, w: 184, h: 52 }
    // Same top-left corner → the view moves by exactly the slot's displacement.
    expect(anchoredOrgView(view, moreCard, revealed)).toEqual({ x: 300, y: 50, w: 800, h: 600 })
  })

  it('lays out a 2000-person organization quickly', () => {
    const tree = buildOrgTree('/org/', flatOrg(2000), registry, { maxNodes: 2001 })!
    const start = performance.now()
    const layout = computeOrgLayout(tree)
    expect(layout.cards).toHaveLength(2001)
    // Measured ~1 ms; a generous bound that still catches a quadratic regression.
    expect(performance.now() - start).toBeLessThan(250)
  })
})

describe('computeOrgLayout', () => {
  it('places children below their parent without overlapping cards', () => {
    const tree = buildOrgTree('/orgs/acme/', companyNotes(), registry)!
    const layout = computeOrgLayout(tree)
    const byId = new Map(layout.cards.map((c) => [c.id, c]))
    expect(byId.get('/people/ada/')!.y).toBeGreaterThan(byId.get('/orgs/acme/')!.y)
    expect(byId.get('/people/bob/')!.y).toBeGreaterThan(byId.get('/people/ada/')!.y)
    for (const [i, a] of layout.cards.entries()) {
      for (const b of layout.cards.slice(i + 1)) {
        expect(overlaps(a, b), `${a.id} overlaps ${b.id}`).toBe(false)
      }
    }
    for (const card of layout.cards) {
      expect(card.x - card.w / 2).toBeGreaterThanOrEqual(0)
      expect(card.x + card.w / 2).toBeLessThanOrEqual(layout.width)
      expect(card.y + card.h / 2).toBeLessThanOrEqual(layout.height)
    }
  })

  it('draws a labelled box per department and one link per child', () => {
    const tree = buildOrgTree('/orgs/acme/', companyNotes(), registry)!
    const layout = computeOrgLayout(tree)
    expect(layout.groups.map((g) => g.label).sort()).toEqual(['Design', 'Engineering'])
    expect(layout.links.filter((l) => l.kind !== 'secondary')).toHaveLength(layout.cards.length - 1)
    expect(layout.links.filter((l) => l.kind === 'assistant')).toHaveLength(1)
    // Cards sit inside their department's box.
    const eng = layout.groups.find((g) => g.label === 'Engineering')!
    const carol = layout.cards.find((c) => c.id === '/people/carol/')!
    expect(carol.x).toBeGreaterThan(eng.x)
    expect(carol.x).toBeLessThan(eng.x + eng.w)
    expect(carol.y).toBeGreaterThan(eng.y)
  })

  it('stacks a large leaf-only team into a narrow column', () => {
    const team = Array.from({ length: 10 }, (_, i) => ({ path: `/t${i}/`, fm: { title: `T${i}` } }))
    const notes = buildSiteNotes(
      [{ path: '/lead/', fm: { title: 'Lead' } }, ...team],
      team.map((p) => [p.path, 'reports_to', '/lead/'] as [string, string, string])
    )
    const layout = computeOrgLayout(buildOrgTree('/lead/', notes, registry)!)
    expect(layout.width).toBeLessThan(ORG_CARD_W * 2)
    const xs = new Set(layout.cards.filter((c) => c.id !== '/lead/').map((c) => c.x))
    expect(xs.size).toBe(1)
  })

  it('draws a dashed secondary link to a second manager on the chart', () => {
    const notes = buildSiteNotes(
      [
        { path: '/a/', fm: { title: 'A' } },
        { path: '/b/', fm: { title: 'B' } },
        { path: '/c/', fm: { title: 'C' } },
      ],
      [
        ['/b/', 'reports_to', '/a/'],
        ['/c/', 'reports_to', '/a/'],
        ['/c/', 'reports_to', '/b/'],
      ]
    )
    const layout = computeOrgLayout(buildOrgTree('/a/', notes, registry)!)
    expect(layout.links.filter((l) => l.kind === 'secondary')).toHaveLength(1)
  })
})

describe('computeOrgInitialView', () => {
  const canvas = { canvasWidth: 1000, canvasHeight: 600 }
  const scaleOf = (view: { w: number }) => canvas.canvasWidth / view.w

  it('fits a chart to the canvas when titles stay readable', () => {
    const view = computeOrgInitialView({ contentWidth: 1050, contentHeight: 300, ...canvas, focusX: 100, focusY: 50 })
    expect(scaleOf(view)).toBeCloseTo(1000 / 1050, 6)
    expect(view.x).toBe(0)
    // Shallower than the view: centered vertically.
    expect(view.y + view.h / 2).toBeCloseTo(150, 6)
  })

  it('clamps a wide chart to a 12px title and centers on the focus', () => {
    const view = computeOrgInitialView({ contentWidth: 3000, contentHeight: 400, ...canvas, focusX: 1800, focusY: 200 })
    expect(scaleOf(view) * ORG_TITLE_PX).toBeCloseTo(ORG_MIN_TITLE_PX, 6)
    expect(view.x + view.w / 2).toBeCloseTo(1800, 6)
  })

  it('clamps the focus window to the content edges', () => {
    const view = computeOrgInitialView({ contentWidth: 3000, contentHeight: 400, ...canvas, focusX: 40, focusY: 200 })
    expect(view.x).toBe(0)
    const right = computeOrgInitialView({ contentWidth: 3000, contentHeight: 400, ...canvas, focusX: 2990, focusY: 200 })
    expect(right.x + right.w).toBeCloseTo(3000, 6)
  })

  it('does not blow a small chart up past the maximum scale, and centers it', () => {
    const view = computeOrgInitialView({ contentWidth: 250, contentHeight: 150, ...canvas, focusX: 125, focusY: 75 })
    expect(scaleOf(view)).toBeCloseTo(ORG_MAX_INITIAL_SCALE, 6)
    expect(view.x + view.w / 2).toBeCloseTo(125, 6)
    expect(view.y + view.h / 2).toBeCloseTo(75, 6)
  })

  it('fits height too, so a person’s management chain is not cut off', () => {
    const view = computeOrgInitialView({ contentWidth: 700, contentHeight: 620, ...canvas, focusX: 350, focusY: 300 })
    expect(scaleOf(view)).toBeCloseTo(600 / 620, 6)
    expect(view.y).toBeLessThanOrEqual(0)
    expect(view.y + view.h).toBeGreaterThanOrEqual(620)
  })

  it('pans vertically to the focus on a tall chart', () => {
    const view = computeOrgInitialView({ contentWidth: 900, contentHeight: 3000, ...canvas, focusX: 450, focusY: 2000 })
    expect(view.y + view.h / 2).toBeCloseTo(2000, 6)
  })

  it('falls back to fit-all before the canvas has a size', () => {
    expect(
      computeOrgInitialView({ contentWidth: 500, contentHeight: 300, canvasWidth: 0, canvasHeight: 0, focusX: 0, focusY: 0 })
    ).toEqual({ x: 0, y: 0, w: 500, h: 300 })
  })
})
