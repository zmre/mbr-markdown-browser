import { describe, it, expect } from 'vitest'
import {
  type MarkdownFile,
  type SortField,
  type FolderNode,
  DEFAULT_SORT_CONFIG,
  getFileName,
  getFileFieldValue,
  getFolderFieldValue,
  compareValues,
  sortFiles,
  sortFolders,
  buildFolderTree,
  flattenToLinearSequence,
} from './sorting.js'

// Test data helpers
function makeFile(name: string, title?: string, order?: number, pinned?: boolean, modified?: number): MarkdownFile {
  const frontmatter: Record<string, any> = {};
  if (title !== undefined) frontmatter.title = title;
  if (order !== undefined) frontmatter.order = order;
  if (pinned !== undefined) frontmatter.pinned = pinned;

  return {
    url_path: `/docs/${name}/`,
    raw_path: `docs/${name}.md`,
    created: 1000,
    modified: modified ?? 2000,
    frontmatter: Object.keys(frontmatter).length > 0 ? frontmatter : null,
  };
}

function makeFolder(name: string, title?: string, order?: number): FolderNode {
  const frontmatter: Record<string, any> = {};
  if (title !== undefined) frontmatter.title = title;
  if (order !== undefined) frontmatter.order = order;

  return {
    name,
    title,
    path: `/${name}/`,
    children: new Map(),
    files: [],
    fileCount: 0,
    frontmatter: Object.keys(frontmatter).length > 0 ? frontmatter : null,
  };
}

