import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { getMbrAssetBase, loadCss, loadScript, scheduleIdleTask, waitForDom } from './dynamic-loader.ts'

/**
 * Tests for the shared on-demand asset loader used by <mbr-hljs>, <mbr-katex>,
 * <mbr-mermaid>, <mbr-slides> and the lazy graph/genealogy chunks.
 *
 * The interesting behaviour here is the "already loaded?" short-circuit: every
 * caller of `loadScript`/`loadCss` treats a resolved promise as "the global is
 * now on `window`", so the conditions under which the loader resolves WITHOUT
 * having loaded anything are the contract worth pinning down.
 */

// ============================================================================
// happy-dom resource control
// ============================================================================

/**
 * happy-dom genuinely tries to fetch a `<script src>` / `<link href>` the moment
 * it is appended, and dispatches load/error synchronously from that insertion.
 * These settings switch the fetch off and let each test pick the outcome, so
 * both loader branches are driven deterministically and without a network.
 */
interface ResourceLoadingSettings {
  disableJavaScriptFileLoading: boolean
  disableCSSFileLoading: boolean
  handleDisabledFileLoadingAsSuccess: boolean
}

const settings = (window as unknown as { happyDOM: { settings: ResourceLoadingSettings } }).happyDOM.settings
const originalSettings: ResourceLoadingSettings = { ...settings }

/** Make every subsequently inserted script/stylesheet succeed or fail. */
function resourcesLoad(succeed: boolean): void {
  settings.disableJavaScriptFileLoading = true
  settings.disableCSSFileLoading = true
  settings.handleDisabledFileLoadingAsSuccess = succeed
}

function scriptTags(src: string): HTMLScriptElement[] {
  return Array.from(document.querySelectorAll<HTMLScriptElement>(`script[src="${src}"]`))
}

function linkTags(href: string): HTMLLinkElement[] {
  return Array.from(document.querySelectorAll<HTMLLinkElement>(`link[href="${href}"]`))
}

/** Drop every script/stylesheet a previous test injected. */
function clearInjectedAssets(): void {
  document.querySelectorAll('script[src], link[rel="stylesheet"]').forEach((node) => node.remove())
}

/**
 * Emit the user.css link exactly as `templates/_head.html` does — it is the
 * anchor `loadCss` inserts in front of. Cleared by `clearInjectedAssets`.
 */
function addUserCssAnchor(): HTMLLinkElement {
  const anchor = document.createElement('link')
  anchor.id = 'mbr-user-css'
  anchor.rel = 'stylesheet'
  anchor.setAttribute('href', '/.mbr/user.css')
  document.head.appendChild(anchor)
  return anchor
}

/** Position of a node among <head>'s element children, for cascade assertions. */
function headIndex(node: Element): number {
  return Array.from(document.head.children).indexOf(node)
}

// ============================================================================
// getMbrAssetBase
// ============================================================================

describe('getMbrAssetBase', () => {
  const originalConfig = window.__MBR_CONFIG__

  afterEach(() => {
    window.__MBR_CONFIG__ = originalConfig
  })

  it('uses a root-absolute path in server mode', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    expect(getMbrAssetBase()).toBe('/.mbr/')
  })

  it('ignores basePath in server mode', () => {
    // basePath is only meaningful for a static build; the server always serves
    // /.mbr/ from the root, so a stale basePath must not leak into the URL.
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, basePath: '../../' }
    expect(getMbrAssetBase()).toBe('/.mbr/')
  })

  it('stays page-relative at the root of a static build', () => {
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: './' }
    expect(getMbrAssetBase()).toBe('./.mbr/')
  })

  it('walks back up to the site root from a nested static page', () => {
    // The case a static deployment actually hits: /docs/guide/index.html has to
    // reach ../../.mbr/, not /.mbr/, because the site may be served under a
    // path prefix (GitHub Pages project site) where /.mbr/ does not exist.
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../../' }
    expect(getMbrAssetBase()).toBe('../../.mbr/')
  })

  it('handles a single-level static page', () => {
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../' }
    expect(getMbrAssetBase()).toBe('../.mbr/')
  })

  it('falls back to a page-relative base when basePath is empty or absent', () => {
    // An empty basePath is falsy, so getBasePath() substitutes './'. Without
    // that fallback the URL would collapse to a bare '.mbr/...'.
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '' }
    expect(getMbrAssetBase()).toBe('./.mbr/')

    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false }
    expect(getMbrAssetBase()).toBe('./.mbr/')
  })

  it('assumes a static page when there is no config at all', () => {
    window.__MBR_CONFIG__ = undefined
    expect(getMbrAssetBase()).toBe('./.mbr/')
  })
})

