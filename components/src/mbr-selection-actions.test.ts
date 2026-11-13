import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { registerSelectionAction, type MbrSelectionActions } from './mbr-selection-actions.ts'

/** happy-dom lays nothing out; give every range a visible rect. */
function stubRects(): void {
  vi.spyOn(Range.prototype, 'getBoundingClientRect').mockReturnValue(
    { top: 200, bottom: 220, left: 50, right: 150, width: 100, height: 20, x: 50, y: 200, toJSON: () => ({}) } as DOMRect
  )
}

function select(node: Node): void {
  const range = document.createRange()
  range.selectNodeContents(node)
  const selection = window.getSelection()!
  selection.removeAllRanges()
  selection.addRange(range)
}

let element: MbrSelectionActions

async function measure(): Promise<void> {
  element.measure()
  await element.updateComplete
}

const buttons = (): string[] =>
  Array.from(element.shadowRoot?.querySelectorAll('button') ?? []).map((b) => b.textContent?.trim() ?? '')

beforeEach(() => {
  stubRects()
  document.body.innerHTML = '<nav><p>menu</p></nav><main id="wrapper"><p>The quick brown fox.</p></main>'
  element = document.createElement('mbr-selection-actions')
  document.body.appendChild(element)
})

afterEach(() => {
  element.remove()
  document.body.innerHTML = ''
  window.getSelection()?.removeAllRanges()
  delete (document as unknown as { execCommand?: unknown }).execCommand
  vi.restoreAllMocks()
})

describe('<mbr-selection-actions>', () => {
  it('renders nothing without a selection', async () => {
    await measure()
    expect(element.shadowRoot?.querySelector('.selection-toolbar')).toBeNull()
  })

  it('offers Copy text and Copy rich for a selection in the note on a page without source', async () => {
    select(document.querySelector('main p')!)
    await measure()
    expect(buttons()).toEqual(['Copy text', 'Copy rich'])
  })

  it('ignores a selection outside the note', async () => {
    select(document.querySelector('nav p')!)
    await measure()
    expect(element.shadowRoot?.querySelector('.selection-toolbar')).toBeNull()
  })

  it('puts a registered action first while it is available, and hides after running it', async () => {
    let available = true
    const run = vi.fn()
    const unregister = registerSelectionAction({ label: 'Add note', title: 't', isAvailable: () => available, run })
    try {
      select(document.querySelector('main p')!)
      await measure()
      expect(buttons()).toEqual(['Add note', 'Copy text', 'Copy rich'])

      element.shadowRoot!.querySelector('button')!.click()
      await element.updateComplete
      expect(run).toHaveBeenCalledOnce()
      expect(element.shadowRoot?.querySelector('.selection-toolbar')).toBeNull()

      available = false
      await measure()
      expect(buttons()).toEqual(['Copy text', 'Copy rich'])
    } finally {
      unregister()
    }
    await measure()
    expect(buttons()).toEqual(['Copy text', 'Copy rich'])
  })

  it('copies the selection when a copy button is clicked', async () => {
    const data = new Map<string, string>()
    ;(document as unknown as { execCommand: () => boolean }).execCommand = () => {
      const event = new Event('copy', { cancelable: true }) as ClipboardEvent
      Object.defineProperty(event, 'clipboardData', {
        value: { setData: (type: string, value: string) => data.set(type, value) },
      })
      document.dispatchEvent(event)
      return true
    }
    select(document.querySelector('main p')!)
    await measure()
    element.shadowRoot!.querySelector<HTMLButtonElement>('.copy-rich')!.click()
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(data.get('text/plain')).toBe('The quick brown fox.')
    expect(data.get('text/html')).toBe('The quick brown fox.')
    expect(document.querySelector('.mbr-toast')?.textContent).toBe('Copied as rich text')
  })

  it("says Copy markdown when the selection maps to the page's source, and warms the cache", async () => {
    ;(window as { __MBR_CONFIG__?: unknown }).__MBR_CONFIG__ = { serverMode: true, editEnabled: true }
    window.frontmatter = { markdown_source: 'a.md' }
    const fetchMock = vi.fn(async () => new Response('The quick brown fox.\n', { status: 200 }))
    vi.stubGlobal('fetch', fetchMock)
    try {
      document.querySelector('main p')!.setAttribute('data-mbr-line', '1')
      select(document.querySelector('main p')!)
      await measure()
      expect(buttons()).toEqual(['Copy markdown', 'Copy rich'])
      expect(fetchMock).toHaveBeenCalledOnce()
    } finally {
      delete (window as { __MBR_CONFIG__?: unknown }).__MBR_CONFIG__
      window.frontmatter = undefined
      vi.unstubAllGlobals()
    }
  })

  it('installs the Edit menu global on load', () => {
    expect(typeof (window as unknown as Record<string, unknown>).mbrCopyPage).toBe('function')
  })
})
