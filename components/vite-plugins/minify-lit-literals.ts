import type { Plugin } from 'vite'

/**
 * `mbr-minify-lit-literals`: collapses whitespace in the **static** parts of
 * Lit's `css` and `html` tagged templates.
 *
 * Terser never touches a template's text, so every indentation space in a
 * component's styles and markup ships verbatim (the calendar chunk's alone
 * was ~8 kB). Used by the main bundle, the tasks chunk and the calendar chunk.
 *
 * Only the text between `${…}` holes is rewritten; an expression is never
 * touched (a nested template inside one is a template of its own, visited on
 * its own terms). The rules:
 *
 * - **`css`**: comments go; runs of whitespace become one space; the spaces
 *   around `{ } ; ,` and after `:` go; `;}` becomes `}`. Quoted strings
 *   (`content: "a, b"`) are copied as written.
 * - **`html`**: a run of whitespace that holds a newline becomes one space,
 *   which HTML renders the same — *except* where whitespace is content. So a
 *   literal containing `<pre` or `<textarea` is left as written, and so is
 *   every `html` literal nested inside it; and a source file that styles
 *   anything `white-space: pre…` / `break-spaces` keeps all of its `html`
 *   literals as written, since the element that rule targets cannot be told
 *   apart here.
 *
 * A literal whose source span does not read back as its own raw text (an
 * unexpected AST shape) is left alone rather than guessed at.
 */
export function minifyLitLiterals(): Plugin {
  return {
    name: 'mbr-minify-lit-literals',
    // After TypeScript is stripped, so the bundler's own parser reads it.
    transform(code, id) {
      if (!/\/src\/.*\.ts$/.test(id) || id.includes('node_modules')) return null
      const out = minifyLitSource(code, (source) => this.parse(source))
      return out === null ? null : { code: out, map: null }
    },
  }
}

/**
 * `mbr-strip-pure-annotations`: drops the `/* @__PURE__ *\/` markers from the
 * finished chunks.
 *
 * Vite forces terser's `preserve_annotations` on for an ES-format library
 * build, so a downstream bundler can tree-shake it. These bundles are final
 * assets served to browsers, never bundled again, so the markers are dead
 * bytes (32 in the calendar chunk, ~120 bytes gzipped).
 */
export function stripPureAnnotations(): Plugin {
  return {
    name: 'mbr-strip-pure-annotations',
    generateBundle(_options, bundle) {
      for (const chunk of Object.values(bundle)) {
        if (chunk.type === 'chunk') chunk.code = stripPure(chunk.code)
      }
    },
  }
}

/** `code` without its pure annotations; only where code follows one. */
export function stripPure(code: string): string {
  return code.replace(/\/\* [@#]__PURE__ \*\/(?=new\b|[\w$.(])/g, '')
}

/** `css` static text, collapsed; quoted strings are kept byte for byte. */
export function collapseCss(text: string): string {
  return text
    .split(/("(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*')/)
    .map((part, i) =>
      i % 2 === 1
        ? part
        : part
            .replace(/\/\*[\s\S]*?\*\//g, '')
            .replace(/\s+/g, ' ')
            .replace(/\s*([{};,])\s*/g, '$1')
            .replace(/:\s+/g, ':')
            .replace(/;}/g, '}')
    )
    .join('')
}

/** `html` static text: a whitespace run holding a newline becomes one space. */
export function collapseHtml(text: string): string {
  return text.replace(/\s*\n\s*/g, ' ')
}

/** Whitespace is content in these elements. */
const WHITESPACE_ELEMENT = /<(?:pre|textarea)\b/i

/** A rule making an element's whitespace significant. */
const WHITESPACE_STYLE = /white-space\s*:\s*(?:pre|break-spaces)/i

interface Quasi {
  start: number
  end: number
  value: { raw: string }
}

/**
 * `code` with its Lit literals collapsed, or `null` when nothing changed.
 * `parse` is the bundler's ESTree parser (`this.parse` in a plugin; tests
 * pass Vite's `parseAst`).
 */
export function minifyLitSource(code: string, parse: (code: string) => unknown): string | null {
  if (!/\b(css|html)`/.test(code)) return null
  const htmlAllowed = !WHITESPACE_STYLE.test(code)
  const edits: { start: number; end: number; text: string }[] = []
  const visit = (node: unknown, inWhitespaceElement: boolean): void => {
    if (!node || typeof node !== 'object') return
    if (Array.isArray(node)) {
      node.forEach((child) => visit(child, inWhitespaceElement))
      return
    }
    const n = node as Record<string, unknown>
    let nested = inWhitespaceElement
    const tag = n.tag as { type?: string; name?: string } | undefined
    if (n.type === 'TaggedTemplateExpression' && tag?.type === 'Identifier' && (tag.name === 'css' || tag.name === 'html')) {
      const quasis = (n.quasi as { quasis: Quasi[] }).quasis
      let collapse: ((text: string) => string) | null = tag.name === 'css' ? collapseCss : collapseHtml
      if (tag.name === 'html') {
        const sensitive = quasis.some((q) => WHITESPACE_ELEMENT.test(q.value.raw))
        nested = inWhitespaceElement || sensitive
        if (!htmlAllowed || nested) collapse = null
      }
      if (collapse) {
        for (const q of quasis) {
          if (code.slice(q.start, q.end) === q.value.raw) {
            edits.push({ start: q.start, end: q.end, text: collapse(q.value.raw) })
          }
        }
      }
    }
    for (const key in n) if (key !== 'parent') visit(n[key], nested)
  }
  visit(parse(code), false)
  const changed = edits.filter((e) => e.text !== code.slice(e.start, e.end))
  if (changed.length === 0) return null
  let out = code
  for (const e of changed.sort((x, y) => y.start - x.start)) out = out.slice(0, e.start) + e.text + out.slice(e.end)
  return out
}
