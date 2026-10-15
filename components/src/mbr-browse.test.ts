import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import './mbr-browse.js'
import type { MbrBrowseElement } from './mbr-browse.js'
import { buildFolderTree, type MarkdownFile } from './sorting.js'

/**
 * The element reads site data through shared.ts's module-level subscription.
 * Only `subscribeSiteNav` is mocked so each test can inject its own site.json
 * payload; `getCanonicalPath`/`resolveUrl` stay real so the percent-encoding
 * and base-path handling under test is the shipped implementation.
 */
const mocks = vi.hoisted(() => ({
  state: { isLoading: true, data: null as unknown, error: null as string | null },
}))

vi.mock('./shared.ts', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./shared.js')>()
  return {
    ...actual,
    subscribeSiteNav: (cb: (s: unknown) => void) => {
      cb({ ...mocks.state })
      return () => { }
    },
  }
})

describe('MbrBrowseElement', () => {
  let element: MbrBrowseElement

  beforeEach(() => {
    element = document.createElement('mbr-browse') as MbrBrowseElement
    document.body.appendChild(element)
  })

  afterEach(() => {
    element.remove()
  })

  describe('registration', () => {
    it('should be defined as a custom element', () => {
      expect(customElements.get('mbr-browse')).toBeDefined()
    })

    it('should create an instance', () => {
      expect(element).toBeInstanceOf(HTMLElement)
      expect(element.tagName.toLowerCase()).toBe('mbr-browse')
    })
  })

  describe('visibility', () => {
    it('should be closed by default', () => {
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
    })

    it('should open when open() is called', async () => {
      element.open()
      await element.updateComplete
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).not.toBeNull()
    })

    it('should close when close() is called', async () => {
      element.open()
      await element.updateComplete
      element.close()
      await element.updateComplete
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
    })

    it('should toggle visibility', async () => {
      element.toggle()
      await element.updateComplete
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).not.toBeNull()

      element.toggle()
      await element.updateComplete
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
    })
  })

  describe('keyboard navigation', () => {
    it('should open with "-" key', async () => {
      const event = new KeyboardEvent('keydown', { key: '-', bubbles: true })
      document.dispatchEvent(event)
      await element.updateComplete
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).not.toBeNull()
    })

    it('should open with F2 key', async () => {
      const event = new KeyboardEvent('keydown', { key: 'F2', bubbles: true })
      document.dispatchEvent(event)
      await element.updateComplete
      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).not.toBeNull()
    })

    it('should close with Escape key', async () => {
      element.open()
      await element.updateComplete

      const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })
      document.dispatchEvent(event)
      await element.updateComplete

      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
    })

    it('should not open with "-" when in an input field', async () => {
      const input = document.createElement('input')
      document.body.appendChild(input)
      input.focus()

      const event = new KeyboardEvent('keydown', { key: '-', bubbles: true })
      input.dispatchEvent(event)
      await element.updateComplete

      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
      input.remove()
    })

    it('should not open with "-" typed in an input inside a shadow root', async () => {
      // Regression: the event is retargeted to the shadow HOST at the
      // document level, so the guard must use composedPath.
      const host = document.body.appendChild(document.createElement('div'))
      const shadow = host.attachShadow({ mode: 'open' })
      const input = shadow.appendChild(document.createElement('input'))

      const event = new KeyboardEvent('keydown', { key: '-', bubbles: true, composed: true })
      input.dispatchEvent(event)
      await element.updateComplete

      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
      host.remove()
    })
  })

  describe('structure', () => {
    it('should render left pane when open', async () => {
      element.open()
      await element.updateComplete

      const leftPane = element.shadowRoot?.querySelector('.left-pane')
      expect(leftPane).not.toBeNull()
    })

    it('should render pane header with title', async () => {
      element.open()
      await element.updateComplete

      const header = element.shadowRoot?.querySelector('.pane-header h2')
      expect(header?.textContent).toBe('Navigate')
    })

    it('should render close button', async () => {
      element.open()
      await element.updateComplete

      const closeBtn = element.shadowRoot?.querySelector('.close-button')
      expect(closeBtn).not.toBeNull()
    })

    it('should close when backdrop is clicked', async () => {
      element.open()
      await element.updateComplete

      const backdrop = element.shadowRoot?.querySelector('.navigator-backdrop') as HTMLElement
      backdrop?.click()
      await element.updateComplete

      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
    })

    it('should close when close button is clicked', async () => {
      element.open()
      await element.updateComplete

      const closeBtn = element.shadowRoot?.querySelector('.close-button') as HTMLElement
      closeBtn?.click()
      await element.updateComplete

      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
    })
  })

  describe('loading state', () => {
    it('should show loading state initially', async () => {
      element.open()
      await element.updateComplete

      // Component fetches site.json on mount - verify pane content exists
      const paneContent = element.shadowRoot?.querySelector('.pane-content')
      expect(paneContent).not.toBeNull()
    })
  })
})

