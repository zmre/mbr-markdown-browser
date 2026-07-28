import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import './mbr-nav.js'
import type { MbrNavElement } from './mbr-nav.js'
import { type MarkdownFile, type SortField, sortFiles } from './sorting.js'

// shared.ts fetches site.json at module scope, so the fixture has to be in place
// before mbr-nav.js (and transitively shared.ts) is evaluated. vi.hoisted runs
// above the import statements.
vi.hoisted(() => {
  const fixtureFile = (url_path: string, raw_path: string, title: string) => ({
    url_path,
    raw_path,
    created: 1000,
    modified: 2000,
    frontmatter: { title },
  })

  /**
   * site.json fixture used by the whole file.
   *
   * `url_path` values are stored DECODED (literal spaces), exactly as Rust
   * serializes them, and the repo is configured with `_index.md` so the folder
   * landing page must attach to the folder node rather than the root file list.
   *
   * Linear order with the default title sort:
   *   /Walsh/Alice/ → /Walsh/Patrick Joseph Walsh b.1977-10-01/ → /Walsh/ → /Walsh/Zoe/
   */
  const site = {
    index_file: '_index.md',
    markdown_files: [
      fixtureFile('/Walsh/', 'Walsh/_index.md', 'Walsh Family'),
      fixtureFile('/Walsh/Alice/', 'Walsh/Alice.md', 'Alice'),
      fixtureFile(
        '/Walsh/Patrick Joseph Walsh b.1977-10-01/',
        'Walsh/Patrick Joseph Walsh b.1977-10-01.md',
        'Patrick Joseph Walsh',
      ),
      fixtureFile('/Walsh/Zoe/', 'Walsh/Zoe.md', 'Zoe'),
    ],
  }

  globalThis.fetch = (() =>
    Promise.resolve({ ok: true, json: () => Promise.resolve(site) })) as unknown as typeof fetch
})

describe('MbrNavElement', () => {
  let element: MbrNavElement

  beforeEach(() => {
    element = document.createElement('mbr-nav') as MbrNavElement
    document.body.appendChild(element)
  })

  afterEach(() => {
    element.remove()
  })

  describe('registration', () => {
    it('should be defined as a custom element', () => {
      expect(customElements.get('mbr-nav')).toBeDefined()
    })

    it('should create an instance', () => {
      expect(element).toBeInstanceOf(HTMLElement)
      expect(element.tagName.toLowerCase()).toBe('mbr-nav')
    })
  })

  describe('structure', () => {
    it('should render navigation structure', async () => {
      await element.updateComplete
      const nav = element.shadowRoot?.querySelector('nav')
      expect(nav).not.toBeNull()
    })

    it('should render prev and next buttons', async () => {
      await element.updateComplete
      const buttons = element.shadowRoot?.querySelectorAll('.nav-button')
      expect(buttons?.length).toBe(2)
    })

    it('should have disabled buttons by default', async () => {
      await element.updateComplete
      const prevButton = element.shadowRoot?.querySelector('.nav-button.prev')
      const nextButton = element.shadowRoot?.querySelector('.nav-button.next')
      expect(prevButton?.hasAttribute('disabled')).toBe(true)
      expect(nextButton?.hasAttribute('disabled')).toBe(true)
    })
  })
})