// ============================================================================
// waitForDom
// ============================================================================

describe('waitForDom', () => {
  /** Shadow the read-only document.readyState getter for one test. */
  function setReadyState(state: DocumentReadyState): void {
    Object.defineProperty(document, 'readyState', { value: state, configurable: true })
  }

  afterEach(() => {
    delete (document as unknown as { readyState?: DocumentReadyState }).readyState
  })

  it('resolves without waiting for an event once the document is complete', async () => {
    setReadyState('complete')
    let resolved = false
    void waitForDom().then(() => {
      resolved = true
    })

    await Promise.resolve()
    expect(resolved).toBe(true)
  })

  it('resolves immediately at readyState "interactive"', async () => {
    // "interactive" already means DOMContentLoaded has fired, so waiting for
    // the event would hang forever.
    setReadyState('interactive')
    await expect(waitForDom()).resolves.toBeUndefined()
  })

  it('waits for DOMContentLoaded while the document is still loading', async () => {
    setReadyState('loading')
    let resolved = false
    const pending = waitForDom().then(() => {
      resolved = true
    })

    await new Promise((r) => setTimeout(r, 0))
    expect(resolved).toBe(false)

    document.dispatchEvent(new Event('DOMContentLoaded'))
    await pending
    expect(resolved).toBe(true)
  })

  it('releases every pending caller on a single DOMContentLoaded', async () => {
    setReadyState('loading')
    const all = Promise.all([waitForDom(), waitForDom(), waitForDom()])

    document.dispatchEvent(new Event('DOMContentLoaded'))
    await expect(all).resolves.toEqual([undefined, undefined, undefined])
  })

  it('does not re-fire on a second DOMContentLoaded (listener is once-only)', async () => {
    setReadyState('loading')
    let resolutions = 0
    const pending = waitForDom().then(() => {
      resolutions++
    })

    document.dispatchEvent(new Event('DOMContentLoaded'))
    document.dispatchEvent(new Event('DOMContentLoaded'))
    await pending

    expect(resolutions).toBe(1)
  })
})

// ============================================================================
// loadScript
// ============================================================================

