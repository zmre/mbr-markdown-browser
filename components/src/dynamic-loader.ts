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
    script.defer = true
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
    // Check if already loaded
    if (document.querySelector(`link[href="${href}"]`)) {
      resolve()
      return
    }

    const link = document.createElement('link')
    link.rel = 'stylesheet'
    link.href = href
    if (integrity) {
      link.integrity = integrity
      link.crossOrigin = 'anonymous'
    }
    link.onload = () => resolve()
    link.onerror = () => reject(new Error(`Failed to load CSS: ${href}`))
    document.head.appendChild(link)
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
