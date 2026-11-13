import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  COPY_PAGE_GLOBAL,
  cleanForCopy,
  copyDocument,
  copySelection,
  markdownSourcePath,
  resetCopyPrefetch,
  selectionCopyKind,
  copyOutcomeMessage,
  copyWithExecCommand,
  documentPayload,
  fragmentForRange,
  htmlOf,
  installCopyPageGlobal,
  plainTextOf,
  selectionPayload,
  writeClipboard,
} from './copy-content.ts'
import { readSourceLines, resetTaskToggleState } from './task-toggle.ts'

function fragmentOf(markup: string): DocumentFragment {
  const template = document.createElement('template')
  template.innerHTML = markup
  return template.content
}

function select(node: Node, start?: number, end?: number): Selection {
  const range = document.createRange()
  if (start === undefined) range.selectNodeContents(node)
  else {
    range.setStart(node, start)
    range.setEnd(node, end ?? start)
  }
  const selection = window.getSelection()!
  selection.removeAllRanges()
  selection.addRange(range)
  return selection
}

afterEach(() => {
  document.body.innerHTML = ''
  window.getSelection()?.removeAllRanges()
  vi.restoreAllMocks()
})

describe('cleanForCopy', () => {
  it('removes permalinks, review markers, link icons, pies, the calendar bar and UI elements', () => {
    const fragment = fragmentOf(
      '<nav class="mbr-calendar-nav"><a href="/x/">prev</a></nav>' +
        '<span class="sr-only">Title</span>' +
        '<h2 id="intro" class="mbr-heading-enhanced">Intro<a class="mbr-heading-anchor" href="#intro">#</a></h2>' +
        '<p data-mbr-line="3">Text<span class="mbr-review-marker" tabindex="0"></span> ' +
        '<a href="https://example.com" class="mbr-link-internet keep">site<span class="mbr-link-icon"></span></a></p>' +
        '<h3>Deck<span class="mbr-fc-pie"></span></h3>' +
        '<mbr-contact-backlinks count="2"></mbr-contact-backlinks><button>Do</button><script>x()</script>'
    )
    cleanForCopy(fragment)
    expect(htmlOf(fragment)).toBe(
      '<h2>Intro</h2><p>Text <a href="https://example.com" class="keep">site</a></p><h3>Deck</h3>'
    )
  })

  it('drops ids, data-mbr-*, data-pagefind-*, tooltips and tabindex but keeps other attributes', () => {
    const fragment = fragmentOf(
      '<pre data-mbr-line="1" data-pagefind-ignore><code class="language-rust" id="c" data-tooltip="t" tabindex="0">fn</code></pre>'
    )
    cleanForCopy(fragment)
    expect(htmlOf(fragment)).toBe('<pre><code class="language-rust">fn</code></pre>')
  })

  it('makes link and image URLs absolute against the base, leaving fragments alone', () => {
    const fragment = fragmentOf('<a href="../other/">o</a><a href="#top">t</a><img src="pic.png" alt="p">')
    cleanForCopy(fragment, 'http://127.0.0.1:5200/notes/page/')
    expect(htmlOf(fragment)).toBe(
      '<a href="http://127.0.0.1:5200/notes/other/">o</a><a href="#top">t</a><img src="http://127.0.0.1:5200/notes/page/pic.png" alt="p">'
    )
  })
})

describe('plainTextOf', () => {
  it('separates paragraphs and headings by a blank line and collapses whitespace', () => {
    const fragment = fragmentOf('<h1>Title</h1>\n  <p>One\n   two  <em>three</em></p>\n<p>Four</p>')
    expect(plainTextOf(fragment)).toBe('Title\n\nOne two three\n\nFour')
  })

  it('puts list items on their own lines and keeps <pre> verbatim', () => {
    const fragment = fragmentOf('<ul>\n<li>a</li>\n<li>b <strong>c</strong></li>\n</ul><pre><code>x  = 1\n  y</code></pre>')
    expect(plainTextOf(fragment)).toBe('a\nb c\n\nx  = 1\n  y')
  })

  it('separates table cells by tabs and rows by newlines', () => {
    const fragment = fragmentOf(
      '<table><thead><tr><th>A</th> <th>B</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>'
    )
    expect(plainTextOf(fragment)).toBe('A\tB\n1\t2')
  })

  it('turns <br> into a newline and an image into its alt text', () => {
    const fragment = fragmentOf('<p>one<br>two <img alt="pic" src="x.png"></p>')
    expect(plainTextOf(fragment)).toBe('one\ntwo pic')
  })

  it('has no text from the permalink or markers once cleaned', () => {
    const fragment = fragmentOf('<h2>Intro<a class="mbr-heading-anchor" href="#intro">#</a></h2>')
    cleanForCopy(fragment)
    expect(plainTextOf(fragment)).toBe('Intro')
  })
})

