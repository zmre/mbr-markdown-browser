import { describe, it, expect, afterEach, vi } from 'vitest'
import { render } from 'lit'
import {
  IPC_REVEAL_PREFIX,
  fileLocationFrom,
  postReveal,
  COPY_FULL_PATH_LABEL,
  renderFileLocationButton,
  revealLabel,
  type FileLocation,
  type FileLocationActions,
} from './info-file-location.ts'

/** Renders the button into a container and returns it. */
function mount(location: FileLocation | null, relativePath: string, actions?: FileLocationActions): HTMLElement {
  const holder = document.createElement('span')
  document.body.appendChild(holder)
  render(renderFileLocationButton(location, relativePath, actions), holder)
  return holder
}

afterEach(() => {
  document.body.innerHTML = ''
  delete (window as { ipc?: unknown }).ipc
})

describe('fileLocationFrom', () => {
  it('reads the GUI payload', () => {
    expect(fileLocationFrom({ path: '/Users/me/notes/a.md', reveal: true })).toEqual({
      path: '/Users/me/notes/a.md',
      reveal: true,
    })
  })

  it('treats anything but reveal === true as no reveal', () => {
    expect(fileLocationFrom({ path: 'C:\\notes\\a.md', reveal: 'yes' })).toEqual({
      path: 'C:\\notes\\a.md',
      reveal: false,
    })
  })

  it('is null outside the GUI and for malformed values', () => {
    for (const value of [null, undefined, '', '/a.md', 42, {}, { path: '' }, { path: '  ' }, { path: 3 }]) {
      expect(fileLocationFrom(value)).toBeNull()
    }
  })
})

describe('revealLabel', () => {
  it('names the platform file manager', () => {
    expect(revealLabel('MacIntel')).toBe('Show in Finder')
    expect(revealLabel('macOS')).toBe('Show in Finder')
    expect(revealLabel('Win32')).toBe('Show in Explorer')
    expect(revealLabel('Linux x86_64')).toBe('Show in file manager')
  })
})

describe('postReveal', () => {
  it('posts the repository-relative path behind the prefix', () => {
    const postMessage = vi.fn()
    expect(postReveal('notes/a b.md', { postMessage })).toBe(true)
    expect(postMessage).toHaveBeenCalledWith('mbr:reveal:notes/a b.md')
  })

  it('does nothing without the GUI channel or a path', () => {
    expect(postReveal('notes/a.md', undefined)).toBe(false)
    const postMessage = vi.fn()
    expect(postReveal('', { postMessage })).toBe(false)
    expect(postMessage).not.toHaveBeenCalled()
  })

  it('uses the prefix the Rust side parses', () => {
    // `src/external_open.rs`'s `reveal_prefix_matches_the_component` pins the
    // same literal on the other side.
    expect(IPC_REVEAL_PREFIX).toBe('mbr:reveal:')
  })
})

describe('renderFileLocationButton', () => {
  it('renders nothing without a location (server mode, static builds)', () => {
    const body = mount(null, 'notes/a.md')
    expect(body.querySelector('button')).toBeNull()
  })

  it('is a reveal button that sends the relative path, never the absolute one', () => {
    const reveal = vi.fn(() => true)
    const copy = vi.fn(async () => true)
    const body = mount({ path: '/Users/me/notes/a.md', reveal: true }, 'notes/a.md', { reveal, copy })

    expect(body.textContent).not.toContain('/Users/me')
    const button = body.querySelector<HTMLButtonElement>('button.file-location-action')!
    expect(button.dataset.action).toBe('reveal')
    expect(button.getAttribute('aria-label')).toMatch(/^Show in /)
    expect(button.querySelector('svg')).not.toBeNull()

    button.click()
    expect(reveal).toHaveBeenCalledWith('notes/a.md')
    expect(copy).not.toHaveBeenCalled()
  })

  it('reveals through the GUI channel by default', () => {
    const postMessage = vi.fn()
    ;(window as { ipc?: unknown }).ipc = { postMessage }
    const body = mount({ path: '/r/a.md', reveal: true }, 'a.md')
    body.querySelector<HTMLButtonElement>('button')!.click()
    expect(postMessage).toHaveBeenCalledWith('mbr:reveal:a.md')
  })

  it('copies the full absolute path where the platform cannot reveal', async () => {
    const reveal = vi.fn(() => true)
    const copy = vi.fn(async () => true)
    const report = vi.fn()
    const body = mount({ path: '/home/me/notes/a.md', reveal: false }, 'notes/a.md', { reveal, copy, report })

    const button = body.querySelector<HTMLButtonElement>('button')!
    expect(button.dataset.action).toBe('copy')
    expect(button.getAttribute('aria-label')).toBe(COPY_FULL_PATH_LABEL)
    expect(button.title).toBe('Copy full path')
    button.click()
    await vi.waitFor(() => expect(report).toHaveBeenCalledWith('Full path copied'))
    expect(copy).toHaveBeenCalledWith('/home/me/notes/a.md')
    expect(reveal).not.toHaveBeenCalled()
  })

  it('reports a failed copy', async () => {
    const report = vi.fn()
    const body = mount({ path: '/a.md', reveal: false }, 'a.md', { copy: async () => false, report })
    body.querySelector<HTMLButtonElement>('button')!.click()
    await vi.waitFor(() => expect(report).toHaveBeenCalledWith('Could not copy the path'))
  })

  it('offers a copy when there is no relative path to resolve', () => {
    const body = mount({ path: '/a.md', reveal: true }, '')
    expect(body.querySelector<HTMLButtonElement>('button')!.dataset.action).toBe('copy')
  })
})
