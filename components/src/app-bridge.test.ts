import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import {
  APP_ACTIONS,
  APP_MESSAGE_HANDLER,
  availableActions,
  installAppBridge,
  isEmbedded,
  openAppAction,
  pageBreadcrumbs,
  pageState,
  pageTitle,
  postEditorState,
  postPageState,
  postSlidesState,
} from './app-bridge.ts'
import { EDITOR_STATE_EVENT } from './mbr-editor.ts'
import { SLIDES_STATE_EVENT } from './mbr-slides.ts'
// The Swift side of the bridge and theme.css, as text (vitest.config.ts reads
// them: Vite's `?raw` is denied outside components/, and CSS imports are
// stubbed empty). AppBridge.swift's string literals are the other half of the
// contract this file pins.
declare const __MBR_APP_BRIDGE_SWIFT__: string
declare const __MBR_THEME_CSS__: string
const appBridgeSwift = __MBR_APP_BRIDGE_SWIFT__
const themeCss = __MBR_THEME_CSS__

/**
 * The iOS app calls `window.mbrApp.open('<action>')` and listens on the
 * `mbrApp` script-message handler, from Swift STRING LITERALS. A rename here
 * would compile on both sides and silently empty the app's menu, so the names
 * are pinned twice: as constants here, and as literals in AppBridge.swift.
 */
describe('app bridge contract (referenced from Swift)', () => {
  it('keeps the action names and their order', () => {
    expect(APP_ACTIONS).toEqual(['browse', 'search', 'edit', 'info', 'tasks', 'flashcards', 'slides', 'new'])
    expect(APP_MESSAGE_HANDLER).toBe('mbrApp')
  })

  it('matches the Swift side literally', () => {
    for (const action of APP_ACTIONS) {
      expect(appBridgeSwift, `AppBridge.swift names "${action}"`).toContain(`"${action}"`)
    }
    expect(appBridgeSwift).toContain(`"${APP_MESSAGE_HANDLER}"`)
    expect(appBridgeSwift).toContain('window.mbrApp')
    expect(appBridgeSwift).toContain('__MBR_EMBEDDED__')
    expect(appBridgeSwift).toContain('mbr-embedded')
    // The message types PageInfo.apply switches on.
    expect(appBridgeSwift).toContain('"page"')
    expect(appBridgeSwift).toContain('"editor"')
    expect(appBridgeSwift).toContain('"slides"')
  })
})

describe('embedded header hiding', () => {
  it('is scoped to html.mbr-embedded and collapses rather than display:none', () => {
    const block = themeCss.slice(themeCss.indexOf('html.mbr-embedded body > header {'))
    const rule = block.slice(0, block.indexOf('}'))
    expect(rule).toContain('height: 0')
    expect(rule).toContain('overflow: hidden')
    // display:none would also hide the overlays the header's components host.
    expect(rule).not.toContain('display: none')
    expect(themeCss).toMatch(/html\.mbr-embedded body > nav\.breadcrumbs \{\s*display: none;/)
  })
})

/** A page shaped like `_nav.html` + `_breadcrumbs.html`, with stub elements. */
function installPage(
  options: { deck?: boolean; slides?: boolean; canEdit?: boolean; canCreate?: boolean; tasks?: boolean } = {},
): {
  opened: string[]
} {
  const opened: string[] = []
  document.body.className = [options.deck ? 'flashcard' : '', options.slides ? 'slides' : ''].join(' ').trim()
  document.body.innerHTML = `
    <header class="container"><nav><ul></ul>
      <ul class="mbr-title-group"><li class="mbr-nav-title"><strong> A Very Long Title </strong></li></ul>
      <ul></ul></nav></header>
    <nav class="container breadcrumbs"><ul>
      <li><small><a href="/">Home</a></small></li>
      <li><small><a href="/projects/">projects</a></small></li>
      <li><small><a href="/projects/acme/">acme</a></small></li>
      <li><small>A Very Long Title</small></li>
    </ul></nav>
    <main id="wrapper">${options.deck ? '<dl><dt>Q?</dt><dd>A</dd></dl>' : ''}</main>`
  // The real tag names (the bridge queries them), with `open()` recorded and
  // the gates the bridge reads (`canEdit`, `isSlideDocument`) set directly.
  const stub = (tag: string, extra: Record<string, unknown> = {}): void => {
    const el = document.createElement(tag) as HTMLElement & Record<string, unknown>
    // defineProperty, not assignment: where the real element is defined, its
    // gates are getter-only.
    for (const [key, value] of Object.entries({ open: () => opened.push(tag), ...extra })) {
      Object.defineProperty(el, key, { value, configurable: true })
    }
    document.body.querySelector('header nav ul')?.append(el)
  }
  stub('mbr-browse-single')
  stub('mbr-search')
  stub('mbr-editor', {
    canEdit: options.canEdit ?? false,
    canCreate: options.canCreate ?? false,
    openNew: () => opened.push('mbr-editor:new'),
  })
  stub('mbr-info')
  stub('mbr-tasks')
  stub('mbr-flashcards')
  stub('mbr-slides', { isSlideDocument: options.slides ?? false })
  window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, tasksEnabled: options.tasks ?? false }
  return { opened }
}