describe('Shared Sorting Module', () => {
  describe('DEFAULT_SORT_CONFIG', () => {
    it('should default to title ascending', () => {
      expect(DEFAULT_SORT_CONFIG).toEqual([
        { field: 'title', order: 'asc', compare: 'string' }
      ]);
    });
  });

  describe('getFileName', () => {
    it('should extract filename from URL path with trailing slash', () => {
      expect(getFileName('/docs/guide/intro/')).toBe('intro');
    });

    it('should extract filename from URL path without trailing slash', () => {
      expect(getFileName('/docs/guide/intro')).toBe('intro');
    });

    it('should handle root paths', () => {
      expect(getFileName('/README/')).toBe('README');
    });
  });

  describe('getFileFieldValue', () => {
    it('should return frontmatter title', () => {
      const file = makeFile('test', 'My Title');
      expect(getFileFieldValue(file, 'title')).toBe('My Title');
    });

    it('should fallback to filename when no title', () => {
      const file = makeFile('test');
      expect(getFileFieldValue(file, 'title')).toBe('test');
    });

    it('should return filename', () => {
      const file = makeFile('myfile', 'Title');
      expect(getFileFieldValue(file, 'filename')).toBe('myfile');
    });

    it('should return created timestamp as string', () => {
      const file = makeFile('test');
      expect(getFileFieldValue(file, 'created')).toBe('1000');
    });

    it('should return modified timestamp as string', () => {
      const file = makeFile('test', undefined, undefined, undefined, 5000);
      expect(getFileFieldValue(file, 'modified')).toBe('5000');
    });

    it('should return custom frontmatter field', () => {
      const file = makeFile('test', undefined, 5);
      expect(getFileFieldValue(file, 'order')).toBe('5');
    });

    it('should convert boolean true to 1', () => {
      const file = makeFile('test', undefined, undefined, true);
      expect(getFileFieldValue(file, 'pinned')).toBe('1');
    });

    it('should convert boolean false to 0', () => {
      const file = makeFile('test', undefined, undefined, false);
      expect(getFileFieldValue(file, 'pinned')).toBe('0');
    });

    it('should return null for missing field', () => {
      const file = makeFile('test');
      expect(getFileFieldValue(file, 'order')).toBeNull();
    });
  });

  describe('getFolderFieldValue', () => {
    it('should return folder title', () => {
      const folder = makeFolder('test', 'Folder Title');
      expect(getFolderFieldValue(folder, 'title')).toBe('Folder Title');
    });

    it('should fallback to name when no title', () => {
      const folder = makeFolder('myname');
      expect(getFolderFieldValue(folder, 'title')).toBe('myname');
    });

    it('should return folder name for filename field', () => {
      const folder = makeFolder('myname', 'Title');
      expect(getFolderFieldValue(folder, 'filename')).toBe('myname');
    });

    it('should return custom frontmatter field', () => {
      const folder = makeFolder('test', 'Title', 3);
      expect(getFolderFieldValue(folder, 'order')).toBe('3');
    });
  });

  describe('compareValues', () => {
    const stringConfig: SortField = { field: 'title', order: 'asc', compare: 'string' };
    const numericConfig: SortField = { field: 'order', order: 'asc', compare: 'numeric' };
    const descConfig: SortField = { field: 'title', order: 'desc', compare: 'string' };

    it('should return 0 when both null', () => {
      expect(compareValues(null, null, stringConfig)).toBe(0);
    });

    it('should place null after present value (a null)', () => {
      expect(compareValues(null, 'value', stringConfig)).toBe(1);
    });

    it('should place null after present value (b null)', () => {
      expect(compareValues('value', null, stringConfig)).toBe(-1);
    });

    it('should compare strings case-insensitively', () => {
      expect(compareValues('Apple', 'banana', stringConfig)).toBeLessThan(0);
      expect(compareValues('apple', 'BANANA', stringConfig)).toBeLessThan(0);
    });

    it('should compare numbers', () => {
      expect(compareValues('1', '10', numericConfig)).toBeLessThan(0);
      expect(compareValues('10', '2', numericConfig)).toBeGreaterThan(0);
    });

    it('should reverse order for desc', () => {
      expect(compareValues('Apple', 'Banana', descConfig)).toBeGreaterThan(0);
    });

    it('should NOT reverse missing value position for desc', () => {
      // Missing values ALWAYS go last, regardless of sort direction
      expect(compareValues(null, 'value', descConfig)).toBe(1);
      expect(compareValues('value', null, descConfig)).toBe(-1);
    });
  });

  describe('sortFiles', () => {
    it('should sort files by title alphabetically', () => {
      const files = [
        makeFile('zebra', 'Zebra'),
        makeFile('apple', 'Apple'),
        makeFile('mango', 'Mango'),
      ];

      const sorted = sortFiles(files, DEFAULT_SORT_CONFIG);

      expect(sorted[0].frontmatter?.title).toBe('Apple');
      expect(sorted[1].frontmatter?.title).toBe('Mango');
      expect(sorted[2].frontmatter?.title).toBe('Zebra');
    });

    it('should sort files by order numerically', () => {
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

    it('should place files without sort field last', () => {
      const files = [
        makeFile('no_order', 'No Order'),
        makeFile('first', 'First', 1),
        makeFile('second', 'Second', 2),
      ];

      const config: SortField[] = [
        { field: 'order', order: 'asc', compare: 'numeric' },
        { field: 'title', order: 'asc', compare: 'string' },
      ];
      const sorted = sortFiles(files, config);

      expect(sorted[0].frontmatter?.title).toBe('First');
      expect(sorted[1].frontmatter?.title).toBe('Second');
      expect(sorted[2].frontmatter?.title).toBe('No Order');
    });

    it('should support multi-level sorting', () => {
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

    it('should not mutate original array', () => {
      const files = [
        makeFile('zebra', 'Zebra'),
        makeFile('apple', 'Apple'),
      ];

      const sorted = sortFiles(files, DEFAULT_SORT_CONFIG);

      expect(files[0].frontmatter?.title).toBe('Zebra');
      expect(sorted[0].frontmatter?.title).toBe('Apple');
    });
  });

  describe('sortFolders', () => {
    it('should sort folders by title', () => {
      const folders = [
        makeFolder('zebra', 'Zebra'),
        makeFolder('apple', 'Apple'),
        makeFolder('mango', 'Mango'),
      ];

      const sorted = sortFolders(folders, DEFAULT_SORT_CONFIG);

      expect(sorted[0].title).toBe('Apple');
      expect(sorted[1].title).toBe('Mango');
      expect(sorted[2].title).toBe('Zebra');
    });

    it('should sort folders by order numerically', () => {
      const folders = [
        makeFolder('third', 'Third', 3),
        makeFolder('first', 'First', 1),
        makeFolder('second', 'Second', 2),
      ];

      const config: SortField[] = [{ field: 'order', order: 'asc', compare: 'numeric' }];
      const sorted = sortFolders(folders, config);

      expect(sorted[0].title).toBe('First');
      expect(sorted[1].title).toBe('Second');
      expect(sorted[2].title).toBe('Third');
    });

    it('should place folders without sort field last', () => {
      const folders = [
        makeFolder('no_order', 'No Order'),
        makeFolder('first', 'First', 1),
        makeFolder('second', 'Second', 2),
      ];

      const config: SortField[] = [
        { field: 'order', order: 'asc', compare: 'numeric' },
        { field: 'title', order: 'asc', compare: 'string' },
      ];
      const sorted = sortFolders(folders, config);

      expect(sorted[0].title).toBe('First');
      expect(sorted[1].title).toBe('Second');
      expect(sorted[2].title).toBe('No Order');
    });
  });
});

/**
 * Integration tests that verify mbr-nav and mbr-browse use the same
 * sorting logic via the shared module.
 */
describe('Integration: Shared Sorting Consistency', () => {
  it('should produce identical sort order for files in both components', () => {
    // This test verifies that the sorting module, when applied to the same
    // data, produces consistent results that would be identical in both
    // mbr-browse (file list) and mbr-nav (prev/next).

    const files = [
      makeFile('quicklook', 'QuickLook Preview', 4),
      makeFile('gui', 'GUI Window', 1),
      makeFile('build', 'Static Build', 3),
      makeFile('server', 'Web Server', 2),
    ];

    const config: SortField[] = [
      { field: 'order', order: 'asc', compare: 'numeric' },
      { field: 'title', order: 'asc', compare: 'string' },
    ];

    const sorted = sortFiles(files, config);

    // Expected order: gui (1), server (2), build (3), quicklook (4)
    expect(sorted.map(f => f.frontmatter?.title)).toEqual([
      'GUI Window',
      'Web Server',
      'Static Build',
      'QuickLook Preview',
    ]);

    // For mbr-nav: if we're at "Web Server", prev should be "GUI Window"
    // and next should be "Static Build"
    const serverIndex = sorted.findIndex(f => f.frontmatter?.title === 'Web Server');
    expect(serverIndex).toBe(1);
    expect(sorted[serverIndex - 1].frontmatter?.title).toBe('GUI Window');
    expect(sorted[serverIndex + 1].frontmatter?.title).toBe('Static Build');
  });
});

/**
 * Tests for buildFolderTree function.
 */
describe('buildFolderTree', () => {
  function makeFileWithPath(urlPath: string, rawPath: string, title?: string, order?: number): MarkdownFile {
    const frontmatter: Record<string, any> = {};
    if (title !== undefined) frontmatter.title = title;
    if (order !== undefined) frontmatter.order = order;

    return {
      url_path: urlPath,
      raw_path: rawPath,
      created: 1000,
      modified: 2000,
      frontmatter: Object.keys(frontmatter).length > 0 ? frontmatter : null,
    };
  }

  it('should create root node with no children for root-level file', () => {
    const files = [
      makeFileWithPath('/', 'index.md', 'Home'),
    ];

    const tree = buildFolderTree(files);

    expect(tree.name).toBe('');
    expect(tree.path).toBe('/');
    expect(tree.files.length).toBe(1);
    expect(tree.files[0].frontmatter?.title).toBe('Home');
    expect(tree.children.size).toBe(0);
  });

  it('should create folder for single-segment URL', () => {
    const files = [
      makeFileWithPath('/getting-started/', 'getting-started/index.md', 'Getting Started', 1),
    ];

    const tree = buildFolderTree(files);

    expect(tree.children.size).toBe(1);
    expect(tree.children.has('getting-started')).toBe(true);

    const folder = tree.children.get('getting-started')!;
    expect(folder.title).toBe('Getting Started');
    expect(folder.files.length).toBe(1);
  });

  it('should put non-index files in parent folder', () => {
    const files = [
      makeFileWithPath('/getting-started/', 'getting-started/index.md', 'Getting Started'),
      makeFileWithPath('/getting-started/quickstart/', 'getting-started/quickstart.md', 'Quick Start'),
    ];

    const tree = buildFolderTree(files);

    const folder = tree.children.get('getting-started')!;
    expect(folder.files.length).toBe(2);
    expect(folder.files.map(f => f.frontmatter?.title)).toContain('Getting Started');
    expect(folder.files.map(f => f.frontmatter?.title)).toContain('Quick Start');
  });

  it('should capture frontmatter from index file for folder sorting', () => {
    const files = [
      makeFileWithPath('/modes/', 'modes/index.md', 'Modes', 2),
    ];

    const tree = buildFolderTree(files);

    const folder = tree.children.get('modes')!;
    expect(folder.frontmatter?.order).toBe(2);
    expect(folder.title).toBe('Modes');
  });

  it('should calculate file counts recursively', () => {
    const files = [
      makeFileWithPath('/', 'index.md', 'Home'),
      makeFileWithPath('/getting-started/', 'getting-started/index.md', 'Getting Started'),
      makeFileWithPath('/getting-started/quickstart/', 'getting-started/quickstart.md', 'Quick Start'),
      makeFileWithPath('/modes/', 'modes/index.md', 'Modes'),
      makeFileWithPath('/modes/gui/', 'modes/gui.md', 'GUI'),
      makeFileWithPath('/modes/server/', 'modes/server.md', 'Server'),
    ];

    const tree = buildFolderTree(files);

    // Root has 1 file (index.md) + 2 from getting-started + 3 from modes = 6
    expect(tree.fileCount).toBe(6);

    // getting-started has index + quickstart = 2
    expect(tree.children.get('getting-started')!.fileCount).toBe(2);

    // modes has index + gui + server = 3
    expect(tree.children.get('modes')!.fileCount).toBe(3);
  });
});

/**
 * Tests for flattenToLinearSequence function.
 */
describe('flattenToLinearSequence', () => {
  function makeFileWithPath(urlPath: string, rawPath: string, title?: string, order?: number): MarkdownFile {
    const frontmatter: Record<string, any> = {};
    if (title !== undefined) frontmatter.title = title;
    if (order !== undefined) frontmatter.order = order;

    return {
      url_path: urlPath,
      raw_path: rawPath,
      created: 1000,
      modified: 2000,
      frontmatter: Object.keys(frontmatter).length > 0 ? frontmatter : null,
    };
  }

  it('should return files in depth-first order', () => {
    const files = [
      makeFileWithPath('/', 'index.md', 'Home'),
      makeFileWithPath('/getting-started/', 'getting-started/index.md', 'Getting Started', 1),
      makeFileWithPath('/getting-started/quickstart/', 'getting-started/quickstart.md', 'Quick Start'),
      makeFileWithPath('/modes/', 'modes/index.md', 'Modes', 2),
      makeFileWithPath('/modes/gui/', 'modes/gui.md', 'GUI'),
    ];

    const tree = buildFolderTree(files);
    const config: SortField[] = [
      { field: 'order', order: 'asc', compare: 'numeric' },
      { field: 'title', order: 'asc', compare: 'string' },
    ];

    const sequence = flattenToLinearSequence(tree, config);

    // Expected order:
    // 1. Root files (Home - no order, but only file at root)
    // 2. getting-started folder (order 1): Getting Started (order 1), Quick Start (no order → last)
    // 3. modes folder (order 2): Modes (order 2), GUI (no order → last)
    // Files without order field sort after files with order field
    expect(sequence.map(f => f.frontmatter?.title)).toEqual([
      'Home',
      'Getting Started',
      'Quick Start',
      'Modes',
      'GUI',
    ]);
  });

  it('should allow navigation from last in folder to first in next folder', () => {
    const files = [
      makeFileWithPath('/getting-started/', 'getting-started/index.md', 'Getting Started', 1),
      makeFileWithPath('/getting-started/quickstart/', 'getting-started/quickstart.md', 'Quick Start'),
      makeFileWithPath('/modes/', 'modes/index.md', 'Modes', 2),
      makeFileWithPath('/modes/gui/', 'modes/gui.md', 'GUI'),
    ];

    const tree = buildFolderTree(files);
    const config: SortField[] = [
      { field: 'order', order: 'asc', compare: 'numeric' },
      { field: 'title', order: 'asc', compare: 'string' },
    ];

    const sequence = flattenToLinearSequence(tree, config);

    // Find Quick Start (last in getting-started folder - has no order, so after Getting Started)
    const quickstartIndex = sequence.findIndex(f => f.frontmatter?.title === 'Quick Start');

    // Next should be first in modes folder (Modes has order 2, GUI has no order → Modes first)
    expect(sequence[quickstartIndex + 1].frontmatter?.title).toBe('Modes');
  });

  it('should allow navigation from first in folder to last in previous folder', () => {
    const files = [
      makeFileWithPath('/getting-started/', 'getting-started/index.md', 'Getting Started', 1),
      makeFileWithPath('/getting-started/quickstart/', 'getting-started/quickstart.md', 'Quick Start'),
      makeFileWithPath('/modes/', 'modes/index.md', 'Modes', 2),
    ];

    const tree = buildFolderTree(files);
    const config: SortField[] = [
      { field: 'order', order: 'asc', compare: 'numeric' },
      { field: 'title', order: 'asc', compare: 'string' },
    ];

    const sequence = flattenToLinearSequence(tree, config);

    // Find Modes (first in modes folder)
    const modesIndex = sequence.findIndex(f => f.frontmatter?.title === 'Modes');

    // Previous should be last in getting-started folder (Quick Start)
    expect(sequence[modesIndex - 1].frontmatter?.title).toBe('Quick Start');
  });

  it('should sort folders by sort config', () => {
    const files = [
      makeFileWithPath('/zebra/', 'zebra/index.md', 'Zebra', 3),
      makeFileWithPath('/apple/', 'apple/index.md', 'Apple', 1),
      makeFileWithPath('/mango/', 'mango/index.md', 'Mango', 2),
    ];

    const tree = buildFolderTree(files);
    const config: SortField[] = [
      { field: 'order', order: 'asc', compare: 'numeric' },
    ];

    const sequence = flattenToLinearSequence(tree, config);

    expect(sequence.map(f => f.frontmatter?.title)).toEqual([
      'Apple',
      'Mango',
      'Zebra',
    ]);
  });

  it('should sort files within folder by sort config', () => {
    const files = [
      makeFileWithPath('/docs/', 'docs/index.md', 'Docs', 1),
      makeFileWithPath('/docs/zebra/', 'docs/zebra.md', 'Zebra'),
      makeFileWithPath('/docs/apple/', 'docs/apple.md', 'Apple'),
    ];

    const tree = buildFolderTree(files);
    const config: SortField[] = [
      { field: 'title', order: 'asc', compare: 'string' },
    ];

    const sequence = flattenToLinearSequence(tree, config);

    // Files sorted by title: Apple, Docs, Zebra
    expect(sequence.map(f => f.frontmatter?.title)).toEqual([
      'Apple',
      'Docs',
      'Zebra',
    ]);
  });
});

/**
 * Tests for global linear navigation (cross-folder prev/next).
 */
describe('Global Linear Navigation', () => {
  function makeFileWithPath(urlPath: string, rawPath: string, title?: string, order?: number): MarkdownFile {
    const frontmatter: Record<string, any> = {};
    if (title !== undefined) frontmatter.title = title;
    if (order !== undefined) frontmatter.order = order;

    return {
      url_path: urlPath,
      raw_path: rawPath,
      created: 1000,
      modified: 2000,
      frontmatter: Object.keys(frontmatter).length > 0 ? frontmatter : null,
    };
  }

  it('should create a complete linear sequence through all folders', () => {
    // Simulate a docs site structure
    const files = [
      makeFileWithPath('/', 'index.md', 'Home'),
      makeFileWithPath('/getting-started/', 'getting-started/index.md', 'Installation', 1),
      makeFileWithPath('/getting-started/quickstart/', 'getting-started/quickstart.md', 'Quick Start'),
      makeFileWithPath('/modes/', 'modes/index.md', 'Modes', 2),
      makeFileWithPath('/modes/gui/', 'modes/gui.md', 'GUI Window', 1),
      makeFileWithPath('/modes/server/', 'modes/server.md', 'Web Server', 2),
    ];

    const tree = buildFolderTree(files);
    const config: SortField[] = [
      { field: 'order', order: 'asc', compare: 'numeric' },
      { field: 'title', order: 'asc', compare: 'string' },
    ];

    const sequence = flattenToLinearSequence(tree, config);

    // Verify complete sequence
    expect(sequence.length).toBe(6);

    // First file should be Home (root)
    expect(sequence[0].frontmatter?.title).toBe('Home');

    // Last file should be Web Server (last in last folder)
    expect(sequence[sequence.length - 1].frontmatter?.title).toBe('Web Server');
  });

  it('should place Previous as disabled only on first file', () => {
    const files = [
      makeFileWithPath('/', 'index.md', 'Home'),
      makeFileWithPath('/docs/', 'docs/index.md', 'Docs'),
    ];

    const tree = buildFolderTree(files);
    const sequence = flattenToLinearSequence(tree, DEFAULT_SORT_CONFIG);

    // Verify sequence has files
    expect(sequence.length).toBeGreaterThan(0);

    // First file is Home (root folder files come before child folder files)
    // Previous would be disabled for this file (index 0 has nothing before it)
    expect(sequence[0].frontmatter?.title).toBe('Home');
  });

  it('should place Next as disabled only on last file', () => {
    const files = [
      makeFileWithPath('/', 'index.md', 'Home'),
      makeFileWithPath('/docs/', 'docs/index.md', 'Docs'),
    ];

    const tree = buildFolderTree(files);
    const sequence = flattenToLinearSequence(tree, DEFAULT_SORT_CONFIG);

    // Verify sequence has expected files
    expect(sequence.length).toBe(2);

    // Last file is Docs (in child folder, after root)
    // Next would be disabled for this file (nothing after it)
    expect(sequence[sequence.length - 1].frontmatter?.title).toBe('Docs');
  });
});
