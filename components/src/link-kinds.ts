/**
 * Link kinds: marks each link in a rendered note as in-vault, on the internet
 * or out of the vault, so theme.css can draw a small icon after the last two.
 *
 * Main bundle, no custom element (like `nav-title.ts`): it runs at idle on
 * every markdown page in every mode — server, GUI and static builds — and
 * touches only `main#wrapper`, never the header, nav or footer.
 *
 * What it writes on an `<a>`:
 * - `data-mbr-link-kind="internal|internet|outside|other"` on every link it
 *   judged (also what makes a second pass a no-op);
 * - for a link whose text is not its own URL, the class `mbr-link-internal`,
 *   `mbr-link-internet` or `mbr-link-outside`; a bare link (`<https://x>`,
 *   text == href) gets `mbr-link-bare` instead and no icon;
 * - for internet and outside links, one empty
 *   `<span class="mbr-link-icon" aria-hidden="true">` as the last child. The
 *   icon itself is pure CSS (a mask, theme.css "Link Kinds"), so `.mbr/user.css`
 *   hides it with `.mbr-link-icon { display: none }`.
 *
 * Why a child span and not `a::after`: in the GUI, `<mbr-link-enhancement>`
 * gives every link in `<main>` a Pico `data-tooltip`, and Pico's tooltip owns
 * both of the anchor's pseudo-elements (`::after` is the caret, at `opacity: 0;
 * position: absolute`). An icon there is drawn but invisible.
 *
 * Outside links (`/.mbr/outside/?path=…`, see `mbr_core::outside_link`) also
 * get a hover text naming the target, `External: <path>`: a `title` in a
 * browser (an author's own `title` is kept), and in the GUI the tooltip
 * `<mbr-link-enhancement>` draws, which reads `linkHoverText` from here.
 */

import { scheduleIdleTask } from './dynamic-loader.ts'
import { isGuiMode } from './shared.ts'

/** The URL path of the outside-link page, at the end of any base. */
const OUTSIDE_VIEWER_SUFFIX = '/.mbr/outside/'

/** Hover text prefix for a link out of the vault. */
export const OUTSIDE_HOVER_PREFIX = 'External: '

export type LinkKind = 'internal' | 'internet' | 'outside' | 'other'

/** Class per kind, added only to non-bare links. */
export const LINK_KIND_CLASS: Readonly<Record<Exclude<LinkKind, 'other'>, string>> = {
  internal: 'mbr-link-internal',
  internet: 'mbr-link-internet',
  outside: 'mbr-link-outside',
}

export const LINK_BARE_CLASS = 'mbr-link-bare'
export const LINK_ICON_CLASS = 'mbr-link-icon'
export const LINK_KIND_ATTR = 'data-mbr-link-kind'

/** Kinds that get an icon. */
const ICON_KINDS: ReadonlySet<LinkKind> = new Set(['internet', 'outside'])

/**
 * Links that are not prose links: chrome inside `<main>` (calendar bar,
 * contact card), code, media and oembed cards, heading permalinks, footnote
 * references, download buttons.
 */
const SKIP_SELECTOR = [
  'nav',
  'header',
  'pre',
  'figure',
  '.mbr-social-link-box',
  '.mbr-heading-anchor',
  '.footnote-reference',
  'a[download]',
].join(', ')

/**
 * Whether a `links.json` destination is the outside-link page. The server and
 * the build both record it root-relative (`/.mbr/outside/?path=…`) with
 * `internal: false`, so lists that open `internal: false` links in a new
 * window must ask this first: the page is on this site and opens in place
 * (through `resolveUrl`, for static builds).
 */
export function isOutsideLinkTarget(to: string): boolean {
  return to.startsWith(`${OUTSIDE_VIEWER_SUFFIX}?`)
}

/** `url` parsed, or `null`. */
function parseUrl(href: string): URL | null {
  try {
    return new URL(href)
  } catch {
    return null
  }
}

/**
 * The target named by an outside-link URL (`…/.mbr/outside/?path=…`),
 * percent-decoding undone, or `null` when `url` is not one. A static build
 * writes the URL relative (`../.mbr/outside/?path=…`), so this takes the
 * resolved URL and matches the path's end, at any depth or deploy prefix.
 */
export function outsidePathOf(url: URL, origin: string): string | null {
  if (!url.pathname.endsWith(OUTSIDE_VIEWER_SUFFIX)) return null
  // A static site opened from disk has the opaque origin "null" on both sides.
  const sameSite = url.origin === origin || (url.protocol === 'file:' && origin === 'null')
  if (!sameSite) return null
  const path = url.searchParams.get('path')
  return path && path.trim() !== '' ? path : null
}

