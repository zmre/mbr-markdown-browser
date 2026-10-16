import { afterEach, describe, expect, it, vi } from 'vitest'
import { LitElement } from 'lit'
import { FAB_CSS, FAB_STACK_OFFSET, PLAY_FAB_QUERY, PlayFabController, isEmbeddedPage } from './fab.ts'

/** A `MediaQueryList` stand-in whose `change` the test fires by hand. */
function stubMatchMedia(matches: boolean) {
  const listeners = new Set<(e: MediaQueryListEvent) => void>()
  const query = {
    matches,
    addEventListener: (_: string, l: (e: MediaQueryListEvent) => void) => listeners.add(l),
    removeEventListener: (_: string, l: (e: MediaQueryListEvent) => void) => listeners.delete(l),
  }
  const spy = vi.spyOn(window, 'matchMedia').mockReturnValue(query as unknown as MediaQueryList)
  return {
    spy,
    listeners,
    change(next: boolean) {
      query.matches = next
      listeners.forEach((l) => l({ matches: next } as MediaQueryListEvent))
    },
  }
}

class FabHost extends LitElement {
  playable = true
  fab: PlayFabController = new PlayFabController(this, (): boolean => this.playable && this.fab.visible)
}
customElements.define('fab-test-host', FabHost)

async function mount(): Promise<FabHost> {
  const host = document.createElement('fab-test-host') as FabHost
  document.body.append(host)
  await host.updateComplete
  return host
}

const offset = () => document.documentElement.style.getPropertyValue('--mbr-fab-stack-offset')

afterEach(() => {
  document.body.innerHTML = ''
  document.documentElement.classList.remove('mbr-embedded')
  document.documentElement.style.removeProperty('--mbr-fab-stack-offset')
  delete document.documentElement.dataset.mbrFabStack
  vi.restoreAllMocks()
})

describe('PlayFabController', () => {
  it('is hidden on a wide browser page', async () => {
    stubMatchMedia(false)
    const host = await mount()
    expect(host.fab.visible).toBe(false)
    expect(offset()).toBe('')
  })

  it('asks for the narrow query', async () => {
    const { spy } = stubMatchMedia(false)
    await mount()
    expect(spy).toHaveBeenCalledWith(PLAY_FAB_QUERY)
  })

  it('follows the media query and publishes the stack offset', async () => {
    const media = stubMatchMedia(true)
    const host = await mount()
    expect(host.fab.visible).toBe(true)
    expect(offset()).toBe(FAB_STACK_OFFSET)

    media.change(false)
    await host.updateComplete
    expect(host.fab.visible).toBe(false)
    expect(offset()).toBe('')

    media.change(true)
    await host.updateComplete
    expect(offset()).toBe(FAB_STACK_OFFSET)
  })

  it('shows at any width in the iOS app', async () => {
    stubMatchMedia(false)
    document.documentElement.classList.add('mbr-embedded')
    expect(isEmbeddedPage()).toBe(true)
    const host = await mount()
    expect(host.fab.visible).toBe(true)
    expect(offset()).toBe(FAB_STACK_OFFSET)

    document.documentElement.classList.remove('mbr-embedded')
    host.requestUpdate()
    await host.updateComplete
    expect(host.fab.visible).toBe(false)
    expect(offset()).toBe('')
  })

  it('withdraws the offset when the host stops showing a button, or leaves', async () => {
    stubMatchMedia(true)
    const host = await mount()
    host.playable = false
    host.requestUpdate()
    await host.updateComplete
    expect(offset()).toBe('')

    host.playable = true
    host.requestUpdate()
    await host.updateComplete
    expect(offset()).toBe(FAB_STACK_OFFSET)

    host.remove()
    expect(offset()).toBe('')
  })

  it('stops listening once disconnected', async () => {
    const media = stubMatchMedia(true)
    const host = await mount()
    expect(media.listeners.size).toBe(1)
    host.remove()
    expect(media.listeners.size).toBe(0)
  })

  it('keeps the offset while another host still shows a button', async () => {
    stubMatchMedia(true)
    const first = await mount()
    const second = await mount()
    expect(offset()).toBe(FAB_STACK_OFFSET)

    first.remove()
    expect(offset()).toBe(FAB_STACK_OFFSET)

    second.playable = false
    second.requestUpdate()
    await second.updateComplete
    expect(offset()).toBe('')
  })
})

describe('FAB_CSS', () => {
  const text = FAB_CSS.cssText

  it('clears the safe-area insets', () => {
    expect(text).toContain('right: calc(1rem + env(safe-area-inset-right, 0px))')
    expect(text).toContain('bottom: calc(1rem + env(safe-area-inset-bottom, 0px))')
  })

  it('hides the wide nav button where the floating one takes over, and prints neither', () => {
    expect(text).toMatch(/@media \(max-width: 600px\)\s*\{\s*\.play-slides-btn\s*\{\s*display: none/)
    expect(text).toMatch(/@media print\s*\{\s*\.mbr-fab\s*\{\s*display: none/)
  })
})