describe('loadScript', () => {
  beforeEach(() => {
    clearInjectedAssets()
    resourcesLoad(true)
  })

  afterEach(() => {
    clearInjectedAssets()
    Object.assign(settings, originalSettings)
  })

  it('injects the script into <head> and resolves when it loads', async () => {
    await expect(loadScript('/.mbr/mermaid.min.js')).resolves.toBeUndefined()

    const tags = scriptTags('/.mbr/mermaid.min.js')
    expect(tags).toHaveLength(1)
    expect(tags[0].parentElement).toBe(document.head)
  })

  it('does not mark the script as deferred', () => {
    // Deliberate: a deferred, dynamically inserted script fires onload before it
    // has executed, so callers would touch `window.mermaid` before it exists.
    void loadScript('/.mbr/hljs.js')
    expect(scriptTags('/.mbr/hljs.js')[0].defer).toBe(false)
  })

  it('applies SRI attributes only when an integrity hash is supplied', async () => {
    await loadScript('https://cdn.example.com/lang.min.js', 'sha384-abc123')
    const withSri = scriptTags('https://cdn.example.com/lang.min.js')[0]
    expect(withSri.getAttribute('integrity')).toBe('sha384-abc123')
    expect(withSri.getAttribute('crossorigin')).toBe('anonymous')

    await loadScript('/.mbr/katex.min.js')
    const withoutSri = scriptTags('/.mbr/katex.min.js')[0]
    expect(withoutSri.getAttribute('integrity')).toBeNull()
    expect(withoutSri.getAttribute('crossorigin')).toBeNull()
  })

  it('rejects with the failing URL when the script cannot be fetched', async () => {
    resourcesLoad(false)
    await expect(loadScript('/.mbr/missing.js')).rejects.toThrow('Failed to load script: /.mbr/missing.js')
  })

  it('requests a given script only once', async () => {
    await loadScript('/.mbr/reveal.js')
    await expect(loadScript('/.mbr/reveal.js')).resolves.toBeUndefined()

    expect(scriptTags('/.mbr/reveal.js')).toHaveLength(1)
  })

  it('treats scripts already present in the page as loaded', async () => {
    // Templates emit some of these tags themselves; the loader must not append
    // a duplicate on top of one the page already has.
    const existing = document.createElement('script')
    existing.src = '/.mbr/components/mbr-graph.min.js'
    document.head.appendChild(existing)

    await expect(loadScript('/.mbr/components/mbr-graph.min.js')).resolves.toBeUndefined()
    expect(scriptTags('/.mbr/components/mbr-graph.min.js')).toHaveLength(1)
  })

  it('distinguishes scripts by exact src', async () => {
    await loadScript('/.mbr/hljs.lang.rust.js')
    await loadScript('/.mbr/hljs.lang.ruby.js')

    expect(scriptTags('/.mbr/hljs.lang.rust.js')).toHaveLength(1)
    expect(scriptTags('/.mbr/hljs.lang.ruby.js')).toHaveLength(1)
  })

  /**
   * DEFECT (characterised, not fixed): the short-circuit is a pure DOM-presence
   * check, so a `<script>` tag that FAILED to load still counts as loaded. The
   * failed tag is never removed, so every later caller for that URL gets a
   * resolved promise and then dereferences a global that was never defined.
   *
   * The same check makes a second, concurrent caller resolve while the first
   * request is still in flight.
   *
   * When this is fixed (track in-flight/failed URLs in a Map instead of reading
   * the DOM), the retry below should re-request and resolve for real — flip the
   * tag-count assertion to 2 (or 1 after the dead tag is removed) at that point.
   */
  it('DEFECT: a retry after a failed load resolves without re-requesting the script', async () => {
    resourcesLoad(false)
    await expect(loadScript('/.mbr/flaky.js')).rejects.toThrow('Failed to load script: /.mbr/flaky.js')

    // Even with the network healthy again, the dead tag short-circuits the retry.
    resourcesLoad(true)
    await expect(loadScript('/.mbr/flaky.js')).resolves.toBeUndefined()
    expect(scriptTags('/.mbr/flaky.js')).toHaveLength(1)
  })
})

// ============================================================================
// loadCss
// ============================================================================

