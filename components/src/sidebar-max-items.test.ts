import { describe, expect, it } from 'vitest'
import { DEFAULT_SIDEBAR_MAX_ITEMS, sidebarMaxItems } from './sidebar-max-items.ts'

describe('sidebarMaxItems', () => {
  it('reads a positive integer', () => {
    expect(sidebarMaxItems('25')).toBe(25)
    expect(sidebarMaxItems('10000')).toBe(10000)
  })

  it.each([null, '', '0', '-3', 'many'])('falls back to the default for %j', (attr) => {
    expect(sidebarMaxItems(attr)).toBe(DEFAULT_SIDEBAR_MAX_ITEMS)
  })

  it('matches the config default', () => {
    expect(DEFAULT_SIDEBAR_MAX_ITEMS).toBe(100)
  })
})