describe('available actions follow the page', () => {
  afterEach(() => {
    document.body.innerHTML = ''
    document.body.className = ''
    window.__MBR_CONFIG__ = undefined
  })

  it('lists only what works: no edit, tasks, deck or slides on a plain read-only page', () => {
    installPage()
    expect(availableActions()).toEqual(['browse', 'search', 'info'])
  })

  it('adds edit, tasks, flashcards and slides when the page has them', () => {
    installPage({ canEdit: true, canCreate: true, tasks: true, deck: true, slides: true })
    expect(availableActions()).toEqual(['browse', 'search', 'edit', 'info', 'tasks', 'flashcards', 'slides', 'new'])
  })

  it('offers new only when the editor can create, and opens it through openNew', () => {
    const { opened } = installPage({ canEdit: false, canCreate: false })
    expect(availableActions()).not.toContain('new')
    expect(openAppAction('new')).toBe(false)
    document.body.innerHTML = ''
    // A listing page: nothing to edit, but a note can be created there.
    const second = installPage({ canEdit: false, canCreate: true })
    expect(availableActions()).toContain('new')
    expect(availableActions()).not.toContain('edit')
    expect(openAppAction('new')).toBe(true)
    expect(opened).toEqual([])
    expect(second.opened).toEqual(['mbr-editor:new'])
  })

  it('does not offer new on an editor without openNew', () => {
    installPage({ canCreate: true })
    const editor = document.querySelector('mbr-editor') as HTMLElement
    Object.defineProperty(editor, 'openNew', { value: undefined, configurable: true })
    expect(availableActions()).not.toContain('new')
    expect(openAppAction('new')).toBe(false)
  })

  it('opens an action through the element and refuses unknown or unavailable ones', () => {
    const { opened } = installPage({ canEdit: false })
    expect(openAppAction('search')).toBe(true)
    expect(openAppAction('edit')).toBe(false)
    expect(openAppAction('nope')).toBe(false)
    expect(opened).toEqual(['mbr-search'])
  })
})

describe('title and breadcrumbs', () => {
  afterEach(() => {
    document.body.innerHTML = ''
    window.__MBR_CONFIG__ = undefined
  })

  it('reads the header title and the rendered breadcrumb links', () => {
    installPage()
    expect(pageTitle()).toBe('A Very Long Title')
    expect(pageBreadcrumbs(document, 'mbr://localhost/projects/acme/plan/')).toEqual([
      { name: 'Home', url: '/' },
      { name: 'projects', url: '/projects/' },
      { name: 'acme', url: '/projects/acme/' },
    ])
  })

  it('derives the path from the URL on pages without breadcrumbs', () => {
    document.body.innerHTML = ''
    expect(pageBreadcrumbs(document, 'mbr://localhost/notes/My%20Folder/sub/')).toEqual([
      { name: 'Home', url: '/' },
      { name: 'notes', url: '/notes/' },
      { name: 'My Folder', url: '/notes/My%20Folder/' },
    ])
    expect(pageBreadcrumbs(document, 'mbr://localhost/')).toEqual([])
  })

  it('assembles the page state the app receives', () => {
    installPage()
    const state = pageState(document, 'mbr://localhost/projects/acme/plan/')
    expect(state.type).toBe('page')
    expect(state.title).toBe('A Very Long Title')
    expect(state.breadcrumbs).toHaveLength(3)
    expect(state.actions).toContain('search')
  })
})

