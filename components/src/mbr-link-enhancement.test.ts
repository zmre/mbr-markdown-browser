import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import {
  externalHrefForClick,
  handleExternalLinkClick,
  MbrLinkEnhancementElement,
} from './mbr-link-enhancement.ts'

/**
 * The GUI-only hand-off of cross-origin link clicks to the host.
 *
 * This listener exists because wry's navigation handler cannot tell a clicked
 * link from an `<iframe>` load (see `decide_without_frame_info` in
 * `src/external_open.rs`). Everything below is therefore about one question:
 * did we correctly identify a *user clicking a cross-origin link* and nothing
 * else?
 */
describe('external link hand-off', () => {
  let main: HTMLElement
  let posted: string[]

  /** The message prefix Rust matches on; must match `IPC_OPEN_EXTERNAL_PREFIX`. */
  const PREFIX = 'mbr:open-external:'

  /** Where the tests pretend the mbr window is. */
  const PAGE_URL = 'http://localhost:3000/docs/guide/'

  beforeEach(() => {
    // happy-dom really follows a link when a click is not cancelled, so several
    // of the tests below would otherwise move the page to example.com and the
    // *next* test's "external" URL would be same-origin. Pin it every time.
    ;(window as unknown as { happyDOM: { setURL(url: string): void } }).happyDOM.setURL(PAGE_URL)

    main = document.createElement('main')
    document.body.appendChild(main)

    posted = []
    window.ipc = { postMessage: (message: string) => posted.push(message) }

    document.addEventListener('click', handleExternalLinkClick)
  })

  afterEach(() => {
    document.removeEventListener('click', handleExternalLinkClick)
    main.remove()
    delete window.ipc
  })

  /** Builds an anchor in the document and clicks it the way a mouse would. */
  function clickLink(
    attributes: Record<string, string>,
    init: MouseEventInit = {},
  ): { anchor: HTMLAnchorElement; event: MouseEvent } {
    const anchor = document.createElement('a')
    for (const [name, value] of Object.entries(attributes)) {
      anchor.setAttribute(name, value)
    }
    anchor.textContent = 'link'
    main.appendChild(anchor)

    const event = new MouseEvent('click', {
      bubbles: true,
      composed: true,
      cancelable: true,
      button: 0,
      ...init,
    })
    anchor.dispatchEvent(event)

    return { anchor, event }
  }

  it('posts the resolved absolute URL for an off-origin link and cancels the click', () => {
    const { event } = clickLink({ href: 'https://example.com/page?q=1#frag' })

    expect(posted).toEqual([`${PREFIX}https://example.com/page?q=1#frag`])
    expect(event.defaultPrevented).toBe(true)
  })

  it('sends the resolved href, not the raw attribute', () => {
    // A protocol-relative href only becomes usable once resolved against the
    // page; the operating system cannot open `//example.com/x`.
    const { event } = clickLink({ href: '//example.com/x' })

    expect(posted).toEqual([`${PREFIX}${window.location.protocol}//example.com/x`])
    expect(event.defaultPrevented).toBe(true)
  })

  it('leaves same-origin links alone', () => {
    const { event } = clickLink({ href: '/docs/guide/' })

    expect(posted).toEqual([])
    expect(event.defaultPrevented).toBe(false)
  })

  it('leaves same-origin absolute links alone', () => {
    const { event } = clickLink({ href: `${window.location.origin}/docs/guide/` })

    expect(posted).toEqual([])
    expect(event.defaultPrevented).toBe(false)
  })

  it('leaves in-page anchors alone', () => {
    const { event } = clickLink({ href: '#section' })

    expect(posted).toEqual([])
    expect(event.defaultPrevented).toBe(false)
  })

  it.each([
    ['meta (Cmd)', { metaKey: true }],
    ['ctrl', { ctrlKey: true }],
    ['shift', { shiftKey: true }],
    ['alt', { altKey: true }],
  ])('leaves %s-clicks to the browser', (_label, init) => {
    const { event } = clickLink({ href: 'https://example.com/' }, init)

    expect(posted).toEqual([])
    expect(event.defaultPrevented).toBe(false)
  })

  it('leaves non-left buttons alone', () => {
    const { event } = clickLink({ href: 'https://example.com/' }, { button: 1 })

    expect(posted).toEqual([])
    expect(event.defaultPrevented).toBe(false)
  })

  it('leaves application schemes to the Rust navigation handler', () => {
    // An <iframe> can never navigate to these, so the navigation handler is
    // free to act on them and this listener must not double-handle them.
    for (const href of [
      'mailto:someone@example.com',
      'tel:+15555550123',
      'zoommtg://zoom.us/join?confno=1',
      'message://%3Cabc%40mail.example.com%3E',
    ]) {
      const { event } = clickLink({ href })
      expect(posted, `${href} should not be posted over IPC`).toEqual([])
      expect(event.defaultPrevented).toBe(false)
    }
  })

  it('leaves target="_blank" to the new-window handler', () => {
    const { event } = clickLink({ href: 'https://example.com/', target: '_blank' })

    expect(posted).toEqual([])
    expect(event.defaultPrevented).toBe(false)
  })

  it('leaves downloads alone', () => {
    const { event } = clickLink({ href: 'https://example.com/x.zip', download: '' })

    expect(posted).toEqual([])
    expect(event.defaultPrevented).toBe(false)
  })

  it('leaves a click another handler already claimed alone', () => {
    const anchor = document.createElement('a')
    anchor.setAttribute('href', 'https://example.com/')
    main.appendChild(anchor)
    anchor.addEventListener('click', (e) => e.preventDefault())

    anchor.dispatchEvent(
      new MouseEvent('click', { bubbles: true, composed: true, cancelable: true, button: 0 }),
    )

    expect(posted).toEqual([])
  })

  describe('without a host to talk to', () => {
    it('degrades to an ordinary in-window navigation rather than throwing', () => {
      delete window.ipc

      // Swallowing the click here would leave the link doing nothing at all,
      // which is worse than the pre-fix behaviour it falls back to.
      expect(() => clickLink({ href: 'https://example.com/' })).not.toThrow()
    })

    it('does not cancel the click when window.ipc is missing', () => {
      delete window.ipc

      const { event } = clickLink({ href: 'https://example.com/' })

      expect(event.defaultPrevented).toBe(false)
    })

    it('does not cancel the click when postMessage throws', () => {
      window.ipc = {
        postMessage: vi.fn(() => {
          throw new Error('channel closed')
        }),
      }

      const { event } = clickLink({ href: 'https://example.com/' })

      expect(event.defaultPrevented).toBe(false)
    })
  })

  describe('externalHrefForClick', () => {
    // Driven directly rather than through dispatchEvent: happy-dom follows a
    // link the moment the event reaches the anchor, so a click starting on a
    // *child* node has already moved `window.location` to the link's own origin
    // by the time a document listener runs, and every link would look local.
    // Real browsers run the default action after dispatch. The predicate is
    // pure, so calling it is both honest and deterministic.
    function eventOn(path: EventTarget[]): MouseEvent {
      return {
        defaultPrevented: false,
        button: 0,
        metaKey: false,
        ctrlKey: false,
        shiftKey: false,
        altKey: false,
        target: path[0],
        composedPath: () => path,
      } as unknown as MouseEvent
    }

    it('reports null when the click did not land on a link', () => {
      const paragraph = document.createElement('p')
      main.appendChild(paragraph)

      expect(externalHrefForClick(eventOn([paragraph, main, document]))).toBeNull()
    })

    it('finds the anchor when the click landed on a child element', () => {
      const anchor = document.createElement('a')
      anchor.setAttribute('href', 'https://example.com/deep')
      const inner = document.createElement('strong')
      inner.textContent = 'text'
      anchor.appendChild(inner)
      main.appendChild(anchor)

      expect(externalHrefForClick(eventOn([inner, anchor, main, document]))).toBe(
        'https://example.com/deep',
      )
    })

    it('ignores an ancestor anchor with no href', () => {
      const anchor = document.createElement('a')
      const inner = document.createElement('span')
      anchor.appendChild(inner)
      main.appendChild(anchor)

      expect(externalHrefForClick(eventOn([inner, anchor, main, document]))).toBeNull()
    })
  })
})

