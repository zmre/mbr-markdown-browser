/**
 * `<mbr-not-found-suggestions>` — on a 404 page, where the missing page
 * probably went (a note of the same name elsewhere, a renamed one, else the
 * notes left in its folder).
 *
 * Emitted by `templates/_not_found_suggestions.html`, which `error.html`
 * includes for a 404 only. Works wherever the page does — server, GUI, the
 * iOS app and a static build's `404.html`, which a host serves at the missing
 * URL — because everything comes from site.json in the browser.
 *
 * MAIN BUNDLE, kept tiny and Lit-free: it only gathers the inputs (the
 * canonical path, site.json, media.json when the URL names a file) and hands
 * them to the `mbr-not-found.min.js` chunk, which matches and draws into this
 * element's light DOM. Renders nothing until there is something to show.
 */
import { loadNotFoundChunk } from './not-found-chunk.js'
import { getCanonicalPath, loadMediaNav, resolveUrl, siteNav } from './shared.js'
import type { SiteOtherFile } from './not-found/match.js'

export class MbrNotFoundSuggestionsElement extends HTMLElement {
  private _started = false

  connectedCallback(): void {
    if (this._started) return
    this._started = true
    void this._fill()
  }

  private async _fill(): Promise<void> {
    try {
      const path = getCanonicalPath()
      const [chunk, site] = await Promise.all([loadNotFoundChunk(), siteNav])
      if (!chunk || !site || !this.isConnected) return
      let otherFiles: SiteOtherFile[] = []
      if (chunk.namesAFile(path, site)) {
        // Only for a URL naming a file (`report.pdf`): media.json is the bulk
        // of the site's metadata, and no note match needs it.
        otherFiles = await loadMediaNav()
          .then((media): SiteOtherFile[] => (Array.isArray(media?.other_files) ? media.other_files : []))
          .catch((): SiteOtherFile[] => [])
      }
      if (!this.isConnected) return
      chunk.renderNotFound(this, chunk.suggestForMissing(path, site, otherFiles), resolveUrl)
    } catch (err) {
      // Informational only: a failed site.json leaves today's plain 404.
      console.warn('[mbr-not-found-suggestions] no suggestions:', err)
    }
  }
}

if (!customElements.get('mbr-not-found-suggestions')) {
  customElements.define('mbr-not-found-suggestions', MbrNotFoundSuggestionsElement)
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-not-found-suggestions': MbrNotFoundSuggestionsElement
  }
}
