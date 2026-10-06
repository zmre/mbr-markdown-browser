/**
 * `<mbr-contact-backlinks>` — the contact card's "Linked from N notes" chip.
 *
 * Emitted by `templates/_contact_card.html` on person/organization pages only.
 * Two shapes arrive from the server:
 *
 * - `<mbr-contact-backlinks count="3"><button>…</button></mbr-contact-backlinks>`
 *   when the server already knew the count (its inbound index was ready — a
 *   hash lookup). The markup is final; this element only wires the click.
 * - `<mbr-contact-backlinks></mbr-contact-backlinks>` when it did not (static
 *   builds, or a server still building its index). The count is then read from
 *   this page's own `links.json` on idle — the same cached fetch the info panel
 *   makes, so opening the panel afterwards costs nothing — and the chip appears
 *   only if there is at least one backlink.
 *
 * Deliberately not a Lit element: it is a few lines of light-DOM work in the
 * main bundle, and nothing here re-renders.
 */
import { scheduleIdleTask } from './dynamic-loader.js'
import { fetchPageLinks } from './graph/links-cache.js'
import type { PageLinks } from './graph/relationship-graph.js'
import { getCanonicalPath } from './shared.js'

/** Distinct pages linking here (a page may link several times). */
export function countLinkingNotes(links: PageLinks | null): number {
  if (!links || !Array.isArray(links.inbound)) return 0
  return new Set(links.inbound.map((link) => link.from)).size
}

/** Mirrors the server-rendered label in `_contact_card.html`. */
export function backlinkLabel(count: number): string {
  return `Linked from ${count} note${count === 1 ? '' : 's'}`
}

/** Opens `<mbr-info>`, whose Links section lists the backlinks. */
export function openInfoPanel(): void {
  const info = document.querySelector<HTMLElement & { open?: () => void }>('mbr-info')
  info?.open?.()
}

export class MbrContactBacklinksElement extends HTMLElement {
  connectedCallback(): void {
    this.addEventListener('click', this._onClick)
    if (!this.hasAttribute('count')) {
      scheduleIdleTask(() => void this._fill())
    }
  }

  disconnectedCallback(): void {
    this.removeEventListener('click', this._onClick)
  }

  private _onClick = (e: Event): void => {
    if ((e.target as Element | null)?.closest('button')) openInfoPanel()
  }

  private async _fill(): Promise<void> {
    const count = countLinkingNotes(await fetchPageLinks(getCanonicalPath()))
    if (!this.isConnected || count === 0) return
    const button = document.createElement('button')
    button.type = 'button'
    button.className = 'mbr-contact-card-chip'
    button.textContent = backlinkLabel(count)
    this.setAttribute('count', String(count))
    this.replaceChildren(button)
  }
}

if (!customElements.get('mbr-contact-backlinks')) {
  customElements.define('mbr-contact-backlinks', MbrContactBacklinksElement)
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-contact-backlinks': MbrContactBacklinksElement
  }
}
