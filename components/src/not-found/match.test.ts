import { describe, expect, it } from 'vitest'
import { canonicalPathFor } from '../shared.js'
import {
  MAX_FOLDER_NOTES,
  MAX_MATCHES,
  didYouMean,
  findMatches,
  folderFallback,
  namesAFile,
  normalizeName,
  pageExists,
  suggestForMissing,
  type SiteIndex,
  type SiteMarkdownFile,
} from './match.js'

/** A site.json entry from its source path (`a/b.md` → `/a/b/`, `a/index.md` → `/a/`). */
function note(raw: string, title?: string): SiteMarkdownFile {
  const stem = raw.replace(/\.md$/, '')
  const url = stem === 'index' ? '/' : `/${stem.replace(/\/index$/, '')}/`
  return { raw_path: raw, url_path: url, frontmatter: title ? { title } : {} }
}

function site(...raws: Array<string | SiteMarkdownFile>): SiteIndex {
  return {
    index_file: 'index.md',
    markdown_files: raws.map((r) => (typeof r === 'string' ? note(r) : r)),
  }
}

const urls = (pages: { url: string }[]) => pages.map((p) => p.url)

describe('UNIT normalizeName', () => {
  it('treats case, spaces, dashes and underscores alike', () => {
    expect(normalizeName('My Note')).toBe('my note')
    expect(normalizeName('my-note')).toBe('my note')
    expect(normalizeName('MY__note')).toBe('my note')
    expect(normalizeName(' a - b ')).toBe('a b')
  })
})

describe('UNIT findMatches', () => {
  it('finds a note moved to another folder', () => {
    const s = site('index.md', 'archive/2024/project-plan.md', 'work/other.md')
    expect(urls(findMatches('/work/project-plan/', s))).toEqual(['/archive/2024/project-plan/'])
  })

  it('matches across case, spaces, dashes and underscores', () => {
    const s = site('archive/Project_Plan.md')
    expect(urls(findMatches('/work/project plan/', s))).toEqual(['/archive/Project_Plan/'])
  })

  it('strips a markdown extension from the target', () => {
    const s = site('archive/project-plan.md')
    expect(urls(findMatches('/work/project-plan.md', s))).toEqual(['/archive/project-plan/'])
  })

  it('reads an index note as its folder name', () => {
    const s = site('archive/clients/acme/index.md', 'archive/clients/acme/notes.md')
    expect(urls(findMatches('/clients/acme/index.md', s))).toEqual(['/archive/clients/acme/'])
    expect(urls(findMatches('/clients/acme/', s))).toEqual(['/archive/clients/acme/'])
  })

  it('honours a configured index file', () => {
    const s: SiteIndex = { index_file: '_index.md', markdown_files: [note('moved/acme/_index.md')] }
    s.markdown_files![0].url_path = '/moved/acme/'
    expect(urls(findMatches('/clients/acme/_index.md', s))).toEqual(['/moved/acme/'])
  })

  it('finds an extended or shortened name after exact ones', () => {
    const s = site('a/meeting.md', 'b/meeting-notes.md', 'c/weekly-meeting-notes.md', 'd/unrelated.md')
    expect(urls(findMatches('/x/meeting/', s))).toEqual(['/a/meeting/', '/b/meeting-notes/', '/c/weekly-meeting-notes/'])
    expect(urls(findMatches('/x/meeting-notes-2024/', s))).toEqual(['/a/meeting/', '/b/meeting-notes/'])
  })

  it('needs a word start and a minimum length for partial matches', () => {
    const s = site('a/airplane.md', 'b/faq-old.md', 'c/plans.md')
    // "plan" is inside "airplane" but starts no word there.
    expect(urls(findMatches('/x/plan/', s))).toEqual(['/c/plans/'])
    // "faq" is too short to match on a prefix.
    expect(findMatches('/x/faq/', s)).toEqual([])
  })

  it('ranks by tier, then shared leading folders, then URL', () => {
    const s = site('z/notes/todo.md', 'work/old/todo.md', 'a/todo.md', 'work/todo-list.md')
    expect(urls(findMatches('/work/new/todo/', s))).toEqual([
      '/work/old/todo/',
      '/a/todo/',
      '/z/notes/todo/',
      '/work/todo-list/',
    ])
  })

  it('never suggests the requested path itself', () => {
    const s = site('a/thing.md', 'b/thing.md')
    expect(urls(findMatches('/a/thing/', s))).toEqual(['/b/thing/'])
  })

  it('caps the list', () => {
    const s = site(...Array.from({ length: 10 }, (_, i) => `f${i}/topic.md`))
    expect(findMatches('/gone/topic/', s)).toHaveLength(MAX_MATCHES)
  })

  it('uses the frontmatter title for display', () => {
    const s = site(note('a/plan.md', 'The Plan'))
    expect(findMatches('/b/plan/', s)[0]).toEqual({ url: '/a/plan/', title: 'The Plan', kind: 'note' })
  })

  it('matches a percent-decoded name with spaces (as canonicalPathFor yields)', () => {
    const s = site('moved/My Note.md')
    s.markdown_files![0].url_path = '/moved/My Note/'
    const path = canonicalPathFor('/old/My%20Note/', '', true)
    expect(urls(findMatches(path, s))).toEqual(['/moved/My Note/'])
  })

  it('matches from a static 404.html under a deployment prefix', () => {
    const s = site('moved/plan.md')
    // GitHub project pages: the site lives under /my-repo/, 404.html is served
    // at the missing URL and its basePath is root-anchored.
    const path = canonicalPathFor('/my-repo/old/deep/plan/', '/my-repo/', false)
    expect(path).toBe('/old/deep/plan/')
    expect(urls(findMatches(path, s))).toEqual(['/moved/plan/'])
  })

  it('finds a moved non-markdown file by its exact name', () => {
    const s = site('docs/report.md')
    const files = [{ url_path: '/archive/report.pdf' }, { url_path: '/archive/report-old.pdf' }]
    expect(namesAFile('/docs/report.pdf', s)).toBe(true)
    expect(namesAFile('/docs/report/', s)).toBe(false)
    expect(namesAFile('/docs/report.md', s)).toBe(false)
    expect(findMatches('/docs/report.pdf', s, files)).toEqual([
      { url: '/archive/report.pdf', title: 'report.pdf', kind: 'file' },
    ])
    // A static build's canonical path has a trailing slash even for a file.
    expect(urls(findMatches('/docs/report.pdf/', s, files))).toEqual(['/archive/report.pdf'])
  })

  it('finds nothing for the root', () => {
    expect(findMatches('/', site('a/index.md'))).toEqual([])
  })
})

