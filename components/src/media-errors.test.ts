import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import {
  MEDIA_ERROR_EVENT,
  PAGE_ERRORS_LOADED_EVENT,
  buildMediaErrorNotice,
  clearPublishedPageErrors,
  describeMediaError,
  findUnplayableMedia,
  isUnplayableMediaError,
  mergeRuntimeMediaErrors,
  normalizeMediaSrc,
  publishPageErrors,
  reportMediaError,
  sameMediaSrc,
  type MediaErrorEventDetail,
  type PageErrorsLoadedEventDetail,
  type RuntimeMediaError,
  type UnplayableMediaError,
} from './media-errors.js'

const GOPRO: UnplayableMediaError = {
  type: 'unplayable_media',
  src: '../Foo%20Bar.mp4',
  kind: 'video',
  reason: "Contains a 'gpmd' timed-metadata track that Safari/WebKit cannot decode.",
  remedy: 'ffmpeg -i in.mp4 -map 0:v -map 0:a -c copy -movflags +faststart out.mp4',
}

describe('normalizeMediaSrc', () => {
  it('resolves relative paths against the current page', () => {
    expect(normalizeMediaSrc('videos/demo.mp4')).toBe(
      normalizeMediaSrc(new URL('videos/demo.mp4', location.href).pathname)
    )
  })

  it('percent-decodes so encoded and literal srcs match', () => {
    expect(normalizeMediaSrc('/videos/Foo%20Bar.mp4')).toBe('/videos/Foo Bar.mp4')
    expect(normalizeMediaSrc("/videos/Rubik%27s.mp4")).toBe("/videos/Rubik's.mp4")
  })

  it('drops query strings and fragments', () => {
    expect(normalizeMediaSrc('/videos/demo.mp4?t=10#frag')).toBe('/videos/demo.mp4')
  })

  it('falls back to the bare path for malformed input', () => {
    // Lone "%" is not valid percent-encoding; must not throw.
    expect(() => normalizeMediaSrc('%')).not.toThrow()
  })

  it('treats absolute and relative forms of the same file as equal', () => {
    expect(sameMediaSrc('/videos/Foo Bar.mp4', '/videos/Foo%20Bar.mp4')).toBe(true)
    expect(sameMediaSrc('/videos/a.mp4', '/videos/b.mp4')).toBe(false)
  })
})

describe('describeMediaError', () => {
  it('maps each MediaError code to plain English (no enum names)', () => {
    expect(describeMediaError(1).headline).toMatch(/aborted/i)
    expect(describeMediaError(2).headline).toMatch(/network error/i)
    expect(describeMediaError(3).headline).toMatch(/could not decode/i)
    expect(describeMediaError(4).headline).toMatch(/format or MIME type/i)

    for (const code of [1, 2, 3, 4]) {
      expect(describeMediaError(code).headline).not.toMatch(/MEDIA_ERR/)
    }
  })

  it('explains that extra tracks can cause a decode failure', () => {
    expect(describeMediaError(3).headline).toMatch(/extra tracks/i)
  })

  it('falls back to a generic message for unknown codes', () => {
    expect(describeMediaError(0).headline).toMatch(/could not be played/i)
    expect(describeMediaError(undefined).headline).toMatch(/could not be played/i)
  })

  it('adds the browser message as secondary detail', () => {
    const notice = describeMediaError(3, 'Media failed to decode')
    expect(notice.detail).toBe('Media failed to decode')
  })

  it('omits an empty or redundant browser message', () => {
    expect(describeMediaError(3, '   ').detail).toBeUndefined()
    expect(describeMediaError(3, undefined).detail).toBeUndefined()
    const headline = describeMediaError(3).headline
    expect(describeMediaError(3, headline).detail).toBeUndefined()
  })

  it('uses an audio-appropriate noun for audio', () => {
    expect(describeMediaError(3, null, 'audio').headline).toMatch(/audio file/i)
  })
})

describe('buildMediaErrorNotice', () => {
  it('uses the generic code text when the server has no diagnosis', () => {
    const notice = buildMediaErrorNotice({ code: 3, message: 'Media failed to decode' })
    expect(notice.headline).toMatch(/could not decode/i)
    expect(notice.remedy).toBeUndefined()
  })

  it('prefers the server reason and remedy over the generic code-3 text', () => {
    const notice = buildMediaErrorNotice({
      code: 3,
      message: 'Media failed to decode',
      diagnosis: GOPRO,
    })
    expect(notice.headline).toMatch(/cannot be played in this browser/i)
    expect(notice.detail).toBe(GOPRO.reason)
    expect(notice.remedy).toBe(GOPRO.remedy)
  })
})