describe('loadCss', () => {
  beforeEach(() => {
    clearInjectedAssets()
    resourcesLoad(true)
  })

  afterEach(() => {
    clearInjectedAssets()
    Object.assign(settings, originalSettings)
  })

  it('injects a stylesheet link into <head> and resolves when it loads', async () => {
    await expect(loadCss('/.mbr/katex.min.css')).resolves.toBeUndefined()

    const tags = linkTags('/.mbr/katex.min.css')
    expect(tags).toHaveLength(1)
    expect(tags[0].rel).toBe('stylesheet')
    expect(tags[0].parentElement).toBe(document.head)
  })

  it('applies SRI attributes only when an integrity hash is supplied', async () => {
    // Asserted on the IDL properties rather than the attributes: happy-dom does
    // not reflect HTMLLinkElement.integrity to an attribute the way browsers do.
    await loadCss('https://cdn.example.com/theme.css', 'sha384-css')
    const withSri = linkTags('https://cdn.example.com/theme.css')[0]
    expect(withSri.integrity).toBe('sha384-css')
    expect(withSri.crossOrigin).toBe('anonymous')

    await loadCss('/.mbr/reveal.css')
    const withoutSri = linkTags('/.mbr/reveal.css')[0]
    expect(withoutSri.integrity).toBeFalsy()
    expect(withoutSri.crossOrigin).toBeFalsy()
  })

  it('rejects with the failing URL when the stylesheet cannot be fetched', async () => {
    resourcesLoad(false)
    await expect(loadCss('/.mbr/missing.css')).rejects.toThrow('Failed to load CSS: /.mbr/missing.css')
  })

  it('requests a given stylesheet only once', async () => {
    await loadCss('/.mbr/reveal-slides.css')
    await expect(loadCss('/.mbr/reveal-slides.css')).resolves.toBeUndefined()

    expect(linkTags('/.mbr/reveal-slides.css')).toHaveLength(1)
  })

  it('matches a page-relative request against the absolute href a browser reports', async () => {
    // The reason the check is a suffix match: a template emits
    // <link href="/.mbr/theme.css"> while a static-mode component asks for
    // './.mbr/theme.css'. Both must count as the same stylesheet.
    const existing = document.createElement('link')
    existing.rel = 'stylesheet'
    existing.setAttribute('href', '/.mbr/hljs.atom-one-dark.css')
    document.head.appendChild(existing)

    await expect(loadCss('.mbr/hljs.atom-one-dark.css')).resolves.toBeUndefined()
    expect(document.querySelectorAll('link[rel="stylesheet"]')).toHaveLength(1)
  })

  it('ignores non-stylesheet links with the same href', async () => {
    // _head_markdown.html emits <link rel="prefetch"> for the lazy chunks; a
    // prefetch hint is not a loaded stylesheet and must not suppress the load.
    const prefetch = document.createElement('link')
    prefetch.rel = 'prefetch'
    prefetch.setAttribute('href', '/.mbr/reveal-theme-blank.css')
    document.head.appendChild(prefetch)

    await loadCss('/.mbr/reveal-theme-blank.css')
    expect(document.querySelectorAll('link[rel="stylesheet"]')).toHaveLength(1)
  })

  it('distinguishes stylesheets by href', async () => {
    await loadCss('/.mbr/reveal.css')
    await loadCss('/.mbr/reveal-slides.css')

    expect(document.querySelectorAll('link[rel="stylesheet"]')).toHaveLength(2)
  })

  /**
   * Cascade order. `_head.html` emits pico -> theme -> user, and user.css is the
   * per-repo override that must have the last word. Appending to <head> put every
   * on-demand stylesheet AFTER user.css, so reveal.css / katex.min.css /
   * hljs.atom-one-dark.css beat the user's rules at equal specificity.
   */
  it('inserts the stylesheet before user.css so per-repo overrides still win', async () => {
    const anchor = addUserCssAnchor()

    await loadCss('/.mbr/hljs.atom-one-dark.css')

    const injected = linkTags('/.mbr/hljs.atom-one-dark.css')[0]
    expect(injected.parentElement).toBe(document.head)
    expect(headIndex(injected)).toBeLessThan(headIndex(anchor))
  })

  it('keeps every dynamic stylesheet ahead of user.css, in load order', async () => {
    // Among themselves the on-demand sheets must still cascade in the order they
    // were requested — inserting at a fixed anchor must not reverse them.
    const anchor = addUserCssAnchor()

    await loadCss('/.mbr/reveal.css')
    await loadCss('/.mbr/katex.min.css')

    const first = linkTags('/.mbr/reveal.css')[0]
    const second = linkTags('/.mbr/katex.min.css')[0]
    expect(headIndex(first)).toBeLessThan(headIndex(second))
    expect(headIndex(second)).toBeLessThan(headIndex(anchor))
  })

  it('appends to <head> when the page has no user.css anchor', async () => {
    // `.mbr/_head.html` is a documented per-repo override, so a custom template
    // may not carry the id. That must preserve the old behaviour, not throw.
    expect(document.getElementById('mbr-user-css')).toBeNull()

    await expect(loadCss('/.mbr/katex.min.css')).resolves.toBeUndefined()

    const injected = linkTags('/.mbr/katex.min.css')[0]
    expect(injected.parentElement).toBe(document.head)
    expect(document.head.lastElementChild).toBe(injected)
  })

  it('still short-circuits a duplicate request when the anchor is present', async () => {
    // The anchor changes where the link lands, not whether the dedupe scan runs.
    const anchor = addUserCssAnchor()

    await loadCss('/.mbr/reveal-slides.css')
    await expect(loadCss('/.mbr/reveal-slides.css')).resolves.toBeUndefined()

    const tags = linkTags('/.mbr/reveal-slides.css')
    expect(tags).toHaveLength(1)
    expect(headIndex(tags[0])).toBeLessThan(headIndex(anchor))
  })

  it('does not treat the user.css anchor itself as an already-loaded match', async () => {
    // Guards the dedupe's suffix test against the one stylesheet now guaranteed
    // to be in <head>: user.css must not suppress an unrelated on-demand sheet.
    addUserCssAnchor()

    await loadCss('/.mbr/reveal.css')

    expect(linkTags('/.mbr/reveal.css')).toHaveLength(1)
  })

  /**
   * DEFECT (characterised, not fixed): the dedupe uses `existingHref.endsWith(href)`
   * with no separator check, so an unrelated stylesheet whose filename merely
   * ENDS WITH the requested one suppresses the load entirely — the caller gets a
   * resolved promise for CSS that was never fetched.
   *
   * A repo that ships `.mbr/user.css` and a component asking for `er.css` is
   * contrived, but `theme.css` vs a custom `dark-theme.css` is not: any repo
   * with a `.mbr/` stylesheet ending in the requested name silently wins.
   *
   * Fix: compare full URLs (`new URL(href, document.baseURI).href`) instead of a
   * bare suffix test.
   */
  it('DEFECT: an unrelated stylesheet whose href ends with the requested name suppresses the load', async () => {
    const unrelated = document.createElement('link')
    unrelated.rel = 'stylesheet'
    unrelated.setAttribute('href', '/.mbr/dark-theme.css')
    document.head.appendChild(unrelated)

    await expect(loadCss('theme.css')).resolves.toBeUndefined()
    expect(linkTags('theme.css')).toHaveLength(0)
  })

  /** DEFECT: same presence-only short-circuit as loadScript — see above. */
  it('DEFECT: a retry after a failed load resolves without re-requesting the stylesheet', async () => {
    resourcesLoad(false)
    await expect(loadCss('/.mbr/flaky.css')).rejects.toThrow('Failed to load CSS: /.mbr/flaky.css')

    resourcesLoad(true)
    await expect(loadCss('/.mbr/flaky.css')).resolves.toBeUndefined()
    expect(linkTags('/.mbr/flaky.css')).toHaveLength(1)
  })
})

