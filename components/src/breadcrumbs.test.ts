import { describe, it, expect, afterEach } from 'vitest'
import { enhanceBreadcrumbs, EXPANDED_CLASS, MORE_CLASS } from './breadcrumbs.ts'
import tasksPanelSource from './tasks/mbr-tasks-panel.ts?raw'
import editorSource from './editor-crepe.ts?raw'
import searchSource from './mbr-search.ts?raw'
import infoSource from './mbr-info.ts?raw'
import pageErrorsSource from './mbr-page-errors.ts?raw'
// Read by vitest.config.ts (outside components/, and CSS imports are stubbed).
declare const __MBR_THEME_CSS__: string
declare const __MBR_HEAD_TEMPLATE__: string
const themeCss = __MBR_THEME_CSS__
const headTemplate = __MBR_HEAD_TEMPLATE__

function trail(names: string[]): HTMLElement {
  document.body.innerHTML = `<nav class="container breadcrumbs"><ul>${names
    .map((name, i) =>
      i === names.length - 1
        ? `<li><small>${name}</small></li>`
        : `<li><small><a href="/${name}/">${name}</a></small></li>`
    )
    .join('')}</ul></nav>`
  return document.querySelector('nav.breadcrumbs') as HTMLElement
}

describe('collapsed breadcrumbs', () => {
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('adds a "…" toggle in front of a trail with hidden items, once', () => {
    const nav = trail(['Home', 'projects', 'acme', 'Q4 plan'])
    enhanceBreadcrumbs(nav)
    enhanceBreadcrumbs(nav)
    const more = nav.querySelectorAll(`li.${MORE_CLASS}`)
    expect(more).toHaveLength(1)
    expect(nav.querySelector('ul > li')?.classList.contains(MORE_CLASS)).toBe(true)
  })

  it('leaves a short trail alone: nothing would be hidden', () => {
    const nav = trail(['Home', 'Q4 plan'])
    enhanceBreadcrumbs(nav)
    expect(nav.querySelector(`li.${MORE_CLASS}`)).toBeNull()
  })

  it('toggles the full path and says so', () => {
    const nav = trail(['Home', 'projects', 'acme', 'Q4 plan'])
    enhanceBreadcrumbs(nav)
    const button = nav.querySelector('button') as HTMLButtonElement
    button.click()
    expect(nav.classList.contains(EXPANDED_CLASS)).toBe(true)
    expect(button.getAttribute('aria-expanded')).toBe('true')
    button.click()
    expect(nav.classList.contains(EXPANDED_CLASS)).toBe(false)
  })
})

/**
 * happy-dom does no layout, so the narrow-screen fixes are pinned as the rules
 * that carry them: if one of these goes, the phone layout it describes goes too.
 */
describe('narrow-screen rules', () => {
  it('collapses breadcrumbs to one line under 600px and hides the toggle on wide screens', () => {
    expect(themeCss).toMatch(/nav\.breadcrumbs li\.mbr-crumbs-more \{\s*display: none;/)
    const narrow = themeCss.slice(themeCss.indexOf('Everything but the "…"'))
    expect(narrow).toContain('li:not(:nth-last-child(-n + 2)):not(.mbr-crumbs-more)')
    expect(themeCss).toContain('nav.breadcrumbs:not(.mbr-crumbs-expanded) > ul {\n    flex-wrap: nowrap;')
  })

  it('asks for the full screen and keeps content clear of the safe areas', () => {
    expect(headTemplate).toContain('viewport-fit=cover')
    expect(themeCss).toContain('env(safe-area-inset-bottom, 0px)')
  })

  it('makes the tasks overlay one column under 600px, with a folder drawer and 44px targets', () => {
    const narrow = tasksPanelSource.slice(tasksPanelSource.indexOf('@media (max-width: 600px)'))
    expect(narrow).toContain('height: 100dvh')
    expect(narrow).toContain('.folder-pane.drawer-open')
    expect(narrow).toContain('min-height: 44px')
    expect(narrow).toContain('env(safe-area-inset-bottom')
    expect(tasksPanelSource).toContain('class="folders-button')
  })

  it('gives the editor phone margins and a wrapping toolbar under 600px', () => {
    expect(editorSource).toMatch(/NARROW_CSS,\n\s*\]\.join/)
    const narrow = editorSource.slice(editorSource.indexOf('export const NARROW_CSS'))
    expect(narrow).toContain('@media (max-width: 600px)')
    expect(narrow).toMatch(/\.milkdown \.ProseMirror \{ padding: 16px/)
    expect(narrow).toMatch(/\.milkdown \.milkdown-toolbar \{\s*flex-wrap: wrap;/)
  })

  it('sizes the info, page-errors and search popups with dvh', () => {
    for (const [name, source] of [
      ['mbr-info', infoSource],
      ['mbr-page-errors', pageErrorsSource],
      ['mbr-search', searchSource],
    ]) {
      expect(source, name).toContain('dvh')
    }
    expect(infoSource).toContain('height: 100dvh')
    expect(pageErrorsSource).toContain('height: 100dvh')
    expect(searchSource).toContain('max-height: 70dvh')
  })
})
