import { LitElement } from 'lit'
import { customElement } from 'lit/decorators.js'
import { isGuiMode } from './shared.ts'

/** CSS class added to links enhanced by this component */
const ENHANCED_CLASS = 'mbr-link-enhanced'

/**
 * Prefix of the IPC message that asks Rust to open a URL with the system
 * default handler. Must stay identical to `IPC_OPEN_EXTERNAL_PREFIX` in
 * `src/external_open.rs`, which is the only reader.
 */
const IPC_OPEN_EXTERNAL_PREFIX = 'mbr:open-external:'

/**
 * The resolved URL a click should hand to the operating system, or `null` to
 * leave the click alone.
 *
 * Only cross-origin `http(s)` qualifies. Everything else is somebody else's
 * job, and getting that division wrong is how embeds break:
 *
 * - Application schemes (`mailto:`, `message:`, `zoommtg:`) are handled by the
 *   Rust navigation handler, which is safe to act on them because an `<iframe>`
 *   can never navigate to one. See `decide_without_frame_info` in
 *   `src/external_open.rs`.
 * - Cross-origin `http(s)` is *not* safe there — wry's handler receives a bare
 *   URL and is called for frame loads too, so cancelling would blank YouTube
 *   embeds. This listener is the piece that knows a click from an embed.
 * - `target="_blank"` goes to wry's new-window handler, which applies the full
 *   origin-aware policy itself.
 *
 * The checks below mirror what a browser does before following a link, so a
 * modifier-click or a middle-click still behaves the way the user expects.
 */
export function externalHrefForClick(event: MouseEvent): string | null {
  // Something else already claimed this click.
  if (event.defaultPrevented) return null

  // Left button only. Middle-click and right-click are the browser's.
  if (event.button !== 0) return null

  // Cmd/Ctrl/Shift/Alt all mean "not a plain navigation" to a browser.
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return null

  const anchor = anchorFromEvent(event)
  if (!anchor) return null

  // Downloads and explicit targets are handled elsewhere (or by the webview).
  if (anchor.hasAttribute('download')) return null
  const target = anchor.getAttribute('target')
  if (target && target !== '_self') return null

  // In-page anchors never leave the document.
  const href = anchor.getAttribute('href')
  if (!href || href.startsWith('#')) return null

  let url: URL
  try {
    // `anchor.href` is the resolved absolute URL, which is what the OS needs;
    // the raw attribute is usually relative.
    url = new URL(anchor.href)
  } catch {
    return null
  }

  if (url.protocol !== 'http:' && url.protocol !== 'https:') return null
  if (url.origin === window.location.origin) return null

  return anchor.href
}

/**
 * Find the anchor a click landed on, crossing shadow boundaries.
 *
 * `composedPath()` rather than `target.closest()` because a link rendered
 * inside a component's shadow root would otherwise report the host element.
 */
function anchorFromEvent(event: MouseEvent): HTMLAnchorElement | null {
  const path = typeof event.composedPath === 'function' ? event.composedPath() : []

  for (const node of path) {
    if (node instanceof HTMLAnchorElement && node.hasAttribute('href')) return node
  }

  const target = event.target
  return target instanceof Element ? target.closest('a[href]') : null
}

/**
 * Ask the host to open `url` with the system default handler.
 *
 * Returns whether the message was actually posted. It is not posted when
 * `window.ipc` is absent — the bundle also loads under a plain browser during
 * development and in tests — and the caller must then leave the click alone
 * rather than swallow it.
 */
function postOpenExternal(url: string): boolean {
  const ipc = window.ipc
  if (!ipc || typeof ipc.postMessage !== 'function') return false

  try {
    ipc.postMessage(IPC_OPEN_EXTERNAL_PREFIX + url)
    return true
  } catch {
    return false
  }
}

/**
 * Click handler: hand cross-origin links to the operating system.
 *
 * Exported for tests. `preventDefault()` happens only after the message is
 * away, so a failure to reach the host degrades to the previous behaviour
 * (the link opens in the mbr window) instead of a link that does nothing.
 */
export function handleExternalLinkClick(event: MouseEvent): void {
  const url = externalHrefForClick(event)
  if (!url) return
  if (!postOpenExternal(url)) return

  event.preventDefault()
}

