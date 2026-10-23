import { describe, it, expect, vi } from 'vitest'
// The chunk itself is not importable under happy-dom (Milkdown), so its chrome
// is pinned as source: if one of these goes, the behaviour it names goes too.
import editorSource from './editor-crepe.ts?raw'

/** The body of the `const NAME = \`…\`` CSS block in the chunk. */
function cssBlock(name: string): string {
  const start = editorSource.indexOf(`${name} = \``)
  expect(start, name).toBeGreaterThanOrEqual(0)
  const end = editorSource.indexOf('`;', start)
  return editorSource.slice(start, end)
}

describe('editor chrome', () => {
  it('has Cancel on the left, and ⋯ and Save on the right, in a native-sheet header', () => {
    expect(editorSource).toContain("barButton('Cancel', 'mbr-editor-cancel')")
    expect(editorSource).toContain("iconBarButton(iconEllipsis, 'mbr-editor-more')")
    expect(editorSource).toContain("barButton('Save', 'mbr-editor-save')")
    expect(editorSource).toContain('header.append(cancelBtn, title, actions)')
    const header = cssBlock('HEADER_CSS')
    expect(header).toContain('grid-template-columns: auto minmax(0, 1fr) auto')
  })

  it('gives the document a tall minimum, capped by the screen, with the body as the one scroller', () => {
    const modal = cssBlock('MODAL_CSS')
    expect(modal).toMatch(/\.mbr-editor-body \{[^}]*overflow: auto/)
    const crepe = modal.match(/\.mbr-editor-crepe \{([^}]*)\}/)![1]
    expect(crepe).toContain('min-height: min(36rem, 70svh)')
    // Grows to its content instead of shrinking into a second vertical scroller.
    expect(crepe).toContain('flex: 1 0 auto')
    expect(crepe).not.toMatch(/(^|[^-])overflow: auto/)
  })

  it('fills most of a desktop window, keeps the prose at a readable measure, and leaves narrow screens alone', () => {
    const modal = cssBlock('MODAL_CSS')
    const box = modal.match(/\.mbr-editor-modal \{([^}]*)\}/)![1]
    expect(box).toContain('width: min(1400px, 92vw)')
    expect(box).toContain('height: min(1200px, 94svh)')
    expect(modal).toMatch(/@media \(min-width: 601px\) \{\s*\.mbr-editor-modal \.milkdown \.ProseMirror \{[^}]*calc\(\(100% - 48em\) \/ 2\)/)
  })

  it('drops the old × close button and the bordered file-action row', () => {
    expect(editorSource).not.toContain('mbr-editor-close')
    expect(editorSource).not.toContain('mbr-editor-fileactions')
    // Cancel and Save are in the header, not Pico-filled footer buttons.
    expect(editorSource).not.toMatch(/footer\.append\([^)]*saveBtn/)
  })

  it('puts New / Rename / Move in a real menu behind ⋯', () => {
    expect(editorSource).toContain("menu.setAttribute('role', 'menu')")
    expect(editorSource).toContain("item.setAttribute('role', 'menuitem')")
    expect(editorSource).toContain("moreBtn.setAttribute('aria-haspopup', 'menu')")
    expect(editorSource).toContain("moreBtn.setAttribute('aria-expanded', 'true')")
    for (const label of ['New note…', 'Rename…', 'Move…']) {
      expect(editorSource).toContain(`menuItem('${label}'`)
    }
    // Escape closes the menu, not the editor: capture phase + stopPropagation.
    const menuKeys = editorSource.slice(editorSource.indexOf('const onMenuKeydown'))
    expect(menuKeys.slice(0, 400)).toContain('e.stopPropagation()')
    expect(editorSource).toContain("document.addEventListener('keydown', onMenuKeydown, true)")
    expect(editorSource).toContain("document.addEventListener('pointerdown', onOutsidePointer, true)")
  })

  it('sends Cancel, Escape and a backdrop click through the unsaved-changes guard', () => {
    expect(editorSource).toContain("cancelBtn.addEventListener('click', () => void requestClose())")
    expect(editorSource).toMatch(/e\.key === 'Escape'\) \{\s*e\.preventDefault\(\);\s*void requestClose\(\);/)
    expect(editorSource).toContain('if (e.target === backdrop) void requestClose();')
    const guarded = editorSource.slice(editorSource.indexOf('  requestClose = async () => {'))
    expect(guarded.slice(0, 200)).toContain('await guardUnsaved()')
    // Nothing closes without asking any more.
    expect(editorSource).not.toContain("addEventListener('click', close)")
  })

  it('saves on Cmd/Ctrl+S and reports a reader-initiated save through onSaved', () => {
    expect(editorSource).toMatch(/\(e\.metaKey \|\| e\.ctrlKey\)[\s\S]{0,80}e\.key\.toLowerCase\(\) === 's'/)
    expect(editorSource).toContain('saveFromShortcut = () => void saveAndNotify()')
    expect(editorSource).toContain('if (await doSave()) opts.onSaved?.()')
  })

  it('has a format bar of icon + label tools', () => {
    expect(editorSource).toContain("toolButton(FOOTNOTE_ICON, 'Footnote', 'mbr-editor-footnote')")
    expect(editorSource).toContain("toolButton(MEDIA_ICON, 'Media', 'mbr-editor-media')")
  })

  it('styles bar buttons as plain 44px text buttons that beat Pico', () => {
    const header = cssBlock('HEADER_CSS')
    expect(header).toContain('.mbr-editor-modal .mbr-editor-bar-btn')
    expect(header).toContain('min-height: 44px; min-width: 44px')
    expect(header).toContain('background: none; border: 0; box-shadow: none')
    expect(header).toContain('color: var(--pico-primary')
    expect(header).toMatch(/\.mbr-editor-save \{ font-weight: 600; \}/)
    expect(header).toContain(':focus-visible')
  })

  it('creates a note from outside the editor and opens it with #mbr-edit', () => {
    expect(editorSource).toContain('export async function createNote(')
    expect(editorSource).toContain('export async function postCreate(')
    expect(editorSource).toContain("export const EDIT_ON_ARRIVAL_HASH = '#mbr-edit'")
    expect(editorSource).toContain('window.location.href = result.urlPath + EDIT_ON_ARRIVAL_HASH')
    // The one create request: both flows use postCreate.
    expect(editorSource.match(/fetch\(`\/\.mbr\/create\//g)).toHaveLength(1)
  })

  it('edits frontmatter with the properties editor, inside the collapsible details', () => {
    expect(editorSource).toContain("document.createElement('mbr-fm-editor')")
    expect(editorSource).toContain("fmEditor.model = openFrontmatter(frontmatter ?? '')")
    expect(editorSource).toContain('fmDetails.append(fmSummary, fmEditor)')
    // Open when the note has frontmatter, an "Add properties" affordance when not.
    expect(editorSource).toContain('if (frontmatter) fmDetails.open = true;')
    expect(editorSource).toContain("fmSummary.textContent = frontmatter ? 'Properties' : 'Add properties'")
    // The save (and the dirty check, which compares the same string) reads the
    // editor's text, so untouched frontmatter is written as loaded (doc.head).
    expect(editorSource).toContain('return documentToSave(opened, fmEditor.text, next.body);')
    expect(editorSource).not.toContain('fmTextarea')
    // Tag fields come in through the options: the chunk never imports shared.ts.
    expect(editorSource).toContain('builtinCatalog({ tagSources: opts.tagSources })')
    expect(editorSource).not.toMatch(/from '\.\/shared(\.js|\.ts)?'/)
  })

  it('shares site.json for the properties, and fetches it fresh per `[[` session and picker', () => {
    expect(editorSource).toContain('const siteIndex = memoizeSiteIndex();')
    // The link autocomplete and the New / Move pickers: a page created or moved since must show.
    expect(editorSource).toContain('const freshSiteFiles = async (): Promise<SiteMarkdownFile[]> => (await siteIndex.fresh()).markdown_files;')
    expect(editorSource.match(/fetchSiteFiles: freshSiteFiles/g)).toHaveLength(3)
    expect(editorSource).toContain('fmKnowledge ??= siteIndex()')
    // Vault knowledge is loaded when the properties are first used, not on open.
    expect(editorSource).toContain("fmDetails.addEventListener('focusin', () => {")
  })
})

describe('memoizeSiteIndex', () => {
  it('shares one request and retries after a failure', async () => {
    const { memoizeSiteIndex } = await import('./editor-crepe.js')
    const index = { markdown_files: [], relationship_types: [] }
    const fetcher = vi.fn().mockRejectedValueOnce(new Error('offline')).mockResolvedValue(index)
    const load = memoizeSiteIndex(fetcher)
    await expect(load()).rejects.toThrow('offline')
    const [a, b] = await Promise.all([load(), load()])
    expect(a).toBe(index)
    expect(b).toBe(index)
    expect(fetcher).toHaveBeenCalledTimes(2)
  })

  it('fetches again for each fresh() (each `[[` session), sharing one in flight', async () => {
    const { memoizeSiteIndex } = await import('./editor-crepe.js')
    let n = 0
    const fetcher = vi.fn(async () => ({ markdown_files: [{ url_path: `/p${++n}/` }], relationship_types: [] }) as never)
    const load = memoizeSiteIndex(fetcher)
    const first = await load.fresh()
    const second = await load.fresh()
    expect(fetcher).toHaveBeenCalledTimes(2)
    expect(second).not.toBe(first)
    // Plain calls (the properties editor) get the latest without a request of their own.
    expect(await load()).toBe(second)
    expect(fetcher).toHaveBeenCalledTimes(2)
    // Two sessions at once, or a plain call during one, share its request.
    const [a, b, c] = await Promise.all([load.fresh(), load.fresh(), load()])
    expect(fetcher).toHaveBeenCalledTimes(3)
    expect(b).toBe(a)
    expect(c).toBe(a)
  })
})
