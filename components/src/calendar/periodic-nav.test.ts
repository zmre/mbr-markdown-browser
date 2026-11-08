import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import '../mbr-keys.js'
import { openTodayDailyNote, setPeriodicNavDeps } from './periodic-nav.js'
import type { PeriodicSource } from './periodic.js'

const DAYS: PeriodicSource[] = [
  { url_path: '/daily/2026-10-30/', calendar: { period: '2026-10-30' } },
  { url_path: '/daily/2026-10-31/', calendar: { period: '2026-10-31' } },
]

/** Flushes the promise chain behind a key press. */
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0))

const press = (key: string, init: KeyboardEventInit = {}, target: EventTarget = document.body) =>
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, composed: true, ...init }))

describe('periodic keys', () => {
  let navigate: ReturnType<typeof vi.fn<(url: string) => void>>
  let loadFiles: ReturnType<typeof vi.fn<() => Promise<readonly PeriodicSource[]>>>

  beforeEach(() => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    navigate = vi.fn<(url: string) => void>()
    loadFiles = vi.fn<() => Promise<readonly PeriodicSource[]>>(() => Promise.resolve(DAYS))
    setPeriodicNavDeps({ loadFiles, navigate })
    document.body.appendChild(document.createElement('mbr-keys'))
  })

  afterEach(() => {
    setPeriodicNavDeps(null)
    document.body.innerHTML = ''
    window.__MBR_CONFIG__ = undefined
    vi.useRealTimers()
  })

  it('d opens the latest daily note on or before today', async () => {
    await openTodayDailyNote('2026-11-01')
    expect(navigate).toHaveBeenCalledWith('/daily/2026-10-31/')
    await openTodayDailyNote('2026-10-30')
    expect(navigate).toHaveBeenLastCalledWith('/daily/2026-10-30/')
    expect(loadFiles).toHaveBeenCalledTimes(1)
  })

  it('d from the keyboard navigates, and builds the index only once', async () => {
    press('d')
    await settle()
    press('d')
    await settle()
    expect(navigate).toHaveBeenCalledTimes(2)
    expect(loadFiles).toHaveBeenCalledTimes(1)
  })

  it('d shows a toast when there are no daily notes', async () => {
    setPeriodicNavDeps({ loadFiles: () => Promise.resolve([]), navigate })
    press('d')
    await settle()
    expect(navigate).not.toHaveBeenCalled()
    expect(document.querySelector('.mbr-toast')?.textContent).toBe('No daily notes found')
  })

  it('d stands down while typing, with a modifier, or when the calendar is off', async () => {
    const input = document.body.appendChild(document.createElement('input'))
    press('d', {}, input)
    press('d', { altKey: true })
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, calendar: { enabled: false } }
    press('d')
    await settle()
    expect(navigate).not.toHaveBeenCalled()
    expect(loadFiles).not.toHaveBeenCalled()
  })

  it('[ and ] follow the bar links, and are no-ops without a bar', () => {
    const clicked: string[] = []
    const noBar = new KeyboardEvent('keydown', { key: ']', bubbles: true, cancelable: true })
    document.body.dispatchEvent(noBar)
    expect(noBar.defaultPrevented).toBe(false)

    document.body.insertAdjacentHTML(
      'beforeend',
      `<nav class="mbr-calendar-nav">
         <a rel="prev" href="/daily/2026-10-30/">prev</a>
         <a rel="next" href="/daily/2026-11-02/">next</a>
       </nav>`
    )
    document.querySelectorAll('nav.mbr-calendar-nav a').forEach((a) =>
      a.addEventListener('click', (e) => {
        e.preventDefault()
        clicked.push(a.getAttribute('rel')!)
      })
    )
    press('[')
    press(']')
    expect(clicked).toEqual(['prev', 'next'])
  })
})