/**
 * Link enhancement component for GUI mode.
 *
 * In GUI mode (native window), there's no browser URL bar, so users can't
 * see link destinations by hovering. This component adds Pico CSS tooltips
 * to all links in the main content area, showing the destination URL.
 *
 * - For same-origin links: shows just the path (e.g., "/docs/guide/")
 * - For external links: shows the full URL (e.g., "https://example.com/page")
 *
 * Links get the 'mbr-link-enhanced' class for styling (see theme.css).
 *
 * This component does nothing in server mode or static builds where
 * users have access to the browser URL bar.
 */
@customElement('mbr-link-enhancement')
export class MbrLinkEnhancementElement extends LitElement {
  override connectedCallback() {
    super.connectedCallback()

    // Only enhance links in GUI mode
    if (!isGuiMode()) {
      return
    }

    // One delegated listener for the whole document, not per link: it has to
    // cover the nav, footer and anything a component renders later.
    //
    // The isGuiMode() check above is load-bearing, not belt-and-braces, even
    // though mbr's own templates now mount this element only behind a gui_mode
    // gate. Templates are not ours to rely on: `.mbr/` overrides are a headline
    // feature of this project, and a repository can ship its own
    // _display_enhancements.html or _footer.html — very plausibly copied from an
    // older mbr, where this element WAS mounted ungated — and mbr will use it in
    // preference to the built-in. So the template gate decides whether the
    // element is constructed, and this early return decides whether it does
    // anything. Only the second is under our control.
    //
    // Server mode and static builds must not have this listener: they run in a
    // real browser, which already opens https:// itself and hands message:// and
    // friends to the OS. Worse, the listener's whole purpose is to ask the *host
    // process* to launch an application, which a machine that is merely serving
    // markdown must never be asked to do.
    //
    // A repo could also mount the element twice. addEventListener dedups on
    // (type, listener, capture) and handleExternalLinkClick is a module-level
    // function, so exactly one listener is registered and a click cannot open
    // the URL twice.
    document.addEventListener('click', handleExternalLinkClick)

    // Wait for DOM to be ready before enhancing links
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', () => this._enhanceLinks())
    } else {
      // DOM already loaded
      this._enhanceLinks()
    }
  }

  override disconnectedCallback() {
    super.disconnectedCallback()
    document.removeEventListener('click', handleExternalLinkClick)
  }

  /**
   * Add data-tooltip attributes to all links in <main>.
   */
  private _enhanceLinks(): void {
    const main = document.querySelector('main')
    if (!main) return

    const links = main.querySelectorAll('a[href]')
    const currentOrigin = window.location.origin

    links.forEach((link) => {
      const anchor = link as HTMLAnchorElement

      // Skip links that already have tooltips
      if (anchor.hasAttribute('data-tooltip')) return

      // Skip anchor links (internal page navigation)
      const href = anchor.getAttribute('href')
      if (!href || href.startsWith('#')) return

      // Determine the tooltip text
      const tooltipText = this._getTooltipText(anchor, currentOrigin)
      if (tooltipText) {
        anchor.setAttribute('data-tooltip', tooltipText)
        anchor.classList.add(ENHANCED_CLASS)
      }
    })
  }

  /**
   * Get the tooltip text for a link.
   * Returns just the path for same-origin links, full URL for external.
   */
  private _getTooltipText(anchor: HTMLAnchorElement, currentOrigin: string): string | null {
    try {
      // Use the anchor's resolved href (handles relative URLs)
      const url = new URL(anchor.href)

      // Same origin - show just the path
      if (url.origin === currentOrigin) {
        // Include search params and hash if present
        let path = url.pathname
        if (url.search) path += url.search
        if (url.hash) path += url.hash
        return path
      }

      // External link - show full URL
      return anchor.href
    } catch {
      // Invalid URL, just return the raw href
      return anchor.getAttribute('href')
    }
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-link-enhancement': MbrLinkEnhancementElement
  }

  interface Window {
    /**
     * wry's host channel, injected into the page only in GUI mode. Optional
     * because the same bundle runs in server mode and static builds, where
     * nothing is listening.
     */
    ipc?: { postMessage(message: string): void }
  }
}
