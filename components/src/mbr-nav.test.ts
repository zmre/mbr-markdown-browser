import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import './mbr-nav.js'
import type { MbrNavElement } from './mbr-nav.js'
import { type MarkdownFile, type SortField, sortFiles } from './sorting.js'

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
