/**
 * Shared sorting utilities for mbr components.
 *
 * Both mbr-browse and mbr-nav use configurable sorting based on
 * sort configuration from site.json.
 */

/**
 * Sort field configuration from site.json.
 */
export interface SortField {
  field: string;
  order: 'asc' | 'desc';
  compare: 'string' | 'numeric';
}

/**
 * Markdown file metadata from site.json.
 */
export interface MarkdownFile {
  url_path: string;
  raw_path: string;
  created: number;
  modified: number;
  frontmatter: Record<string, any> | null;
}

/**
 * Folder node for tree display in the browser.
 */
export interface FolderNode {
  name: string;
  title?: string;
  path: string;
  children: Map<string, FolderNode>;
  files: MarkdownFile[];
  fileCount: number;
  frontmatter?: Record<string, any> | null;
}

/**
 * Default sort configuration when none is provided.
 */
export const DEFAULT_SORT_CONFIG: SortField[] = [
  { field: 'title', order: 'asc', compare: 'string' }
];

/**
 * Get the filename part of a URL path for sorting.
 * e.g., "/docs/guide/intro/" -> "intro"
 */
export function getFileName(urlPath: string): string {
  const normalized = urlPath.endsWith('/') ? urlPath.slice(0, -1) : urlPath;
  const lastSlash = normalized.lastIndexOf('/');
  return normalized.slice(lastSlash + 1);
}

/**
 * Get a field value from a file for sorting.
 * Returns null for missing values.
 */
export function getFileFieldValue(file: MarkdownFile, field: string): string | null {
  switch (field) {
    case 'title':
      // Try frontmatter title, fallback to filename
      return file.frontmatter?.title
        ?? getFileName(file.url_path)
        ?? null;
    case 'filename':
      return getFileName(file.url_path) ?? null;
    case 'created':
      return file.created.toString();
    case 'modified':
      return file.modified.toString();
    default:
      // Frontmatter field lookup
      if (file.frontmatter && field in file.frontmatter) {
        const val = file.frontmatter[field];
        // Handle booleans for pinned pattern
        if (typeof val === 'boolean') {
          return val ? '1' : '0';
        }
        return String(val);
      }
      return null;
  }
}

/**
 * Get a field value from a folder for sorting.
 * Returns null for missing values.
 */
