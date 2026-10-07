import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import './mbr-search.js'
import type { MbrSearchElement, SearchRequestBody } from './mbr-search.js'
import { searchFolderFor, setSearchExtrasImporter } from './mbr-search.js'

// The lazy chunk, imported directly (happy-dom cannot import a runtime URL).
setSearchExtrasImporter(() => import('./search-extras/index.js'))

/**
 * Private surface of MbrSearchElement that these tests drive.
 *
 * Declared explicitly (rather than `as any`) so a rename on the component is a
 * compile error here instead of a silently-passing test.
 */
interface SearchHandle {
  _query: string
  _results: Array<{ url_path: string; title: string | null; snippetHtml: string | null }>
  _totalMatches: number
  _durationMs: number
  _isLoading: boolean
  _isOpen: boolean
  _error: string | null
  _pagefind: unknown
  _scope: string
  _folders: string[]
  _noteTypes: Array<{ type: string; count: number }>
  _folderOverride: string | null
  _extras: unknown
  _selectedIndex: number
  _openSearch(): void
  _closeSearch(): void
  _performPagefindSearch(): Promise<void>
}

function handle(el: MbrSearchElement): SearchHandle {
  return el as unknown as SearchHandle
}

/** Flush microtasks (and any already-due macrotasks) so async handlers settle. */
function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0))
}

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}

// ============================================================================
// fetch stub
// ============================================================================

interface FetchCallInit {
  method: string
  headers: Record<string, string>
  body: string
  signal: AbortSignal
}

const originalFetch = globalThis.fetch
let fetchMock: ReturnType<typeof vi.fn>

function okResponse(body: Record<string, unknown> = {}) {
  return {
    ok: true,
    status: 200,
    json: () =>
      Promise.resolve({
        query: 'q',
        total_matches: 1,
        duration_ms: 3,
        results: [
          {
            url_path: '/docs/guide/',
            title: 'Guide',
            description: null,
            tags: null,
            score: 5,
            snippet: 'a snippet',
            is_content_match: true,
            filetype: 'markdown',
          },
        ],
        ...body,
      }),
  }
}

/** The exact JSON body of the last POST to the search endpoint. */
function lastBody(): SearchRequestBody {
  const calls = fetchMock.mock.calls
  expect(calls.length, 'expected a fetch to the search endpoint').toBeGreaterThan(0)
  const init = calls[calls.length - 1][1] as FetchCallInit
  return JSON.parse(init.body) as SearchRequestBody
}

// ============================================================================
// DOM driving helpers
// ============================================================================

function setConfig(serverMode: boolean) {
  window.__MBR_CONFIG__ = {
    serverMode,
    guiMode: false,
    searchEndpoint: '/.mbr/search',
  }
}

async function mount(): Promise<MbrSearchElement> {
  const el = document.createElement('mbr-search') as MbrSearchElement
  document.body.appendChild(el)
  handle(el)._openSearch()
  await el.updateComplete
  return el
}

function input(el: MbrSearchElement): HTMLInputElement {
  const node = el.shadowRoot?.querySelector<HTMLInputElement>('#search-input')
  expect(node, 'search input should be rendered when open').not.toBeNull()
  return node!
}

/** Type into the search box (sets `_query`; the search itself is debounced). */
function typeQuery(el: MbrSearchElement, q: string) {
  const node = input(el)
  node.value = q
  node.dispatchEvent(new Event('input'))
}

/**
 * Fire a scope `<select>` change, which runs a search immediately (no debounce).
 */
async function selectScope(el: MbrSearchElement, scope: 'all' | 'metadata' | 'content') {
  const select = el.shadowRoot?.querySelector<HTMLSelectElement>('.scope-select')
  expect(select, 'scope select renders only in server mode').not.toBeNull()
  select!.value = scope
  select!.dispatchEvent(new Event('change'))
  await flush()
}