describe('UNIT folderFallback', () => {
  const s = site(
    note('projects/index.md', 'All Projects'),
    note('projects/beta.md', 'Beta'),
    note('projects/alpha.md', 'alpha'),
    'projects/deep/nested.md',
    'elsewhere/x.md',
  )

  it('lists the notes directly in an existing parent folder', () => {
    expect(folderFallback('/projects/gamma/', s)).toEqual({
      url: '/projects/',
      title: 'All Projects',
      notes: [
        { url: '/projects/alpha/', title: 'alpha', kind: 'note' },
        { url: '/projects/beta/', title: 'Beta', kind: 'note' },
      ],
      more: 0,
    })
  })

  it('counts a folder with no index of its own as existing', () => {
    const r = folderFallback('/projects/deep/missing/', s)
    expect(r?.url).toBe('/projects/deep/')
    expect(r?.title).toBe('deep')
    expect(urls(r!.notes)).toEqual(['/projects/deep/nested/'])
  })

  it('shows nothing when the parent is gone too, without walking up', () => {
    expect(folderFallback('/projects/gone/missing/', s)).toBeNull()
  })

  it('shows nothing for a page at the root', () => {
    expect(folderFallback('/missing/', s)).toBeNull()
  })

  it('caps the list and counts the rest', () => {
    const big = site(...Array.from({ length: MAX_FOLDER_NOTES + 3 }, (_, i) => `big/n${String(i).padStart(2, '0')}.md`))
    const r = folderFallback('/big/missing/', big)!
    expect(r.notes).toHaveLength(MAX_FOLDER_NOTES)
    expect(r.more).toBe(3)
  })
})

describe('UNIT suggestForMissing', () => {
  it('prefers matches and then omits the folder', () => {
    const s = site('projects/index.md', 'archive/gamma.md', 'projects/beta.md')
    const r = suggestForMissing('/projects/gamma/', s)
    expect(urls(r.matches)).toEqual(['/archive/gamma/'])
    expect(r.folder).toBeNull()
  })

  it('falls back to the folder when nothing matches', () => {
    const s = site('projects/index.md', 'projects/beta.md')
    const r = suggestForMissing('/projects/gamma/', s)
    expect(r.matches).toEqual([])
    expect(r.folder?.url).toBe('/projects/')
  })

  it('shows nothing when neither exists', () => {
    expect(suggestForMissing('/nope/gone/', site('a/b.md'))).toEqual({ matches: [], folder: null })
  })
})

describe('UNIT didYouMean', () => {
  const s = site('archive/plan.md', 'a/plan-b.md', 'b/plan-c.md', 'c/plan-d.md', 'docs/exists.md')

  it('caps at three', () => {
    expect(urls(didYouMean('/docs/plan/', s))).toEqual(['/archive/plan/', '/a/plan-b/', '/b/plan-c/'])
  })

  it('says nothing when the target exists (e.g. only its trailing slash was missing)', () => {
    expect(pageExists('/docs/exists', s)).toBe(true)
    expect(didYouMean('/docs/exists', s)).toEqual([])
  })
})