describe('MbrNavElement current-page resolution', () => {
  const originalConfig = window.__MBR_CONFIG__

  afterEach(() => {
    vi.unstubAllGlobals()
    window.__MBR_CONFIG__ = originalConfig
    document.querySelectorAll('mbr-nav').forEach(el => el.remove())
  })

  /** Drain the idle-deferred computation and the site.json promise chain. */
  async function mountNav(): Promise<MbrNavElement> {
    const el = document.createElement('mbr-nav') as MbrNavElement
    document.body.appendChild(el)
    for (let i = 0; i < 10; i++) {
      await new Promise(resolve => setTimeout(resolve, 0))
      await el.updateComplete
    }
    return el
  }

  function hrefs(el: MbrNavElement): { prev: string | null; next: string | null } {
    return {
      prev: el.shadowRoot?.querySelector('a.nav-button.prev')?.getAttribute('href') ?? null,
      next: el.shadowRoot?.querySelector('a.nav-button.next')?.getAttribute('href') ?? null,
    }
  }

  it('matches a percent-encoded pathname against the decoded url_path', async () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    vi.stubGlobal('location', { pathname: '/Walsh/Patrick%20Joseph%20Walsh%20b.1977-10-01/' })

    const el = await mountNav()

    // Comparing the raw pathname finds nothing and leaves both buttons disabled.
    expect(hrefs(el).prev).toBe('/Walsh/Alice/')
  })

  it('honors the configured index_file when ordering the sequence', async () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    vi.stubGlobal('location', { pathname: '/Walsh/Patrick%20Joseph%20Walsh%20b.1977-10-01/' })

    const el = await mountNav()

    // '_index.md' belongs to the /Walsh/ folder, so it sorts among that
    // folder's files ("Walsh Family") instead of leading the root list.
    expect(hrefs(el).next).toBe('/Walsh/')
  })

  it('prefixes hrefs with the base path for a static build under a subdirectory', async () => {
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../../' }
    vi.stubGlobal('location', {
      pathname: '/deploy/Walsh/Patrick%20Joseph%20Walsh%20b.1977-10-01/',
    })

    const el = await mountNav()

    expect(hrefs(el)).toEqual({ prev: '../../Walsh/Alice/', next: '../../Walsh/' })
  })

  it('leaves the buttons disabled when the page is not in site.json', async () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    vi.stubGlobal('location', { pathname: '/not/in/the/index/' })

    const el = await mountNav()

    expect(hrefs(el)).toEqual({ prev: null, next: null })
    expect(el.shadowRoot?.querySelector('button.nav-button.prev')?.hasAttribute('disabled')).toBe(true)
  })

  it('defers the computation through requestIdleCallback when it exists', async () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    vi.stubGlobal('location', { pathname: '/Walsh/Patrick%20Joseph%20Walsh%20b.1977-10-01/' })
    const idleCallbacks: Array<() => void> = []
    vi.stubGlobal('requestIdleCallback', vi.fn((cb: () => void) => idleCallbacks.push(cb)))

    const el = document.createElement('mbr-nav') as MbrNavElement
    document.body.appendChild(el)
    await el.updateComplete

    // Nothing computed yet: the work is queued, not run on the paint path.
    expect(idleCallbacks).toHaveLength(1)
    expect(hrefs(el).prev).toBeNull()

    idleCallbacks.forEach(cb => cb())
    for (let i = 0; i < 10; i++) {
      await new Promise(resolve => setTimeout(resolve, 0))
      await el.updateComplete
    }

    expect(hrefs(el).prev).toBe('/Walsh/Alice/')
  })
})

/**
 * Tests for prev/next navigation using the shared sorting module.
 * The core sorting logic is tested in sorting.test.ts.
 */
