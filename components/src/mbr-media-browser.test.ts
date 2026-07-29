import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import './mbr-media-browser.js'
import type { MbrMediaBrowserElement } from './mbr-media-browser.js'
import type { MediaType, OtherFileInfo, StaticFileKind } from './types.js'

/**
 * Private surface of MbrMediaBrowserElement that these tests drive.
 *
 * Declared explicitly (rather than `as any`) so a rename on the component is a
 * compile error here instead of a silently-passing test.
 */
interface MediaBrowserHandle {
  _allMediaFiles: OtherFileInfo[]
  _isLoading: boolean
  _error: string | null
  _selectedType: MediaType | null
  _availableTypes: MediaType[]
  _textFilter: string
  _sortField: 'created' | 'modified' | 'alpha'
  _sortDirection: 'asc' | 'desc'
  _selectedIndex: number
  _displayLimit: number
  _processMediaFiles(files: OtherFileInfo[]): void
  _matchesType(file: OtherFileInfo): boolean
  _matchesTextFilter(file: OtherFileInfo): boolean
  _compareFiles(a: OtherFileInfo, b: OtherFileInfo): number
  _getFilteredFiles(): OtherFileInfo[]
  _getDisplayedFiles(): OtherFileInfo[]
  _getTypeCount(type: MediaType): number
}

function handle(el: MbrMediaBrowserElement): MediaBrowserHandle {
  return el as unknown as MediaBrowserHandle
}

// ============================================================================
// Fixtures
// ============================================================================

function file(
  urlPath: string,
  kind: StaticFileKind,
  created: number,
  modified = created
): OtherFileInfo {
  return {
    url_path: urlPath,
    metadata: { created, modified, file_size_bytes: 1024, kind },
  }
}

/** 3 videos + 2 PDFs + 1 image, deliberately unsorted by date. */
function library(): OtherFileInfo[] {
  return [
    file('/videos/beta.mp4', { type: 'video' }, 200),
    file('/videos/alpha.mp4', { type: 'video' }, 300),
    file('/videos/gamma.mp4', { type: 'video' }, 100),
    file('/docs/alpha-report.pdf', { type: 'pdf' }, 400),
    file('/docs/zeta.pdf', { type: 'pdf' }, 50),
    file('/img/alpha.png', { type: 'image' }, 500),
  ]
}

/**
 * Independent reference implementation of the filter+sort the component does,
 * used to verify the memoized result is identical to the unmemoized one.
 */
function referenceFilter(
  files: OtherFileInfo[],
  type: MediaType | null,
  text: string,
  field: 'created' | 'modified' | 'alpha',
  direction: 'asc' | 'desc'
): string[] {
  const dir = direction === 'asc' ? 1 : -1
  const needle = text.trim().toLowerCase()
  const titleOf = (f: OtherFileInfo) => f.url_path.split('/').pop() ?? f.url_path
  return files
    .filter((f) => (type === null ? true : f.metadata.kind.type === type))
    .filter((f) => {
      if (!needle) return true
      return (
        titleOf(f).toLowerCase().includes(needle) ||
        f.url_path.toLowerCase().includes(needle)
      )
    })
    .sort((a, b) => {
      if (field === 'alpha') {
        return dir * titleOf(a).toLowerCase().localeCompare(titleOf(b).toLowerCase())
      }
      const av = field === 'created' ? a.metadata.created : a.metadata.modified
      const bv = field === 'created' ? b.metadata.created : b.metadata.modified
      if (av === undefined && bv === undefined) return 0
      if (av === undefined) return 1
      if (bv === undefined) return -1
      return dir * (av - bv)
    })
    .map((f) => f.url_path)
}

async function mount(files: OtherFileInfo[]): Promise<MbrMediaBrowserElement> {
  const el = document.createElement('mbr-media-browser') as MbrMediaBrowserElement
  document.body.appendChild(el)
  const h = handle(el)
  // The shared mediaNav fetch is stubbed globally and never yields other_files,
  // so seed the component directly through its own processing path.
  h._isLoading = false
  h._error = null
  h._processMediaFiles(files)
  await el.updateComplete
  return el
}

