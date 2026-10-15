/**
 * What the editor does to a file it saves.
 *
 * The editor never saves the file it loaded: it saves Crepe's re-serialization
 * of it — `recombine(frontmatter, unescapeWikilinks(crepe.getMarkdown()))` —
 * which is remark (CommonMark + GFM) parsing the body into ProseMirror and
 * printing it back. Anything remark has no node for is reinterpreted as
 * CommonMark and printed as such. These tests run the *real* Crepe, with the
 * editor's own feature set (`CREPE_FEATURES`), under happy-dom; nothing here is
 * a model of the pipeline.
 *
 * The `it.fails` cases pin damage that exists today. Each asserts the behaviour
 * we want, so when a fix lands they start passing and vitest reports them as
 * failures — flip them to `it` then. The `toBe(EXPECTED_…)` cases pin the exact
 * current output, so a Milkdown upgrade that changes the damage is noticed.
 */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { Crepe } from '@milkdown/crepe'
import { editorViewCtx } from '@milkdown/kit/core'
import { CREPE_FEATURES, openEditor } from './editor-crepe.js'
import { unescapeWikilinks } from './editor-frontmatter.js'

const editors: Crepe[] = []

afterEach(() => {
  for (const e of editors.splice(0)) e.destroy()
  document.body.innerHTML = ''
})

async function openCrepe(body: string): Promise<Crepe> {
  const root = document.createElement('div')
  document.body.appendChild(root)
  const crepe = new Crepe({ root, defaultValue: body, features: CREPE_FEATURES })
  await crepe.create()
  editors.push(crepe)
  return crepe
}

/** What a save writes for `body` (no frontmatter), with or without an edit. */
async function saved(body: string, edit?: (crepe: Crepe) => void): Promise<string> {
  const crepe = await openCrepe(body)
  edit?.(crepe)
  return unescapeWikilinks(crepe.getMarkdown())
}

/** Types `text` at the start of the first block, far from the cards. */
function typeAtStart(text: string) {
  return (crepe: Crepe): void => {
    crepe.editor.action((ctx) => {
      const view = ctx.get(editorViewCtx)
      view.dispatch(view.state.tr.insertText(text, 1))
    })
  }
}

// The user's card, verbatim (T1-A09).
const CARD_A09 = `**T1-A09: Who selects a Frequency Coordinator?**

A. The FCC Office of Spectrum Management and Coordination Policy
B. The local chapter of the Office of National Council of Independent Frequency Coordinators
C. Amateur operators in a local or regional area whose stations are eligible to be repeater or auxiliary stations
D. The FCC Regional Field Office
: **C.** Amateur operators in a local or regional area whose stations are eligible to be repeater or auxiliary stations
: **47 CFR 97.3(a)(22):**
: > **Frequency coordinator.** An entity, ***recognized in a local or regional area by amateur operators whose stations are eligible to be auxiliary or repeater stations,*** that recommends transmit/receive channels and associated operating and technical parameters for such stations in order to avoid or minimize potential interference.
: Frequency coordinators are amateur radio operators who are recognized by other local or regional amateurs. They recommend which transmit/receive channels should be used.
: Coordination is important because if an operator ignores the locally established band plan, they might cause harmful interference to others.
: Memory aids:
  * Remember: "Frequency coordinator" = someone recognized by local/regional amateur operators (not an FCC office).
: ___Review History___
  * 2026-10-07 14:53 - Good
`

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
const CARD_B11 = `**T1-B11: What is the maximum peak envelope power output for Technician class operators in their HF band segments?**

A. 200 watts
B. 100 watts
C. 50 watts
D. 10 watts
: **A.** 200 watts
: Technician-class operators are limited to 200 watts PEP (Peak Envelope Power) on any portion of an HF band that they are authorized to use. This is an FCC regulation that must be memorized for the exam.
: Also remember that FCC rule 97.313 requires stations to use the minimum transmitter power necessary to carry out the desired communications, even though the maximum allowed for Technician HF segments is 200 W PEP.
: Memory aids:
  * "HF" has 2 letters → 2-hundred watts (200 W)
  * The word "maximum" corresponds to the largest numeric choice
: ___Review History___
  * 2026-10-07 14:54 - Again
`

