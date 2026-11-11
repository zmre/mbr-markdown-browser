import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import {
  isOutsideLinkTarget,
  classifyLinks,
  isBareLink,
  linkHoverText,
  linkKind,
  outsidePathOf,
  LINK_ICON_CLASS,
  LINK_KIND_ATTR,
} from './link-kinds.ts'

const ORIGIN = 'http://127.0.0.1:5631'
/** The page the links sit on: a note at depth 2. */
const PAGE = `${ORIGIN}/notes/today/`

let main: HTMLElement

beforeEach(() => {
  ;(window as unknown as { happyDOM: { setURL(url: string): void } }).happyDOM.setURL(PAGE)
  main = document.createElement('main')
  main.id = 'wrapper'
  document.body.append(main)
})

afterEach(() => {
  main.remove()
})

function link(html: string): HTMLAnchorElement {
  main.innerHTML = html
  return main.querySelector('a')!
}

describe('outsidePathOf', () => {
  it('reads and decodes the path of a server viewer URL', () => {
    const url = new URL(`${ORIGIN}/.mbr/outside/?path=~/src/my%20project/TODO%26x.md`)
    expect(outsidePathOf(url, ORIGIN)).toBe('~/src/my project/TODO&x.md')
  })

  it('matches a static build URL under any prefix', () => {
    const url = new URL('https://example.com/site/.mbr/outside/?path=docs/a.md')
    expect(outsidePathOf(url, 'https://example.com')).toBe('docs/a.md')
  })

  it('refuses another origin, another path and an empty path', () => {
    expect(outsidePathOf(new URL('https://evil.example/.mbr/outside/?path=x'), ORIGIN)).toBeNull()
    expect(outsidePathOf(new URL(`${ORIGIN}/.mbr/outside/x/?path=x`), ORIGIN)).toBeNull()
    expect(outsidePathOf(new URL(`${ORIGIN}/.mbr/outside/?path=`), ORIGIN)).toBeNull()
    expect(outsidePathOf(new URL(`${ORIGIN}/.mbr/outside/`), ORIGIN)).toBeNull()
  })

  it('accepts a file: URL on a page opened from disk', () => {
    expect(outsidePathOf(new URL('file:///site/.mbr/outside/?path=~/x.md'), 'null')).toBe('~/x.md')
  })
})

describe('linkKind', () => {
  it.each([
    ['https://example.com/docs', 'internet'],
    ['http://example.com/', 'internet'],
    [`${ORIGIN}/other/`, 'internal'],
    ['../other/', 'internal'],
    ['/abs/page/', 'internal'],
    ['#heading', 'internal'],
    ['/.mbr/outside/?path=~/src/x.md', 'outside'],
    ['../../.mbr/outside/?path=~/src/x.md', 'outside'],
    ['mailto:a@b.c', 'other'],
    ['message://%3Cid%3E', 'other'],
  ])('%s is %s', (href, kind) => {
    expect(linkKind(link(`<a href="${href}">text</a>`), ORIGIN)).toBe(kind)
  })
})

describe('isBareLink', () => {
  it.each([
    ['https://example.com/x', 'https://example.com/x', true],
    ['https://example.com/', 'https://example.com', true],
    ['http://www.example.com', 'www.example.com', true],
    ['https://example.com/a%20b', 'https://example.com/a b', true],
    ['mailto:a@b.c', 'a@b.c', true],
    ['https://example.com/docs', 'Docs', false],
    ['https://example.com/docs', 'example.com', false],
  ])('%s with text %s → %s', (href, text, bare) => {
    expect(isBareLink(link(`<a href="${href}">${text}</a>`))).toBe(bare)
  })

  it('an empty text is not bare', () => {
    expect(isBareLink(link('<a href="https://example.com/"></a>'))).toBe(false)
  })
})

describe('linkHoverText', () => {
  it('names the target of an outside link, decoded', () => {
    const a = link('<a href="../../.mbr/outside/?path=%2FUsers%2Fme%2Fother%20repo%2FREADME.md">x</a>')
    expect(linkHoverText(a, ORIGIN)).toBe('External: /Users/me/other repo/README.md')
  })

  it('has nothing better for other links', () => {
    expect(linkHoverText(link('<a href="https://example.com/">x</a>'), ORIGIN)).toBeNull()
  })
})

