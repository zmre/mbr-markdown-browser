import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import './mbr-search.js'
import type { MbrSearchElement, SearchRequestBody } from './mbr-search.js'

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

    it('sends folder_scope=current plus the current folder', async () => {
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
