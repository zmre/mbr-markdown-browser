import { afterEach, describe, expect, it } from 'vitest'
import {
  COMPANION_HIDDEN_CLASS,
  COMPANION_HIDDEN_KEY,
  companionHiddenKey,
  isCompanionHidden,
  setCompanionHidden,
} from './companion-pref.ts'

declare const __MBR_HEAD_TEMPLATE__: string

const REPO_A = 'aaaa1111'
const REPO_B = 'bbbb2222'

afterEach(() => {
  document.documentElement.classList.remove(COMPANION_HIDDEN_CLASS)
  localStorage.clear()
  delete (window as { __MBR_CONFIG__?: unknown }).__MBR_CONFIG__
})

/** The `_head.html` pre-paint script, as shipped. */
function prePaintScript(): string {
  // Sliced by position rather than matched with a regex: this reads one
  // known template, it is not HTML filtering.
  const html = __MBR_HEAD_TEMPLATE__
  const at = html.indexOf('mbr_calendar_companion_hidden')
  expect(at).toBeGreaterThan(-1)
  const open = '<script>'
  const start = html.lastIndexOf(open, at)
  const end = html.indexOf('</script>', at)
  expect(start).toBeGreaterThan(-1)
  expect(end).toBeGreaterThan(at)
  const body = html.slice(start + open.length, end)
  expect(body).not.toContain('<')
  return body
}

/** Runs the pre-paint script on a fresh page served for `repoId`. */
function runPrePaint(repoId: string): boolean {
  document.documentElement.classList.remove(COMPANION_HIDDEN_CLASS)
  ;(window as { __MBR_CONFIG__?: unknown }).__MBR_CONFIG__ = { repoId }
  new Function(prePaintScript())()
  return isCompanionHidden()
}

describe('the companion dismissal preference', () => {
  it('reads storage before paint only on a page with the docked column', () => {
    // `calendar_companion` is the server's flag behind body.mbr-has-companion.
    expect(__MBR_HEAD_TEMPLATE__).toMatch(/\{% if calendar_companion %\}<script>[^<]*mbr_calendar_companion_hidden/)
  })

  it('sets the class and remembers it, and clears both', () => {
    setCompanionHidden(true, REPO_A)
    expect(isCompanionHidden()).toBe(true)
    expect(localStorage.getItem(companionHiddenKey(REPO_A))).toBe('1')
    setCompanionHidden(false, REPO_A)
    expect(isCompanionHidden()).toBe(false)
    expect(localStorage.getItem(companionHiddenKey(REPO_A))).toBeNull()
  })

  it('is scoped per repository: two repos on one origin do not share it', () => {
    expect(companionHiddenKey(REPO_A)).toBe(`${COMPANION_HIDDEN_KEY}:${REPO_A}`)
    expect(companionHiddenKey(REPO_A)).not.toBe(companionHiddenKey(REPO_B))
    expect(companionHiddenKey('')).toBe(COMPANION_HIDDEN_KEY)

    setCompanionHidden(true, REPO_A)
    expect(localStorage.getItem(companionHiddenKey(REPO_B))).toBeNull()
    expect(localStorage.getItem(COMPANION_HIDDEN_KEY)).toBeNull()
    expect(runPrePaint(REPO_A)).toBe(true)
    expect(runPrePaint(REPO_B)).toBe(false)
  })

  it('is read before paint by _head.html, under the same key and class', () => {
    const script = prePaintScript()
    expect(script).toContain(`"${COMPANION_HIDDEN_KEY}"`)
    expect(script).toContain(`classList.add("${COMPANION_HIDDEN_CLASS}")`)
    // A top-level `var` would leak a global into every page.
    expect(script).not.toMatch(/\bvar\b|\blet\b|\bconst\b/)

    // And it computes the runtime key, scoped and unscoped alike.
    for (const repoId of [REPO_A, '']) {
      localStorage.clear()
      setCompanionHidden(true, repoId)
      expect(runPrePaint(repoId), repoId).toBe(true)
      setCompanionHidden(false, repoId)
      expect(runPrePaint(repoId), repoId).toBe(false)
    }
  })

  it('survives a page without __MBR_CONFIG__ (bare key)', () => {
    localStorage.setItem(COMPANION_HIDDEN_KEY, '1')
    new Function(prePaintScript())()
    expect(isCompanionHidden()).toBe(true)
  })
})
