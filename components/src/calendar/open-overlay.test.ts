import { afterEach, describe, expect, it, vi } from 'vitest'
import { buildFolderTree, type MarkdownFile } from '../sorting.ts'
import { calendarOverlayAvailable, folderNodeFiles, openCalendarOverlay, searchScope } from './open-overlay.ts'

const file = (raw: string, date: string, extra: Partial<MarkdownFile> = {}): MarkdownFile => ({
  url_path: `/${raw.replace(/\.md$/, '')}/`,
  raw_path: raw,
  created: 0,
  modified: 0,
  frontmatter: null,
  calendar: { date, source: 'filename' },
  ...extra,
})

const FILES = [
  file('a/one.md', '2026-10-01'),
  file('a/b/two.md', '2026-10-02'),
  file('a b/three.md', '2025-01-01'),
  file('c/four.md', '2026-10-31'),
]

afterEach(() => {
  document.body.innerHTML = ''
})

describe('searchScope', () => {
  it('rebuilds a date-only query in full from site.json', () => {
    const scope = searchScope({ query: 'when:2026-10', results: [] }, FILES, '2026-11-07')
    expect(scope.label).toBe('Search: when:2026-10')
    expect(scope.plotBy).toBe('date')
    expect(scope.files!.map((f) => f.raw_path).sort()).toEqual(['a/b/two.md', 'a/one.md', 'c/four.md'])
  })

  it('takes the results on screen otherwise, matched by url_path (encoded or not)', () => {
    const scope = searchScope(
      { query: 'three type:x', results: [{ url_path: '/a%20b/three/' }, { url_path: '/c/four/' }, { url_path: '/x.pdf' }] },
      FILES,
      '2026-11-07'
    )
    expect(scope.files!.map((f) => f.raw_path)).toEqual(['a b/three.md', 'c/four.md'])
  })
})

describe('folderNodeFiles', () => {
  it('collects a folder with every subfolder', () => {
    const tree = buildFolderTree(FILES, 'index.md')
    const a = tree.children.get('a')!
    expect(folderNodeFiles(a).map((f) => f.raw_path).sort()).toEqual(['a/b/two.md', 'a/one.md'])
  })
})

describe('openCalendarOverlay', () => {
  it('goes through <mbr-calendar>, and reports when there is none', () => {
    expect(calendarOverlayAvailable()).toBe(false)
    expect(openCalendarOverlay({ label: 'x' })).toBe(false)
    const openOverlay = vi.fn(() => true)
    // An un-upgraded stand-in with the two members the helpers use.
    const fake = document.createElement('mbr-calendar')
    Object.defineProperty(fake, 'overlayAvailable', { value: true })
    Object.defineProperty(fake, 'openOverlay', { value: openOverlay })
    document.body.append(fake)
    expect(calendarOverlayAvailable()).toBe(true)
    expect(openCalendarOverlay({ label: 'Folder: a' })).toBe(true)
    expect(openOverlay).toHaveBeenCalledWith({ label: 'Folder: a' })
  })
})