/**
 * Tests for the folder sorting logic extracted from MbrBrowseElement.
 * These test the pure sorting functions without needing the full component.
 */
describe('Folder Sorting Logic', () => {
  interface FolderNode {
    name: string;
    title?: string;
    path: string;
    children: Map<string, FolderNode>;
    files: any[];
    fileCount: number;
    frontmatter?: Record<string, any> | null;
  }

  interface SortField {
    field: string;
    order: 'asc' | 'desc';
    compare: 'string' | 'numeric';
  }

  // Helper functions extracted from the component for testing
  function getFolderFieldValue(folder: FolderNode, field: string): string | null {
    switch (field) {
      case 'title':
        return folder.title ?? folder.name ?? null;
      case 'filename':
        return folder.name ?? null;
      default:
        if (folder.frontmatter && field in folder.frontmatter) {
          const val = folder.frontmatter[field];
          if (typeof val === 'boolean') {
            return val ? '1' : '0';
          }
          return String(val);
        }
        return null;
    }
  }

  function compareFoldersByField(a: FolderNode, b: FolderNode, config: SortField): number {
    const valA = getFolderFieldValue(a, config.field);
    const valB = getFolderFieldValue(b, config.field);

    if (valA === null && valB === null) return 0;
    if (valA === null) return 1;
    if (valB === null) return -1;

    let cmp: number;
    if (config.compare === 'numeric') {
      const numA = parseFloat(valA) || 0;
      const numB = parseFloat(valB) || 0;
      cmp = numA - numB;
    } else {
      cmp = valA.toLowerCase().localeCompare(valB.toLowerCase());
    }

    return config.order === 'desc' ? -cmp : cmp;
  }

  function sortFolders(folders: FolderNode[], sortConfig: SortField[]): FolderNode[] {
    return [...folders].sort((a, b) => {
      for (const sortField of sortConfig) {
        const cmp = compareFoldersByField(a, b, sortField);
        if (cmp !== 0) return cmp;
      }
      return 0;
    });
  }

  // Test data helpers
  function makeFolder(name: string, title?: string, order?: number): FolderNode {
    const frontmatter: Record<string, any> = {};
    if (title !== undefined) frontmatter.title = title;
    if (order !== undefined) frontmatter.order = order;

    return {
      name,
      title: title ?? name,
      path: `/${name}/`,
      children: new Map(),
      files: [],
      fileCount: 0,
      frontmatter: Object.keys(frontmatter).length > 0 ? frontmatter : null,
    };
  }

  describe('default sorting (by title, ascending)', () => {
    it('should sort folders by title alphabetically', () => {
      const folders = [
        makeFolder('zebra', 'Zebra'),
        makeFolder('apple', 'Apple'),
        makeFolder('mango', 'Mango'),
      ];

      const config: SortField[] = [{ field: 'title', order: 'asc', compare: 'string' }];
      const sorted = sortFolders(folders, config);

      expect(sorted[0].title).toBe('Apple');
      expect(sorted[1].title).toBe('Mango');
      expect(sorted[2].title).toBe('Zebra');
    });

    it('should fall back to folder name when no title', () => {
      const folders = [
        makeFolder('zebra'),
        makeFolder('apple', 'Apple'),
        makeFolder('mango'),
      ];

      const config: SortField[] = [{ field: 'title', order: 'asc', compare: 'string' }];
      const sorted = sortFolders(folders, config);

      expect(sorted[0].title).toBe('Apple');
      expect(sorted[1].name).toBe('mango');
      expect(sorted[2].name).toBe('zebra');
    });
  });

  describe('numeric sorting by order field', () => {
    it('should sort folders by order numerically', () => {
      const folders = [
        makeFolder('customization', 'Customization', 4),
        makeFolder('getting-started', 'Installation', 1),
        makeFolder('markdown', 'Markdown Extensions', 3),
        makeFolder('modes', 'Modes of Operation', 2),
        makeFolder('reference', 'Reference', 5),
      ];

      const config: SortField[] = [{ field: 'order', order: 'asc', compare: 'numeric' }];
      const sorted = sortFolders(folders, config);

      expect(sorted[0].title).toBe('Installation');
      expect(sorted[1].title).toBe('Modes of Operation');
      expect(sorted[2].title).toBe('Markdown Extensions');
      expect(sorted[3].title).toBe('Customization');
      expect(sorted[4].title).toBe('Reference');
    });

    it('should place folders without order after those with order', () => {
      const folders = [
        makeFolder('no-order', 'No Order'),
        makeFolder('first', 'First', 1),
        makeFolder('second', 'Second', 2),
      ];

      const config: SortField[] = [{ field: 'order', order: 'asc', compare: 'numeric' }];
      const sorted = sortFolders(folders, config);

      expect(sorted[0].title).toBe('First');
      expect(sorted[1].title).toBe('Second');
      expect(sorted[2].title).toBe('No Order');
    });
  });

  describe('multi-level sorting', () => {
    it('should use secondary sort for ties', () => {
      const folders = [
        makeFolder('c', 'C', 1),
        makeFolder('a', 'A', 2),
        makeFolder('b', 'B', 1),
        makeFolder('d', 'D', 2),
      ];

      const config: SortField[] = [
        { field: 'order', order: 'asc', compare: 'numeric' },
        { field: 'title', order: 'asc', compare: 'string' },
      ];
      const sorted = sortFolders(folders, config);

      // Order 1: B, C (by title)
      // Order 2: A, D (by title)
      expect(sorted[0].title).toBe('B');
      expect(sorted[1].title).toBe('C');
      expect(sorted[2].title).toBe('A');
      expect(sorted[3].title).toBe('D');
    });

    it('should sort by order then title (docs use case)', () => {
      const folders = [
        makeFolder('customization', 'Customization', 4),
        makeFolder('getting-started', 'Installation', 1),
        makeFolder('integration', 'Integration'),  // No order
        makeFolder('markdown', 'Markdown Extensions', 3),
        makeFolder('modes', 'Modes of Operation', 2),
        makeFolder('reference', 'Reference', 5),
      ];

      const config: SortField[] = [
        { field: 'order', order: 'asc', compare: 'numeric' },
        { field: 'title', order: 'asc', compare: 'string' },
      ];
      const sorted = sortFolders(folders, config);

      // Ordered folders first (1-5), then unordered (Integration)
      expect(sorted[0].title).toBe('Installation');
      expect(sorted[1].title).toBe('Modes of Operation');
      expect(sorted[2].title).toBe('Markdown Extensions');
      expect(sorted[3].title).toBe('Customization');
      expect(sorted[4].title).toBe('Reference');
      expect(sorted[5].title).toBe('Integration');
    });
  });

  describe('descending order', () => {
    it('should sort in reverse order', () => {
      const folders = [
        makeFolder('apple', 'Apple'),
        makeFolder('zebra', 'Zebra'),
        makeFolder('mango', 'Mango'),
      ];

      const config: SortField[] = [{ field: 'title', order: 'desc', compare: 'string' }];
      const sorted = sortFolders(folders, config);

      expect(sorted[0].title).toBe('Zebra');
      expect(sorted[1].title).toBe('Mango');
      expect(sorted[2].title).toBe('Apple');
    });
  });

  describe('case insensitive sorting', () => {
    it('should ignore case in string comparisons', () => {
      const folders = [
        makeFolder('b', 'Banana'),
        makeFolder('a', 'apple'),  // lowercase
        makeFolder('c', 'Cherry'),
      ];

      const config: SortField[] = [{ field: 'title', order: 'asc', compare: 'string' }];
      const sorted = sortFolders(folders, config);

      expect(sorted[0].title).toBe('apple');
      expect(sorted[1].title).toBe('Banana');
      expect(sorted[2].title).toBe('Cherry');
    });
  });
})

