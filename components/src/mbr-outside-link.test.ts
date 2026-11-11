import { describe, it, expect, afterEach } from 'vitest'
import {
  IPC_OPEN_OUTSIDE_PREFIX,
  MbrOutsideLinkElement,
  outsidePathFromSearch,
  postOpenOutside,
} from './mbr-outside-link.ts'

/** Sets the page URL happy-dom reports. */
function setUrl(url: string): void {
  ;(window as unknown as { happyDOM: { setURL(url: string): void } }).happyDOM.setURL(url)
}

async function mount(): Promise<MbrOutsideLinkElement> {
  const el = document.createElement('mbr-outside-link')
  document.body.appendChild(el)
  await el.updateComplete
  return el
}

describe('outsidePathFromSearch', () => {
  it('decodes the path parameter', () => {
    expect(outsidePathFromSearch('?path=~/src/a%20b%26c%23d%2Be.md')).toBe('~/src/a b&c#d+e.md')
  })

  it('is null without a usable path', () => {
    expect(outsidePathFromSearch('')).toBeNull()
    expect(outsidePathFromSearch('?path=')).toBeNull()
    expect(outsidePathFromSearch('?other=1')).toBeNull()
  })
})

describe('postOpenOutside', () => {
  afterEach(() => {
    delete window.ipc
  })

  it('posts the prefixed path when the GUI channel exists', () => {
    const posted: string[] = []
    window.ipc = { postMessage: (message: string) => posted.push(message) }
    expect(postOpenOutside('/Users/me/src/x.md')).toBe(true)
    expect(posted).toEqual([`${IPC_OPEN_OUTSIDE_PREFIX}/Users/me/src/x.md`])
  })

  it('does nothing without one', () => {
    expect(postOpenOutside('/x')).toBe(false)
  })

  it('keeps the prefix Rust matches on', () => {
    expect(IPC_OPEN_OUTSIDE_PREFIX).toBe('mbr:open-outside:')
  })
})

describe('<mbr-outside-link>', () => {
  afterEach(() => {
    document.body.innerHTML = ''
    delete window.__MBR_CONFIG__
    delete window.ipc
  })

  it('shows the path as text, never as markup', async () => {
    setUrl('http://localhost:5555/.mbr/outside/?path=' + encodeURIComponent('~/<img src=x onerror=alert(1)>.md'))
    const el = await mount()
    const code = el.querySelector('code.mbr-outside-path')
    expect(code?.textContent).toBe('~/<img src=x onerror=alert(1)>.md')
    expect(el.querySelector('img')).toBeNull()
  })

  it('offers Copy but no Open… outside the GUI', async () => {
    setUrl('http://localhost:5555/.mbr/outside/?path=~/x.md')
    const el = await mount()
    const labels = [...el.querySelectorAll('button')].map((b) => b.textContent?.trim())
    expect(labels).toEqual(['Copy path', 'Back'])
  })

  it('offers Open… in the GUI, which posts the path', async () => {
    window.__MBR_CONFIG__ = { guiMode: true } as typeof window.__MBR_CONFIG__
    const posted: string[] = []
    window.ipc = { postMessage: (message: string) => posted.push(message) }
    setUrl('http://localhost:5555/.mbr/outside/?path=/Users/me/src/x.md')
    const el = await mount()
    const open = [...el.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Open…')
    expect(open).toBeDefined()
    open?.click()
    expect(posted).toEqual(['mbr:open-outside:/Users/me/src/x.md'])
  })

  it('says so when there is no path', async () => {
    setUrl('http://localhost:5555/.mbr/outside/')
    const el = await mount()
    expect(el.textContent).toContain('does not say which file')
    expect(el.querySelector('button')).toBeNull()
  })
})
