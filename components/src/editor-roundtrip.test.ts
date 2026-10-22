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
import { EditorView } from '@codemirror/view'
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

/** Types `value` into the properties form's field for the top-level `key`, as a user would. */
async function editProperty(modal: Element, key: string, value: string): Promise<void> {
  const form = modal.querySelector('mbr-fm-editor') as (HTMLElement & { updateComplete: Promise<boolean> }) | null
  if (!form) throw new Error('no properties editor')
  await form.updateComplete
  const row = [...form.querySelectorAll<HTMLElement>('.fm-row')].find((r) => r.dataset.path === JSON.stringify([key]))
  const input = row?.querySelector<HTMLInputElement>('.fm-value input')
  if (!input) throw new Error(`no field for ${key}`)
  input.value = value
  input.dispatchEvent(new Event('input', { bubbles: true }))
  await form.updateComplete
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
    await editProperty(modal, 'title', 'Technician pool T1-A2')
    await clickSave(modal)
    expect(posts).toHaveLength(1)
    expect(onSaved).toHaveBeenCalledOnce()
    expect(postedContent(posts[0])).toBe(RAW.replace('T1-A\n', 'T1-A2\n'))
  })
})

describe('openEditor: the safety net', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    document.querySelector('.mbr-editor-overlay')?.remove()
  })

  // A lazy line in a footnote is indented once edited (an accepted cosmetic).
  const INEXACT = '---\ntitle: t\n---\n# Note\n\nSee[^1].\n\n[^1]: a\nlazy\n\nIntro.\n'

  it('names the lines the editor would write differently', async () => {
    const { modal } = await openOn(INEXACT)
    const notice = modal.querySelector('.mbr-editor-notice')
    expect(notice?.textContent).toContain('Lines 8–9 ')
    expect(notice?.textContent).toContain('saved as written unless you edit them')
  })

  it('shows no notice when the round trip is exact', async () => {
    const { modal } = await openOn(CARD_A09)
    expect(modal.querySelector('.mbr-editor-notice')).toBeNull()
  })

  it('a property edit changes only its own bytes: comments, quoting and fences stay', async () => {
    const raw = "---\r\n# about\r\ntitle: 'Old'   # keep\r\ntags: [a,  b]\r\n\r\n---\r\n\r\n# Note\r\n"
    const { posts, modal } = await openOn(raw)
    await editProperty(modal, 'title', 'New')
    await clickSave(modal)
    expect(posts).toHaveLength(1)
    // Edited frontmatter is recombined (`---` fences, trailing blank lines
    // dropped), as it was with the textarea; inside it only the value moved.
    expect(postedContent(posts[0])).toBe("---\r\n# about\r\ntitle: 'New'   # keep\r\ntags: [a,  b]\r\n---\r\n\r\n# Note\r\n")
  })

  it('a frontmatter edit leaves the body byte for byte, inexact lines included', async () => {
    const { posts, modal } = await openOn(INEXACT)
    await editProperty(modal, 'title', 't2')
    await clickSave(modal)
    expect(posts).toHaveLength(1)
    expect(postedContent(posts[0])).toBe(INEXACT.replace('title: t', 'title: t2'))
  })
})

describe('openEditor: property edits not yet applied', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    document.querySelector('.mbr-picker-backdrop')?.remove()
  })

  const RAW = '---\ntitle: t\ntags: [a]\nother: &o 1\n---\n# Note\n'
  type Form = HTMLElement & { updateComplete: Promise<boolean> }
  const formOf = async (modal: Element): Promise<Form> => {
    const form = modal.querySelector('mbr-fm-editor') as Form
    await form.updateComplete
    return form
  }
  const rowOf = (form: Element, key: string) => [...form.querySelectorAll<HTMLElement>('.fm-row')].find((r) => r.dataset.path === JSON.stringify([key]))!

  /** Types a tag into the tags field without pressing Enter. */
  const typeTag = async (form: Form, tag: string) => {
    const input = rowOf(form, 'tags').querySelector<HTMLInputElement>('mbr-fm-combobox input')!
    input.value = tag
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await form.updateComplete
  }

  /** Opens "Edit as YAML" on `key` and types `yaml` into it, without applying. */
  const editAsYaml = async (form: Form, key: string, yaml: string) => {
    rowOf(form, key).querySelector<HTMLButtonElement>('.fm-value .fm-link')!.click()
    await form.updateComplete
    const dom = rowOf(form, key).querySelector<HTMLElement>('.fm-yaml-slice .cm-editor')!
    const view = EditorView.findFromDOM(dom)!
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: yaml }, userEvent: 'input.type' })
  }

  it('Save writes a tag typed without Enter and an open "Edit as YAML"', async () => {
    const { posts, modal } = await openOn(RAW)
    const form = await formOf(modal)
    await typeTag(form, 'b')
    await editAsYaml(form, 'other', '&o 2')
    await clickSave(modal)
    expect(posts).toHaveLength(1)
    expect(postedContent(posts[0])).toBe(RAW.replace('[a]', '[a, b]').replace('&o 1', '&o 2'))
  })

  it('Save refuses YAML in an open "Edit as YAML" that does not parse, and says why', async () => {
    const { posts, modal } = await openOn(RAW)
    await editAsYaml(await formOf(modal), 'other', '[1')
    await clickSave(modal)
    expect(posts).toHaveLength(0)
    expect(modal.querySelector('.status')?.textContent).toMatch(/YAML for “other” is not valid/)
  })

  it('Cancel asks first when the only change is a property still being typed', async () => {
    const { modal } = await openOn(RAW)
    await typeTag(await formOf(modal), 'b')
    ;(modal.querySelector('.mbr-editor-cancel') as HTMLButtonElement).click()
    await new Promise((r) => setTimeout(r, 0))
    expect(document.querySelector('.mbr-picker-backdrop[aria-label="Unsaved changes"]')).not.toBeNull()
    expect(document.querySelector('.mbr-editor-modal')).not.toBeNull()
  })
})