/** Toggle one of the two option checkboxes; each also runs a search. */
async function toggleOption(el: MbrSearchElement, index: 0 | 1, checked: boolean) {
  const boxes = el.shadowRoot?.querySelectorAll<HTMLInputElement>(
    '.search-options input[type="checkbox"]'
  )
  expect(boxes?.length).toBe(2)
  const box = boxes![index]
  box.checked = checked
  box.dispatchEvent(new Event('change'))
  await flush()
}

describe('MbrSearchElement', () => {
  let el: MbrSearchElement

  beforeEach(() => {
    fetchMock = vi.fn().mockResolvedValue(okResponse())
    globalThis.fetch = fetchMock as unknown as typeof fetch
    setConfig(true)
  })

  afterEach(() => {
    el?.remove()
    globalThis.fetch = originalFetch
    delete window.__MBR_CONFIG__
    delete window.frontmatter
    vi.restoreAllMocks()
  })

  describe('registration', () => {
    it('is defined as a custom element', () => {
      expect(customElements.get('mbr-search')).toBeDefined()
    })
  })

  // ==========================================================================
  // Server search request body.
  //
  // Field names and values are cross-checked against the Rust `SearchQuery`
  // struct in src/search.rs:155-184:
  //   q: String, limit: usize (#[serde(default = "default_limit")]),
  //   scope: SearchScope (lowercase: metadata|content|all),
  //   filetype: Option<String> (#[serde(default)]; "markdown"|"md"|"all"),
  //   folder: Option<String> (#[serde(default)]),
  //   folder_scope: FolderScope (#[serde(default)]; lowercase: current|everywhere).
  // The struct has NO deny_unknown_fields, so a misspelled key here would be
  // silently ignored by the server and fall back to the Rust default — which is
  // why these assertions pin the exact key set, not just the values.
  // ==========================================================================
  describe('server search request body', () => {
    beforeEach(async () => {
      el = await mount()
      typeQuery(el, 'needle')
    })

    it('sends exactly q/limit/scope/folder_scope by default', async () => {
      await selectScope(el, 'all')

      expect(fetchMock).toHaveBeenCalledTimes(1)
      const [url, init] = fetchMock.mock.calls[0] as [string, FetchCallInit]
      expect(url).toBe('/.mbr/search')
      expect(init.method).toBe('POST')
      expect(init.headers['Content-Type']).toBe('application/json')

      expect(lastBody()).toEqual({
        q: 'needle',
        limit: 20,
        scope: 'all',
        folder_scope: 'everywhere',
      })
      // No `folder` / `filetype` keys at all when they are not applicable.
      expect(Object.keys(lastBody()).sort()).toEqual([
        'folder_scope',
        'limit',
        'q',
        'scope',
      ])
    })

    it('sends scope=metadata', async () => {
      await selectScope(el, 'metadata')
      expect(lastBody()).toEqual({
        q: 'needle',
        limit: 20,
        scope: 'metadata',
        folder_scope: 'everywhere',
      })
    })

    it('sends scope=content', async () => {
      await selectScope(el, 'content')
      expect(lastBody()).toEqual({
        q: 'needle',
        limit: 20,
        scope: 'content',
        folder_scope: 'everywhere',
      })
    })

    it('sends folder_scope=current plus the folder of a section page', async () => {
      // No `markdown_source`: a directory listing, whose URL IS the folder.
      window.history.pushState({}, '', '/docs/guide/')
      await toggleOption(el, 0, true)

      expect(lastBody()).toEqual({
        q: 'needle',
        limit: 20,
        scope: 'all',
        folder_scope: 'current',
        folder: '/docs/guide/',
      })
    })

    it('scopes a note page to the folder holding the note, not the note itself', async () => {
      // Regression: `/docs/guide/` is guide.md's URL, so scoping to it searched
      // that one note and nothing else.
      window.frontmatter = { markdown_source: 'docs/guide.md' }
      window.history.pushState({}, '', '/docs/guide/')
      await toggleOption(el, 0, true)
      expect(lastBody().folder).toBe('/docs/')
    })

    it('sends the decoded folder for a percent-encoded note URL', async () => {
      window.frontmatter = { markdown_source: 'My Notes/Café plan.md' }
      window.history.pushState({}, '', '/My%20Notes/Caf%C3%A9%20plan/')
      await toggleOption(el, 0, true)
      expect(lastBody().folder).toBe('/My Notes/')
    })

    it('derives the folder from the parent when the path has no trailing slash', async () => {
      window.history.pushState({}, '', '/docs/guide')
      await toggleOption(el, 0, true)
      expect(lastBody().folder).toBe('/docs/')
    })

    it('drops the folder key again when switching back to everywhere', async () => {
      window.history.pushState({}, '', '/docs/guide/')
      await toggleOption(el, 0, true)
      expect(lastBody().folder).toBe('/docs/guide/')

      await toggleOption(el, 0, false)
      expect(lastBody().folder_scope).toBe('everywhere')
      expect('folder' in lastBody()).toBe(false)
    })

    it('sends filetype=all when non-markdown files are included', async () => {
      await toggleOption(el, 1, true)
      expect(lastBody()).toEqual({
        q: 'needle',
        limit: 20,
        scope: 'all',
        folder_scope: 'everywhere',
        filetype: 'all',
      })
    })

    it('omits filetype for the markdown-only default', async () => {
      await toggleOption(el, 1, true)
      await toggleOption(el, 1, false)
      expect('filetype' in lastBody()).toBe(false)
    })

    it('combines scope, folder scope and filetype', async () => {
      window.history.pushState({}, '', '/notes/')
      await selectScope(el, 'content')
      await toggleOption(el, 0, true)
      await toggleOption(el, 1, true)

      expect(lastBody()).toEqual({
        q: 'needle',
        limit: 20,
        scope: 'content',
        folder_scope: 'current',
        folder: '/notes/',
        filetype: 'all',
      })
    })
  })

  describe('server search responses', () => {
    beforeEach(async () => {
      el = await mount()
      typeQuery(el, 'needle')
    })

    it('renders results on success', async () => {
      await selectScope(el, 'all')
      await el.updateComplete

      const h = handle(el)
      expect(h._results.map((r) => r.url_path)).toEqual(['/docs/guide/'])
      expect(h._totalMatches).toBe(1)
      expect(h._durationMs).toBe(3)
      expect(h._isLoading).toBe(false)
      expect(el.shadowRoot?.querySelectorAll('a.result').length).toBe(1)
    })

    it('surfaces a non-ok response as an error and clears results', async () => {
      fetchMock.mockResolvedValue({
        ok: false,
        status: 500,
        json: () => Promise.resolve({}),
      })
      await selectScope(el, 'all')
      await el.updateComplete

      const h = handle(el)
      expect(h._error).toBe('Search failed: 500')
      expect(h._results).toEqual([])
      expect(el.shadowRoot?.querySelector('.error')?.textContent).toContain(
        'Search failed: 500'
      )
    })

    it('prefers the server-supplied error message', async () => {
      fetchMock.mockResolvedValue({
        ok: false,
        status: 400,
        json: () => Promise.resolve({ error: 'query too short' }),
      })
      await selectScope(el, 'all')
      await el.updateComplete

      expect(handle(el)._error).toBe('query too short')
    })

    it('treats an error field on a 200 response as a failure', async () => {
      fetchMock.mockResolvedValue({
        ok: true,
        status: 200,
        json: () => Promise.resolve({ error: 'index unavailable', results: [] }),
      })
      await selectScope(el, 'all')
      await el.updateComplete

      expect(handle(el)._error).toBe('index unavailable')
    })

    it('ignores AbortError without showing an error', async () => {
      const abortErr = new Error('The operation was aborted')
      abortErr.name = 'AbortError'
      fetchMock.mockRejectedValue(abortErr)

      await selectScope(el, 'all')
      await el.updateComplete

      const h = handle(el)
      expect(h._error).toBeNull()
      expect(el.shadowRoot?.querySelector('.error')).toBeNull()
    })

    it('aborts the previous request when a new search starts', async () => {
      fetchMock.mockReturnValue(new Promise(() => {})) // never settles
      await selectScope(el, 'all')
      const firstSignal = (fetchMock.mock.calls[0][1] as FetchCallInit).signal
      expect(firstSignal.aborted).toBe(false)

      await selectScope(el, 'content')
      expect(firstSignal.aborted).toBe(true)
    })

    it('does not let a slow response overwrite a newer one', async () => {
      const slow = deferred<ReturnType<typeof okResponse>>()
      fetchMock.mockReturnValueOnce(slow.promise)
      fetchMock.mockResolvedValue(
        okResponse({
          results: [
            {
              url_path: '/fresh/',
              title: 'Fresh',
              description: null,
              tags: null,
              score: 1,
              snippet: null,
              is_content_match: true,
              filetype: 'markdown',
            },
          ],
        })
      )

      await selectScope(el, 'all') // slow, still pending
      typeQuery(el, 'needle2')
      await selectScope(el, 'all') // fast, resolves now

      expect(handle(el)._results.map((r) => r.url_path)).toEqual(['/fresh/'])

      // The stale response lands last but must not clobber the newer results.
      slow.resolve(okResponse())
      await flush()
      expect(handle(el)._results.map((r) => r.url_path)).toEqual(['/fresh/'])
    })
  })

  // ==========================================================================
  // Selection. `_selectedIndex` is the only thing that may mark a row as
  // selected; these pin the three ways a second / wrong row used to light up:
  // no reset when a new list lands, a `:hover` rule painting the row under a
  // resting pointer, and mouse boundary events (re-dispatched by the browser
  // when rows move under a stationary cursor) writing the selection.
  // ==========================================================================
  describe('result selection', () => {
    function results(prefix: string, n: number) {
      return Array.from({ length: n }, (_, i) => ({
        url_path: `/${prefix}-${i}/`,
        title: `${prefix} ${i}`,
        description: null,
        tags: null,
        score: n - i,
        snippet: null,
        is_content_match: true,
        filetype: 'markdown',
      }))
    }

    function rows(): HTMLAnchorElement[] {
      return Array.from(el.shadowRoot?.querySelectorAll<HTMLAnchorElement>('a.result') ?? [])
    }

    /** Indices of every row rendered as selected. */
    function selectedRows(): number[] {
      return rows().flatMap((a, i) => (a.classList.contains('selected') ? [i] : []))
    }

    async function key(k: string, ctrlKey = false) {
      input(el).dispatchEvent(new KeyboardEvent('keydown', { key: k, ctrlKey, bubbles: true }))
      await el.updateComplete
    }

    async function pointer(row: number, type: string, x: number, y: number) {
      rows()[row].dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, bubbles: true }))
      await el.updateComplete
    }

    beforeEach(async () => {
      fetchMock.mockResolvedValue(okResponse({ results: results('a', 6), total_matches: 6 }))
      el = await mount()
    })

    it('selects the first result when the first search of the page lands', async () => {
      typeQuery(el, 'needle')
      await new Promise((r) => setTimeout(r, 200)) // past the 150 ms debounce
      await el.updateComplete

      expect(rows().length).toBe(6)
      expect(handle(el)._selectedIndex).toBe(0)
      expect(selectedRows()).toEqual([0])
    })

    it('resets to the first row when a new list replaces one the keyboard moved in', async () => {
      typeQuery(el, 'needle')
      await selectScope(el, 'all')
      await key('n', true)
      await key('n', true)
      await key('n', true)
      expect(selectedRows()).toEqual([3])

      // New query: Ctrl-N on the still-visible old list while it is in flight.
      fetchMock.mockResolvedValue(okResponse({ results: results('b', 6), total_matches: 6 }))
      typeQuery(el, 'needle2')
      await key('n', true)
      await key('n', true)
      await selectScope(el, 'all')
      await el.updateComplete

      expect(rows()[0].getAttribute('href')).toContain('/b-0/')
      expect(handle(el)._selectedIndex).toBe(0)
      expect(selectedRows()).toEqual([0])
    })

    it('resets to the first row when the scope changes the list', async () => {
      typeQuery(el, 'needle')
      await selectScope(el, 'all')
      await key('ArrowDown')
      await key('ArrowDown')
      expect(selectedRows()).toEqual([2])

      fetchMock.mockResolvedValue(okResponse({ results: results('c', 4), total_matches: 4 }))
      await selectScope(el, 'content')
      await el.updateComplete
      expect(selectedRows()).toEqual([0])
    })

    it('does not let a row under a resting pointer take the selection', async () => {
      typeQuery(el, 'needle')
      await selectScope(el, 'all')

      // What the browser sends when results render under a stationary cursor.
      await pointer(3, 'mouseover', 40, 200)
      await pointer(3, 'mouseenter', 40, 200)
      await pointer(3, 'mousemove', 40, 200)
      await pointer(3, 'mousemove', 40, 200)
      expect(selectedRows()).toEqual([0])

      await key('n', true)
      expect(selectedRows()).toEqual([1])
    })

    it('never styles a row as highlighted by :hover', () => {
      const ctor = customElements.get('mbr-search') as unknown as {
        styles: { cssText: string } | Array<{ cssText: string }>
      }
      const css = [ctor.styles].flat().map((s) => s.cssText).join('\n')
      expect(css).toContain('.result.selected')
      expect(css).not.toMatch(/\.result:hover/)
    })

    it('keeps keyboard and mouse on one selection', async () => {
      typeQuery(el, 'needle')
      await selectScope(el, 'all')

      // A real move onto row 3 selects it, and only it.
      await pointer(2, 'mousemove', 40, 150)
      await pointer(3, 'mousemove', 40, 200)
      expect(selectedRows()).toEqual([3])

      // The keyboard continues from where the mouse left the selection.
      await key('n', true)
      expect(selectedRows()).toEqual([4])

      // Ctrl-N scrolled the list, so another row is now under the unmoved
      // pointer and the browser re-dispatches a move there: ignored.
      await pointer(1, 'mousemove', 40, 200)
      expect(selectedRows()).toEqual([4])

      // Enter opens the row the index names.
      const clicks = vi.spyOn(rows()[4], 'click').mockImplementation(() => {})
      await key('Enter')
      expect(clicks).toHaveBeenCalledTimes(1)
    })

    it('a modal reopened under a pointer that moved while closed does not select by it', async () => {
      typeQuery(el, 'needle')
      await selectScope(el, 'all')
      await pointer(2, 'mousemove', 40, 150)
      await pointer(3, 'mousemove', 40, 200)
      expect(selectedRows()).toEqual([3])

      handle(el)._closeSearch()
      handle(el)._openSearch()
      await el.updateComplete
      typeQuery(el, 'needle')
      await selectScope(el, 'all')

      // First event of the new session at a new position is only a baseline.
      await pointer(5, 'mousemove', 40, 300)
      expect(selectedRows()).toEqual([0])
    })

    it('keeps the selection when a stale response lands after a newer list', async () => {
      const slow = deferred<ReturnType<typeof okResponse>>()
      fetchMock.mockReturnValueOnce(slow.promise)
      typeQuery(el, 'needle')
      await selectScope(el, 'all') // slow, still pending

      fetchMock.mockResolvedValue(okResponse({ results: results('fresh', 4), total_matches: 4 }))
      typeQuery(el, 'needle2')
      await selectScope(el, 'all')
      await key('n', true)
      await key('n', true)
      expect(selectedRows()).toEqual([2])

      slow.resolve(okResponse({ results: results('stale', 6), total_matches: 6 }))
      await flush()
      await el.updateComplete
      expect(rows().map((a) => a.getAttribute('href'))).toEqual(
        results('fresh', 4).map((r) => r.url_path)
      )
      expect(selectedRows()).toEqual([2])
    })
  })

  // ==========================================================================
  // Pagefind (static mode). `_performPagefindSearch` is invoked directly so the
  // interleaving of two overlapping searches is deterministic; the element is
  // kept in server mode so connectedCallback does not attempt the real
  // `pagefind.js` dynamic import.
  // ==========================================================================
  describe('pagefind search (static mode)', () => {
    interface PagefindStub {
      init: () => Promise<void>
      options: () => Promise<void>
      search: (query: string) => Promise<{
        results: Array<{ id: string; data: () => Promise<unknown> }>
      }>
    }

    function pagefindResults(urls: string[]) {
      return urls.map((url, i) => ({
        id: String(i),
        data: () =>
          Promise.resolve({
            url,
            excerpt: `<mark>${url}</mark>`,
            meta: { title: url },
          }),
      }))
    }

    function installPagefind(
      el: MbrSearchElement,
      search: PagefindStub['search']
    ): void {
      handle(el)._pagefind = {
        init: () => Promise.resolve(),
        options: () => Promise.resolve(),
        search,
      } satisfies PagefindStub
    }

    beforeEach(async () => {
      el = await mount()
    })

    it('maps pagefind results into the unified result shape', async () => {
      installPagefind(el, () => Promise.resolve({ results: pagefindResults(['/a/']) }))
      const h = handle(el)
      h._query = 'abc'
      await h._performPagefindSearch()

      expect(h._results).toHaveLength(1)
      expect(h._results[0].url_path).toBe('/a/')
      expect(h._results[0].title).toBe('/a/')
      expect(h._results[0].snippetHtml).toBe('<mark>/a/</mark>')
      expect(h._totalMatches).toBe(1)
      expect(h._isLoading).toBe(false)
    })

    it('reports a missing search index', async () => {
      const h = handle(el)
      h._pagefind = null
      h._query = 'abc'
      await h._performPagefindSearch()

      expect(h._error).toContain('Search index not available')
      expect(h._results).toEqual([])
    })

    it('leaves the newer query’s results in place when a slow search lands last', async () => {
      const slow = deferred<{ results: Array<{ id: string; data: () => Promise<unknown> }> }>()
      installPagefind(el, (query) =>
        query === 'ab'
          ? slow.promise
          : Promise.resolve({ results: pagefindResults(['/fresh/']) })
      )

      const h = handle(el)
      h._query = 'ab'
      const stale = h._performPagefindSearch()
      // Let the first run get all the way into pagefind.search() before the
      // second one starts, so it is the post-await guards under test.
      await flush()

      h._query = 'abc'
      await h._performPagefindSearch()
      expect(h._results.map((r) => r.url_path)).toEqual(['/fresh/'])

      // The first search finally resolves — its results are for "ab" and must
      // not replace the "abc" results the user can see.
      slow.resolve({ results: pagefindResults(['/stale-1/', '/stale-2/']) })
      await stale
      expect(h._results.map((r) => r.url_path)).toEqual(['/fresh/'])
      expect(h._totalMatches).toBe(1)
      expect(h._query).toBe('abc')
    })

    it('leaves results empty when the popup is closed mid-flight', async () => {
      const slow = deferred<{ results: Array<{ id: string; data: () => Promise<unknown> }> }>()
      installPagefind(el, () => slow.promise)

      const h = handle(el)
      h._query = 'abc'
      const inflight = h._performPagefindSearch()
      await flush() // parked inside pagefind.search()

      h._closeSearch()
      slow.resolve({ results: pagefindResults(['/late/']) })
      await inflight

      expect(h._results).toEqual([])
      expect(h._totalMatches).toBe(0)
      expect(h._isLoading).toBe(false)
      expect(h._isOpen).toBe(false)
    })

    it('does not surface an error from a superseded search', async () => {
      const slow = deferred<{ results: Array<{ id: string; data: () => Promise<unknown> }> }>()
      let rejectSlow: (err: Error) => void = () => {}
      const failing = new Promise<{
        results: Array<{ id: string; data: () => Promise<unknown> }>
      }>((_, reject) => {
        rejectSlow = reject
      })
      installPagefind(el, (query) => (query === 'ab' ? failing : slow.promise))

      const h = handle(el)
      h._query = 'ab'
      const stale = h._performPagefindSearch()
      await flush() // parked inside pagefind.search('ab')

      h._query = 'abc'
      const fresh = h._performPagefindSearch()

      rejectSlow(new Error('index read failed'))
      await stale
      expect(h._error).toBeNull()

      slow.resolve({ results: pagefindResults(['/fresh/']) })
      await fresh
      expect(h._results.map((r) => r.url_path)).toEqual(['/fresh/'])
    })
  })
})

