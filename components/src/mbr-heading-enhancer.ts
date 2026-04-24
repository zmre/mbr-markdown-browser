/**
 * Collapsible headings + permalink anchors.
 *
 * After the page renders, walks markdown headings in <main> and:
 * 1. Appends a permalink `#` anchor to any heading that has an id.
 * 2. Makes non-link headings clickable to collapse following sibling content
 *    up to the next heading of the same or higher level.
 *
 * Enhancement runs during browser idle time to avoid blocking first paint.
 * Idempotent via the `mbr-heading-enhanced` marker class.
 */
import { LitElement, nothing } from 'lit'
import { customElement } from 'lit/decorators.js'
import { waitForDom, scheduleIdleTask } from './dynamic-loader.ts'

const ENHANCED_CLASS = 'mbr-heading-enhanced'
const COLLAPSIBLE_CLASS = 'mbr-collapsible'
const COLLAPSED_CLASS = 'mbr-section-collapsed'
const HIDDEN_CLASS = 'sectionhidden'
const ANCHOR_CLASS = 'mbr-heading-anchor'

/**
 * Toggle the collapsed state of a heading, hiding or revealing following
 * siblings until the next heading of the same or higher level.
 *
 * Exported for unit testing.
 */
export function toggleCollapse(heading: HTMLElement): void {
  const currentLevel = parseInt(heading.tagName.charAt(1), 10)
  const willCollapse = !heading.classList.contains(COLLAPSED_CLASS)
  heading.classList.toggle(COLLAPSED_CLASS, willCollapse)
  if (heading.hasAttribute('aria-expanded')) {
    heading.setAttribute('aria-expanded', willCollapse ? 'false' : 'true')
  }

  let sibling = heading.nextElementSibling
  while (sibling) {
    const m = sibling.tagName.match(/^H([1-6])$/)
    if (m && parseInt(m[1], 10) <= currentLevel) break
    sibling.classList.toggle(HIDDEN_CLASS, willCollapse)
    sibling = sibling.nextElementSibling
  }
}

@customElement('mbr-heading-enhancer')
export class MbrHeadingEnhancerElement extends LitElement {
  override connectedCallback() {
    super.connectedCallback()
    waitForDom()
      .then(() => scheduleIdleTask(() => this._enhance()))
      .catch((e) => console.warn('heading enhancement failed:', e))
  }

  private _enhance(): void {
    const headings = document.querySelectorAll<HTMLElement>(
      'main :is(h1, h2, h3, h4, h5, h6)'
    )

    headings.forEach((heading) => {
      if (heading.classList.contains(ENHANCED_CLASS)) return
      heading.classList.add(ENHANCED_CLASS)

      const id = heading.getAttribute('id')
      if (id) {
        const anchor = document.createElement('a')
        anchor.className = ANCHOR_CLASS
        anchor.href = `#${id}`
        anchor.setAttribute('aria-label', 'Permalink')
        anchor.textContent = '#'
        // Prevent the heading's collapse handler from firing when users
        // click the permalink to copy or navigate to the anchor.
        anchor.addEventListener('click', (event) => event.stopPropagation())
        heading.appendChild(anchor)
      }

      const hasInnerLink = heading.querySelector(`a:not(.${ANCHOR_CLASS})`) !== null
      const insideLink = heading.closest('a') !== null
      if (!hasInnerLink && !insideLink) {
        heading.classList.add(COLLAPSIBLE_CLASS)
        heading.setAttribute('tabindex', '0')
        heading.setAttribute('aria-expanded', 'true')
        heading.addEventListener('click', () => toggleCollapse(heading))
        heading.addEventListener('keydown', (event) => {
          // Only toggle when the heading itself is focused, not when a
          // descendant (like the permalink anchor) receives the key event.
          if (event.target !== heading) return
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault()
            toggleCollapse(heading)
          }
        })
      }
    })
  }

  override render() {
    return nothing
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-heading-enhancer': MbrHeadingEnhancerElement
  }
}
