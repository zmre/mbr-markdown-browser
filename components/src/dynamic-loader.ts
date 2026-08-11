/**
 * Dynamic loader utilities for on-demand loading of CSS and JS resources.
 * Used by mbr-hljs, mbr-mermaid, and mbr-katex components.
 */
import { getBasePath } from './shared.ts'

/**
 * Wait for DOM to be ready.
 * Resolves immediately if DOM is already loaded.
 */
export function waitForDom(): Promise<void> {
  if (document.readyState !== 'loading') {
    return Promise.resolve()
  }
  return new Promise((resolve) => {
    document.addEventListener('DOMContentLoaded', () => resolve(), { once: true })
  })
}

/**
 * Get the base URL for .mbr assets, handling both server and static modes.
 * Server mode: '/.mbr/'
 * Static mode: './.mbr/' or '../../.mbr/' depending on page depth
 */
export function getMbrAssetBase(): string {
  const base = getBasePath()
  // In server mode, base is '' so we use absolute path
  // In static mode, base is './' or '../' etc, so we append .mbr/
  return base ? `${base}.mbr/` : '/.mbr/'
}

/**
 * Dynamically load a JavaScript file.
 * Returns a promise that resolves when the script is loaded.
 * If the script is already loaded (by src match), resolves immediately.
 *
 * @param src - The script URL to load
 * @param integrity - Optional SRI hash for security
 */
export function loadScript(src: string, integrity?: string): Promise<void> {
  return new Promise((resolve, reject) => {
    // Check if already loaded
    if (document.querySelector(`script[src="${src}"]`)) {
      resolve()
      return
    }

    const script = document.createElement('script')
    script.src = src
    // Don't use defer - it causes onload to fire before execution for dynamically inserted scripts
    if (integrity) {
      script.integrity = integrity
      script.crossOrigin = 'anonymous'
    }
    script.onload = () => resolve()
    script.onerror = () => reject(new Error(`Failed to load script: ${src}`))
    document.head.appendChild(script)
  })
}

/**
 * Dynamically load a CSS stylesheet.
 * Returns a promise that resolves when the stylesheet is loaded.
 * If the stylesheet is already loaded (by href match), resolves immediately.
 *
 * @param href - The stylesheet URL to load
 * @param integrity - Optional SRI hash for security
 */
export function loadCss(href: string, integrity?: string): Promise<void> {
  return new Promise((resolve, reject) => {
    // Check if already loaded - browsers normalize href to absolute URL
    // so check both the original and any existing link with matching end
    const existingLinks = document.querySelectorAll('link[rel="stylesheet"]')
    for (const existing of existingLinks) {
      const existingHref = existing.getAttribute('href') || ''
      if (existingHref === href || existingHref.endsWith(href)) {
        resolve()
        return
      }
    }

    const link = document.createElement('link')
    link.rel = 'stylesheet'
    link.href = href
    if (integrity) {
      link.integrity = integrity
      link.crossOrigin = 'anonymous'
    }
    link.onload = () => resolve()
    link.onerror = (e) => {
      console.error('[loadCss] Failed to load:', href, e)
      reject(new Error(`Failed to load CSS: ${href}`))
    }
    // Insert before user.css rather than appending: user.css is the per-repo
    // override and must have the last word. Appending to <head> put reveal.css,
    // katex.min.css and hljs.atom-one-dark.css AFTER it, so at equal specificity
    // they outranked the user's rules. Final order: pico -> theme -> dynamic -> user.
    // The fallback is load-bearing, not defensive padding: `.mbr/_head.html` is a
    // documented per-repo override, so a custom template may not carry the id.
    // Appending there preserves today's behaviour instead of throwing.
    const anchor = document.getElementById('mbr-user-css')
    if (anchor) anchor.before(link)
    else document.head.appendChild(link)
  })
}

/**
 * Schedule a task to run during browser idle time.
 * Falls back to setTimeout(0) if requestIdleCallback is not available.
 * This allows the page to become interactive faster by deferring non-critical work.
 *
 * @param task - The function to execute during idle time
 * @param timeout - Maximum time to wait before forcing execution (default: 2000ms)
 */
export function scheduleIdleTask(task: () => void, timeout = 2000): void {
  if ('requestIdleCallback' in window) {
    requestIdleCallback(() => task(), { timeout })
  } else {
    setTimeout(task, 0)
  }
}