export function getFolderFieldValue(folder: FolderNode, field: string): string | null {
  switch (field) {
    case 'title':
      return folder.title ?? folder.name ?? null;
    case 'filename':
      return folder.name ?? null;
    default:
      // Frontmatter field lookup
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

/**
 * Compare two values by a sort field configuration.
 * Missing values sort AFTER present values (not affected by sort direction).
 */
export function compareValues(valA: string | null, valB: string | null, config: SortField): number {
  // Handle missing values: items without field sort AFTER items with it
  // This is NOT affected by sort direction
  if (valA === null && valB === null) return 0;
  if (valA === null) return 1;  // a missing → a comes after
  if (valB === null) return -1; // b missing → a comes before

  // Compare values - only this part is affected by sort direction
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

/**
 * Compare two files by a single sort field configuration.
 */
export function compareFilesByField(a: MarkdownFile, b: MarkdownFile, config: SortField): number {
  const valA = getFileFieldValue(a, config.field);
  const valB = getFileFieldValue(b, config.field);
  return compareValues(valA, valB, config);
}

/**
 * Compare two folders by a single sort field configuration.
 */
export function compareFoldersByField(a: FolderNode, b: FolderNode, config: SortField): number {
  const valA = getFolderFieldValue(a, config.field);
  const valB = getFolderFieldValue(b, config.field);
  return compareValues(valA, valB, config);
}

/**
 * Sort files using the configured sort order.
 * Supports multi-level sorting (first field takes precedence).
 */
export function sortFiles(files: MarkdownFile[], sortConfig: SortField[]): MarkdownFile[] {
  return [...files].sort((a, b) => {
    for (const sortField of sortConfig) {
      const cmp = compareFilesByField(a, b, sortField);
      if (cmp !== 0) return cmp;
    }
    return 0;
  });
}

/**
 * Sort folders using the configured sort order.
 * Supports multi-level sorting (first field takes precedence).
 */
export function sortFolders(folders: FolderNode[], sortConfig: SortField[]): FolderNode[] {
  return [...folders].sort((a, b) => {
    for (const sortField of sortConfig) {
      const cmp = compareFoldersByField(a, b, sortField);
      if (cmp !== 0) return cmp;
    }
    return 0;
  });
}

/**
 * Build a folder tree from markdown files.
 * Index files are included in their folder's files array.
 *
 * @param files - All markdown files from site.json
 * @param indexFile - The index filename (default: 'index.md')
 * @returns The root folder node of the tree
 */
export function buildFolderTree(files: MarkdownFile[], indexFile: string = 'index.md'): FolderNode {
  const root: FolderNode = {
    name: '',
    path: '/',
    children: new Map(),
    files: [],
    fileCount: 0,
  };

  for (const file of files) {
    const parts = file.url_path.split('/').filter(p => p.length > 0);
    let currentNode = root;

    // Navigate/create folder structure (all parts except last are folders)
    for (let i = 0; i < parts.length - 1; i++) {
      const part = parts[i];
      const folderPath = '/' + parts.slice(0, i + 1).join('/') + '/';

      if (!currentNode.children.has(part)) {
        currentNode.children.set(part, {
          name: part,
          path: folderPath,
          children: new Map(),
          files: [],
          fileCount: 0,
        });
      }

      currentNode = currentNode.children.get(part)!;
    }

    // Handle index files
    const fileName = file.raw_path.split('/').pop() || '';
    const isIndexFile = fileName === indexFile;

    if (isIndexFile) {
      // Index files define folder metadata (title)
      let targetNode = currentNode;

      if (parts.length > 0) {
        const lastPart = parts[parts.length - 1];
        const folderPath = '/' + parts.join('/') + '/';

        if (!currentNode.children.has(lastPart)) {
          currentNode.children.set(lastPart, {
            name: lastPart,
            path: folderPath,
            children: new Map(),
            files: [],
            fileCount: 0,
          });
        }
        targetNode = currentNode.children.get(lastPart)!;
      }

      // Capture frontmatter from index file for folder sorting
      if (file.frontmatter) {
        targetNode.frontmatter = file.frontmatter;
        if (file.frontmatter.title) {
          targetNode.title = file.frontmatter.title;
        }
      }
      // Include index file in the folder's file list
      targetNode.files.push(file);
      continue;
    }

    currentNode.files.push(file);
  }

  // Calculate descendant counts
  calculateFolderCounts(root);

  return root;
}

/**
 * Recursively calculate file counts for folders.
 */
function calculateFolderCounts(node: FolderNode): number {
  let count = node.files.length;
  for (const child of node.children.values()) {
    count += calculateFolderCounts(child);
  }
  node.fileCount = count;
  return count;
}

/**
 * Flatten a folder tree to a linear sequence for prev/next navigation.
 * Order: folder's files first (sorted), then depth-first through sorted child folders.
 *
 * @param root - The root folder node
 * @param sortConfig - Sort configuration for files and folders
 * @returns Array of files in linear navigation order
 */
export function flattenToLinearSequence(root: FolderNode, sortConfig: SortField[]): MarkdownFile[] {
  const result: MarkdownFile[] = [];

  function traverse(node: FolderNode): void {
    // Add this folder's files (sorted)
    const sortedFiles = sortFiles(node.files, sortConfig);
    result.push(...sortedFiles);

    // Recursively process child folders (sorted)
    const sortedChildren = sortFolders([...node.children.values()], sortConfig);
    for (const child of sortedChildren) {
      traverse(child);
    }
  }

  traverse(root);
  return result;
}
