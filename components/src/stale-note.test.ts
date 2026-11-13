import { describe, it, expect, vi } from 'vitest'
import {
  isLineEndingRefusal,
  NOTE_CHANGED_MESSAGE,
  NOTE_DOWNLOADING_MESSAGE,
  RELOAD_QUESTION,
  offerReload,
  staleKindForRead,
  staleKindForWrite,
  staleNoteMessage,
} from './stale-note.js'

describe('staleKindForRead', () => {
  it.each([
    [404, 'changed'],
    [409, 'changed'],
    [503, 'downloading'],
    [401, null],
    [403, null],
    [500, null],
    [0, null],
  ] as const)('%i → %s', (status, kind) => {
    expect(staleKindForRead(status)).toBe(kind)
  })
})

describe('staleKindForWrite', () => {
  it.each([
    [409, 'changed'],
    [503, 'downloading'],
    [404, null],
    [400, null],
    [401, null],
    [403, null],
    [500, null],
  ] as const)('%i → %s', (status, kind) => {
    expect(staleKindForWrite(status)).toBe(kind)
  })
})

describe('staleNoteMessage', () => {
  it('says what happened for each kind', () => {
    expect(staleNoteMessage('changed')).toBe(NOTE_CHANGED_MESSAGE)
    expect(staleNoteMessage('downloading')).toBe(NOTE_DOWNLOADING_MESSAGE)
    expect(NOTE_CHANGED_MESSAGE).toMatch(/changed somewhere else/)
    expect(NOTE_DOWNLOADING_MESSAGE).toMatch(/still downloading/)
  })
})

describe('offerReload', () => {
  it('asks with the statement and the question, and reloads on yes', () => {
    const confirm = vi.fn(() => true)
    const reload = vi.fn()
    expect(offerReload('It moved.', { confirm, reload })).toBe(true)
    expect(confirm).toHaveBeenCalledWith(`It moved. ${RELOAD_QUESTION}`)
    expect(reload).toHaveBeenCalledOnce()
  })

  it('does nothing on no', () => {
    const reload = vi.fn()
    expect(offerReload(undefined, { confirm: () => false, reload })).toBe(false)
    expect(reload).not.toHaveBeenCalled()
  })

  it("defaults to the changed-note statement and the page's own confirm", () => {
    const confirm = vi.fn(() => false)
    const original = window.confirm
    window.confirm = confirm
    try {
      expect(offerReload()).toBe(false)
      expect(confirm).toHaveBeenCalledWith(`${NOTE_CHANGED_MESSAGE} ${RELOAD_QUESTION}`)
    } finally {
      window.confirm = original
    }
  })
})

describe('isLineEndingRefusal', () => {
  const json = (body: unknown, status: number) => new Response(JSON.stringify(body), { status })

  it('is true only for a 422 naming the lone-carriage-return reason', async () => {
    expect(await isLineEndingRefusal(json({ error: 'x', reason: 'lone_carriage_return' }, 422))).toBe(true)
    expect(await isLineEndingRefusal(json({ error: 'x', reason: 'lone_carriage_return' }, 409))).toBe(false)
    expect(await isLineEndingRefusal(json({ error: 'x', reason: 'other' }, 422))).toBe(false)
    expect(await isLineEndingRefusal(new Response('not json', { status: 422 }))).toBe(false)
  })

  it('leaves the body readable for the caller', async () => {
    const response = json({ reason: 'lone_carriage_return' }, 422)
    await isLineEndingRefusal(response)
    expect(await response.json()).toEqual({ reason: 'lone_carriage_return' })
  })
})
