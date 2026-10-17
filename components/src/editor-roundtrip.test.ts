/**
 * What the editor does to a file it saves.
 *
 * The editor never saves the file it loaded: it saves Crepe's re-serialization
 * of it — `recombine(frontmatter, serializeBody(crepe))` —
 * which is remark (CommonMark + GFM) parsing the body into ProseMirror and
 * printing it back. Anything remark has no node for is reinterpreted as
 * CommonMark and printed as such. These tests run the *real* Crepe, with the
 * editor's own configuration (`editor-test-support.ts`), under happy-dom;
 * nothing here is a model of the pipeline.
 *
 * The byte-identical target for every construct lives in the parity harness
 * (`editor-parity.test.ts`, over `tests/fixtures/editor-roundtrip/`). This file
 * pins the user's flashcard decks — damaged by saves before definition lists
 * were taught to the editor (`editor-plugins/definition-list.ts`) — plus
 * behaviour around a save (an edit elsewhere, a second save, the no-change
 * save).
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Crepe } from '@milkdown/crepe'
import { editorViewCtx } from '@milkdown/kit/core'
import { openEditor } from './editor-crepe.js'
import { destroyEditors, fixture, savedFile } from './editor-test-support.js'

afterEach(destroyEditors)

/** What a save writes for `source`, with or without an edit. */
const saved = savedFile

/** Types `text` at the start of the first block, far from the cards. */
function typeAtStart(text: string) {
  return (crepe: Crepe): void => {
    crepe.editor.action((ctx) => {
      const view = ctx.get(editorViewCtx)
      view.dispatch(view.state.tr.insertText(text, 1))
    })
  }
}

// The user's card, verbatim (T1-A09), with deck frontmatter.
const CARD_A09 = fixture('flashcards-a09')

// The presumed original of the user's second damaged card (T1-B11).
const CARD_B11 = fixture('flashcards-b11')

// The file as the user found it, byte for byte (its body; the user's
// frontmatter is not part of the damage).
const DAMAGED_B11 = fixture('flashcards-b11-damaged')

/** `source` without its frontmatter, as the body Crepe sees. */
const bodyOf = (source: string): string => source.slice(source.indexOf('\n---\n', 1) + 5)

describe('editor round trip: definition lists (flashcard decks)', () => {
  it('a save with no edits writes the deck back byte for byte', async () => {
    expect(await saved(CARD_A09)).toBe(CARD_A09)
  })

  it('a save after editing an unrelated paragraph changes only that paragraph', async () => {
    const out = await saved(CARD_A09, typeAtStart('EDITED '))
    expect(out).toBe(CARD_A09.replace('**T1-A09', '**EDITED T1-A09'))
  })

  it('no longer turns T1-B11 into the damaged card the user found', async () => {
    const out = await saved(CARD_B11)
    expect(out).toBe(CARD_B11)
    expect(bodyOf(out)).not.toBe(DAMAGED_B11)
  })

  it('the damaged form is a fixed point: later saves change nothing more', async () => {
    expect(await saved(DAMAGED_B11)).toBe(DAMAGED_B11)
  })

  // Byte-identical preservation of these decks, of a `-` review history and of
  // flat definition lines: `editor-parity.test.ts` (flashcards-* fixtures).
})

describe('editor round trip: authored spelling (TODO item 6)', () => {
  // Section attributes, `[-]`/`[>]` and wikilinks: `editor-parity.test.ts`;
  // each spelling, edited: `editor-plugins/authored-style.test.ts`.
  it('keeps `-` bullets (remark would write `*`)', async () => {
    expect(await saved(`- a\n- b\n`)).toBe(`- a\n- b\n`)
  })
})

/** Opens the real editor on `raw`, with `fetch` stubbed; POSTs are recorded. */
async function openOn(raw: string): Promise<{ posts: RequestInit[]; onSaved: ReturnType<typeof vi.fn>; modal: Element }> {
  const posts: RequestInit[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (_url: string, init?: RequestInit) => {
      if (init?.method === 'POST') {
        posts.push(init)
        return new Response('', { status: 200, headers: { 'X-MBR-Content-Hash': 'h2' } })
      }
      return new Response(raw, { status: 200, headers: { 'X-MBR-Content-Hash': 'h1' } })
    }),
  )
  const onSaved = vi.fn()
  await openEditor({
    rawUrl: '/.mbr/raw/deck.md',
    saveUrl: '/.mbr/edit/deck.md',
    filePath: 'deck.md',
    onClose: () => {},
    onSaved,
  })
  const modal = document.querySelector('.mbr-editor-modal')
  if (!modal) throw new Error('editor modal not built')
  return { posts, onSaved, modal }
}

const clickSave = async (modal: Element): Promise<void> => {
  ;(modal.querySelector('.mbr-editor-save') as HTMLButtonElement).click()
  await new Promise((r) => setTimeout(r, 0))
}

/** The `content` a recorded POST sent. */
const postedContent = (init: RequestInit): string => (JSON.parse(String(init.body)) as { content: string }).content

describe('openEditor: a save with nothing changed writes nothing', () => {
  const RAW = CARD_A09
  const open = () => openOn(RAW)

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('Save on an untouched deck sends no request and does not reload', async () => {
    const { posts, onSaved, modal } = await open()
    await clickSave(modal)
    expect(posts).toHaveLength(0)
    expect(onSaved).not.toHaveBeenCalled()
    expect(modal.querySelector('.status')?.textContent).toBe('No changes to save.')
  })

  it('Save after a frontmatter edit still posts', async () => {
    const { posts, onSaved, modal } = await open()
    const fm = modal.querySelector('textarea') as HTMLTextAreaElement
    fm.value = fm.value.replace('T1-A', 'T1-A2')
    await clickSave(modal)
    expect(posts).toHaveLength(1)
    expect(onSaved).toHaveBeenCalledOnce()
  })
})

describe('openEditor: the safety net', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    document.querySelector('.mbr-editor-overlay')?.remove()
  })

  // A list's gap after the marker is its first item's: `- second` is printed `-  second`.
  const INEXACT = '---\ntitle: t\n---\n# Note\n\n-  item\n- second item\n\nIntro.\n'

  it('names the lines the editor would write differently', async () => {
    const { modal } = await openOn(INEXACT)
    const notice = modal.querySelector('.mbr-editor-notice')
    expect(notice?.textContent).toContain('Lines 6–7')
    expect(notice?.textContent).toContain('saved as written unless you edit them')
  })

  it('shows no notice when the round trip is exact', async () => {
    const { modal } = await openOn(CARD_A09)
    expect(modal.querySelector('.mbr-editor-notice')).toBeNull()
  })

  it('a frontmatter edit leaves the body byte for byte, inexact lines included', async () => {
    const { posts, modal } = await openOn(INEXACT)
    const fm = modal.querySelector('textarea') as HTMLTextAreaElement
    fm.value = 'title: t2\n'
    await clickSave(modal)
    expect(posts).toHaveLength(1)
    expect(postedContent(posts[0])).toBe(INEXACT.replace('title: t', 'title: t2'))
  })
})

