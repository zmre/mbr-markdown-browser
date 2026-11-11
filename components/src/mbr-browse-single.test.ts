import { describe, it, expect, afterEach, vi } from 'vitest'
import './mbr-browse-single.js'
import type { MbrBrowseSingleElement } from './mbr-browse-single.js'

/**
 * `_browse_footer.html` arrives as light-DOM children of `<mbr-browse-single>`
 * (see index.html / home.html / section.html …) and is shown through a slot
 * pinned below the scrolling nav, in the docked sidebar and in the drawer.
 */

vi.mock('./shared.ts', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./shared.js')>()
  return {
    ...actual,
    subscribeSiteNav: (cb: (s: unknown) => void) => {
      cb({ isLoading: true, data: null, error: null })
      return () => { }
    },
  }
})

/** Docked (wide) or drawer (narrow): the element asks `matchMedia`. */
function setNarrow(narrow: boolean): void {
  vi.spyOn(window, 'matchMedia').mockImplementation((query: string) => ({
    matches: narrow,
    media: query,
    onchange: null,
    addEventListener: () => { },
    removeEventListener: () => { },
    addListener: () => { },
    removeListener: () => { },
    dispatchEvent: () => false,
  }) as MediaQueryList)
}

describe('MbrBrowseSingleElement sidebar footer', () => {
  let element: MbrBrowseSingleElement

  async function mount(markup: string): Promise<void> {
    element = document.createElement('mbr-browse-single') as MbrBrowseSingleElement
    element.innerHTML = markup
    document.body.appendChild(element)
    await element.updateComplete
  }

  function footer(): HTMLElement | null {
    return element.shadowRoot?.querySelector('.sidebar-footer') ?? null
  }

  afterEach(() => {
    element.remove()
    vi.restoreAllMocks()
  })

  it('pins the slotted links below the scrolling nav in the docked sidebar', async () => {
    setNarrow(false)
    await mount('<!-- c --><a href="https://example.com" aria-label="Example">E</a>')
    const f = footer()
    expect(f).not.toBeNull()
    expect(f!.hidden).toBe(false)
    expect(f!.parentElement?.classList.contains('sidebar')).toBe(true)
    expect(f!.previousElementSibling?.classList.contains('sidebar-nav')).toBe(true)
    expect(f!.querySelector('slot')!.assignedElements().map((el) => el.tagName.toLowerCase())).toEqual(['a'])
  })

  it('hides the footer when the partial holds only its comment', async () => {
    setNarrow(false)
    await mount('\n<!-- here just for user overrides -->\n')
    expect(footer()!.hidden).toBe(true)
  })

  it('follows slotchange when links are added later', async () => {
    setNarrow(false)
    await mount('<!-- c -->')
    expect(footer()!.hidden).toBe(true)
    element.append(Object.assign(document.createElement('a'), { href: '/', textContent: 'Home' }))
    await new Promise((resolve) => setTimeout(resolve, 0))
    await element.updateComplete
    expect(footer()!.hidden).toBe(false)
  })

  it('shows the footer in the drawer once it opens', async () => {
    setNarrow(true)
    await mount('<a href="/">Home</a>')
    expect(footer()).toBeNull()
    element.open()
    await element.updateComplete
    const f = footer()
    expect(f).not.toBeNull()
    expect(f!.hidden).toBe(false)
    expect(f!.parentElement?.classList.contains('drawer')).toBe(true)
  })

  it('hides the drawer footer when the partial is empty', async () => {
    setNarrow(true)
    await mount('<!-- c -->')
    element.open()
    await element.updateComplete
    expect(footer()!.hidden).toBe(true)
  })

  it('collapses a hidden footer to nothing (no border or padding)', () => {
    const css = [(customElements.get('mbr-browse-single') as unknown as { styles: { cssText: string } | { cssText: string }[] }).styles].flat().map((s) => s.cssText).join('\n')
    expect(css).toMatch(/\.sidebar-footer\[hidden\]\s*\{\s*display:\s*none;?\s*\}/)
  })
})