describe('searchFolderFor', () => {
  it('treats a page without a markdown source as a folder', () => {
    expect(searchFolderFor('/people/')).toBe('/people/')
    expect(searchFolderFor('/')).toBe('/')
  })

  it('uses the parent folder of a note', () => {
    expect(searchFolderFor('/people/john/', 'people/john.md')).toBe('/people/')
    expect(searchFolderFor('/people/john', 'people/john.md')).toBe('/people/')
  })

  it('uses the root for a top-level note', () => {
    expect(searchFolderFor('/README/', 'README.md')).toBe('/')
  })

  it('keeps an index note on its own folder', () => {
    expect(searchFolderFor('/people/', 'people/index.md')).toBe('/people/')
    expect(searchFolderFor('/', 'index.md')).toBe('/')
  })

  it('percent-decodes spaces and unicode', () => {
    expect(searchFolderFor('/My%20Notes/Caf%C3%A9/', 'My Notes/Café.md')).toBe('/My Notes/')
    expect(searchFolderFor('/%C3%9Cber%20uns/', 'Über uns/index.md')).toBe('/Über uns/')
    expect(searchFolderFor('/Caf%C3%A9%20Folder/')).toBe('/Café Folder/')
  })

  it('keeps the raw path when an escape is malformed', () => {
    expect(searchFolderFor('/100%/')).toBe('/100%/')
  })
})