/**
 * Behavioural tests that mount the element against injected site data.
 */
describe('MbrBrowseElement with site data', () => {
  let element: MbrBrowseElement | null = null
  const originalConfig = window.__MBR_CONFIG__

  beforeEach(() => {
    localStorage.clear()
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
  })

  afterEach(() => {
    element?.remove()
    element = null
    mocks.state = { isLoading: true, data: null, error: null }
    window.__MBR_CONFIG__ = originalConfig
    vi.unstubAllGlobals()
    localStorage.clear()
  })

  function makeFile(
    urlPath: string,
    rawPath: string,
    frontmatter: Record<string, any> | null = null,
    modified = 1000,
  ): MarkdownFile {
    return { url_path: urlPath, raw_path: rawPath, created: 1000, modified, frontmatter }
  }

  /** Mount an open <mbr-browse> for `pathname` with `files` as the site index. */
  async function mount(files: MarkdownFile[], pathname: string): Promise<MbrBrowseElement> {
    vi.stubGlobal('location', { pathname })
    mocks.state = {
      isLoading: false,
      data: { index_file: 'index.md', markdown_files: files },
      error: null,
    }
    const el = document.createElement('mbr-browse') as MbrBrowseElement
    document.body.appendChild(el)
    element = el
    el.open()
    await el.updateComplete
    return el
  }

  function root(el: MbrBrowseElement): ShadowRoot {
    return el.shadowRoot as ShadowRoot
  }

  function texts(el: MbrBrowseElement, selector: string): string[] {
    return [...root(el).querySelectorAll(selector)].map(n => n.textContent?.trim() ?? '')
  }

  /** Click the left-pane section header whose title matches `title`. */
  async function expandSection(el: MbrBrowseElement, title: string): Promise<void> {
    const header = [...root(el).querySelectorAll('.section-header')].find(
      b => b.querySelector('.section-title')?.textContent?.trim() === title
    ) as HTMLElement | undefined
    expect(header, `section "${title}" not rendered`).toBeDefined()
    header!.click()
    await el.updateComplete
  }

  function tagLabel(el: MbrBrowseElement, name: string): HTMLElement | undefined {
    return [...root(el).querySelectorAll('.tag-item .tree-label')].find(
      b => b.querySelector('.label-text')?.textContent?.trim() === name
    ) as HTMLElement | undefined
  }

  /** Expand the Tags section if needed, then click the tag named `name`. */
  async function selectTag(el: MbrBrowseElement, name: string): Promise<void> {
    if (!tagLabel(el, name)) {
      await expandSection(el, 'Tags')
    }
    const label = tagLabel(el, name)
    expect(label, `tag "${name}" not rendered`).toBeDefined()
    label!.click()
    await el.updateComplete
  }

  describe('canonical paths (percent-encoded pathnames)', () => {
    const idea = () => makeFile('/My Notes/Idea/', 'My Notes/Idea.md', { tags: ['note'] }, 1000)
    const other = () => makeFile('/other/', 'other.md', { tags: ['note'] }, 9000)

    it('stores the decoded canonical path in the recent list', async () => {
      await mount([idea(), other()], '/My%20Notes/Idea/')

      expect(JSON.parse(localStorage.getItem('mbr_recent_files') ?? '[]')).toEqual([
        '/My Notes/Idea/',
      ])
    })

    it('surfaces a visited percent-encoded page in Recent ahead of newer files', async () => {
      const el = await mount([idea(), other()], '/My%20Notes/Idea/')
      await expandSection(el, 'Recent')

      // "other" has the newer mtime, so the visited page can only come first if
      // its stored path resolved against site.json.
      expect(texts(el, '.compact-file .compact-title')).toEqual(['Idea', 'other'])
    })

    it('recovers a previously stored percent-encoded entry', async () => {
      localStorage.setItem('mbr_recent_files', JSON.stringify(['/My%20Notes/Idea/']))

      const el = await mount([idea(), other()], '/')
      await expandSection(el, 'Recent')

      expect(texts(el, '.compact-file .compact-title')).toEqual(['Idea', 'other'])
    })

    it('marks the current page card as current', async () => {
      const el = await mount([idea(), other()], '/My%20Notes/Idea/')
      await selectTag(el, 'note')

      const current = [...root(el).querySelectorAll('.file-card.current')]
      expect(current).toHaveLength(1)
      expect(current[0].querySelector('.file-title')?.textContent?.trim()).toBe('Idea')
    })
  })

  describe('auto-expanding the current folder path', () => {
    const files = (): MarkdownFile[] => [
      makeFile('/docs/', 'docs/index.md', { title: 'Docs' }),
      makeFile('/docs/guide/', 'docs/guide/index.md', { title: 'Guide' }),
      makeFile('/docs/guide/setup/', 'docs/guide/setup.md', { title: 'Setup' }),
    ]

    it('expands folders using the key shape buildFolderTree mints', async () => {
      const el = await mount(files(), '/docs/guide/setup/')

      const tree = buildFolderTree(files(), 'index.md')
      const docs = tree.children.get('docs')!
      const guide = docs.children.get('guide')!
      const expanded = (el as any)._expandedFolders as Set<string>

      expect(expanded.has(docs.path)).toBe(true)
      expect(expanded.has(guide.path)).toBe(true)
    })

    it('renders nested folders expanded down to the current page', async () => {
      const el = await mount(files(), '/docs/guide/setup/')

      // "Guide" only renders when the "/docs/" node is expanded.
      expect(texts(el, '.folder-item .label-text')).toContain('Docs')
      expect(texts(el, '.folder-item .label-text')).toContain('Guide')
    })

    it('selects the containing folder when the page URL is a file', async () => {
      const el = await mount(files(), '/docs/guide/setup/')

      expect(texts(el, '.tree-row.selected .label-text')).toEqual(['Guide'])
    })

    it('selects the folder itself when the page URL is a folder index', async () => {
      const el = await mount(files(), '/docs/guide/')

      expect(texts(el, '.tree-row.selected .label-text')).toEqual(['Guide'])
    })
  })

  describe('middle pane paging', () => {
    function taggedFiles(count: number): MarkdownFile[] {
      return Array.from({ length: count }, (_, i) =>
        makeFile(
          `/notes/n${String(i).padStart(4, '0')}/`,
          `notes/n${String(i).padStart(4, '0')}.md`,
          { tags: ['note'], title: `Note ${String(i).padStart(4, '0')}` },
        )
      )
    }

    it('renders at most one page of cards for a large tag match', async () => {
      const el = await mount(taggedFiles(500), '/')
      await selectTag(el, 'note')

      expect(root(el).querySelectorAll('.file-card')).toHaveLength(100)
      expect(root(el).querySelector('.pane-count')?.textContent?.trim()).toBe('500')
    })

    it('reveals the next page when show more is clicked', async () => {
      const el = await mount(taggedFiles(500), '/')
      await selectTag(el, 'note')

      const showMore = root(el).querySelector('.middle-pane .show-more') as HTMLElement
      expect(showMore).not.toBeNull()
      showMore.click()
      await el.updateComplete

      expect(root(el).querySelectorAll('.file-card')).toHaveLength(200)
      // The count label keeps reporting the true total, not the page size.
      expect(root(el).querySelector('.pane-count')?.textContent?.trim()).toBe('500')
    })

    it('does not page a result set that fits in one page', async () => {
      const el = await mount(taggedFiles(20), '/')
      await selectTag(el, 'note')

      expect(root(el).querySelectorAll('.file-card')).toHaveLength(20)
      expect(root(el).querySelector('.middle-pane .show-more')).toBeNull()
    })

    it('resets paging when a new selection is made', async () => {
      const el = await mount(taggedFiles(500), '/')
      await selectTag(el, 'note')
      ;(root(el).querySelector('.middle-pane .show-more') as HTMLElement).click()
      await el.updateComplete
      expect(root(el).querySelectorAll('.file-card')).toHaveLength(200)

      await selectTag(el, 'note')
      expect(root(el).querySelectorAll('.file-card')).toHaveLength(100)
    })
  })

  describe('subfolders in the middle pane', () => {
    /**
     * /projects/ holds only folders: /projects/alpha/ (two notes plus an index)
     * and /projects/beta/ (one note). /projects/alpha/deep/ is a grandchild.
     */
    const files = (): MarkdownFile[] => [
      makeFile('/top/', 'top.md', { title: 'Top' }),
      makeFile('/projects/alpha/', 'projects/alpha/index.md', { title: 'Alpha' }),
      makeFile('/projects/alpha/one/', 'projects/alpha/one.md', { title: 'One' }),
      makeFile('/projects/alpha/two/', 'projects/alpha/two.md', { title: 'Two' }),
      makeFile('/projects/alpha/deep/x/', 'projects/alpha/deep/x.md', { title: 'X' }),
      makeFile('/projects/beta/b1/', 'projects/beta/b1.md', { title: 'B1' }),
    ]

    function folderLabel(el: MbrBrowseElement, name: string): HTMLElement {
      const label = [...root(el).querySelectorAll('.folder-item .tree-label')].find(
        b => b.querySelector('.label-text')?.textContent?.trim() === name
      ) as HTMLElement | undefined
      expect(label, `folder "${name}" not rendered`).toBeDefined()
      return label!
    }

    async function selectFolder(el: MbrBrowseElement, name: string): Promise<void> {
      folderLabel(el, name).click()
      await el.updateComplete
    }

    function subfolderNames(el: MbrBrowseElement): string[] {
      return texts(el, '.middle-pane .subfolder-row .subfolder-name')
    }

    async function openSubfolder(el: MbrBrowseElement, name: string): Promise<void> {
      const row = [...root(el).querySelectorAll('.middle-pane .subfolder-row')].find(
        b => b.querySelector('.subfolder-name')?.textContent?.trim() === name
      ) as HTMLElement | undefined
      expect(row, `subfolder row "${name}" not rendered`).toBeDefined()
      row!.click()
      await el.updateComplete
    }

    it('lists the subfolders of a folder that holds only folders', async () => {
      const el = await mount(files(), '/')
      await selectFolder(el, 'projects')

      expect(subfolderNames(el)).toEqual(['Alpha', 'beta'])
      expect(texts(el, '.middle-pane .subfolder-count')).toEqual(['4', '1'])
      expect(root(el).querySelectorAll('.middle-pane .file-card')).toHaveLength(0)
      // The old dead-end message is gone, and this is not "empty".
      expect(root(el).querySelector('.middle-pane .no-results')).toBeNull()
      expect(root(el).querySelector('.middle-pane')?.textContent).not.toContain('folder tree')
      // The header count agrees with the tree row: notes below, recursively.
      expect(root(el).querySelector('.pane-count')?.textContent?.trim()).toBe('5')
    })

    it('lists subfolders above the notes when a folder has both', async () => {
      const el = await mount(files(), '/')
      await selectFolder(el, 'projects')
      await openSubfolder(el, 'Alpha')

      expect(subfolderNames(el)).toEqual(['deep'])
      expect(texts(el, '.middle-pane .file-title')).toEqual(['Alpha', 'One', 'Two'])
      expect(texts(el, '.middle-pane .pane-group-label')).toEqual(['Folders', 'Notes'])
      const content = root(el).querySelector('.middle-pane .pane-content')!
      const list = content.querySelector('.subfolder-list')!
      const firstCard = content.querySelector('.file-card')!
      expect(list.compareDocumentPosition(firstCard) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    })

    it('omits the folder group for a folder without subfolders', async () => {
      const el = await mount(files(), '/')
      await selectFolder(el, 'projects')
      await openSubfolder(el, 'beta')

      expect(root(el).querySelector('.middle-pane .subfolder-list')).toBeNull()
      expect(root(el).querySelector('.middle-pane .pane-group-label')).toBeNull()
      expect(texts(el, '.middle-pane .file-title')).toEqual(['B1'])
    })

    it('navigates into a subfolder the way the tree does', async () => {
      const el = await mount(files(), '/')
      await selectFolder(el, 'projects')
      await openSubfolder(el, 'Alpha')

      const selection = (el as any)._currentSelection
      expect(selection).toEqual({ type: 'folder', value: '/projects/alpha/', label: 'Alpha' })
      expect(root(el).querySelector('.pane-header h3')?.textContent?.trim()).toBe('Alpha')
      // The tree is expanded down to the row, which is selected.
      const expanded = (el as any)._expandedFolders as Set<string>
      expect(expanded.has('/projects/')).toBe(true)
      expect(texts(el, '.tree-row.selected .label-text')).toEqual(['Alpha'])

      await openSubfolder(el, 'deep')
      expect(((el as any)._expandedFolders as Set<string>).has('/projects/alpha/')).toBe(true)
      expect(texts(el, '.tree-row.selected .label-text')).toEqual(['deep'])
      expect(texts(el, '.middle-pane .file-title')).toEqual(['X'])
    })

    it('says a folder is empty only when it has neither notes nor subfolders', async () => {
      const el = await mount(files(), '/')
      // No markdown file can yield a truly empty FolderNode from site.json,
      // so select a path with no node and no files.
      ;(el as any)._selectFolder('/nowhere/', 'Nowhere')
      await el.updateComplete
      expect(texts(el, '.middle-pane .no-results')).toEqual(['This folder is empty'])

      await selectFolder(el, 'projects')
      expect(root(el).querySelector('.middle-pane .no-results')).toBeNull()
    })

    it('keeps "No files found" for an empty non-folder selection', async () => {
      const el = await mount(files(), '/')
      ;(el as any)._selectTag('missing', 'missing')
      await el.updateComplete
      expect(texts(el, '.middle-pane .no-results')).toEqual(['No files found'])
    })

    it('pages a large set of subfolders separately from the notes', async () => {
      const many: MarkdownFile[] = [
        makeFile('/big/readme/', 'big/readme.md', { title: 'Readme' }),
        ...Array.from({ length: 250 }, (_, i) => {
          const n = String(i).padStart(4, '0')
          return makeFile(`/big/f${n}/note/`, `big/f${n}/note.md`, { title: `Note ${n}` })
        }),
      ]
      const el = await mount(many, '/')
      await selectFolder(el, 'big')

      expect(root(el).querySelectorAll('.middle-pane .subfolder-row')).toHaveLength(100)
      expect(root(el).querySelectorAll('.middle-pane .file-card')).toHaveLength(1)
      const more = root(el).querySelector('.middle-pane .show-more-folders') as HTMLElement
      expect(more?.textContent).toContain('100 of 250')
      more.click()
      await el.updateComplete
      expect(root(el).querySelectorAll('.middle-pane .subfolder-row')).toHaveLength(200)

      // A new selection starts the folder list back at one page.
      await openSubfolder(el, 'f0000')
      await selectFolder(el, 'big')
      expect(root(el).querySelectorAll('.middle-pane .subfolder-row')).toHaveLength(100)
    })
  })

  describe('frontmatter value counts', () => {
    function statusFiles(): MarkdownFile[] {
      return [
        ...Array.from({ length: 7 }, (_, i) =>
          makeFile(`/d${i}/`, `d${i}.md`, { status: 'draft' })),
        ...Array.from({ length: 5 }, (_, i) =>
          makeFile(`/f${i}/`, `f${i}.md`, { status: 'final' })),
      ]
    }

    function valueRows(el: MbrBrowseElement): Array<[string, string]> {
      return [...root(el).querySelectorAll('.frontmatter-value')].map(b => [
        b.querySelector('.value-name')?.textContent?.trim() ?? '',
        b.querySelector('.value-count')?.textContent?.trim() ?? '',
      ])
    }

    it('shows the counts computed during field detection', async () => {
      const el = await mount(statusFiles(), '/')
      await expandSection(el, 'Status')

      expect(valueRows(el)).toEqual([['draft', '7'], ['final', '5']])
    })

    it('does not rescan the file list while rendering', async () => {
      const el = await mount(statusFiles(), '/')
      await expandSection(el, 'Status')

      // Counts are cached by the detector, so dropping the file list must not
      // change them - render no longer scans _allFiles per value.
      ;(el as any)._allFiles = []
      await el.updateComplete

      expect(valueRows(el)).toEqual([['draft', '7'], ['final', '5']])
    })
  })
})

/**
 * Regression: on a phone, tapping a folder slid in a full-screen file pane
 * fixed at `top: 0`, outside the container's safe-area padding. In the iOS
 * app (viewport-fit=cover, pages laid out under the status and native
 * navigation bars) its header — the only way back — and its first cards sat
 * under the native bar, so a folder with a file or two, or only subfolders
 * ("No files found"), showed a blank screen with nothing to tap.
 */
describe('MbrBrowseElement narrow layout', () => {
  const css = (customElements.get('mbr-browse') as unknown as { styles: { cssText: string } }).styles.cssText
  const narrow = css.slice(css.indexOf('@media (max-width: 768px)'))
  const middlePane = narrow.slice(narrow.indexOf('.middle-pane {'), narrow.indexOf('}', narrow.indexOf('.middle-pane {')))

  it('keeps the full-screen file pane clear of the safe areas', () => {
    expect(middlePane).toContain('position: fixed')
    expect(middlePane).toContain('top: 0')
    expect(middlePane).toContain('box-sizing: border-box')
    expect(middlePane).toContain('padding-top: env(safe-area-inset-top, 0px)')
    expect(middlePane).toContain('padding-bottom: env(safe-area-inset-bottom, 0px)')
    expect(middlePane).toContain('padding-left: env(safe-area-inset-left, 0px)')
    expect(middlePane).toContain('padding-right: env(safe-area-inset-right, 0px)')
  })
})

/**
 * Regression: the current page's card used --pico-primary-background (Pico's
 * solid primary fill, dark blue in the default theme) behind the normal text
 * colours, which left the title and metadata nearly unreadable.
 */
describe('MbrBrowseElement current file card', () => {
  const css = (customElements.get('mbr-browse') as unknown as { styles: { cssText: string } }).styles.cssText
  const start = css.indexOf('.file-card.current {')
  const rule = css.slice(start, css.indexOf('}', start))

  it('tints the card instead of filling it with the primary background', () => {
    expect(start).toBeGreaterThan(-1)
    expect(rule).not.toContain('--pico-primary-background')
    expect(rule).toMatch(/color-mix\(\s*in srgb,\s*var\(--pico-primary[^)]*\)\s*1[0-5]%/)
    expect(rule).toContain('var(--pico-background-color')
    expect(rule).toMatch(/border-left: \d+px solid var\(--pico-primary/)
    expect(rule).toContain('color: var(--pico-color')
  })

  it('lifts the muted text on the tint to AA contrast', () => {
    const muted = css.slice(css.indexOf('.file-card.current .file-meta'))
    const body = muted.slice(0, muted.indexOf('}'))
    expect(body).toMatch(/color-mix\(\s*in srgb,\s*var\(--pico-muted-color[^)]*\)\s*70%,\s*var\(--pico-color/)
  })
})