/**
 * The element's own GUI gate, which is a security boundary rather than a
 * feature flag.
 *
 * Handing a URL to the operating system means *starting an application*, and
 * only the native GUI window may ask for that. Server mode and static builds run
 * in a real browser that already does the job, and a machine that is merely
 * serving markdown must never be induced to launch applications on its host.
 *
 * mbr's own templates mount this element behind a `gui_mode` gate, but templates
 * are overridable: a repository can ship its own `.mbr/_display_enhancements.html`
 * or `.mbr/_footer.html` — quite plausibly copied from an older mbr, which
 * mounted this element ungated — and mbr will prefer it to the built-in. So the
 * element must be inert on its own when constructed outside GUI mode, no matter
 * what mounted it. These tests construct it exactly the way such an override
 * would.
 */
describe('mount gating outside GUI mode', () => {
  const PREFIX = 'mbr:open-external:'
  const PAGE_URL = 'http://localhost:3000/docs/guide/'

  const originalConfig = window.__MBR_CONFIG__

  let main: HTMLElement
  let posted: string[]
  let mounted: HTMLElement[]

  beforeEach(() => {
    // Uncancelled clicks really navigate under happy-dom, which would make the
    // next test's "external" URL same-origin. Pin the page every time.
    ;(window as unknown as { happyDOM: { setURL(url: string): void } }).happyDOM.setURL(PAGE_URL)

    main = document.body.appendChild(document.createElement('main'))
    posted = []
    mounted = []
    window.ipc = { postMessage: (message: string) => posted.push(message) }
  })

  afterEach(() => {
    // Disconnecting is what removes any listener the element registered, so a
    // GUI-mode test cannot leak one into the next test.
    for (const element of mounted) element.remove()
    main.remove()
    delete window.ipc
    window.__MBR_CONFIG__ = originalConfig
  })

  /** Mounts the element the way a template does, in the given mode. */
  function mount(guiMode: boolean): HTMLElement {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode }
    const element = document.body.appendChild(document.createElement('mbr-link-enhancement'))
    mounted.push(element)
    return element
  }

  /** Clicks a fresh cross-origin link and reports what the click produced. */
  function clickExternalLink(): MouseEvent {
    const anchor = document.createElement('a')
    anchor.setAttribute('href', 'https://example.com/page')
    anchor.textContent = 'link'
    main.appendChild(anchor)

    const event = new MouseEvent('click', {
      bubbles: true,
      composed: true,
      cancelable: true,
      button: 0,
    })
    anchor.dispatchEvent(event)
    return event
  }

  it('registers no click listener when isGuiMode() is false', () => {
    const addEventListener = vi.spyOn(document, 'addEventListener')

    mount(false)

    const clickRegistrations = addEventListener.mock.calls.filter(([type]) => type === 'click')
    expect(clickRegistrations).toEqual([])

    addEventListener.mockRestore()
  })

  it('sends no IPC and does not cancel a cross-origin click in server mode', () => {
    mount(false)

    const event = clickExternalLink()

    // Nothing asked the host to launch anything...
    expect(posted).toEqual([])
    // ...and the click was left entirely to the browser, which is the whole
    // contract for server and static modes.
    expect(event.defaultPrevented).toBe(false)
  })

  it('stays inert even when a stale template override mounts it twice', () => {
    // Exactly the shape of the bug this gate defends against: an old
    // `.mbr/_display_enhancements.html` mounting the element alongside the
    // footer's copy, on a server-mode page.
    mount(false)
    mount(false)

    const event = clickExternalLink()

    expect(posted).toEqual([])
    expect(event.defaultPrevented).toBe(false)
  })

  it('tags no links with tooltips in server mode', () => {
    // The browser has a URL bar and a status line; the tooltip is GUI-only for
    // the same reason the listener is.
    const anchor = main.appendChild(document.createElement('a'))
    anchor.setAttribute('href', 'https://example.com/page')

    mount(false)

    expect(anchor.hasAttribute('data-tooltip')).toBe(false)
  })

  // The positive control. Without it the tests above would still pass if the
  // element stopped working entirely.
  it('does hand off the click in GUI mode', () => {
    mount(true)

    const event = clickExternalLink()

    expect(posted).toEqual([`${PREFIX}https://example.com/page`])
    expect(event.defaultPrevented).toBe(true)
  })

  it('constructs as the expected element class', () => {
    // Guards the import above from being dropped as unused, and pins that the
    // tag name the templates write is the one this module defines.
    expect(mount(false)).toBeInstanceOf(MbrLinkEnhancementElement)
  })
})
