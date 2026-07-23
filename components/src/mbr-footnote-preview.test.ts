import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import './mbr-footnote-preview.ts'
import {
  resolveFootnoteDefinition,
  buildPreviewFragment,
} from './mbr-footnote-preview.ts'

describe('resolveFootnoteDefinition', () => {
  let main: HTMLElement

  beforeEach(() => {
    main = document.createElement('main')
    document.body.appendChild(main)
  })

  afterEach(() => {
    main.remove()
  })

  it('resolves the matching definition by href/id', () => {
    main.innerHTML = `
      <p>Text<sup class="footnote-reference"><a href="#note1">1</a></sup></p>
      <div class="footnote-definition" id="note1">
        <sup class="footnote-definition-label">1</sup> <p>Body</p>
      </div>
    `
    const anchor = main.querySelector<HTMLAnchorElement>(
      'sup.footnote-reference > a'
    )!
    const def = resolveFootnoteDefinition(anchor)
    expect(def).not.toBeNull()
    expect(def!.id).toBe('note1')
  })

  it('returns null when the target is missing', () => {
    main.innerHTML = `<sup class="footnote-reference"><a href="#missing">1</a></sup>`
    const anchor = main.querySelector<HTMLAnchorElement>('a')!
    expect(resolveFootnoteDefinition(anchor)).toBeNull()
  })

  it('returns null when the target is not a footnote definition', () => {
    main.innerHTML = `
      <sup class="footnote-reference"><a href="#heading">1</a></sup>
      <h2 id="heading">Not a footnote</h2>
    `
    const anchor = main.querySelector<HTMLAnchorElement>('a')!
    expect(resolveFootnoteDefinition(anchor)).toBeNull()
  })

  it('decodes percent-encoded href fragments', () => {
    main.innerHTML = `
      <sup class="footnote-reference"><a href="#note%20one">1</a></sup>
      <div class="footnote-definition" id="note one"><p>Body</p></div>
    `
    const anchor = main.querySelector<HTMLAnchorElement>('a')!
    const def = resolveFootnoteDefinition(anchor)
    expect(def).not.toBeNull()
    expect(def!.id).toBe('note one')
  })

  it('returns null for anchors without a hash href', () => {
    main.innerHTML = `<sup class="footnote-reference"><a href="">1</a></sup>`
    const anchor = main.querySelector<HTMLAnchorElement>('a')!
    expect(resolveFootnoteDefinition(anchor)).toBeNull()
  })
})

describe('buildPreviewFragment', () => {
  function makeDef(html: string): HTMLElement {
    const div = document.createElement('div')
    div.className = 'footnote-definition'
    div.innerHTML = html
    return div
  }

  it('excludes the numeric label but keeps the note content', () => {
    const def = makeDef(
      `<sup class="footnote-definition-label">1</sup> <p>Body text</p>`
    )
    const host = document.createElement('div')
    host.appendChild(buildPreviewFragment(def))

    expect(host.querySelector('.footnote-definition-label')).toBeNull()
    expect(host.textContent).toContain('Body text')
  })

  it('keeps links present in the note content', () => {
    const def = makeDef(
      `<sup class="footnote-definition-label">2</sup> <p>See <a href="https://example.com">source</a></p>`
    )
    const host = document.createElement('div')
    host.appendChild(buildPreviewFragment(def))

    const link = host.querySelector('a')
    expect(link).not.toBeNull()
    expect(link!.getAttribute('href')).toBe('https://example.com')
  })

  it('does not mutate the original definition', () => {
    const def = makeDef(
      `<sup class="footnote-definition-label">3</sup> <p>Body</p>`
    )
    buildPreviewFragment(def)
    expect(def.querySelector('.footnote-definition-label')).not.toBeNull()
  })
})
