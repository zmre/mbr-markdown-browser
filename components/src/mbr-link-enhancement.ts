import { LitElement } from 'lit'
import { customElement } from 'lit/decorators.js'
import { isGuiMode } from './shared.ts'

/** CSS class added to links enhanced by this component */
const ENHANCED_CLASS = 'mbr-link-enhanced'

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

    // Wait for DOM to be ready before enhancing links
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', () => this._enhanceLinks())
    } else {
      // DOM already loaded
      this._enhanceLinks()
    }
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
}