describe('classifyLinks', () => {
  function classify() {
    return classifyLinks(main, { origin: ORIGIN })
  }

  it('classes non-bare links and adds icons to internet and outside links', () => {
    main.innerHTML = `<p>
      <a id="net" href="https://example.com/docs">Docs</a>
      <a id="bare" href="https://example.com/x">https://example.com/x</a>
      <a id="in" href="../other/">Other</a>
      <a id="out" href="/.mbr/outside/?path=~/src/TODO.md">TODOs</a>
      <a id="mail" href="mailto:a@b.c">mail</a>
    </p>`
    expect(classify()).toBe(5)
    const by = (id: string) => main.querySelector<HTMLAnchorElement>(`#${id}`)!
    const icon = (id: string) => by(id).querySelector(`.${LINK_ICON_CLASS}`)

    expect(by('net').classList.contains('mbr-link-internet')).toBe(true)
    expect(icon('net')?.getAttribute('aria-hidden')).toBe('true')
    expect(by('net').lastElementChild).toBe(icon('net'))
    expect(by('net').textContent).toBe('Docs')

    expect(by('bare').classList.contains('mbr-link-bare')).toBe(true)
    expect(by('bare').classList.contains('mbr-link-internet')).toBe(false)
    expect(by('bare').getAttribute(LINK_KIND_ATTR)).toBe('internet')
    expect(icon('bare')).toBeNull()

    expect(by('in').classList.contains('mbr-link-internal')).toBe(true)
    expect(icon('in')).toBeNull()

    expect(by('out').classList.contains('mbr-link-outside')).toBe(true)
    expect(icon('out')).not.toBeNull()
    expect(by('out').title).toBe('External: ~/src/TODO.md')

    expect(by('mail').getAttribute(LINK_KIND_ATTR)).toBe('other')
    expect(by('mail').className).toBe('')
    expect(icon('mail')).toBeNull()
  })

  it('an outside link whose text is its path still gets the icon', () => {
    main.innerHTML = '<a href="/.mbr/outside/?path=~/x.md">~/x.md</a>'
    classify()
    expect(main.querySelector('a')!.classList.contains('mbr-link-outside')).toBe(true)
  })

  it('keeps an author title', () => {
    main.innerHTML = '<a href="/.mbr/outside/?path=~/x.md" title="Mine">x</a>'
    classify()
    expect(main.querySelector('a')!.title).toBe('Mine')
  })

  it('sets no title when the GUI tooltip shows the hover text', () => {
    main.innerHTML = '<a href="/.mbr/outside/?path=~/x.md">x</a>'
    classifyLinks(main, { origin: ORIGIN, skipTitles: true })
    expect(main.querySelector('a')!.hasAttribute('title')).toBe(false)
  })

  it('is idempotent', () => {
    main.innerHTML = '<a href="https://example.com/docs">Docs</a><a href="/.mbr/outside/?path=~/x">x</a>'
    expect(classify()).toBe(2)
    const before = main.innerHTML
    expect(classify()).toBe(0)
    expect(main.innerHTML).toBe(before)
    expect(main.querySelectorAll(`.${LINK_ICON_CLASS}`)).toHaveLength(2)
  })

  it('skips chrome, code, media, permalinks and footnote references', () => {
    main.innerHTML = `
      <nav class="mbr-calendar-nav"><a href="https://example.com/a">prev</a></nav>
      <header class="mbr-contact-card"><a href="https://example.com/b">site</a></header>
      <pre><code><a href="https://example.com/c">c</a></code></pre>
      <figure><a href="https://example.com/d">d</a></figure>
      <article class="mbr-social-link-box"><a class="mbr-social-link" href="https://example.com/e"><header>E</header></a></article>
      <h2>T<a class="mbr-heading-anchor" href="https://example.com/f">#</a></h2>
      <sup class="footnote-reference"><a href="#fn1">1</a></sup>
      <a href="https://example.com/g"><img src="g.png" alt=""></a>
      <a href="https://example.com/h" download>h</a>`
    expect(classify()).toBe(0)
    expect(main.querySelectorAll(`.${LINK_ICON_CLASS}`)).toHaveLength(0)
  })

  it('classifies a link in a heading and one around inline code', () => {
    main.innerHTML = '<h2><a href="https://example.com/">Site</a></h2><p><a href="https://example.com/x"><code>x()</code></a></p>'
    expect(classify()).toBe(2)
    expect(main.querySelectorAll(`.${LINK_ICON_CLASS}`)).toHaveLength(2)
  })
})

describe('isOutsideLinkTarget', () => {
  it('matches only the root-relative outside-link page links.json records', () => {
    expect(isOutsideLinkTarget('/.mbr/outside/?path=~/src/x.md')).toBe(true)
    expect(isOutsideLinkTarget('https://example.com/.mbr/outside/?path=x')).toBe(false)
    expect(isOutsideLinkTarget('/docs/.mbr/outside/')).toBe(false)
    expect(isOutsideLinkTarget('/notes/')).toBe(false)
  })
})
