import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import './mbr-video-extras.js'
import type { MbrVideoExtrasElement } from './mbr-video-extras.js'
import {
  MEDIA_ERROR_EVENT,
  clearPublishedPageErrors,
  publishPageErrors,
  type MediaErrorEventDetail,
  type UnplayableMediaError,
} from './media-errors.js'

/**
 * Regression coverage for the "video goes broken with no feedback anywhere"
 * report: WebKit fires `loadedmetadata`, then fails with MediaError.code 3 on
 * MP4s carrying GoPro `gpmd` tracks. The failure used to be swallowed entirely.
 */

const SRC = '/videos/Foo Bar.mp4'

const GOPRO: UnplayableMediaError = {
  type: 'unplayable_media',
  // Deliberately encoded and relative: matching must be canonical, not literal.
  src: '../videos/Foo%20Bar.mp4',
  kind: 'video',
  reason: "Contains a 'gpmd' timed-metadata track that Safari/WebKit cannot decode.",
  remedy: 'ffmpeg -i in.mp4 -map 0:v -map 0:a -c copy -movflags +faststart out.mp4',
}

/** Wait for the element's deferred (rAF + async) setup to finish. */
async function settle(el: MbrVideoExtrasElement): Promise<void> {
  await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
  await new Promise<void>((resolve) => setTimeout(resolve, 0))
  await el.updateComplete
}

/** Give the <video> a MediaError, as the browser would, then fire `error`. */
function failVideo(video: HTMLVideoElement, code: number, message: string): void {
  Object.defineProperty(video, 'error', {
    value: { code, message },
    configurable: true,
  })
  video.dispatchEvent(new Event('error'))
}

describe('<mbr-video-extras> media failures', () => {
  let figure: HTMLElement
  let video: HTMLVideoElement
  let extras: MbrVideoExtrasElement
  let reported: MediaErrorEventDetail[]

  function build(attrs = ''): void {
    figure = document.createElement('figure')
    figure.innerHTML = `
      <video src="${SRC}"></video>
      <figcaption><mbr-video-extras src="${SRC}" ${attrs}></mbr-video-extras></figcaption>
    `
    document.body.appendChild(figure)
    video = figure.querySelector('video')!
    extras = figure.querySelector('mbr-video-extras')!
  }

  const collect = (e: CustomEvent<MediaErrorEventDetail>) => {
    reported.push(e.detail)
  }

  beforeEach(() => {
    clearPublishedPageErrors()
    reported = []
    document.addEventListener(MEDIA_ERROR_EVENT, collect)
  })

  afterEach(() => {
    document.removeEventListener(MEDIA_ERROR_EVENT, collect)
    document.body.innerHTML = ''
    clearPublishedPageErrors()
  })

  function noticeText(): string {
    const notice = extras.shadowRoot?.querySelector('.media-error')
    return notice?.textContent?.replace(/\s+/g, ' ').trim() ?? ''
  }

  it('renders nothing about errors until the video actually fails', async () => {
    build()
    await settle(extras)
    expect(extras.shadowRoot?.querySelector('.media-error')).toBeNull()
  })

  it('shows a plain-English decode message in the caption', async () => {
    build()
    await settle(extras)

    failVideo(video, 3, 'Media failed to decode')
    await extras.updateComplete

    expect(noticeText()).toMatch(/could not decode/i)
    // Underlying browser message kept as secondary detail.
    expect(noticeText()).toMatch(/Media failed to decode/)
    expect(noticeText()).not.toMatch(/MEDIA_ERR/)
  })

  it('marks the message as an alert for assistive tech', async () => {
    build()
    await settle(extras)
    failVideo(video, 3, 'Media failed to decode')
    await extras.updateComplete

    const notice = extras.shadowRoot?.querySelector('.media-error')
    expect(notice?.getAttribute('role')).toBe('alert')
  })

  it('maps the other MediaError codes too', async () => {
    const cases: Array<[number, RegExp]> = [
      [1, /aborted/i],
      [2, /network error/i],
      [4, /format or MIME type/i],
    ]

    for (const [code, expected] of cases) {
      build()
      await settle(extras)
      failVideo(video, code, '')
      await extras.updateComplete
      expect(noticeText()).toMatch(expected)
      document.body.innerHTML = ''
    }
  })

  it('reports the failure on the mbr-media-error channel', async () => {
    build()
    await settle(extras)
    failVideo(video, 3, 'Media failed to decode')
    await extras.updateComplete

    expect(reported).toEqual([
      { src: SRC, kind: 'video', code: 3, message: 'Media failed to decode' },
    ])
  })

  it('works with no <mbr-page-errors> on the page (static builds)', async () => {
    build()
    await settle(extras)
    failVideo(video, 3, 'Media failed to decode')
    await extras.updateComplete

    // No page errors were ever published; the generic message still shows.
    expect(noticeText()).toMatch(/could not decode/i)
  })

  it('uses the server diagnosis when errors.json loaded first', async () => {
    build()
    await settle(extras)

    publishPageErrors([GOPRO])
    failVideo(video, 3, 'Media failed to decode')
    await extras.updateComplete

    expect(noticeText()).toMatch(/gpmd/)
    expect(noticeText()).toContain(GOPRO.remedy!)
    expect(noticeText()).not.toMatch(/could not decode/i)
  })

  it('uses the server diagnosis when errors.json loads after the failure', async () => {
    build()
    await settle(extras)

    failVideo(video, 3, 'Media failed to decode')
    await extras.updateComplete
    expect(noticeText()).toMatch(/could not decode/i)

    publishPageErrors([GOPRO])
    await extras.updateComplete

    expect(noticeText()).toMatch(/gpmd/)
    expect(noticeText()).toContain(GOPRO.remedy!)
  })

  it('picks up a diagnosis published before the element connected', async () => {
    publishPageErrors([GOPRO])
    build()
    await settle(extras)

    failVideo(video, 3, 'Media failed to decode')
    await extras.updateComplete
    expect(noticeText()).toMatch(/gpmd/)
  })

  it('catches an error that fired before the deferred setup ran', async () => {
    build()
    // Fail immediately, before the rAF-deferred listener is attached.
    Object.defineProperty(video, 'error', {
      value: { code: 3, message: 'Media failed to decode' },
      configurable: true,
    })
    await settle(extras)

    expect(noticeText()).toMatch(/could not decode/i)
    expect(reported).toHaveLength(1)
  })

  it('stays silent when a host component owns error reporting', async () => {
    build('suppress-error')
    await settle(extras)

    failVideo(video, 3, 'Media failed to decode')
    await extras.updateComplete

    expect(extras.shadowRoot?.querySelector('.media-error')).toBeNull()
    expect(reported).toHaveLength(0)
  })
})
