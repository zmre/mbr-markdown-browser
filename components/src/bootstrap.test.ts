/**
 * `crates/mbr-core/templates/mbr-bootstrap.js`: the classic script that turns
 * the templates' `<script type="application/json" data-mbr-globals>` blocks
 * into the page globals every component reads, so mbr's pages run no inline
 * script (a strict Content-Security-Policy). Run here as shipped.
 */
import { afterEach, describe, expect, it, vi } from 'vitest'

declare const __MBR_BOOTSTRAP_JS__: string

type Globals = {
  __MBR_CONFIG__?: unknown
  frontmatter?: unknown
  headings?: unknown
  extendedMeta?: unknown
  __MBR_MEDIA_VIEWER__?: unknown
  evil?: unknown
}

const globals = window as unknown as Globals

function addBlock(json: string): HTMLScriptElement {
  const block = document.createElement('script')
  block.type = 'application/json'
  block.setAttribute('data-mbr-globals', '')
  block.textContent = json
  document.head.append(block)
  return block
}

function runBootstrap(): void {
  new Function(__MBR_BOOTSTRAP_JS__)()
}

afterEach(() => {
  document.head.innerHTML = ''
  document.body.innerHTML = ''
  for (const name of ['__MBR_CONFIG__', 'frontmatter', 'headings', 'extendedMeta', '__MBR_MEDIA_VIEWER__', 'evil']) {
    delete (globals as Record<string, unknown>)[name]
  }
})

describe('mbr-bootstrap.js', () => {
  it('defines the page globals from the data blocks', () => {
    addBlock('{"__MBR_CONFIG__": {"serverMode": true, "repoId": "abc"}}')
    addBlock('{"frontmatter": {"title": "a \\u003c/script\\u003e"}, "headings": [{"id": "x"}], "extendedMeta": {"wordCount": 3}}')
    runBootstrap()
    expect(globals.__MBR_CONFIG__).toEqual({ serverMode: true, repoId: 'abc' })
    expect(globals.frontmatter).toEqual({ title: 'a </script>' })
    expect(globals.headings).toEqual([{ id: 'x' }])
    expect(globals.extendedMeta).toEqual({ wordCount: 3 })
  })

  it('defines only its own names, and reads each block once', () => {
    addBlock('{"__MBR_CONFIG__": {"a": 1}, "evil": 1}')
    runBootstrap()
    expect(globals.evil).toBeUndefined()
    // A later run (the bootstrap is loaded again after the page data block)
    // does not reapply an earlier block over a value set since.
    globals.__MBR_CONFIG__ = { a: 2 }
    addBlock('{"frontmatter": {}}')
    runBootstrap()
    expect(globals.__MBR_CONFIG__).toEqual({ a: 2 })
    expect(globals.frontmatter).toEqual({})
  })

  it('ignores JSON blocks that are not page data, and survives a broken one', () => {
    const plain = document.createElement('script')
    plain.type = 'application/json'
    plain.textContent = '{"frontmatter": {"x": 1}}'
    document.head.append(plain)
    addBlock('{not json')
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    runBootstrap()
    expect(globals.frontmatter).toBeUndefined()
    expect(error).toHaveBeenCalled()
    error.mockRestore()
  })

  it('reads the media viewer from its meta tag', () => {
    const meta = document.createElement('meta')
    meta.name = 'mbr-media-viewer'
    meta.setAttribute('data-media-type', 'videos')
    meta.setAttribute('data-title', 'a "b" & c.mp4')
    document.head.append(meta)
    runBootstrap()
    expect(globals.__MBR_MEDIA_VIEWER__).toEqual({ mediaType: 'videos', title: 'a "b" & c.mp4' })
  })

  it('wires data-mbr-history-back buttons in place of an inline onclick', () => {
    runBootstrap()
    runBootstrap()
    const back = vi.spyOn(history, 'back').mockImplementation(() => {})
    document.body.innerHTML = '<button type="button" data-mbr-history-back><span>Back</span></button>'
    document.querySelector('span')?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    // Installed once, however many times the bootstrap runs on a page.
    expect(back).toHaveBeenCalledTimes(1)
    back.mockRestore()
  })

  it('leaks no top-level names', () => {
    expect(__MBR_BOOTSTRAP_JS__.trimStart()).toMatch(/^\/\*![\s\S]*?\*\/\s*\(function \(\) \{/)
  })
})