// ============================================================================
// scheduleIdleTask
// ============================================================================

describe('scheduleIdleTask', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('never runs the task synchronously', () => {
    // The whole point is to keep work off the critical path to interactive.
    const task = vi.fn()
    scheduleIdleTask(task)
    expect(task).not.toHaveBeenCalled()
  })

  it('falls back to a timeout when requestIdleCallback is unavailable', () => {
    // happy-dom implements no requestIdleCallback, which is exactly the branch
    // Safari <18 takes in production.
    const task = vi.fn()
    scheduleIdleTask(task)

    vi.advanceTimersByTime(0)
    expect(task).toHaveBeenCalledTimes(1)
  })

  it('prefers requestIdleCallback and defaults its timeout to 2000ms', () => {
    const ric = vi.fn()
    vi.stubGlobal('requestIdleCallback', ric)
    const task = vi.fn()

    scheduleIdleTask(task)

    expect(ric).toHaveBeenCalledTimes(1)
    expect(ric.mock.calls[0][1]).toEqual({ timeout: 2000 })
    // Nothing scheduled on the timer queue: the idle callback owns the task.
    expect(vi.getTimerCount()).toBe(0)

    ric.mock.calls[0][0]()
    expect(task).toHaveBeenCalledTimes(1)
  })

  it('passes a custom timeout through to requestIdleCallback', () => {
    const ric = vi.fn()
    vi.stubGlobal('requestIdleCallback', ric)

    scheduleIdleTask(() => {}, 500)

    expect(ric.mock.calls[0][1]).toEqual({ timeout: 500 })
  })
})
