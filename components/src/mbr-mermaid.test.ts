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
})
