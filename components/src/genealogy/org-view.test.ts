/**
 * Tests for the Org chart view's "+N more" cards. Layout is covered by
 * `org-layout.test.ts`; what matters here is what activating a card DOES —
 * in particular that one standing for the focus's own reports expands the
 * chart in place instead of "navigating" to the page already on screen.
 */
import { describe, it, expect, vi, afterEach } from 'vitest'
import { buildRegistry, type SiteNote } from '../graph/relationship-graph.js'
import { CONTACT_TYPES, buildSiteNotes } from '../graph/test-fixtures.js'
import type { GenealogyContext } from './chart-registry.js'
import { ORG_EXPAND_STEP, type OrgCard } from './org-layout.js'
import { moreCardText, orgChartType } from './org-view.js'

const registry = buildRegistry(CONTACT_TYPES)

function flatOrg(n: number): Map<string, SiteNote> {
  const people = Array.from({ length: n }, (_, i) => ({
    path: `/staff/p${String(i).padStart(4, '0')}/`,
    fm: { title: `Person ${String(i).padStart(4, '0')}` },
  }))
  return buildSiteNotes(
    [{ path: '/org/', fm: { type: 'organization', title: 'Acme' } }, ...people],
    people.map((p) => [p.path, 'employer', '/org/'] as [string, string, string])
  )
}

function context(focusPath: string, notesByPath: Map<string, SiteNote>): GenealogyContext {
  return {
    graph: { focus: focusPath, nodes: [], edges: [] },
    notesByPath,
    registry,
    focusPath,
    resolveUrl: (p) => p,
    navigate: vi.fn(),
    graphDepth: 2,
    loadGraphChunk: () => Promise.resolve(false),
    fetchPageLinks: () => Promise.resolve(null),
  }
}

function mount(ctx: GenealogyContext) {
  const container = document.createElement('div')
  document.body.appendChild(container)
  const instance = orgChartType.mount(container, ctx)
  return { container, instance }
}

const moreCards = (root: Element) => [...root.querySelectorAll<SVGGElement>('.org-more')]
const personCards = (root: Element) => [...root.querySelectorAll('.org-person')]

afterEach(() => {
  document.body.innerHTML = ''
})

describe('UNIT org chart "+N more" on an organization page', () => {
  it('labels the focus-targeted card as a button that shows more', () => {
    const { container, instance } = mount(context('/org/', flatOrg(100)))
    const [more] = moreCards(container)
    expect(more.getAttribute('role')).toBe('button')
    expect(more.getAttribute('aria-label')).toBe('Show 21 more')
    expect(more.querySelector('title')?.textContent).toBe('Show 21 more')
    instance.destroy()
  })

  it('expands in place on click instead of navigating to the same page', () => {
    const ctx = context('/org/', flatOrg(100))
    const { container, instance } = mount(ctx)
    expect(personCards(container)).toHaveLength(79)

    moreCards(container)[0].dispatchEvent(new MouseEvent('click', { bubbles: true }))

    expect(ctx.navigate).not.toHaveBeenCalled()
    expect(personCards(container)).toHaveLength(100)
    expect(moreCards(container)).toHaveLength(0)
    // Keyboard focus moves to the first card the click revealed.
    expect((document.activeElement as Element | null)?.getAttribute('data-org-id')).toBe('/staff/p0079/')
    // The viewport is rebuilt around the larger content (not the old box).
    const viewBox = container.querySelector('svg')!.getAttribute('viewBox')!.split(' ').map(Number)
    expect(viewBox.every(Number.isFinite)).toBe(true)
    instance.destroy()
  })

  it('expands the same way from the keyboard (Enter and Space)', () => {
    const notes = flatOrg(80 + ORG_EXPAND_STEP + 50)
    const ctx = context('/org/', notes)
    const { container, instance } = mount(ctx)

    // Large: the first activation reveals one step and leaves a fresh card.
    moreCards(container)[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    expect(personCards(container)).toHaveLength(79 + ORG_EXPAND_STEP)
    const [rest] = moreCards(container)
    expect(rest.getAttribute('aria-label')).toBe('Show 51 more')

    rest.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }))
    expect(personCards(container)).toHaveLength(80 + ORG_EXPAND_STEP + 50)
    expect(moreCards(container)).toHaveLength(0)
    expect(ctx.navigate).not.toHaveBeenCalled()
    instance.destroy()
  })
})

describe('UNIT org chart "+N more" pointing elsewhere', () => {
  it('still navigates to the person whose reports it stands for', () => {
    // A → B → C → {D, E}: with two report levels, C's reports collapse to a
    // depth-limit card that targets C, not the focus.
    const notes = buildSiteNotes(
      ['a', 'b', 'c', 'd', 'e'].map((x) => ({ path: `/${x}/`, fm: { title: x.toUpperCase() } })),
      [
        ['/b/', 'reports_to', '/a/'],
        ['/c/', 'reports_to', '/b/'],
        ['/d/', 'reports_to', '/c/'],
        ['/e/', 'reports_to', '/c/'],
      ]
    )
    const ctx = context('/a/', notes)
    const { container, instance } = mount(ctx)
    const [more] = moreCards(container)
    expect(more.getAttribute('role')).toBe('link')
    expect(more.getAttribute('aria-label')).toBe('2 more — open C to see them')

    more.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    expect(ctx.navigate).toHaveBeenCalledWith('/c/')
    instance.destroy()
  })
})

describe('UNIT moreCardText', () => {
  const card = (moreCount: number): OrgCard => ({
    id: '/org/#more-0',
    kind: 'more',
    title: `+${moreCount} more`,
    isFocus: false,
    moreCount,
    target: '/org/',
    x: 0,
    y: 0,
    w: 184,
    h: 30,
  })
  const titleOf = () => 'Acme'

  it('says how many an in-place expansion reveals, stepping when large', () => {
    expect(moreCardText(card(21), true, titleOf)).toBe('Show 21 more')
    expect(moreCardText(card(ORG_EXPAND_STEP), true, titleOf)).toBe(`Show ${ORG_EXPAND_STEP} more`)
    expect(moreCardText(card(ORG_EXPAND_STEP + 1), true, titleOf)).toBe(
      `Show ${ORG_EXPAND_STEP} more (of ${ORG_EXPAND_STEP + 1})`
    )
  })

  it('names the destination when the card navigates', () => {
    expect(moreCardText(card(21), false, titleOf)).toBe('21 more — open Acme to see them')
  })
})
