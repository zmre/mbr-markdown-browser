import { describe, it, expect } from 'vitest'
import { safeHref, isDangerousUrl, NEUTRALIZED_HREF } from './safe-href.ts'

/**
 * `safeHref` guards every `href` binding fed from `links.json` destinations,
 * which are raw markdown URLs. The hostile table below encodes the browser
 * normalization rules the check has to mirror (case folding, embedded
 * tab/newline, leading control characters).
 */
describe('safeHref', () => {
  const hostile = [
    'javascript:alert(1)',
    'JaVaScRiPt:alert(1)',
    'JAVASCRIPT:alert(document.domain)',
    'java\tscript:alert(1)',
    'java\nscript:alert(1)',
    'java\rscript:alert(1)',
    'jav\ta\nscr\ript:alert(1)',
    '  javascript:alert(1)',
    '\u0000javascript:alert(1)',
    '\u0001\u0002 javascript:alert(1)',
    '\n\n javascript:alert(1)',
    'javascript:void(0)',
    'vbscript:msgbox(1)',
    'VBScript:msgbox(1)',
    'vb\tscript:msgbox(1)',
    'data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==',
    'data:text/html,<script>alert(1)</script>',
    'DATA:TEXT/HTML,<script>alert(1)</script>',
    'da\tta:text/html,<script>alert(1)</script>',
    'data:application/javascript,alert(1)',
    // SVG can carry script, so it is not covered by the data:image/ exception.
    'data:image/svg+xml,<svg onload="alert(1)"/>',
  ]

  it.each(hostile)('neutralizes %j', (url) => {
    expect(isDangerousUrl(url)).toBe(true)
    expect(safeHref(url)).toBe(NEUTRALIZED_HREF)
  })

  const benign = [
    'https://example.com/page',
    'http://example.com/page?q=javascript:alert(1)',
    'https://example.com/#javascript:alert(1)',
    '//example.com/protocol-relative',
    '/docs/guide/',
    './sibling/',
    '../parent/',
    'relative/page/',
    '#anchor',
    '#javascript:alert(1)',
    'mailto:someone@example.com',
    'tel:+15555555555',
    'ftp://example.com/file.txt',
    // A colon later in the path is not a scheme.
    '/notes/todo:tomorrow/',
    'my-javascript:notes/',
    'data:image/png;base64,iVBORw0KGgo=',
    'data:image/gif;base64,R0lGODlhAQABAAAAACw=',
    '',
  ]

  it.each(benign)('leaves %j untouched', (url) => {
    expect(isDangerousUrl(url)).toBe(false)
    expect(safeHref(url)).toBe(url)
  })

  it('neutralizes a non-string destination from malformed JSON', () => {
    expect(safeHref(undefined as unknown as string)).toBe(NEUTRALIZED_HREF)
    expect(safeHref(null as unknown as string)).toBe(NEUTRALIZED_HREF)
  })
})
