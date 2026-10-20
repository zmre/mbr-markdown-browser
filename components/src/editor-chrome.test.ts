import { describe, it, expect } from 'vitest'
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
})
