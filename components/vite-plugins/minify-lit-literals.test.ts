import { describe, expect, it } from 'vitest'
import { parseAst } from 'vite'
import { collapseCss, collapseHtml, minifyLitSource, stripPure } from './minify-lit-literals'

const minify = (code: string): string => minifyLitSource(code, (source) => parseAst(source)) ?? code

describe('collapseCss', () => {
  it('collapses whitespace around punctuation and after colons', () => {
    expect(collapseCss('\n  .a {\n    color: red;\n    margin: 0 auto;\n  }\n  .b, .c { x: 1 }\n')).toBe(
      ' .a{color:red;margin:0 auto}.b,.c{x:1}'
    )
  })

  it('drops comments', () => {
    expect(collapseCss('/* note */ .a { /* why */ color: red; }')).toBe(' .a{color:red}')
  })

  it('keeps quoted strings byte for byte', () => {
    expect(collapseCss(".a::before { content: 'a,  b ; c'; }")).toBe(".a::before{content:'a,  b ; c'}")
    expect(collapseCss('.a { font-family: "Segoe  UI", sans-serif; }')).toBe('.a{font-family:"Segoe  UI",sans-serif}')
    expect(collapseCss(".a { content: '/* not a comment */'; }")).toBe(".a{content:'/* not a comment */'}")
  })
})

describe('collapseHtml', () => {
  it('turns a whitespace run holding a newline into one space', () => {
    expect(collapseHtml('\n  <div>\n    <span>a</span>\n    <span>b</span>\n  </div>\n')).toBe(' <div> <span>a</span> <span>b</span> </div> ')
  })

  it('leaves whitespace without a newline alone', () => {
    expect(collapseHtml('<b>a</b>  <i>b</i>')).toBe('<b>a</b>  <i>b</i>')
  })
})

describe('minifyLitSource', () => {
  it('never touches ${…} expressions', () => {
    const code = [
      'const x = css`',
      '  .a {',
      '    width: ${ calc( 1,  2 ) }px;',
      '  }',
      '`',
      'const y = html`',
      '  <p title=${ "a\\n   b" }>',
      '    ${ items.map((i) =>   i) }',
      '  </p>',
      '`',
    ].join('\n')
    const out = minify(code)
    expect(out).toContain('${ calc( 1,  2 ) }')
    expect(out).toContain('${ "a\\n   b" }')
    expect(out).toContain('${ items.map((i) =>   i) }')
    expect(out).toContain('css` .a{width:${ calc( 1,  2 ) }px}`')
    expect(out).toContain('html` <p title=${ "a\\n   b" }> ${ items.map((i) =>   i) } </p> `')
  })

  it('collapses templates nested inside expressions', () => {
    const out = minify('const a = html`<ul>\n  ${xs.map((x) => html`\n    <li>${x}</li>\n  `)}\n</ul>`')
    expect(out).toBe('const a = html`<ul> ${xs.map((x) => html` <li>${x}</li> `)} </ul>`')
  })

  it('leaves a <pre> or <textarea> literal, and templates nested in it, as written', () => {
    const pre = 'const a = html`<pre>\n  line one\n    line two\n  ${b ? html`\n  kept\n` : ""}</pre>`'
    expect(minify(pre)).toBe(pre)
    const area = 'const a = html`<TEXTAREA>\n  keep\n</TEXTAREA>`'
    expect(minify(area)).toBe(area)
  })

  it('still collapses css in a file whose html is left alone', () => {
    const code = 'const s = css`\n  .p {\n    white-space: pre-wrap;\n  }\n`\nconst h = html`<div class="p">\n  a\n  b\n</div>`'
    const out = minify(code)
    expect(out).toContain('css` .p{white-space:pre-wrap}`')
    expect(out).toContain('html`<div class="p">\n  a\n  b\n</div>`')
  })

  it('ignores other tags and returns null when nothing changes', () => {
    const code = 'const a = sql`\n  select 1\n`\nconst b = html`<b>x</b>`'
    expect(minifyLitSource(code, (source) => parseAst(source))).toBeNull()
  })
})

describe('stripPure', () => {
  it('drops pure annotations in front of code', () => {
    expect(stripPure('a=/* @__PURE__ */new WeakMap,b=/* #__PURE__ */f(1)')).toBe('a=new WeakMap,b=f(1)')
  })

  it('leaves anything else that merely looks like one', () => {
    expect(stripPure('"/* @__PURE__ */ "')).toBe('"/* @__PURE__ */ "')
  })
})