describe('findUnplayableMedia', () => {
  beforeEach(() => {
    clearPublishedPageErrors()
  })

  it('matches on the canonical src, not the raw string', () => {
    const found = findUnplayableMedia('/Foo Bar.mp4', [GOPRO])
    expect(found).toBe(GOPRO)
  })

  it('ignores entries of other types and non-matching srcs', () => {
    const others = [
      { type: 'broken_internal_link', target: '/x/', text: 'x' },
      { type: 'unplayable_media', src: '/other.mp4', kind: 'video', reason: 'r' },
    ]
    expect(findUnplayableMedia('/Foo Bar.mp4', others)).toBeNull()
  })

  it('returns null for an empty src', () => {
    expect(findUnplayableMedia('', [GOPRO])).toBeNull()
  })

  it('consults the last published payload by default', () => {
    expect(findUnplayableMedia('/Foo Bar.mp4')).toBeNull()
    publishPageErrors([GOPRO])
    expect(findUnplayableMedia('/Foo Bar.mp4')).toBe(GOPRO)
    clearPublishedPageErrors()
    expect(findUnplayableMedia('/Foo Bar.mp4')).toBeNull()
  })
})

describe('isUnplayableMediaError', () => {
  it('narrows only the unplayable_media variant', () => {
    expect(isUnplayableMediaError(GOPRO)).toBe(true)
    expect(isUnplayableMediaError({ type: 'unresolved_wikilink' })).toBe(false)
  })
})

describe('event channel', () => {
  let received: unknown[] = []

  beforeEach(() => {
    received = []
    clearPublishedPageErrors()
  })

  afterEach(() => {
    clearPublishedPageErrors()
  })

  it('publishPageErrors announces the loaded payload', () => {
    const listener = (e: CustomEvent<PageErrorsLoadedEventDetail>) => received.push(e.detail)
    document.addEventListener(PAGE_ERRORS_LOADED_EVENT, listener)
    publishPageErrors([GOPRO])
    document.removeEventListener(PAGE_ERRORS_LOADED_EVENT, listener)

    expect(received).toEqual([{ errors: [GOPRO] }])
  })

  it('reportMediaError dispatches the failure detail', () => {
    const listener = (e: CustomEvent<MediaErrorEventDetail>) => received.push(e.detail)
    document.addEventListener(MEDIA_ERROR_EVENT, listener)
    const detail = reportMediaError('/videos/a.mp4', 'video', {
      code: 3,
      message: 'Media failed to decode',
    } as MediaError)
    document.removeEventListener(MEDIA_ERROR_EVENT, listener)

    expect(detail).toEqual({
      src: '/videos/a.mp4',
      kind: 'video',
      code: 3,
      message: 'Media failed to decode',
    })
    expect(received).toEqual([detail])
  })

  it('reportMediaError tolerates a missing MediaError', () => {
    const detail = reportMediaError('/videos/a.mp4', 'video', null)
    expect(detail.code).toBe(0)
    expect(detail.message).toBe('')
  })
})

describe('mergeRuntimeMediaErrors', () => {
  const runtime = (src: string): RuntimeMediaError => ({
    type: 'runtime_media_error',
    src,
    kind: 'video',
    code: 3,
    message: 'Media failed to decode',
  })

  it('keeps runtime errors the server knows nothing about', () => {
    const merged = mergeRuntimeMediaErrors([], [runtime('/videos/a.mp4')])
    expect(merged).toHaveLength(1)
    expect(merged[0].type).toBe('runtime_media_error')
  })

  it('prefers the richer server entry for the same file', () => {
    const merged = mergeRuntimeMediaErrors([GOPRO], [runtime('/Foo Bar.mp4')])
    expect(merged).toEqual([GOPRO])
  })

  it('dedupes repeated runtime reports for the same src', () => {
    const merged = mergeRuntimeMediaErrors(
      [],
      [runtime('/videos/Foo Bar.mp4'), runtime('/videos/Foo%20Bar.mp4')]
    )
    expect(merged).toHaveLength(1)
  })

  it('leaves unrelated server errors untouched and keeps ordering', () => {
    const link = { type: 'broken_internal_link' as const, target: '/x/', text: 'x' }
    const merged = mergeRuntimeMediaErrors([link], [runtime('/videos/a.mp4')])
    expect(merged[0]).toBe(link)
    expect(merged[1].type).toBe('runtime_media_error')
  })
})