/** What an anchor points at. `origin` is the page's (`location.origin`). */
export function linkKind(anchor: HTMLAnchorElement, origin: string): LinkKind {
  const raw = anchor.getAttribute('href')
  if (raw === null) return 'other'
  if (raw.startsWith('#')) return 'internal'
  const url = parseUrl(anchor.href)
  if (!url) return 'other'
  if (outsidePathOf(url, origin) !== null) return 'outside'
  if (url.protocol === 'http:' || url.protocol === 'https:') {
    return url.origin === origin ? 'internal' : 'internet'
  }
  // A static build opened from disk links its own pages with file: URLs.
  if (url.protocol === 'file:' && origin === 'null') return 'internal'
  return 'other'
}

/** A URL or link text reduced for comparison: no scheme, no trailing slash. */
function comparable(text: string): string {
  let t = text.trim()
  try {
    t = decodeURI(t)
  } catch {
    // Keep it as written.
  }
  return t
    .replace(/^[a-z][a-z0-9+.-]*:(\/\/)?/i, '')
    .replace(/\/+$/, '')
    .toLowerCase()
}

/**
 * Whether the link's visible text is its own URL (`<https://x>`, an
 * autolinked `www.x.com`, `[mailto:a@b](mailto:a@b)`): such a link already
 * says where it goes.
 */
export function isBareLink(anchor: HTMLAnchorElement): boolean {
  const text = comparable(anchor.textContent ?? '')
  if (text === '') return false
  const raw = anchor.getAttribute('href') ?? ''
  return text === comparable(raw) || text === comparable(anchor.href)
}

/**
 * The hover text for a link, when mbr has a better one than its URL: only for
 * outside links today (`External: ~/src/x.md`), else `null`.
 */
export function linkHoverText(anchor: HTMLAnchorElement, origin: string): string | null {
  const url = parseUrl(anchor.href)
  if (!url) return null
  const path = outsidePathOf(url, origin)
  return path === null ? null : OUTSIDE_HOVER_PREFIX + path
}

/** Whether a link shows nothing but media (an image link): no icon after it. */
function isMediaOnly(anchor: HTMLAnchorElement): boolean {
  return (anchor.textContent ?? '').trim() === '' && anchor.querySelector('img, svg, picture, video') !== null
}

export interface ClassifyOptions {
  /** The page's origin (default `location.origin`). */
  origin?: string
  /**
   * Set no `title` (default false). The GUI passes true: its own tooltip shows
   * the hover text, and a `title` would add a second, native one.
   */
  skipTitles?: boolean
}

/**
 * Mark every prose link under `root`. Idempotent: a link already marked is
 * left alone. Returns how many links it marked.
 */
export function classifyLinks(root: ParentNode, options: ClassifyOptions = {}): number {
  const origin = options.origin ?? window.location.origin
  let marked = 0
  for (const anchor of root.querySelectorAll<HTMLAnchorElement>('a[href]')) {
    if (anchor.hasAttribute(LINK_KIND_ATTR)) continue
    if (anchor.closest(SKIP_SELECTOR)) continue
    if (isMediaOnly(anchor)) continue

    const kind = linkKind(anchor, origin)
    anchor.setAttribute(LINK_KIND_ATTR, kind)
    marked += 1

    if (kind === 'outside' && !options.skipTitles && !anchor.hasAttribute('title')) {
      const hover = linkHoverText(anchor, origin)
      if (hover) anchor.title = hover
    }

    if (kind === 'other') continue
    // An outside link's URL is the viewer page, never its text, so it is
    // never bare.
    if (kind !== 'outside' && isBareLink(anchor)) {
      anchor.classList.add(LINK_BARE_CLASS)
      continue
    }
    anchor.classList.add(LINK_KIND_CLASS[kind])
    if (ICON_KINDS.has(kind)) {
      const icon = document.createElement('span')
      icon.className = LINK_ICON_CLASS
      icon.setAttribute('aria-hidden', 'true')
      anchor.append(icon)
    }
  }
  return marked
}

/** Whether this page is a rendered note (`_head_markdown.html` sets it). */
function isMarkdownPage(): boolean {
  return (window as { frontmatter?: unknown }).frontmatter !== undefined
}

function start(): void {
  if (!isMarkdownPage()) return
  scheduleIdleTask(() => {
    const main = document.querySelector('main#wrapper')
    if (main) classifyLinks(main, { skipTitles: isGuiMode() })
  })
}

if (typeof document !== 'undefined') {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start, { once: true })
  } else {
    start()
  }
}
