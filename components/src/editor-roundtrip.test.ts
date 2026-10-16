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
 * pins the *exact* current damage to the user's flashcard decks, so a Milkdown
 * upgrade that changes it is noticed, plus behaviour around a save (an edit
 * elsewhere, a second save, the no-change save). When definition lists land
 * (TODO item 1) these pins change to the fixture itself.
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

// Everything above `: Memory aids:` survives; the two nested lists do not.
// `  * x` interrupts the `: Memory aids:` paragraph as a top-level list (blank
// line + column 0), and `: ___Review History___` becomes a lazy continuation of
// that list item's paragraph, printed under it at the item's indent.
const SAVED_A09 = CARD_A09.replace(
  `: Memory aids:
  * Remember: "Frequency coordinator" = someone recognized by local/regional amateur operators (not an FCC office).
: ___Review History___
  * 2026-10-07 14:53 - Good
`,
  `: Memory aids:

* Remember: "Frequency coordinator" = someone recognized by local/regional amateur operators (not an FCC office).
  : ___Review History___
* 2026-10-07 14:53 - Good
`,
)

// The presumed original of the user's second damaged card (T1-B11).
const CARD_B11 = fixture('flashcards-b11')

// The file as the user found it, byte for byte (its body; the user's
// frontmatter is not part of the damage).
const DAMAGED_B11 = fixture('flashcards-b11-damaged')

/** `source` without its frontmatter, as the body Crepe sees. */
const bodyOf = (source: string): string => source.slice(source.indexOf('\n---\n', 1) + 5)

describe('editor round trip: definition lists (flashcard decks)', () => {
  it('a save with no edits rewrites the nested lists (exact current output)', async () => {
    expect(await saved(CARD_A09)).toBe(SAVED_A09)
  })

  it('a save after editing an unrelated paragraph does the same damage', async () => {
    const out = await saved(CARD_A09, typeAtStart('EDITED '))
    expect(out).toBe(SAVED_A09.replace('**T1-A09', '**EDITED T1-A09'))
  })

  it('reproduces the user\'s damaged T1-B11 card from its presumed original in one save', async () => {
    expect(bodyOf(await saved(CARD_B11))).toBe(DAMAGED_B11)
  })

  it('the damaged form is a fixed point: later saves change nothing more', async () => {
    expect(await saved(DAMAGED_B11)).toBe(DAMAGED_B11)
  })

  // Byte-identical preservation of these decks, of a `-` review history and of
  // flat definition lines: `editor-parity.test.ts` (flashcards-* fixtures).
})

describe('editor round trip: cosmetic normalization (TODO item 6)', () => {
  // Section attributes, `[-]`/`[>]` and wikilinks: `editor-parity.test.ts`.
  it('normalizes `-` bullets to `*` (cosmetic, but a whole-file diff)', async () => {
    expect(await saved(`- a\n- b\n`)).toBe(`* a\n* b\n`)
  })
})

describe('openEditor: a save with nothing changed writes nothing', () => {
  const RAW = CARD_A09

  async function open(): Promise<{ posts: RequestInit[]; onSaved: ReturnType<typeof vi.fn>; modal: Element }> {
    const posts: RequestInit[] = []
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_url: string, init?: RequestInit) => {
        if (init?.method === 'POST') {
          posts.push(init)
          return new Response('', { status: 200, headers: { 'X-MBR-Content-Hash': 'h2' } })
        }
        return new Response(RAW, { status: 200, headers: { 'X-MBR-Content-Hash': 'h1' } })
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