describe('Prev/Next Navigation Sorting', () => {
  // Test data helpers
  function makeFile(name: string, title?: string, order?: number, pinned?: boolean): MarkdownFile {
    const frontmatter: Record<string, any> = {};
    if (title !== undefined) frontmatter.title = title;
    if (order !== undefined) frontmatter.order = order;
    if (pinned !== undefined) frontmatter.pinned = pinned;

    return {
      url_path: `/docs/${name}/`,
      raw_path: `docs/${name}.md`,
      created: 1000,
      modified: 2000,
      frontmatter: Object.keys(frontmatter).length > 0 ? frontmatter : null,
    };
  }

  describe('default sorting (by title, ascending)', () => {
    it('should sort by title alphabetically', () => {
      const files = [
        makeFile('zebra', 'Zebra'),
        makeFile('apple', 'Apple'),
        makeFile('mango', 'Mango'),
      ];

      const config: SortField[] = [{ field: 'title', order: 'asc', compare: 'string' }];
      const sorted = sortFiles(files, config);

      expect(sorted[0].frontmatter?.title).toBe('Apple');
      expect(sorted[1].frontmatter?.title).toBe('Mango');
      expect(sorted[2].frontmatter?.title).toBe('Zebra');
    });

    it('should fall back to filename when no title', () => {
      const files = [
        makeFile('zebra'),
        makeFile('apple', 'Apple'),
        makeFile('mango'),
      ];

      const config: SortField[] = [{ field: 'title', order: 'asc', compare: 'string' }];
      const sorted = sortFiles(files, config);

      expect(sorted[0].frontmatter?.title).toBe('Apple');
      expect(sorted[1].url_path).toBe('/docs/mango/');
      expect(sorted[2].url_path).toBe('/docs/zebra/');
    });
  });

  describe('descending order', () => {
    it('should sort in reverse order', () => {
      const files = [
        makeFile('apple', 'Apple'),
        makeFile('zebra', 'Zebra'),
        makeFile('mango', 'Mango'),
      ];

      const config: SortField[] = [{ field: 'title', order: 'desc', compare: 'string' }];
      const sorted = sortFiles(files, config);

      expect(sorted[0].frontmatter?.title).toBe('Zebra');
      expect(sorted[1].frontmatter?.title).toBe('Mango');
      expect(sorted[2].frontmatter?.title).toBe('Apple');
    });
  });

  describe('numeric sorting', () => {
    it('should sort by order numerically', () => {
      const files = [
        makeFile('third', 'Third', 3),
        makeFile('first', 'First', 1),
        makeFile('second', 'Second', 2),
      ];

      const config: SortField[] = [{ field: 'order', order: 'asc', compare: 'numeric' }];
      const sorted = sortFiles(files, config);

      expect(sorted[0].frontmatter?.title).toBe('First');
      expect(sorted[1].frontmatter?.title).toBe('Second');
      expect(sorted[2].frontmatter?.title).toBe('Third');
    });
  });

  describe('missing value handling', () => {
    it('should place files without sort field after files with it', () => {
      const files = [
        makeFile('no_order', 'No Order'),
        makeFile('first', 'First', 1),
        makeFile('second', 'Second', 2),
      ];

      const config: SortField[] = [{ field: 'order', order: 'asc', compare: 'numeric' }];
      const sorted = sortFiles(files, config);

      expect(sorted[0].frontmatter?.title).toBe('First');
      expect(sorted[1].frontmatter?.title).toBe('Second');
      expect(sorted[2].frontmatter?.title).toBe('No Order');
    });

    it('should not reverse missing value behavior for descending order', () => {
      const files = [
        makeFile('no_order', 'No Order'),
        makeFile('first', 'First', 1),
        makeFile('second', 'Second', 2),
      ];

      // Descending order - but files without field should STILL come last
      const config: SortField[] = [{ field: 'order', order: 'desc', compare: 'numeric' }];
      const sorted = sortFiles(files, config);

      // With descending, order 2 > order 1, but no_order still comes last
      expect(sorted[0].frontmatter?.title).toBe('Second');
      expect(sorted[1].frontmatter?.title).toBe('First');
      expect(sorted[2].frontmatter?.title).toBe('No Order');
    });
  });

  describe('pinned pattern', () => {
    it('should sort pinned items first with descending order', () => {
      const files = [
        makeFile('normal1', 'Normal 1'),
        makeFile('pinned1', 'Pinned 1', undefined, true),
        makeFile('normal2', 'Normal 2'),
        makeFile('unpinned', 'Unpinned', undefined, false),
      ];

      const config: SortField[] = [
        { field: 'pinned', order: 'desc', compare: 'numeric' },
        { field: 'title', order: 'asc', compare: 'string' },
      ];
      const sorted = sortFiles(files, config);

      // Pinned true (1) first, then false (0), then missing (last)
      expect(sorted[0].frontmatter?.title).toBe('Pinned 1');
      expect(sorted[1].frontmatter?.title).toBe('Unpinned');
      expect(sorted[2].frontmatter?.title).toBe('Normal 1');
      expect(sorted[3].frontmatter?.title).toBe('Normal 2');
    });
  });

  describe('multi-level sorting', () => {
    it('should use secondary sort for ties', () => {
      const files = [
        makeFile('c', 'C', 1),
        makeFile('a', 'A', 2),
        makeFile('b', 'B', 1),
        makeFile('d', 'D', 2),
      ];

      const config: SortField[] = [
        { field: 'order', order: 'asc', compare: 'numeric' },
        { field: 'title', order: 'asc', compare: 'string' },
      ];
      const sorted = sortFiles(files, config);

      // Order 1: B, C (by title)
      // Order 2: A, D (by title)
      expect(sorted[0].frontmatter?.title).toBe('B');
      expect(sorted[1].frontmatter?.title).toBe('C');
      expect(sorted[2].frontmatter?.title).toBe('A');
      expect(sorted[3].frontmatter?.title).toBe('D');
    });
  });

  describe('case insensitive sorting', () => {
    it('should ignore case in string comparisons', () => {
      const files = [
        makeFile('b', 'Banana'),
        makeFile('a', 'apple'),  // lowercase
        makeFile('c', 'Cherry'),
      ];

      const config: SortField[] = [{ field: 'title', order: 'asc', compare: 'string' }];
      const sorted = sortFiles(files, config);

      expect(sorted[0].frontmatter?.title).toBe('apple');
      expect(sorted[1].frontmatter?.title).toBe('Banana');
      expect(sorted[2].frontmatter?.title).toBe('Cherry');
    });
  });

  describe('modified timestamp sorting', () => {
    it('should sort by modified time descending', () => {
      const files = [
        { url_path: '/old/', raw_path: 'old.md', created: 1000, modified: 1000, frontmatter: { title: 'Old' } },
        { url_path: '/new/', raw_path: 'new.md', created: 1000, modified: 3000, frontmatter: { title: 'New' } },
        { url_path: '/mid/', raw_path: 'mid.md', created: 1000, modified: 2000, frontmatter: { title: 'Middle' } },
      ];

      const config: SortField[] = [{ field: 'modified', order: 'desc', compare: 'numeric' }];
      const sorted = sortFiles(files, config);

      expect(sorted[0].frontmatter?.title).toBe('New');
      expect(sorted[1].frontmatter?.title).toBe('Middle');
      expect(sorted[2].frontmatter?.title).toBe('Old');
    });
  });
})

/**
 * NOTE: Global linear navigation tests are now in sorting.test.ts
 *
 * The mbr-nav component uses buildFolderTree and flattenToLinearSequence
 * from the shared sorting module. Comprehensive tests for cross-folder
 * navigation are in the 'buildFolderTree', 'flattenToLinearSequence',
 * and 'Global Linear Navigation' test suites in sorting.test.ts.
 */