describe('fragmentForRange', () => {
  it('keeps the inline formatting a selection sits inside', () => {
    document.body.innerHTML = '<main id="wrapper"><p>Plain <strong>very bold words</strong></p></main>'
    const text = document.querySelector('strong')!.firstChild!
    const range = document.createRange()
    range.setStart(text, 5)
    range.setEnd(text, 9)
    const root = document.querySelector('main')!
    expect(htmlOf(fragmentForRange(range, root))).toBe('<strong>bold</strong>')
  })

  it('keeps the list around selected items', () => {
    document.body.innerHTML = '<main id="wrapper"><ul><li>a</li><li>b</li><li>c</li></ul></main>'
    const items = document.querySelectorAll('li')
    const range = document.createRange()
    range.setStart(items[0].firstChild!, 0)
    range.setEnd(items[1].firstChild!, 1)
    expect(htmlOf(fragmentForRange(range, document.querySelector('main')))).toBe('<ul><li>a</li><li>b</li></ul>')
  })
})

describe('selectionPayload and documentPayload', () => {
  it('is null for a selection outside the note', () => {
    document.body.innerHTML = '<nav><p>menu</p></nav><main id="wrapper"><p>body</p></main>'
    expect(selectionPayload('text', select(document.querySelector('nav p')!))).toBeNull()
  })

  it('is null for a collapsed selection', () => {
    document.body.innerHTML = '<main id="wrapper"><p>body</p></main>'
    expect(selectionPayload('text', select(document.querySelector('p')!.firstChild!, 2))).toBeNull()
  })

  it('gives plain text only for a text copy, both flavours for a rich one', () => {
    document.body.innerHTML =
      '<main id="wrapper"><h2 id="h">Head<a class="mbr-heading-anchor" href="#h">#</a></h2><p data-mbr-line="2">Body <em>text</em></p></main>'
    const selection = select(document.querySelector('main')!)
    expect(selectionPayload('text', selection)).toEqual({ text: 'Head\n\nBody text' })
    expect(selectionPayload('rich', selection)).toEqual({
      text: 'Head\n\nBody text',
      html: '<h2>Head</h2><p>Body <em>text</em></p>',
    })
  })

  it('copies the whole note without touching the page', () => {
    document.body.innerHTML =
      '<header>chrome</header><main id="wrapper"><nav class="mbr-calendar-nav">bar</nav><h1 id="t">Title</h1><p>Body</p></main>'
    const before = document.body.innerHTML
    const payload = documentPayload('rich')
    expect(payload?.text).toBe('Title\n\nBody')
    expect(payload?.html).toBe('<h1>Title</h1><p>Body</p>')
    expect(document.body.innerHTML).toBe(before)
  })

  it('is null on a page without a note', () => {
    document.body.innerHTML = '<div>listing</div>'
    expect(documentPayload('text')).toBeNull()
  })
})

