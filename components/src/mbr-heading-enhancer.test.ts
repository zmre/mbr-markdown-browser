import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import './mbr-heading-enhancer.ts'
import { toggleCollapse } from './mbr-heading-enhancer.ts'

describe('toggleCollapse', () => {
  let main: HTMLElement

  beforeEach(() => {
    main = document.createElement('main')
    document.body.appendChild(main)
  })

  afterEach(() => {
    main.remove()
  })

  it('hides sibling content until the next same-level heading', () => {
    main.innerHTML = `
      <h2 id="a">A</h2>
      <p id="p1">one</p>
      <p id="p2">two</p>
      <h2 id="b">B</h2>
      <p id="p3">three</p>
    `

    const a = main.querySelector<HTMLElement>('#a')!
    toggleCollapse(a)

    expect(main.querySelector('#p1')!.classList.contains('sectionhidden')).toBe(true)
    expect(main.querySelector('#p2')!.classList.contains('sectionhidden')).toBe(true)
    expect(main.querySelector('#b')!.classList.contains('sectionhidden')).toBe(false)
    expect(main.querySelector('#p3')!.classList.contains('sectionhidden')).toBe(false)
    expect(a.classList.contains('mbr-section-collapsed')).toBe(true)
  })

  it('stops walking at a higher-level heading (h3 stops at h2)', () => {
    main.innerHTML = `
      <h3 id="h3a">Sub</h3>
      <p id="p1">one</p>
      <h2 id="h2b">Top</h2>
      <p id="p2">two</p>
    `

    const h3 = main.querySelector<HTMLElement>('#h3a')!
    toggleCollapse(h3)

    expect(main.querySelector('#p1')!.classList.contains('sectionhidden')).toBe(true)
    expect(main.querySelector('#h2b')!.classList.contains('sectionhidden')).toBe(false)
    expect(main.querySelector('#p2')!.classList.contains('sectionhidden')).toBe(false)
  })

  it('stops walking at a same-level heading (h3 stops at h3)', () => {
    main.innerHTML = `
      <h3 id="first">First</h3>
      <p id="p1">one</p>
      <h4 id="sub">Sub</h4>
      <p id="p2">two</p>
      <h3 id="second">Second</h3>
      <p id="p3">three</p>
    `

    const first = main.querySelector<HTMLElement>('#first')!
    toggleCollapse(first)

    expect(main.querySelector('#p1')!.classList.contains('sectionhidden')).toBe(true)
    expect(main.querySelector('#sub')!.classList.contains('sectionhidden')).toBe(true)
    expect(main.querySelector('#p2')!.classList.contains('sectionhidden')).toBe(true)
    expect(main.querySelector('#second')!.classList.contains('sectionhidden')).toBe(false)
    expect(main.querySelector('#p3')!.classList.contains('sectionhidden')).toBe(false)
  })

  it('round-trips: second call removes sectionhidden', () => {
    main.innerHTML = `
      <h2 id="a">A</h2>
      <p id="p1">one</p>
      <p id="p2">two</p>
      <h2 id="b">B</h2>
    `

    const a = main.querySelector<HTMLElement>('#a')!
    toggleCollapse(a)
    toggleCollapse(a)

    expect(main.querySelector('#p1')!.classList.contains('sectionhidden')).toBe(false)
    expect(main.querySelector('#p2')!.classList.contains('sectionhidden')).toBe(false)
    expect(a.classList.contains('mbr-section-collapsed')).toBe(false)
  })

  it('updates aria-expanded on headings that expose it', () => {
    main.innerHTML = `
      <h2 id="a" aria-expanded="true">A</h2>
      <p id="p1">one</p>
      <h2 id="b">B</h2>
    `

    const a = main.querySelector<HTMLElement>('#a')!
    toggleCollapse(a)
    expect(a.getAttribute('aria-expanded')).toBe('false')

    toggleCollapse(a)
    expect(a.getAttribute('aria-expanded')).toBe('true')
  })

  it('leaves aria-expanded alone on headings without the attribute', () => {
    main.innerHTML = `
      <h2 id="a">A</h2>
      <p id="p1">one</p>
      <h2 id="b">B</h2>
    `

    const a = main.querySelector<HTMLElement>('#a')!
    toggleCollapse(a)
    expect(a.hasAttribute('aria-expanded')).toBe(false)
  })
})

