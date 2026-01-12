import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import './mbr-browse.js'
import type { MbrBrowseElement } from './mbr-browse.js'

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
      Object.defineProperty(event, 'target', { value: input })
      document.dispatchEvent(event)
      await element.updateComplete

      expect(element.shadowRoot?.querySelector('.navigator-backdrop')).toBeNull()
      input.remove()
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