describe('writing the clipboard', () => {
  /** Stand in for the engine: fire a `copy` event with a DataTransfer-like store. */
  function fakeExecCommand(result = true): Map<string, string> {
    const data = new Map<string, string>()
    ;(document as unknown as { execCommand: (cmd: string) => boolean }).execCommand = (cmd: string) => {
      expect(cmd).toBe('copy')
      const event = new Event('copy', { bubbles: true, cancelable: true }) as ClipboardEvent
      Object.defineProperty(event, 'clipboardData', {
        value: { setData: (type: string, value: string) => data.set(type, value) },
      })
      document.dispatchEvent(event)
      return result
    }
    return data
  }

  afterEach(() => {
    delete (document as unknown as { execCommand?: unknown }).execCommand
  })

  it('sets both flavours through the copy event', () => {
    const data = fakeExecCommand()
    expect(copyWithExecCommand({ text: 'plain', html: '<b>rich</b>' })).toBe(true)
    expect(data.get('text/plain')).toBe('plain')
    expect(data.get('text/html')).toBe('<b>rich</b>')
  })

  it('sets only text/plain for a plain copy', () => {
    const data = fakeExecCommand()
    expect(copyWithExecCommand({ text: 'plain' })).toBe(true)
    expect([...data.keys()]).toEqual(['text/plain'])
  })

  it("restores the reader's selection and leaves no helper behind", () => {
    fakeExecCommand()
    document.body.innerHTML = '<p>keep me</p>'
    const p = document.querySelector('p')!
    select(p.firstChild!, 0, 4)
    copyWithExecCommand({ text: 'x' })
    const selection = window.getSelection()!
    expect(selection.toString()).toBe('keep')
    expect(document.body.children).toHaveLength(1)
  })

  it('falls back to the async Clipboard API when execCommand cannot copy', async () => {
    fakeExecCommand(false)
    const write = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('isSecureContext', true)
    vi.stubGlobal('ClipboardItem', class {
      constructor(public items: Record<string, Blob>) {}
    })
    Object.defineProperty(navigator, 'clipboard', { value: { write, writeText: vi.fn() }, configurable: true })
    try {
      expect(await writeClipboard({ text: 't', html: '<i>h</i>' })).toBe(true)
      expect(write).toHaveBeenCalledOnce()
      const item = write.mock.calls[0][0][0] as { items: Record<string, Blob> }
      expect(Object.keys(item.items).sort()).toEqual(['text/html', 'text/plain'])
    } finally {
      vi.unstubAllGlobals()
      delete (navigator as unknown as { clipboard?: unknown }).clipboard
    }
  })

  it('reports failure when no route works', async () => {
    fakeExecCommand(false)
    vi.stubGlobal('isSecureContext', false)
    try {
      expect(await writeClipboard({ text: 't' })).toBe(false)
    } finally {
      vi.unstubAllGlobals()
    }
  })
})

describe('the Edit menu global', () => {
  it('is installed as window.mbrCopyPage, the name src/browser.rs calls', async () => {
    // `src/browser.rs` (COPY_PAGE_RICH_SCRIPT / COPY_PAGE_MARKDOWN_SCRIPT) names
    // this in a string literal; its own test pins the same spelling.
    expect(COPY_PAGE_GLOBAL).toBe('mbrCopyPage')
    const report = vi.fn()
    installCopyPageGlobal(report)
    const fn = (window as unknown as Record<string, unknown>).mbrCopyPage
    expect(typeof fn).toBe('function')

    document.body.innerHTML = '<div>no note</div>'
    await expect((fn as (f: string) => Promise<boolean>)('rich')).resolves.toBe(false)
    expect(report).toHaveBeenCalledWith({ ok: false, as: 'rich' })
  })

  it('words the outcome', () => {
    expect(copyOutcomeMessage({ ok: true, as: 'rich' }, 'page')).toBe('Page copied as rich text')
    expect(copyOutcomeMessage({ ok: true, as: 'markdown' }, 'page')).toBe('Page copied as markdown')
    expect(copyOutcomeMessage({ ok: true, as: 'markdown' }, 'selection')).toBe('Copied as markdown')
    expect(copyOutcomeMessage({ ok: true, as: 'text' }, 'selection')).toBe('Copied as text')
    expect(copyOutcomeMessage({ ok: false, as: 'markdown' }, 'page')).toBe('Copy failed')
  })
})

