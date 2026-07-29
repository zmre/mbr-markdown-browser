/**
 * Scheme allowlisting for URLs that are bound into an `href`.
 *
 * Link destinations reach the UI verbatim: `link.to` in a page's `links.json`
 * is the raw markdown destination, and the Rust side deliberately classifies
 * `javascript:`/`data:` as "external" and passes them through unchanged. Lit
 * does not sanitize attribute bindings (`setSanitizer` is never called), so
 * anything rendered as `href="${...}"` must be filtered here.
 *
 * The check mirrors how browsers normalize a URL before resolving its scheme:
 * leading C0 control characters and spaces are ignored, and ASCII tab/CR/LF
 * are stripped from *anywhere* in the URL — so `java<TAB>script:alert(1)` and
 * `javascript:alert(1)` both navigate. Comparison is case-insensitive.
 */

/** Replacement for a rejected destination: renders, but navigates nowhere. */
export const NEUTRALIZED_HREF = '#'

/** Schemes that can execute script when navigated to. */
const DANGEROUS_SCHEMES = ['javascript:', 'vbscript:', 'data:']

/**
 * `data:` URLs that are still allowed: static raster images. SVG stays blocked
 * because an SVG document can carry script.
 */
const SAFE_DATA_PREFIX = 'data:image/'
const UNSAFE_DATA_IMAGE_PREFIX = 'data:image/svg'

/** ASCII tab/LF/CR, which browsers strip from anywhere in a URL. */
const URL_STRIPPED_CHARS = /[\u0009\u000A\u000D]/g

/** Leading C0 control characters and spaces, which browsers ignore. */
const URL_LEADING_JUNK = /^[\u0000-\u0020]+/

/**
 * Normalize a URL the way a browser does before scheme resolution: drop ASCII
 * tab/newline anywhere, trim leading C0 controls and spaces, then lowercase.
 */
function normalizeForSchemeCheck(url: string): string {
  return url.replace(URL_STRIPPED_CHARS, '').replace(URL_LEADING_JUNK, '').toLowerCase()
}

/**
 * True when the URL resolves to a script-capable scheme and must not be used
 * as an `href`.
 */
export function isDangerousUrl(url: string): boolean {
  const normalized = normalizeForSchemeCheck(url)
  for (const scheme of DANGEROUS_SCHEMES) {
    if (!normalized.startsWith(scheme)) continue
    if (
      scheme === 'data:' &&
      normalized.startsWith(SAFE_DATA_PREFIX) &&
      !normalized.startsWith(UNSAFE_DATA_IMAGE_PREFIX)
    ) {
      return false
    }
    return true
  }
  return false
}

/**
 * Return `url` unchanged when it is safe to use as an `href`, otherwise a
 * neutral `#`. Ordinary http/https, protocol-relative, root-relative,
 * relative, `mailto:` and pure-fragment URLs pass through untouched.
 */
export function safeHref(url: string): string {
  // links.json is parsed JSON, so a non-string can arrive despite the type.
  if (typeof url !== 'string') return NEUTRALIZED_HREF
  return isDangerousUrl(url) ? NEUTRALIZED_HREF : url
}
