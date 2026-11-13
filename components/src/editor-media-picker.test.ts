import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  openMediaPicker,
  type MediaCatalogUpdate,
  type MediaFile,
  type MediaPickResult,
} from './editor-media-picker.js'

/** A media.json entry; `title` stands in for metadata the server probes later. */
function file(url_path: string, type = 'image', title?: string): MediaFile {
  return { url_path, metadata: { kind: { type, ...(title ? { title } : {}) } } }
}

/** A controllable catalog subscription, like `<mbr-editor>`'s over `shared.ts`. */
function catalog() {
  let listener: ((update: MediaCatalogUpdate) => void) | null = null
  const unsubscribe = vi.fn(() => {
    listener = null
  })
  return {
    subscribe: (onUpdate: (update: MediaCatalogUpdate) => void) => {
      listener = onUpdate
      onUpdate({ files: null, error: null })
      return unsubscribe
    },
    push: (files: MediaFile[]) => listener?.({ files, error: null }),
    unsubscribe,
    get subscribed() {
      return listener !== null
    },
  }
}

const rows = () => [...document.querySelectorAll('.mbr-picker-item-sub')].map((el) => el.textContent)
const mains = () => [...document.querySelectorAll('.mbr-picker-item-main')].map((el) => el.textContent)
const selectedRows = () =>
  [...document.querySelectorAll('.mbr-picker-item.selected .mbr-picker-item-sub')].map((el) => el.textContent)
const search = () => document.querySelector<HTMLInputElement>('input[aria-label="Filter media"]')!
const confirm = () => [...document.querySelectorAll<HTMLButtonElement>('.mbr-picker-footer button')].pop()!
const cancel = () => document.querySelector<HTMLButtonElement>('.mbr-picker-close')!

afterEach(() => {
  document.body.textContent = ''
})

describe('media picker: a catalog that fills in while open', () => {
  it('updates the list in place, keeping the query and the selection', async () => {
    const media = catalog()
    const fetchMediaFiles = vi.fn(() => Promise.resolve([]))
    const picked = openMediaPicker({ mode: 'insert', subscribeMediaFiles: media.subscribe, fetchMediaFiles })
    expect(document.querySelector('.mbr-picker-status')?.textContent).toBe('Loading media…')

    media.push([file('/img/cat.png'), file('/img/dog.png'), file('/notes.txt', 'text')])
    expect(rows()).toEqual(['/img/cat.png', '/img/dog.png'])

    search().value = 'dog'
    search().dispatchEvent(new Event('input'))
    document.querySelector<HTMLElement>('.mbr-picker-item')!.click()
    expect(selectedRows()).toEqual(['/img/dog.png'])

    // The next poll: metadata arrived, and a new file matching the query.
    media.push([file('/img/cat.png', 'image', 'Cat'), file('/img/dog.png', 'image', 'Dog'), file('/img/dogs.png')])
    expect(search().value).toBe('dog')
    expect(mains()).toContain('Dog')
    expect(rows()).toContain('/img/dogs.png')
    expect(selectedRows()).toEqual(['/img/dog.png'])

    confirm().click()
    const result: MediaPickResult | null = await picked
    expect(result).toEqual({ form: 'image', src: '/img/dog.png', caption: '' })
    // Unsubscribed on close; the one-shot fetch is never used with a subscription.
    expect(media.unsubscribe).toHaveBeenCalledOnce()
    expect(media.subscribed).toBe(false)
    expect(fetchMediaFiles).not.toHaveBeenCalled()
  })

  it('unsubscribes when cancelled', async () => {
    const media = catalog()
    const picked = openMediaPicker({
      mode: 'insert',
      subscribeMediaFiles: media.subscribe,
      fetchMediaFiles: () => Promise.resolve([]),
    })
    cancel().click()
    expect(await picked).toBeNull()
    expect(media.unsubscribe).toHaveBeenCalledOnce()
  })

  it('preselects the embed being edited once, then follows updates', async () => {
    const media = catalog()
    const picked = openMediaPicker({
      mode: 'edit',
      initial: { src: '/img/b.png', caption: 'B' },
      subscribeMediaFiles: media.subscribe,
      fetchMediaFiles: () => Promise.resolve([]),
    })
    media.push([file('/img/a.png'), file('/img/b.png')])
    expect(selectedRows()).toEqual(['/img/b.png'])
    expect(document.querySelector('.mbr-picker-status')?.textContent).toBe('')

    media.push([file('/img/a.png'), file('/img/b.png', 'image', 'Bee')])
    expect(selectedRows()).toEqual(['/img/b.png'])
    confirm().click()
    expect(await picked).toEqual({ form: 'image', src: '/img/b.png', caption: 'B' })
  })

  it('says when the embed being edited is not in the catalog', async () => {
    const media = catalog()
    void openMediaPicker({
      mode: 'edit',
      initial: { src: '/img/gone.png', caption: '' },
      subscribeMediaFiles: media.subscribe,
      fetchMediaFiles: () => Promise.resolve([]),
    })
    media.push([file('/img/a.png')])
    expect(document.querySelector('.mbr-picker-status')?.textContent).toContain('not in media.json')
    cancel().click()
  })

  it('falls back to one fetch without a subscription', async () => {
    const fetchMediaFiles = vi.fn(() => Promise.resolve([file('/img/a.png')]))
    void openMediaPicker({ mode: 'insert', fetchMediaFiles })
    await vi.waitFor(() => expect(rows()).toEqual(['/img/a.png']))
    expect(fetchMediaFiles).toHaveBeenCalledOnce()
    cancel().click()
  })
})
