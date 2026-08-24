import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'

/**
 * The loader talks to the network (`loadScript`) and to the page's asset base,
 * neither of which exists under happy-dom. Mock the module so the tests can
 * drive `_enhance()` straight through to the mermaid global.
 */
const mocks = vi.hoisted(() => ({
  loadScript: vi.fn(() => Promise.resolve()),
}))

vi.mock('./dynamic-loader.ts', () => ({
  waitForDom: () => Promise.resolve(),
  loadScript: mocks.loadScript,
  getMbrAssetBase: () => '/.mbr/',
}))

import './mbr-mermaid.ts'

interface MermaidStub {
  initialize: ReturnType<typeof vi.fn>
  run: ReturnType<typeof vi.fn>
}

/** Install a fake mermaid global and hand it back for assertions. */
function stubMermaid(): MermaidStub {
  const mermaid: MermaidStub = {
    initialize: vi.fn(),
    run: vi.fn(() => Promise.resolve()),
  }
  vi.stubGlobal('mermaid', mermaid)
  return mermaid
}

/** Let waitForDom/loadScript microtasks settle. */
async function flush(): Promise<void> {
  for (let i = 0; i < 4; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}

async function mount(): Promise<HTMLElement> {
  const el = document.createElement('mbr-mermaid')
  document.body.appendChild(el)
  await flush()
  return el
}

/** Stub window.matchMedia to report a fixed prefers-color-scheme match. */
function mockPrefersDark(matches: boolean): void {
  vi.spyOn(window, 'matchMedia').mockReturnValue({ matches } as MediaQueryList)
}

describe('UNIT MbrMermaidElement', () => {
  let element: HTMLElement | null = null

  beforeEach(() => {
    document.body.innerHTML = ''
  })

  afterEach(() => {
    element?.remove()
    element = null
    document.body.innerHTML = ''
    vi.unstubAllGlobals()
    vi.clearAllMocks()
    vi.restoreAllMocks()
  })

  it('pins securityLevel to strict when initializing mermaid', async () => {
    document.body.insertAdjacentHTML('beforeend', '<pre class="mermaid">graph LR;A--&gt;B</pre>')
    const mermaid = stubMermaid()

    element = await mount()

    expect(mermaid.initialize).toHaveBeenCalledTimes(1)
    expect(mermaid.initialize.mock.calls[0][0]).toMatchObject({
      startOnLoad: false,
      securityLevel: 'strict',
    })
  })

  it('renders exactly the blocks it detected', async () => {
    document.body.insertAdjacentHTML(
      'beforeend',
      `<pre class="mermaid">a</pre>
       <div class="mermaid">b</div>
       <pre><code class="language-mermaid">c</code></pre>`
    )
    const mermaid = stubMermaid()

    element = await mount()

    expect(mocks.loadScript).toHaveBeenCalledWith('/.mbr/mermaid.min.js')
    const nodes = mermaid.run.mock.calls[0][0].nodes as HTMLElement[]
    expect(nodes.length).toBe(3)
    expect(nodes.map((n) => n.textContent?.trim())).toEqual(['a', 'b', 'c'])
  })

  it('loads nothing when the page has no diagrams', async () => {
    const mermaid = stubMermaid()

    element = await mount()

    expect(mocks.loadScript).not.toHaveBeenCalled()
    expect(mermaid.initialize).not.toHaveBeenCalled()
  })

  it('only enhances once per element', async () => {
    document.body.insertAdjacentHTML('beforeend', '<pre class="mermaid">graph LR;A</pre>')
    const mermaid = stubMermaid()

    element = await mount()
    element.remove()
    document.body.appendChild(element)
    await flush()

    expect(mermaid.initialize).toHaveBeenCalledTimes(1)
  })

  it('uses the dark theme when prefers-color-scheme: dark matches', async () => {
    mockPrefersDark(true)
    document.body.insertAdjacentHTML('beforeend', '<pre class="mermaid">graph LR;A--&gt;B</pre>')
    const mermaid = stubMermaid()

    element = await mount()

    expect(mermaid.initialize.mock.calls[0][0]).toMatchObject({ theme: 'dark' })
  })

  it('uses the default theme when prefers-color-scheme: dark does not match', async () => {
    mockPrefersDark(false)
    document.body.insertAdjacentHTML('beforeend', '<pre class="mermaid">graph LR;A--&gt;B</pre>')
    const mermaid = stubMermaid()

    element = await mount()

    expect(mermaid.initialize.mock.calls[0][0]).toMatchObject({ theme: 'default' })
  })

  describe('print theme swap', () => {
    it('re-renders in the light theme on beforeprint after a dark render', async () => {
      mockPrefersDark(true)
      document.body.insertAdjacentHTML(
        'beforeend',
        '<pre class="mermaid">graph LR;A--&gt;B</pre>'
      )
      const mermaid = stubMermaid()

      element = await mount()
      const originalNode = mermaid.run.mock.calls[0][0].nodes[0] as HTMLElement
      const originalText = originalNode.textContent

      window.dispatchEvent(new Event('beforeprint'))

      expect(mermaid.initialize).toHaveBeenCalledTimes(2)
      expect(mermaid.initialize.mock.calls[1][0]).toMatchObject({ theme: 'default' })
      expect(mermaid.run).toHaveBeenCalledTimes(2)

      const freshNodes = mermaid.run.mock.calls[1][0].nodes as HTMLElement[]
      expect(freshNodes[0]).not.toBe(originalNode)
      expect(freshNodes[0].textContent).toBe(originalText)
    })

    it('restores the dark theme on afterprint', async () => {
      mockPrefersDark(true)
      document.body.insertAdjacentHTML(
        'beforeend',
        '<pre class="mermaid">graph LR;A--&gt;B</pre>'
      )
      const mermaid = stubMermaid()

      element = await mount()
      const originalText = (mermaid.run.mock.calls[0][0].nodes[0] as HTMLElement).textContent

      window.dispatchEvent(new Event('beforeprint'))
      const printNode = mermaid.run.mock.calls[1][0].nodes[0] as HTMLElement

      window.dispatchEvent(new Event('afterprint'))

      expect(mermaid.initialize).toHaveBeenCalledTimes(3)
      expect(mermaid.initialize.mock.calls[2][0]).toMatchObject({ theme: 'dark' })
      expect(mermaid.run).toHaveBeenCalledTimes(3)

      const restoredNodes = mermaid.run.mock.calls[2][0].nodes as HTMLElement[]
      expect(restoredNodes[0]).not.toBe(printNode)
      expect(restoredNodes[0].textContent).toBe(originalText)
    })

    it('does nothing on beforeprint/afterprint when prefers-color-scheme is light', async () => {
      mockPrefersDark(false)
      document.body.insertAdjacentHTML(
        'beforeend',
        '<pre class="mermaid">graph LR;A--&gt;B</pre>'
      )
      const mermaid = stubMermaid()

      element = await mount()

      window.dispatchEvent(new Event('beforeprint'))
      window.dispatchEvent(new Event('afterprint'))

      expect(mermaid.initialize).toHaveBeenCalledTimes(1)
      expect(mermaid.run).toHaveBeenCalledTimes(1)
    })
  })
})
