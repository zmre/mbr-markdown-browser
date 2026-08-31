import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { copyText } from './clipboard.ts'

/** Set `isSecureContext`, which happy-dom leaves undefined. */
function setSecure(value: boolean): void {
  Object.defineProperty(globalThis, 'isSecureContext', { value, configurable: true })
}

/** Install a `navigator.clipboard` stub, or remove it entirely. */
function setClipboard(writeText: ((text: string) => Promise<void>) | null): void {
  Object.defineProperty(navigator, 'clipboard', {
    value: writeText === null ? undefined : { writeText },
    configurable: true,
  })
}

beforeEach(() => {
  document.body.innerHTML = ''
})

afterEach(() => {
  vi.restoreAllMocks()
  setClipboard(null)
  document.body.innerHTML = ''
})

describe('copyText', () => {
  it('uses the async API in a secure context', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    setSecure(true)
    setClipboard(writeText)

    await expect(copyText('# Code Review\n')).resolves.toBe(true)
    expect(writeText).toHaveBeenCalledWith('# Code Review\n')
  })

  it('does not touch the async API outside a secure context', async () => {
    // `--host 0.0.0.0` reached by IP: navigator.clipboard is undefined there.
    const writeText = vi.fn().mockResolvedValue(undefined)
    setSecure(false)
    setClipboard(writeText)
    document.execCommand = vi.fn().mockReturnValue(true)

    await expect(copyText('text')).resolves.toBe(true)
    expect(writeText).not.toHaveBeenCalled()
    expect(document.execCommand).toHaveBeenCalledWith('copy')
  })

  it('falls back when the async write rejects', async () => {
    setSecure(true)
    setClipboard(vi.fn().mockRejectedValue(new Error('denied')))
    document.execCommand = vi.fn().mockReturnValue(true)

    await expect(copyText('text')).resolves.toBe(true)
    expect(document.execCommand).toHaveBeenCalledWith('copy')
  })

  it('reports failure when both routes fail, so the caller can offer a manual copy', async () => {
    setSecure(false)
    setClipboard(null)
    document.execCommand = vi.fn().mockReturnValue(false)

    await expect(copyText('text')).resolves.toBe(false)
  })

  it('reports failure rather than throwing when execCommand throws', async () => {
    setSecure(false)
    setClipboard(null)
    document.execCommand = vi.fn().mockImplementation(() => {
      throw new Error('not allowed')
    })

    await expect(copyText('text')).resolves.toBe(false)
  })

  it('leaves no textarea behind', async () => {
    setSecure(false)
    setClipboard(null)
    document.execCommand = vi.fn().mockReturnValue(true)

    await copyText('text')
    expect(document.querySelectorAll('textarea')).toHaveLength(0)
  })

  it('restores focus after the fallback', async () => {
    // Otherwise a keyboard user who pressed `c` is dumped out of the panel.
    setSecure(false)
    setClipboard(null)
    document.execCommand = vi.fn().mockReturnValue(true)

    const button = document.createElement('button')
    document.body.appendChild(button)
    button.focus()

    await copyText('text')
    expect(document.activeElement).toBe(button)
  })
})
