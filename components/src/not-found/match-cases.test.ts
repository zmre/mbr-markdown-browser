/**
 * The matcher's shared fixture: `tests/fixtures/not-found-match-cases.json`
 * is read here and by `crates/mbr-core/src/not_found.rs`, the Rust port the
 * link checker (`mbr --check-links-only`) uses, so the two cannot drift. Add a
 * case there when a rule changes.
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import {
  MAX_MATCHES,
  didYouMean,
  findMatches,
  folderFallback,
  type SiteIndex,
  type SiteOtherFile,
} from './match.js'

interface Fixture {
  sites: Record<string, SiteIndex & { other_files?: SiteOtherFile[] }>
  cases: Array<{
    name: string
    site: string
    path: string
    matches: string[]
    did_you_mean: string[]
    folder: [string, string] | null
  }>
}

const fixture: Fixture = JSON.parse(
  readFileSync(join(process.cwd(), '..', 'tests', 'fixtures', 'not-found-match-cases.json'), 'utf8'),
)

describe('UNIT not-found matcher shared cases', () => {
  for (const c of fixture.cases) {
    it(c.name, () => {
      const site = fixture.sites[c.site]
      const urls = (pages: { url: string }[]) => pages.map((p) => p.url)
      expect(urls(findMatches(c.path, site, site.other_files ?? [], MAX_MATCHES))).toEqual(c.matches)
      expect(urls(didYouMean(c.path, site))).toEqual(c.did_you_mean)
      const folder = folderFallback(c.path, site)
      expect(folder ? [folder.url, folder.title] : null).toEqual(c.folder)
    })
  }
})