function renderedPaths(el: MbrMediaBrowserElement): string[] {
  return Array.from(el.shadowRoot?.querySelectorAll('.media-card .media-path') ?? []).map(
    (n) => n.getAttribute('title') ?? ''
  )
}

describe('MbrMediaBrowserElement', () => {
  let el: MbrMediaBrowserElement

  afterEach(() => {
    el?.remove()
    vi.restoreAllMocks()
  })

  describe('registration', () => {
    it('is defined as a custom element', () => {
      expect(customElements.get('mbr-media-browser')).toBeDefined()
    })
  })

  describe('memoized filtering (perf)', () => {
    it('does not recompute the filtered list when _selectedIndex changes', async () => {
      el = await mount(library())
      const h = handle(el)

      const matchesType = vi.spyOn(h, '_matchesType')
      const compareFiles = vi.spyOn(h, '_compareFiles')

      // _selectedIndex is written on every card @mouseenter and by keyboard nav.
      h._selectedIndex = 1
      await el.updateComplete

      expect(matchesType).not.toHaveBeenCalled()
      expect(compareFiles).not.toHaveBeenCalled()
      // The selection still renders — the state is required for keyboard nav.
      expect(el.shadowRoot?.querySelectorAll('.media-card.selected').length).toBe(1)
    })

    it('recomputes exactly once when the text filter changes', async () => {
      const files = library()
      el = await mount(files)
      const h = handle(el)

      const matchesType = vi.spyOn(h, '_matchesType')
      const compareFiles = vi.spyOn(h, '_compareFiles')

      // Matches all three videos, so the sort comparator runs too.
      h._textFilter = 'a'
      await el.updateComplete

      // Exactly one pass over the library, not one per _getFilteredFiles() call.
      expect(matchesType).toHaveBeenCalledTimes(files.length)
      expect(compareFiles).toHaveBeenCalled()
    })

    it('recomputes when the type filter changes', async () => {
      const files = library()
      el = await mount(files)
      const h = handle(el)

      const matchesType = vi.spyOn(h, '_matchesType')
      h._selectedType = 'pdf'
      await el.updateComplete

      expect(matchesType).toHaveBeenCalledTimes(files.length)
      expect(h._getFilteredFiles().map((f) => f.url_path)).toEqual(
        referenceFilter(files, 'pdf', '', 'created', 'desc')
      )
    })

    it('recomputes when the sort field or direction changes', async () => {
      const files = library()
      el = await mount(files)
      const h = handle(el)

      const compareFiles = vi.spyOn(h, '_compareFiles')
      h._sortField = 'alpha'
      h._sortDirection = 'asc'
      await el.updateComplete

      expect(compareFiles).toHaveBeenCalled()
      expect(h._getFilteredFiles().map((f) => f.url_path)).toEqual(
        referenceFilter(files, 'video', '', 'alpha', 'asc')
      )
    })

    it('recomputes when the underlying media file list changes', async () => {
      el = await mount(library())
      const h = handle(el)
      expect(h._getFilteredFiles().length).toBe(3)

      const extra = [...library(), file('/videos/delta.mp4', { type: 'video' }, 600)]
      h._processMediaFiles(extra)
      await el.updateComplete

      expect(h._getFilteredFiles().map((f) => f.url_path)).toEqual(
        referenceFilter(extra, 'video', '', 'created', 'desc')
      )
    })
  })

  describe('memoized result equals the unmemoized result', () => {
    const combos: Array<{
      type: MediaType
      text: string
      field: 'created' | 'modified' | 'alpha'
      direction: 'asc' | 'desc'
    }> = [
      { type: 'video', text: '', field: 'created', direction: 'desc' },
      { type: 'video', text: 'alpha', field: 'created', direction: 'desc' },
      { type: 'video', text: '', field: 'alpha', direction: 'asc' },
      { type: 'video', text: '', field: 'alpha', direction: 'desc' },
      { type: 'pdf', text: '', field: 'modified', direction: 'desc' },
      { type: 'pdf', text: 'alpha', field: 'alpha', direction: 'asc' },
      { type: 'image', text: 'zzz', field: 'created', direction: 'asc' },
    ]

    for (const combo of combos) {
      it(`matches for type=${combo.type} text="${combo.text}" sort=${combo.field}-${combo.direction}`, async () => {
        const files = library()
        el = await mount(files)
        const h = handle(el)

        h._selectedType = combo.type
        h._textFilter = combo.text
        h._sortField = combo.field
        h._sortDirection = combo.direction
        await el.updateComplete

        const expected = referenceFilter(
          files,
          combo.type,
          combo.text,
          combo.field,
          combo.direction
        )
        expect(h._getFilteredFiles().map((f) => f.url_path)).toEqual(expected)
        expect(renderedPaths(el)).toEqual(expected)
      })
    }
  })

  describe('type counts', () => {
    it('reports counts per type and zero for absent types', async () => {
      el = await mount(library())
      const h = handle(el)

      expect(h._getTypeCount('video')).toBe(3)
      expect(h._getTypeCount('pdf')).toBe(2)
      expect(h._getTypeCount('image')).toBe(1)
      expect(h._getTypeCount('audio')).toBe(0)
    })

    it('renders the counts on the type tabs', async () => {
      el = await mount(library())
      const counts = Array.from(
        el.shadowRoot?.querySelectorAll('.type-tab .type-count') ?? []
      ).map((n) => n.textContent?.trim())
      // Tabs are ordered by MEDIA_TYPE_PRIORITY: video, pdf, audio, image
      expect(counts).toEqual(['3', '2', '1'])
    })

    it('updates the counts when the media file list changes', async () => {
      el = await mount(library())
      const h = handle(el)

      h._processMediaFiles([
        ...library(),
        file('/audio/song.mp3', { type: 'audio' }, 10),
        file('/audio/other.mp3', { type: 'audio' }, 20),
      ])
      await el.updateComplete

      expect(h._getTypeCount('audio')).toBe(2)
      expect(h._getTypeCount('video')).toBe(3)
    })
  })

  describe('pagination', () => {
    it('slices the memoized list to the display limit', async () => {
      const many = Array.from({ length: 10 }, (_, i) =>
        file(`/videos/v${i}.mp4`, { type: 'video' }, i)
      )
      el = await mount(many)
      const h = handle(el)

      h._displayLimit = 4
      await el.updateComplete

      expect(h._getDisplayedFiles().length).toBe(4)
      expect(h._getFilteredFiles().length).toBe(10)
      expect(renderedPaths(el).length).toBe(4)
      expect(el.shadowRoot?.querySelector('.load-more-button')).not.toBeNull()
    })
  })

  describe('text filter input', () => {
    beforeEach(async () => {
      el = await mount(library())
    })

    it('filters the rendered cards from the input event', async () => {
      const input = el.shadowRoot?.querySelector<HTMLInputElement>('#text-filter')
      expect(input).not.toBeNull()
      input!.value = 'alpha'
      input!.dispatchEvent(new Event('input'))
      await el.updateComplete

      expect(renderedPaths(el)).toEqual(['/videos/alpha.mp4'])
    })

    it('shows the empty state when nothing matches', async () => {
      const input = el.shadowRoot?.querySelector<HTMLInputElement>('#text-filter')
      input!.value = 'no-such-file'
      input!.dispatchEvent(new Event('input'))
      await el.updateComplete

      expect(el.shadowRoot?.querySelector('.empty-state')).not.toBeNull()
      expect(renderedPaths(el)).toEqual([])
    })
  })
})
