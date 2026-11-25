import { afterEach, describe, expect, it } from 'vitest'
import {
  COMPANION_HIDDEN_CLASS,
  COMPANION_HIDDEN_KEY,
  companionHiddenKey,
  isCompanionHidden,
  setCompanionHidden,
} from './companion-pref.ts'

declare const __MBR_HEAD_TEMPLATE__: string
declare const __MBR_BOOTSTRAP_JS__: string

const REPO_A = 'aaaa1111'
const REPO_B = 'bbbb2222'

afterEach(() => {
  document.documentElement.classList.remove(COMPANION_HIDDEN_CLASS)
  localStorage.clear()
  delete (window as { __MBR_CONFIG__?: unknown }).__MBR_CONFIG__
})

/**
 * Runs `mbr-bootstrap.js`, as shipped, as the `<script>` `_head.html` writes:
 * with `data-mbr-companion-prepaint` exactly when the page has the column.
 */
function runBootstrap(prePaint: boolean): void {
  const script = document.createElement('script')
  if (prePaint) script.setAttribute('data-mbr-companion-prepaint', '')
  Object.defineProperty(document, 'currentScript', { configurable: true, get: () => script })
  try {
    new Function(__MBR_BOOTSTRAP_JS__)()
  } finally {
    delete (document as { currentScript?: unknown }).currentScript
  }
}

/** Runs the pre-paint check on a fresh page served for `repoId`. */
function runPrePaint(repoId: string): boolean {
  document.documentElement.classList.remove(COMPANION_HIDDEN_CLASS)
  ;(window as { __MBR_CONFIG__?: unknown }).__MBR_CONFIG__ = { repoId }
  runBootstrap(true)
  return isCompanionHidden()
}

describe('the companion dismissal preference', () => {
  it('reads storage before paint only on a page with the docked column', () => {
    // `calendar_companion` is the server's flag behind body.mbr-has-companion;
    // the bootstrap is a synchronous script in the head, so it runs pre-paint.
    expect(__MBR_HEAD_TEMPLATE__).toMatch(
      /<script src="\{\{ asset_base \}\}mbr-bootstrap\.js"\{% if calendar_companion %\} data-mbr-companion-prepaint\{% endif %\}><\/script>/
    )
    setCompanionHidden(true, REPO_A)
    document.documentElement.classList.remove(COMPANION_HIDDEN_CLASS)
    ;(window as { __MBR_CONFIG__?: unknown }).__MBR_CONFIG__ = { repoId: REPO_A }
    runBootstrap(false)
    expect(isCompanionHidden()).toBe(false)
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

  it('is read before paint by the bootstrap, under the same key and class', () => {
    expect(__MBR_BOOTSTRAP_JS__).toContain(`"${COMPANION_HIDDEN_KEY}"`)
    expect(__MBR_BOOTSTRAP_JS__).toContain(`classList.add("${COMPANION_HIDDEN_CLASS}")`)

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
    runBootstrap(true)
    expect(isCompanionHidden()).toBe(true)
  })
})