describe('Copy markdown', () => {
  const SOURCE = '---\ntitle: T\n---\n# Title\n\nSome **bold** text.\n\n- one\n- two\n'
  const PAGE =
    '<main id="wrapper"><h1 id="t" data-mbr-line="4">Title</h1>' +
    '<p data-mbr-line="6">Some <strong>bold</strong> text.</p>' +
    '<ul><li data-mbr-line="8">one</li><li data-mbr-line="9">two</li></ul></main>'

  function serveSource(body: string | null, status = 200): ReturnType<typeof vi.fn> {
    const fetchMock = vi.fn(async () => new Response(body ?? '', { status }))
    vi.stubGlobal('fetch', fetchMock)
    return fetchMock
  }

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

  function serverPage(edit = true): void {
    ;(window as { __MBR_CONFIG__?: unknown }).__MBR_CONFIG__ = { serverMode: true, editEnabled: edit }
    window.frontmatter = { markdown_source: 'notes/a.md' }
    document.body.innerHTML = PAGE
  }

  afterEach(() => {
    delete (window as { __MBR_CONFIG__?: unknown }).__MBR_CONFIG__
    window.frontmatter = undefined
    delete (document as unknown as { execCommand?: unknown }).execCommand
    delete (navigator as unknown as { clipboard?: unknown }).clipboard
    resetTaskToggleState()
    resetCopyPrefetch()
    vi.unstubAllGlobals()
  })

  it('has a source only on a server page with editing on', () => {
    serverPage()
    expect(markdownSourcePath()).toBe('notes/a.md')
    serverPage(false)
    expect(markdownSourcePath()).toBeNull()
    ;(window as { __MBR_CONFIG__?: unknown }).__MBR_CONFIG__ = { serverMode: false, editEnabled: true }
    expect(markdownSourcePath()).toBeNull()
  })

  it('labels a selection markdown only where it maps to the source', () => {
    serverPage()
    expect(selectionCopyKind(select(document.querySelector('strong')!))).toBe('markdown')
    serverPage(false)
    expect(selectionCopyKind(select(document.querySelector('strong')!))).toBe('text')
    serverPage()
    document.querySelectorAll('[data-mbr-line]').forEach((el) => el.removeAttribute('data-mbr-line'))
    expect(selectionCopyKind(select(document.querySelector('strong')!))).toBe('text')
  })

  it('copies the page body without frontmatter from the cached source, synchronously', async () => {
    serverPage()
    const fetchMock = serveSource(SOURCE)
    await readSourceLines('notes/a.md')
    const data = fakeExecCommand()
    await expect(copyDocument('markdown')).resolves.toEqual({ ok: true, as: 'markdown' })
    expect(data.get('text/plain')).toBe('# Title\n\nSome **bold** text.\n\n- one\n- two')
    expect(data.has('text/html')).toBe(false)
    expect(fetchMock).toHaveBeenCalledOnce()
  })

  it('copies the exact markdown of a selection', async () => {
    serverPage()
    serveSource(SOURCE)
    await readSourceLines('notes/a.md')
    const data = fakeExecCommand()
    select(document.querySelector('strong')!)
    await expect(copySelection('markdown')).resolves.toEqual({ ok: true, as: 'markdown' })
    expect(data.get('text/plain')).toBe('**bold**')

    const items = document.querySelectorAll('li')
    const range = document.createRange()
    range.setStart(document.querySelector('p')!.firstChild!, 2)
    range.setEnd(items[1]!.firstChild!, 2)
    window.getSelection()!.removeAllRanges()
    window.getSelection()!.addRange(range)
    await copySelection('markdown')
    expect(data.get('text/plain')).toBe('Some **bold** text.\n\n- one\n- two')
  })

  it('reads an uncached source inside the gesture through a promised ClipboardItem', async () => {
    serverPage()
    serveSource(SOURCE)
    const written: Promise<Blob>[] = []
    vi.stubGlobal('isSecureContext', true)
    vi.stubGlobal('ClipboardItem', class {
      constructor(public items: Record<string, Promise<Blob>>) {
        written.push(items['text/plain']!)
      }
    })
    const write = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { value: { write }, configurable: true })
    const pending = copyDocument('markdown')
    // The write was started before the source arrived.
    expect(write).toHaveBeenCalledOnce()
    await expect(pending).resolves.toEqual({ ok: true, as: 'markdown' })
    expect(await new Response(await written[0]!).text()).toBe('# Title\n\nSome **bold** text.\n\n- one\n- two')
  })

  it('falls back to text when the source cannot be read', async () => {
    serverPage()
    serveSource(null, 403)
    const data = fakeExecCommand()
    vi.stubGlobal('isSecureContext', false)
    await expect(copyDocument('markdown')).resolves.toEqual({ ok: true, as: 'text' })
    expect(data.get('text/plain')).toBe('Title\n\nSome bold text.\n\none\ntwo')
  })

  it('copies text, never fetching, on a page with no source', async () => {
    document.body.innerHTML = PAGE
    const fetchMock = serveSource(SOURCE)
    const data = fakeExecCommand()
    select(document.querySelector('strong')!)
    await expect(copySelection('markdown')).resolves.toEqual({ ok: true, as: 'text' })
    expect(data.get('text/plain')).toBe('bold')
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('accepts the menu global with markdown', async () => {
    serverPage()
    serveSource(SOURCE)
    await readSourceLines('notes/a.md')
    const data = fakeExecCommand()
    const report = vi.fn()
    installCopyPageGlobal(report)
    const fn = (window as unknown as Record<string, (f: string) => Promise<boolean>>).mbrCopyPage!
    await expect(fn('markdown')).resolves.toBe(true)
    expect(report).toHaveBeenCalledWith({ ok: true, as: 'markdown' })
    expect(data.get('text/plain')).toMatch(/^# Title/)
  })
})