describe('MbrSearchElement note types and folder picker', () => {
  let el: MbrSearchElement

  beforeEach(async () => {
    fetchMock = vi.fn().mockResolvedValue(okResponse())
    globalThis.fetch = fetchMock as unknown as typeof fetch
    setConfig(true)
    el = await mount()
    // The chunk import resolves on the modal open above; give it a moment.
    await vi.waitFor(() => expect(customElements.get('mbr-folder-picker')).toBeDefined())
    await flush()
    const h = handle(el)
    h._noteTypes = [
      { type: 'Meeting Notes', count: 3 },
      { type: 'person', count: 12 },
    ]
    h._folders = ['/notes/', '/people/', '/people/staff/']
    // Normally set when site.json arrives; the stub site.json is empty.
    h._extras = await import('./search-extras/index.js')
    await el.updateComplete
  })

  afterEach(() => {
    el?.remove()
    globalThis.fetch = originalFetch
    delete window.__MBR_CONFIG__
    vi.restoreAllMocks()
  })

  function scopeSelect(): HTMLSelectElement {
    return el.shadowRoot!.querySelector<HTMLSelectElement>('.scope-select')!
  }

  it('lists note types after a disabled separator', () => {
    const options = [...scopeSelect().options]
    expect(options.map((o) => o.textContent?.trim())).toEqual([
      'All',
      'Titles & Tags',
      'Content',
      '── Note types ──',
      'Meeting Notes (3)',
      'person (12)',
    ])
    expect(options[3].disabled).toBe(true)
  })

  it('choosing a type writes a facet token and keeps showing the scope', async () => {
    typeQuery(el, 'jane')
    const select = scopeSelect()
    select.value = 'content'
    select.dispatchEvent(new Event('change'))
    await flush()

    select.value = 'type:person'
    select.dispatchEvent(new Event('change'))
    await flush()
    await el.updateComplete

    expect(handle(el)._query).toBe('jane type:person')
    expect(input(el).value).toBe('jane type:person')
    // Content scope never applies a facet to a facet-only query → widened.
    expect(handle(el)._scope).toBe('all')
    expect(scopeSelect().value).toBe('all')
    expect(lastBody()).toMatchObject({ q: 'jane type:person', scope: 'all' })
  })

  it('quotes a type with spaces and replaces a previous type', async () => {
    const select = scopeSelect()
    select.value = 'type:person'
    select.dispatchEvent(new Event('change'))
    await flush()
    select.value = 'type:Meeting Notes'
    select.dispatchEvent(new Event('change'))
    await flush()
    expect(lastBody().q).toBe('type:"Meeting Notes"')
  })

  function picker(): HTMLElementTagNameMap['mbr-folder-picker'] {
    const node = el.shadowRoot!.querySelector('mbr-folder-picker')
    expect(node, 'the chunk element is rendered next to the checkbox').not.toBeNull()
    return node!
  }

  async function openPicker(): Promise<HTMLInputElement> {
    picker().shadowRoot!.querySelector<HTMLButtonElement>('button')!.click()
    await picker().updateComplete
    const filter = picker().shadowRoot!.querySelector<HTMLInputElement>('input')
    expect(filter).not.toBeNull()
    return filter!
  }

  function key(target: HTMLElement, k: string) {
    target.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, composed: true }))
  }

  it('filters folders and chooses one with the keyboard', async () => {
    typeQuery(el, 'needle')
    const filter = await openPicker()
    filter.value = 'peo'
    filter.dispatchEvent(new Event('input'))
    await picker().updateComplete
    const rows = picker().shadowRoot!.querySelectorAll('.option')
    expect([...rows].map((r) => r.textContent?.trim())).toEqual(['/people/', '/people/staff/'])

    key(filter, 'ArrowDown')
    await picker().updateComplete
    key(filter, 'Enter')
    await flush()
    await el.updateComplete

    expect(picker().isOpen).toBe(false)
    expect(lastBody()).toMatchObject({ folder_scope: 'current', folder: '/people/staff/' })
    const label = el.shadowRoot!.querySelector('.folder-name')
    expect(label?.textContent).toBe('/people/staff/')
    expect(label?.getAttribute('title')).toBe('/people/staff/')
    const box = el.shadowRoot!.querySelector<HTMLInputElement>('.search-options input[type="checkbox"]')!
    expect(box.checked).toBe(true)
  })

  it('unchecking the scope clears a picked folder', async () => {
    typeQuery(el, 'needle')
    await openPicker()
    picker().shadowRoot!.querySelectorAll<HTMLElement>('.option')[0].click()
    await flush()
    expect(lastBody().folder).toBe('/notes/')

    await toggleOption(el, 0, false)
    expect(handle(el)._folderOverride).toBeNull()
    expect('folder' in lastBody()).toBe(false)
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('.folder-name')).toBeNull()
  })

  it('Escape in the filter closes only the picker', async () => {
    const filter = await openPicker()
    key(filter, 'Escape')
    await picker().updateComplete
    expect(picker().isOpen).toBe(false)
    expect(handle(el)._isOpen).toBe(true)
  })

  it('a click elsewhere in the modal closes the picker', async () => {
    await openPicker()
    el.shadowRoot!.querySelector<HTMLElement>('.results-container')!.click()
    await picker().updateComplete
    expect(picker().isOpen).toBe(false)
  })
})
