import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import './mbr-media-viewer.js'
import type { MbrMediaViewerElement } from './mbr-media-viewer.js'
import {
  MEDIA_ERROR_EVENT,
  clearPublishedPageErrors,
  publishPageErrors,
  type MediaErrorEventDetail,
  type UnplayableMediaError,
} from './media-errors.js'

const PATH = '/videos/Foo Bar.mp4'

const GOPRO: UnplayableMediaError = {
  type: 'unplayable_media',
  src: '/videos/Foo%20Bar.mp4',
  kind: 'video',
  reason: "Contains a 'gpmd' timed-metadata track that Safari/WebKit cannot decode.",
  remedy: 'ffmpeg -i in.mp4 -map 0:v -map 0:a -c copy -movflags +faststart out.mp4',
}

describe('<mbr-media-viewer> video failures', () => {
  let el: MbrMediaViewerElement
  let reported: MediaErrorEventDetail[]
  const originalHref = window.location.href

  const collect = (e: CustomEvent<MediaErrorEventDetail>) => {
    reported.push(e.detail)
  }

  async function mount(): Promise<HTMLVideoElement> {
    window.location.href = `/media/?path=${encodeURIComponent(PATH)}`
    el = document.createElement('mbr-media-viewer')
    el.setAttribute('media-type', 'video')
    document.body.appendChild(el)
    await el.updateComplete
    return el.shadowRoot!.querySelector('video')!
  }

  function fail(video: HTMLVideoElement, code: number, message: string): void {
    Object.defineProperty(video, 'error', {
      value: { code, message },
      configurable: true,
    })
    video.dispatchEvent(new Event('error'))
  }

  function noticeText(): string {
    const notice = el.shadowRoot?.querySelector('.media-error')
    return notice?.textContent?.replace(/\s+/g, ' ').trim() ?? ''
  }

  beforeEach(() => {
    clearPublishedPageErrors()
    // Server mode: resolveUrl keeps the absolute path, matching how the viewer
    // page and errors.json coexist at runtime.
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
    reported = []
    document.addEventListener(MEDIA_ERROR_EVENT, collect)
  })

  afterEach(() => {
    document.removeEventListener(MEDIA_ERROR_EVENT, collect)
    document.body.innerHTML = ''
    window.location.href = originalHref
    window.__MBR_CONFIG__ = undefined
    clearPublishedPageErrors()
  })

  it('shows nothing until playback fails', async () => {
    await mount()
    expect(el.shadowRoot?.querySelector('.media-error')).toBeNull()
  })

  it('replaces the silent console.warn with a visible message', async () => {
    const video = await mount()
    fail(video, 3, 'Media failed to decode')
    await el.updateComplete

    expect(noticeText()).toMatch(/could not decode/i)
    expect(el.shadowRoot?.querySelector('.media-error')?.getAttribute('role')).toBe(
      'alert'
    )
  })

  it('keeps the player and the direct link reachable after a failure', async () => {
    const video = await mount()
    fail(video, 3, 'Media failed to decode')
    await el.updateComplete

    expect(el.shadowRoot?.querySelector('video')).not.toBeNull()
    expect(el.shadowRoot?.querySelector('.directlink a')).not.toBeNull()
  })

  it('reports the failure on the shared mbr-media-error channel', async () => {
    const video = await mount()
    fail(video, 3, 'Media failed to decode')
    await el.updateComplete

    expect(reported).toHaveLength(1)
    expect(reported[0].code).toBe(3)
    expect(reported[0].kind).toBe('video')
  })

  it('uses the server diagnosis when one exists for this file', async () => {
    publishPageErrors([GOPRO])
    const video = await mount()
    fail(video, 3, 'Media failed to decode')
    await el.updateComplete

    expect(noticeText()).toMatch(/gpmd/)
    expect(noticeText()).toContain(GOPRO.remedy!)
  })

  it('suppresses the nested caption notice so the message appears once', async () => {
    await mount()
    const extras = el.shadowRoot?.querySelector('mbr-video-extras')
    expect(extras?.hasAttribute('suppress-error')).toBe(true)
  })
})
