import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import type { PageLinks } from './graph/relationship-graph.js'

const mocks = vi.hoisted(() => ({
  links: null as PageLinks | null,
  fetched: [] as string[],
}))

vi.mock('./graph/links-cache.js', () => ({
  fetchPageLinks: async (path: string) => {
    mocks.fetched.push(path)
    return mocks.links
  },
}))
vi.mock('./shared.js', () => ({ getCanonicalPath: () => '/people/jane/' }))
vi.mock('./dynamic-loader.js', () => ({
  scheduleIdleTask: (task: () => void) => task(),
}))

import { backlinkLabel, countLinkingNotes } from './mbr-contact-backlinks.js'

const inbound = (...froms: string[]): PageLinks =>
  ({ inbound: froms.map((from) => ({ from, text: 'x' })), outbound: [] }) as unknown as PageLinks

/** Lets the idle task's awaited fetch settle. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('UNIT contact backlink helpers', () => {
  it('counts distinct linking pages', () => {
    expect(countLinkingNotes(inbound('/a/', '/b/', '/a/'))).toBe(2)
    expect(countLinkingNotes(null)).toBe(0)
    expect(countLinkingNotes({} as PageLinks)).toBe(0)
  })

  it('matches the server-rendered wording', () => {
    expect(backlinkLabel(1)).toBe('Linked from 1 note')
    expect(backlinkLabel(4)).toBe('Linked from 4 notes')
  })
})

describe('<mbr-contact-backlinks>', () => {
  beforeEach(() => {
    mocks.links = null
    mocks.fetched = []
    document.body.innerHTML = ''
  })
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('fills a placeholder from links.json', async () => {
    mocks.links = inbound('/a/', '/b/')
    document.body.innerHTML = '<mbr-contact-backlinks></mbr-contact-backlinks>'
    await settle()
    const el = document.querySelector('mbr-contact-backlinks')!
    expect(mocks.fetched).toEqual(['/people/jane/'])
    expect(el.getAttribute('count')).toBe('2')
    expect(el.querySelector('button')?.textContent).toBe('Linked from 2 notes')
  })

  it('stays empty when nothing links here or links.json is unavailable', async () => {
    mocks.links = inbound()
    document.body.innerHTML = '<mbr-contact-backlinks></mbr-contact-backlinks>'
    await settle()
    expect(document.querySelector('mbr-contact-backlinks')!.children).toHaveLength(0)

    mocks.links = null
    document.body.innerHTML = '<mbr-contact-backlinks></mbr-contact-backlinks>'
    await settle()
    expect(document.querySelector('mbr-contact-backlinks')!.children).toHaveLength(0)
  })

  it('does not fetch when the server already rendered the count', async () => {
    document.body.innerHTML =
      '<mbr-contact-backlinks count="3"><button type="button">Linked from 3 notes</button></mbr-contact-backlinks>'
    await settle()
    expect(mocks.fetched).toEqual([])
  })

  it('opens the info panel on click', async () => {
    const open = vi.fn()
    const info = document.createElement('mbr-info') as HTMLElement & { open?: () => void }
    info.open = open
    document.body.append(info)
    const chip = document.createElement('mbr-contact-backlinks')
    chip.setAttribute('count', '1')
    chip.innerHTML = '<button type="button">Linked from 1 note</button>'
    document.body.append(chip)

    chip.querySelector('button')!.click()
    expect(open).toHaveBeenCalledOnce()
  })
})