describe('MbrHeadingEnhancerElement._enhance', () => {
  let main: HTMLElement
  let enhancer: HTMLElement

  beforeEach(() => {
    main = document.createElement('main')
    document.body.appendChild(main)
    enhancer = document.createElement('mbr-heading-enhancer')
  })

  afterEach(() => {
    enhancer.remove()
    main.remove()
  })

  // Access the private _enhance via a cast; the public entry point defers
  // work to requestIdleCallback which is awkward to await in tests.
  function runEnhance(): void {
    ;(enhancer as unknown as { _enhance: () => void })._enhance()
  }

  it('appends a permalink anchor to headings with an id', () => {
    main.innerHTML = `<h2 id="intro">Intro</h2>`
    document.body.appendChild(enhancer)
    runEnhance()

    const anchors = main.querySelectorAll('a.mbr-heading-anchor')
    expect(anchors.length).toBe(1)
    expect(anchors[0].getAttribute('href')).toBe('#intro')
    expect(anchors[0].textContent).toBe('#')
  })

  it('does not append a permalink to headings without an id', () => {
    main.innerHTML = `<h2>No id</h2>`
    document.body.appendChild(enhancer)
    runEnhance()

    expect(main.querySelectorAll('a.mbr-heading-anchor').length).toBe(0)
  })

  it('skips collapse for headings that contain a link', () => {
    main.innerHTML = `<h2 id="linked"><a href="/somewhere">Linked</a></h2>`
    document.body.appendChild(enhancer)
    runEnhance()

    const h2 = main.querySelector<HTMLElement>('#linked')!
    expect(h2.classList.contains('mbr-collapsible')).toBe(false)
  })

  it('skips collapse for headings nested inside a link', () => {
    main.innerHTML = `<a href="/outer"><h2 id="nested">Nested</h2></a>`
    document.body.appendChild(enhancer)
    runEnhance()

    const h2 = main.querySelector<HTMLElement>('#nested')!
    expect(h2.classList.contains('mbr-collapsible')).toBe(false)
  })

  it('is idempotent: running twice does not duplicate permalink anchors', () => {
    main.innerHTML = `<h2 id="dup">Dup</h2><p>text</p>`
    document.body.appendChild(enhancer)
    runEnhance()
    runEnhance()

    expect(main.querySelectorAll('a.mbr-heading-anchor').length).toBe(1)
  })

  it('permalink anchor click does not trigger collapse', () => {
    main.innerHTML = `<h2 id="stop">Stop</h2><p id="content">body</p><h2 id="next">Next</h2>`
    document.body.appendChild(enhancer)
    runEnhance()

    const anchor = main.querySelector<HTMLElement>('a.mbr-heading-anchor')!
    anchor.click()

    const h2 = main.querySelector<HTMLElement>('#stop')!
    expect(h2.classList.contains('mbr-section-collapsed')).toBe(false)
    expect(main.querySelector('#content')!.classList.contains('sectionhidden')).toBe(false)
  })

  it('clicking the heading toggles collapse', () => {
    main.innerHTML = `<h2 id="clicky">Clicky</h2><p id="c1">one</p><h2 id="after">After</h2>`
    document.body.appendChild(enhancer)
    runEnhance()

    const h2 = main.querySelector<HTMLElement>('#clicky')!
    h2.click()

    expect(h2.classList.contains('mbr-section-collapsed')).toBe(true)
    expect(main.querySelector('#c1')!.classList.contains('sectionhidden')).toBe(true)
    expect(main.querySelector('#after')!.classList.contains('sectionhidden')).toBe(false)
  })

  it('adds ARIA attributes to collapsible headings', () => {
    main.innerHTML = `<h2 id="a">A</h2><p>one</p>`
    document.body.appendChild(enhancer)
    runEnhance()

    const h2 = main.querySelector<HTMLElement>('#a')!
    expect(h2.getAttribute('tabindex')).toBe('0')
    expect(h2.getAttribute('aria-expanded')).toBe('true')
  })

  it('does not add ARIA attributes to linked headings', () => {
    main.innerHTML = `<h2 id="linked"><a href="/x">Linked</a></h2>`
    document.body.appendChild(enhancer)
    runEnhance()

    const h2 = main.querySelector<HTMLElement>('#linked')!
    expect(h2.hasAttribute('tabindex')).toBe(false)
    expect(h2.hasAttribute('aria-expanded')).toBe(false)
  })

  it('updates aria-expanded when the heading is collapsed and expanded', () => {
    main.innerHTML = `<h2 id="a">A</h2><p id="p1">one</p><h2 id="b">B</h2>`
    document.body.appendChild(enhancer)
    runEnhance()

    const h2 = main.querySelector<HTMLElement>('#a')!
    h2.click()
    expect(h2.getAttribute('aria-expanded')).toBe('false')

    h2.click()
    expect(h2.getAttribute('aria-expanded')).toBe('true')
  })

  it('Enter key on focused heading toggles collapse', () => {
    main.innerHTML = `<h2 id="k">K</h2><p id="p1">one</p><h2 id="next">Next</h2>`
    document.body.appendChild(enhancer)
    runEnhance()

    const h2 = main.querySelector<HTMLElement>('#k')!
    h2.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))

    expect(h2.classList.contains('mbr-section-collapsed')).toBe(true)
    expect(main.querySelector('#p1')!.classList.contains('sectionhidden')).toBe(true)
  })

  it('Space key on focused heading toggles collapse', () => {
    main.innerHTML = `<h2 id="k">K</h2><p id="p1">one</p><h2 id="next">Next</h2>`
    document.body.appendChild(enhancer)
    runEnhance()

    const h2 = main.querySelector<HTMLElement>('#k')!
    h2.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }))

    expect(h2.classList.contains('mbr-section-collapsed')).toBe(true)
  })

  it('other keys on focused heading do not toggle collapse', () => {
    main.innerHTML = `<h2 id="k">K</h2><p id="p1">one</p>`
    document.body.appendChild(enhancer)
    runEnhance()

    const h2 = main.querySelector<HTMLElement>('#k')!
    h2.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }))

    expect(h2.classList.contains('mbr-section-collapsed')).toBe(false)
  })

  it('keydown originating from the permalink anchor does not toggle collapse', () => {
    main.innerHTML = `<h2 id="k">K</h2><p id="p1">one</p><h2 id="next">Next</h2>`
    document.body.appendChild(enhancer)
    runEnhance()

    const anchor = main.querySelector<HTMLElement>('a.mbr-heading-anchor')!
    anchor.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))

    const h2 = main.querySelector<HTMLElement>('#k')!
    expect(h2.classList.contains('mbr-section-collapsed')).toBe(false)
  })
})