// The file as the user found it, byte for byte.
const DAMAGED_B11 = `**T1-B11: What is the maximum peak envelope power output for Technician class operators in their HF band segments?**

A. 200 watts
B. 100 watts
C. 50 watts
D. 10 watts
: **A.** 200 watts
: Technician-class operators are limited to 200 watts PEP (Peak Envelope Power) on any portion of an HF band that they are authorized to use. This is an FCC regulation that must be memorized for the exam.
: Also remember that FCC rule 97.313 requires stations to use the minimum transmitter power necessary to carry out the desired communications, even though the maximum allowed for Technician HF segments is 200 W PEP.
: Memory aids:

* "HF" has 2 letters → 2-hundred watts (200 W)
* The word "maximum" corresponds to the largest numeric choice
  : ___Review History___
* 2026-10-07 14:54 - Again
`

describe('editor round trip: definition lists (flashcard decks)', () => {
  it('a save with no edits rewrites the nested lists (exact current output)', async () => {
    expect(await saved(CARD_A09)).toBe(SAVED_A09)
  })

  it('a save after editing an unrelated paragraph does the same damage', async () => {
    const out = await saved(CARD_A09, typeAtStart('EDITED '))
    expect(out).toBe(SAVED_A09.replace('**T1-A09', '**EDITED T1-A09'))
  })

  it('reproduces the user\'s damaged T1-B11 card from its presumed original in one save', async () => {
    expect(await saved(CARD_B11)).toBe(DAMAGED_B11)
  })

  it('the damaged form is a fixed point: later saves change nothing more', async () => {
    expect(await saved(DAMAGED_B11)).toBe(DAMAGED_B11)
  })

  it.fails('preserves a definition with an indented nested list', async () => {
    expect(await saved(CARD_A09)).toBe(CARD_A09)
  })

  it.fails('preserves a definition list with a `-` review history', async () => {
    const deck = `Q?
: Answer
: ___Review History___
  - 2026-10-07 14:53 - Good
  - 2026-10-08 09:00 - Easy
`
    expect(await saved(deck)).toBe(deck)
  })

  it('preserves flat definition lines, emphasis markers and `: >` (paragraph text to remark)', async () => {
    const flat = `Term
: Definition one
: ___Review History___
: > quoted ***strong em*** text
`
    expect(await saved(flat)).toBe(flat)
  })
})

describe('editor round trip: other mbr extensions', () => {
  it.fails('preserves `--- {attrs}` section attributes', async () => {
    const md = `Intro\n\n--- {#intro .highlight data-transition="slide"}\n\nBody\n`
    // Today: `\\--- {#intro …}` — the escape turns the section rule into text.
    expect(await saved(md)).toBe(md)
  })

  it.fails('preserves `[-]` and `[>]` task markers', async () => {
    const md = `- [ ] open\n- [-] cancelled\n- [>] deferred\n`
    // Today: `* [ ] open` / `* \\[-] cancelled` / `* \\[>] deferred`.
    expect(await saved(md)).toBe(md)
  })

  it('normalizes `-` bullets to `*` (cosmetic, but a whole-file diff)', async () => {
    expect(await saved(`- a\n- b\n`)).toBe(`* a\n* b\n`)
  })

  it('keeps wikilinks once unescapeWikilinks has run', async () => {
    const md = `See [[Some Note|alias]] here.\n`
    expect(await saved(md)).toBe(md)
  })
})

describe('openEditor: a save with nothing changed writes nothing', () => {
  const RAW = `---\ntitle: Deck\ntype: flashcard\n---\n${CARD_A09}`

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
    fm.value = fm.value.replace('Deck', 'Deck 2')
    await clickSave(modal)
    expect(posts).toHaveLength(1)
    expect(onSaved).toHaveBeenCalledOnce()
  })
})
