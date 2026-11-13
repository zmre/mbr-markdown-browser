/**
 * The info panel's "Copy page: Markdown · Rich" block. In its own file because
 * `mbr-info.test.ts` is shared by several features.
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import './mbr-info.ts'
import { resetCopyPrefetch } from './copy-content.ts'
import { resetTaskToggleState } from './task-toggle.ts'

async function openPanel(): Promise<HTMLElement> {
  const element = document.createElement('mbr-info')
  document.body.appendChild(element)
  const el = element as unknown as { _isOpen: boolean; requestUpdate(): void; updateComplete: Promise<boolean> }
  el._isOpen = true
  el.requestUpdate()
  await el.updateComplete
  return element
}

/** Stand in for the engine's copy command; returns what it put on the clipboard. */
function fakeExecCommand(): Map<string, string> {
  const data = new Map<string, string>()
  ;(document as unknown as { execCommand: () => boolean }).execCommand = () => {
    const event = new Event('copy', { cancelable: true }) as ClipboardEvent
    Object.defineProperty(event, 'clipboardData', {
      value: { setData: (type: string, value: string) => data.set(type, value) },
    })
    document.dispatchEvent(event)
    return true
  }
  return data
}

const NOTE =
  '<main id="wrapper"><h1 id="t" data-mbr-line="4">Title<a class="mbr-heading-anchor" href="#t">#</a></h1>' +
  '<p data-mbr-line="6">Body <strong>bold</strong></p></main>'

afterEach(() => {
  document.body.innerHTML = ''
  window.frontmatter = undefined
  delete window.extendedMeta
  delete (window as { __MBR_CONFIG__?: unknown }).__MBR_CONFIG__
  delete (document as unknown as { execCommand?: unknown }).execCommand
  resetTaskToggleState()
  resetCopyPrefetch()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('info panel copy page', () => {
  it('is absent on a page with no note body', async () => {
    const element = await openPanel()
    expect(element.shadowRoot?.querySelector('.copy-page')).toBeNull()
  })

  it('sits below Metadata and above Document Info', async () => {
    window.frontmatter = { title: 'Note', author: 'Me' }
    window.extendedMeta = {
      wordCount: 3,
      readingTimeMinutes: 1,
      fleschReadingEase: null,
      fleschKincaidGrade: null,
      filePath: 'a.md',
      modifiedTimestamp: 0,
    }
    document.body.innerHTML = NOTE
    const element = await openPanel()
    const blocks = Array.from(element.shadowRoot!.querySelectorAll('.info-panel-content > .info-section'))
    const names = blocks.map((b) =>
      b.classList.contains('copy-page') ? 'copy' : (b.querySelector('summary strong')?.textContent ?? '')
    )
    const copy = names.indexOf('copy')
    expect(copy).toBeGreaterThan(-1)
    expect(names[copy - 1]).toBe('Metadata')
    expect(names[copy + 1]).toBe('Document Info')
  })

  it('says Text where the page cannot read its source (static build)', async () => {
    window.frontmatter = { markdown_source: 'a.md' }
    document.body.innerHTML = NOTE
    const data = fakeExecCommand()
    const element = await openPanel()
    const button = element.shadowRoot!.querySelector<HTMLButtonElement>('.copy-page-markdown')!
    expect(button.textContent?.trim()).toBe('Text')
    button.click()
    await vi.waitFor(() => expect(data.get('text/plain')).toBe('Title\n\nBody bold'))
    await vi.waitFor(() => expect(document.querySelector('.mbr-toast')?.textContent).toBe('Page copied as text'))
  })

  it('copies the markdown body, without frontmatter, where the server serves the source', async () => {
    ;(window as { __MBR_CONFIG__?: unknown }).__MBR_CONFIG__ = { serverMode: true, editEnabled: true }
    window.frontmatter = { markdown_source: 'notes/a.md' }
    document.body.innerHTML = NOTE
    const source = '---\ntitle: Title\n---\n# Title\n\nBody **bold**\n'
    const fetchMock = vi.fn(async () => new Response(source, { status: 200 }))
    vi.stubGlobal('fetch', fetchMock)
    const data = fakeExecCommand()
    const element = await openPanel()
    const button = element.shadowRoot!.querySelector<HTMLButtonElement>('.copy-page-markdown')!
    expect(button.textContent?.trim()).toBe('Markdown')
    // Opening the panel warmed the shared source cache, once.
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce())
    expect(String((fetchMock.mock.calls[0] as unknown[])[0])).toBe('/.mbr/raw/notes/a.md')
    await new Promise((resolve) => setTimeout(resolve, 0))
    button.click()
    await vi.waitFor(() => expect(data.get('text/plain')).toBe('# Title\n\nBody **bold**'))
    expect(fetchMock).toHaveBeenCalledOnce()
    await vi.waitFor(() => expect(document.querySelector('.mbr-toast')?.textContent).toBe('Page copied as markdown'))
  })

  it('copies the whole, cleaned note as rich text', async () => {
    window.frontmatter = {}
    document.body.innerHTML =
      '<main id="wrapper"><h1 id="t">Title<a class="mbr-heading-anchor" href="#t">#</a></h1><p data-mbr-line="3">Body</p></main>'
    const data = fakeExecCommand()
    const element = await openPanel()
    element.shadowRoot!.querySelector<HTMLButtonElement>('.copy-page-rich')!.click()
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(data.get('text/plain')).toBe('Title\n\nBody')
    expect(data.get('text/html')).toBe('<h1>Title</h1><p>Body</p>')
  })
})
