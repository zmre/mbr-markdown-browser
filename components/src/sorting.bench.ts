/**
 * Benchmarks for sorting utilities.
 *
 * Measures sortFiles and buildFolderTree performance at various dataset sizes.
 */

import { bench, describe } from 'vitest'
import { sortFiles, buildFolderTree, type MarkdownFile, type SortField } from './sorting'

function generateFiles(count: number): MarkdownFile[] {
  return Array.from({ length: count }, (_, i) => ({
    url_path: `/folder_${i % 10}/doc_${i}/`,
    raw_path: `folder_${i % 10}/doc_${i}.md`,
    created: 1000 + i * 100,
    modified: 2000 + (count - i) * 50,
    frontmatter: {
      title: `Document ${String(count - i).padStart(5, '0')}`,
      order: i % 100,
      category: `cat_${i % 10}`,
    },
  }))
}

describe('sortFiles', () => {
  const singleFieldConfig: SortField[] = [
    { field: 'title', order: 'asc', compare: 'string' },
  ]

  const multiFieldConfig: SortField[] = [
    { field: 'category', order: 'asc', compare: 'string' },
    { field: 'order', order: 'asc', compare: 'numeric' },
    { field: 'title', order: 'asc', compare: 'string' },
  ]

  for (const size of [100, 500, 2000]) {
    const files = generateFiles(size)

    bench(`single field sort (${size} items)`, () => {
      sortFiles(files, singleFieldConfig)
    })

    bench(`multi field sort (${size} items)`, () => {
      sortFiles(files, multiFieldConfig)
    })
  }
})

describe('buildFolderTree', () => {
  for (const size of [100, 500, 2000]) {
    const files = generateFiles(size)

    bench(`build tree (${size} items)`, () => {
      buildFolderTree(files)
    })
  }
})
