import { describe, expect, it } from 'vitest'
import {
  deriveUrlPath,
  indexFileOf,
  knownUrlPaths,
  resolveFileUrlPath,
} from './file-url.ts'

describe('deriveUrlPath', () => {
  it('maps a top-level page', () => {
    expect(deriveUrlPath('README.md', 'index.md')).toBe('/README/')
  })

  it('maps a nested page', () => {
    expect(deriveUrlPath('docs/guide.md', 'index.md')).toBe('/docs/guide/')
  })

  it('collapses an index file into its directory', () => {
    expect(deriveUrlPath('docs/index.md', 'index.md')).toBe('/docs/')
  })

  it('maps the root index to the root', () => {
    expect(deriveUrlPath('index.md', 'index.md')).toBe('/')
  })

  it('honours a non-default index file name', () => {
    expect(deriveUrlPath('docs/README.md', 'README.md')).toBe('/docs/')
    expect(deriveUrlPath('docs/index.md', 'README.md')).toBe('/docs/index/')
  })

  it('accepts the other markdown extensions mbr serves', () => {
    expect(deriveUrlPath('notes.markdown', 'index.md')).toBe('/notes/')
    expect(deriveUrlPath('notes.mkd', 'index.md')).toBe('/notes/')
  })

  it('rejects a non-markdown file', () => {
    expect(deriveUrlPath('images/logo.png', 'index.md')).toBeNull()
    expect(deriveUrlPath('Makefile', 'index.md')).toBeNull()
  })

  it('tolerates leading ./ and backslashes', () => {
    expect(deriveUrlPath('./docs/guide.md', 'index.md')).toBe('/docs/guide/')
    expect(deriveUrlPath('docs\\guide.md', 'index.md')).toBe('/docs/guide/')
  })

  it('rejects an empty path', () => {
    expect(deriveUrlPath('', 'index.md')).toBeNull()
  })
})

describe('resolveFileUrlPath', () => {
  const known = new Set(['/docs/guide/', '/docs/', '/'])

  it('returns a verified path', () => {
    expect(resolveFileUrlPath('docs/guide.md', 'index.md', known)).toBe('/docs/guide/')
  })

  it('returns null when the derived page is not in the site index', () => {
    // The static_folder overlay hides a directory level, so a derived path can
    // be wrong. A note that renders as plain text beats a link that 404s.
    expect(resolveFileUrlPath('content/hidden.md', 'index.md', known)).toBeNull()
  })

  it('falls back to the unverified guess before site.json has loaded', () => {
    expect(resolveFileUrlPath('docs/guide.md', 'index.md', new Set())).toBe('/docs/guide/')
  })

  it('returns null for a non-page', () => {
    expect(resolveFileUrlPath('logo.png', 'index.md', known)).toBeNull()
  })
})

describe('knownUrlPaths', () => {
  it('collects url_path values', () => {
    const set = knownUrlPaths({
      markdown_files: [{ url_path: '/a/' }, { url_path: '/b/' }],
    })
    expect([...set].sort()).toEqual(['/a/', '/b/'])
  })

  it('is empty for a missing or malformed payload', () => {
    expect(knownUrlPaths(null).size).toBe(0)
    expect(knownUrlPaths({}).size).toBe(0)
    expect(knownUrlPaths({ markdown_files: 'nope' }).size).toBe(0)
  })

  it('skips entries with no usable url_path', () => {
    const set = knownUrlPaths({
      markdown_files: [{ url_path: '/a/' }, {}, { url_path: '' }, null, 7],
    })
    expect([...set]).toEqual(['/a/'])
  })
})

describe('indexFileOf', () => {
  it('reads the configured name', () => {
    expect(indexFileOf({ index_file: 'README.md' })).toBe('README.md')
  })

  it('defaults to index.md', () => {
    expect(indexFileOf({})).toBe('index.md')
    expect(indexFileOf(null)).toBe('index.md')
    expect(indexFileOf({ index_file: '' })).toBe('index.md')
  })
})