describe('embedded mode only', () => {
  beforeEach(() => {
    window.__MBR_EMBEDDED__ = undefined
    window.mbrApp = undefined
    window.webkit = undefined
  })
  afterEach(() => {
    window.__MBR_EMBEDDED__ = undefined
    window.mbrApp = undefined
    window.webkit = undefined
    document.body.innerHTML = ''
  })

  it('does nothing in a browser: no window.mbrApp, nothing posted', () => {
    const postMessage = vi.fn()
    window.webkit = { messageHandlers: { mbrApp: { postMessage } } }
    expect(isEmbedded()).toBe(false)
    installAppBridge()
    expect(window.mbrApp).toBeUndefined()
    expect(postPageState()).toBe(false)
    expect(postMessage).not.toHaveBeenCalled()
  })

  it('in the app, installs window.mbrApp and posts the page to the handler', () => {
    const postMessage = vi.fn()
    window.webkit = { messageHandlers: { mbrApp: { postMessage } } }
    window.__MBR_EMBEDDED__ = true
    installPage()
    installAppBridge()
    expect(window.mbrApp?.actions).toEqual(APP_ACTIONS)
    expect(window.mbrApp?.open('search')).toBe(true)
    expect(postMessage).toHaveBeenCalled()
    expect(postMessage.mock.calls[0][0]).toMatchObject({ type: 'page', title: 'A Very Long Title' })
  })

  it('in the app, posts the editor opening and closing', () => {
    const postMessage = vi.fn()
    window.webkit = { messageHandlers: { mbrApp: { postMessage } } }
    window.__MBR_EMBEDDED__ = true
    installAppBridge()
    // Installed twice, still one message per event.
    installAppBridge()
    postMessage.mockClear()
    document.dispatchEvent(new CustomEvent(EDITOR_STATE_EVENT, { detail: { open: true } }))
    document.dispatchEvent(new CustomEvent(EDITOR_STATE_EVENT, { detail: { open: false } }))
    document.dispatchEvent(new CustomEvent(EDITOR_STATE_EVENT, { detail: {} }))
    expect(postMessage.mock.calls.map((call) => call[0])).toEqual([
      { type: 'editor', open: true },
      { type: 'editor', open: false },
    ])
  })

  it('carries the editor state in the page state, for the load re-post', () => {
    window.webkit = { messageHandlers: { mbrApp: { postMessage: vi.fn() } } }
    window.__MBR_EMBEDDED__ = true
    installAppBridge()
    document.dispatchEvent(new CustomEvent(EDITOR_STATE_EVENT, { detail: { open: true } }))
    expect(pageState().editorOpen).toBe(true)
    document.dispatchEvent(new CustomEvent(EDITOR_STATE_EVENT, { detail: { open: false } }))
    expect(pageState().editorOpen).toBe(false)
    expect(appBridgeSwift).toContain('"editorOpen"')
  })

  it('in the app, posts slides starting and stopping', () => {
    const postMessage = vi.fn()
    window.webkit = { messageHandlers: { mbrApp: { postMessage } } }
    window.__MBR_EMBEDDED__ = true
    installAppBridge()
    // Installed twice, still one message per event.
    installAppBridge()
    postMessage.mockClear()
    document.dispatchEvent(new CustomEvent(SLIDES_STATE_EVENT, { detail: { playing: true } }))
    document.dispatchEvent(new CustomEvent(SLIDES_STATE_EVENT, { detail: { playing: false } }))
    document.dispatchEvent(new CustomEvent(SLIDES_STATE_EVENT, { detail: {} }))
    expect(postMessage.mock.calls.map((call) => call[0])).toEqual([
      { type: 'slides', playing: true },
      { type: 'slides', playing: false },
    ])
    // The Swift side reads the message's field by this name.
    expect(appBridgeSwift).toContain('"playing"')
  })

  it('carries the slides state in the page state, for the load re-post', () => {
    window.webkit = { messageHandlers: { mbrApp: { postMessage: vi.fn() } } }
    window.__MBR_EMBEDDED__ = true
    installAppBridge()
    document.dispatchEvent(new CustomEvent(SLIDES_STATE_EVENT, { detail: { playing: true } }))
    expect(pageState().slidesPlaying).toBe(true)
    document.dispatchEvent(new CustomEvent(SLIDES_STATE_EVENT, { detail: { playing: false } }))
    expect(pageState().slidesPlaying).toBe(false)
    expect(appBridgeSwift).toContain('"slidesPlaying"')
  })

  it('posts no slides state outside the app', () => {
    const postMessage = vi.fn()
    window.webkit = { messageHandlers: { mbrApp: { postMessage } } }
    expect(postSlidesState(true)).toBe(false)
    document.dispatchEvent(new CustomEvent(SLIDES_STATE_EVENT, { detail: { playing: true } }))
    expect(postMessage).not.toHaveBeenCalled()
  })

  it('posts no editor state outside the app', () => {
    const postMessage = vi.fn()
    window.webkit = { messageHandlers: { mbrApp: { postMessage } } }
    expect(postEditorState(true)).toBe(false)
    document.dispatchEvent(new CustomEvent(EDITOR_STATE_EVENT, { detail: { open: true } }))
    expect(postMessage).not.toHaveBeenCalled()
  })
})
