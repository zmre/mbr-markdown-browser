import { describe, expect, it } from 'vitest'
import { frontmatterLinksOf, linkedValueParts, type FrontmatterLinks } from './frontmatter-links.ts'

const links: FrontmatterLinks = {
  customer: { url: '/customers/x/', title: 'Customer X', text: 'Customer X', raw: '[[Customer X]]', resolved: true },
  contracts: [
    { url: '/c/msa/', title: 'MSA 2026', text: 'MSA', raw: '[[MSA 2026|MSA]]', resolved: true },
    { url: '/c/gone.pdf', title: 'gone.pdf', text: 'gone.pdf', raw: '/c/gone.pdf', resolved: false },
  ],
}

describe('frontmatterLinksOf', () => {
  it('reads the entry, defaulting to none', () => {
    expect(frontmatterLinksOf({ frontmatter_links: links })).toBe(links)
    expect(frontmatterLinksOf({})).toEqual({})
    expect(frontmatterLinksOf(null)).toEqual({})
    expect(frontmatterLinksOf({ frontmatter_links: [] })).toEqual({})
  })
})

describe('linkedValueParts', () => {
  it('links a resolved scalar by its text', () => {
    expect(linkedValueParts('[[Customer X]]', links, 'customer')).toEqual([
      { text: 'Customer X', url: '/customers/x/' },
    ])
  })

  it('links resolved list elements and keeps the rest as authored', () => {
    expect(linkedValueParts(['[[MSA 2026|MSA]]', 'plain', '/c/gone.pdf'], links, 'contracts')).toEqual([
      { text: 'MSA', url: '/c/msa/' },
      { text: 'plain' },
      { text: '/c/gone.pdf' },
    ])
  })

  it('leaves keys without links to the caller', () => {
    expect(linkedValueParts('x', links, 'reference')).toBeNull()
  })
})
