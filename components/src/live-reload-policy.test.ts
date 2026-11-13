import { describe, expect, it } from 'vitest'
import {
  folderOfPathname,
  normalizeRepoPath,
  shouldReloadFor,
  type LiveReloadChange,
  type LiveReloadPage,
} from './live-reload-policy.ts'

const note: LiveReloadPage = { source: 'docs/guide.md', shownHash: 'h1', pathname: '/docs/guide/' }
const listing = (pathname: string): LiveReloadPage => ({ source: null, shownHash: null, pathname })
const change = (relative_path: string, extra: Partial<LiveReloadChange> = {}): LiveReloadChange => ({
  relative_path,
  event: 'modified',
  ...extra,
})

describe('normalizeRepoPath', () => {
  it('spells paths the way the server does', () => {
    expect(normalizeRepoPath('/docs/guide.md')).toBe('docs/guide.md')
    expect(normalizeRepoPath('./docs/guide.md')).toBe('docs/guide.md')
    expect(normalizeRepoPath('docs\\guide.md')).toBe('docs/guide.md')
    expect(normalizeRepoPath('docs/')).toBe('docs')
  })

  it('compares decomposed and composed names alike', () => {
    // macOS can report `é` as e + U+0301.
    expect(normalizeRepoPath('cafe\u0301.md')).toBe(normalizeRepoPath('caf\u00e9.md'))
  })
})

describe('folderOfPathname', () => {
  it('decodes the URL', () => {
    expect(folderOfPathname('/')).toBe('')
    expect(folderOfPathname('/docs/a%20b/')).toBe('docs/a b')
    expect(folderOfPathname('/bad%E0%A4%A/')).toBe('bad%E0%A4%A')
  })
})

describe('shouldReloadFor on a note page', () => {
  it('reloads for its own note when the fingerprint differs or is unknown', () => {
    expect(shouldReloadFor(change('docs/guide.md', { content_hash: 'h2' }), note)).toBe(true)
    expect(shouldReloadFor(change('docs/guide.md'), note)).toBe(true)
    expect(shouldReloadFor(change('docs/guide.md', { content_hash: 'h1' }), { ...note, shownHash: null })).toBe(true)
  })

  it('does not reload for an event whose fingerprint is what the page shows', () => {
    for (const event of ['modified', 'created'] as const) {
      expect(shouldReloadFor(change('docs/guide.md', { event, content_hash: 'h1' }), note)).toBe(false)
    }
  })

  it('reloads when its note is deleted', () => {
    expect(shouldReloadFor(change('docs/guide.md', { event: 'deleted', content_hash: 'h1' }), note)).toBe(true)
  })

  it('matches its note however the path is spelled', () => {
    expect(shouldReloadFor(change('docs\\guide.md'), { ...note, source: '/docs/guide.md' })).toBe(true)
  })

  it('ignores every other file in the repository', () => {
    for (const path of [
      'docs/other.md',
      'docs/index.md',
      'index.md',
      'site/page.html',
      'docs/style.css',
      'docs/app.js',
      'docs/guide.md~',
      'docs/4913',
      'docs/guide.md.tmp',
      'docs',
    ]) {
      expect(shouldReloadFor(change(path), note), path).toBe(false)
    }
  })

  it('reloads for template changes and configuration reloads', () => {
    expect(shouldReloadFor(change('.mbr/theme.css', { template: true }), note)).toBe(true)
    expect(shouldReloadFor(change('../templates/index.html', { template: true }), note)).toBe(true)
    expect(shouldReloadFor(change('.mbr/config.toml', { event: 'reloaded' }), note)).toBe(true)
  })

  it('leaves the raw config.toml event to the reloaded event that follows it', () => {
    expect(shouldReloadFor(change('.mbr/config.toml', { template: true }), note)).toBe(false)
  })
})

describe('shouldReloadFor on a page that is not a note', () => {
  it('follows the files and folders directly in a listing', () => {
    const docs = listing('/docs/')
    expect(shouldReloadFor(change('docs/new.md', { event: 'created' }), docs)).toBe(true)
    expect(shouldReloadFor(change('docs/old.md', { event: 'deleted' }), docs)).toBe(true)
    expect(shouldReloadFor(change('docs/sub', { event: 'created' }), docs)).toBe(true)
    expect(shouldReloadFor(change('docs/sub/index.md'), docs)).toBe(true)
  })

  it('ignores changes elsewhere', () => {
    const docs = listing('/docs/')
    expect(shouldReloadFor(change('other/new.md'), docs)).toBe(false)
    expect(shouldReloadFor(change('docs/sub/note.md'), docs)).toBe(false)
    expect(shouldReloadFor(change('docs.md'), listing('/'))).toBe(true)
    expect(shouldReloadFor(change('docs/x.md'), listing('/'))).toBe(false)
  })

  it('lets a 404 follow the note its URL would serve', () => {
    const missing = listing('/notes/idea/')
    expect(shouldReloadFor(change('notes/idea.md', { event: 'created' }), missing)).toBe(true)
    expect(shouldReloadFor(change('notes/idea/index.md', { event: 'created' }), missing)).toBe(true)
    expect(shouldReloadFor(change('notes/other.md', { event: 'created' }), missing)).toBe(false)
  })

  it('decodes the URL before comparing', () => {
    expect(shouldReloadFor(change('my notes/a.md'), listing('/my%20notes/'))).toBe(true)
  })
})
