/**
 * Draws a {@link NotFoundSuggestions} into the 404 page's
 * `<mbr-not-found-suggestions>`, in the light DOM so the page's own Pico and
 * theme.css styles apply (it sits inside the error `<article>`).
 *
 * Plain DOM, no Lit, and every text goes through `textContent`: titles come
 * from frontmatter.
 */
import type { NotFoundSuggestions, SuggestedPage } from './match.js'

/** Turns a canonical site path into an href for this page (server or static). */
export type ResolveUrl = (path: string) => string

/** `/a/b/note/` → `/a/b/`, shown beside a title so two same-named notes differ. */
function folderLabel(url: string): string {
  const segs = url.split('/').filter((s) => s.length > 0)
  return segs.length <= 1 ? '/' : `/${segs.slice(0, -1).join('/')}/`
}

function link(url: string, text: string, resolveUrl: ResolveUrl): HTMLAnchorElement {
  const a = document.createElement('a')
  a.href = resolveUrl(url)
  a.textContent = text
  return a
}

function pageItem(page: SuggestedPage, resolveUrl: ResolveUrl, withFolder: boolean): HTMLLIElement {
  const li = document.createElement('li')
  li.append(link(page.url, page.title, resolveUrl))
  if (withFolder) {
    const where = document.createElement('small')
    where.className = 'mbr-not-found-where'
    where.textContent = folderLabel(page.url)
    li.append(' ', where)
  }
  return li
}

function heading(text: string): HTMLParagraphElement {
  const p = document.createElement('p')
  p.className = 'mbr-not-found-heading'
  const strong = document.createElement('strong')
  strong.textContent = text
  p.append(strong)
  return p
}

/**
 * Replaces `host`'s children with the suggestions, or empties it when there
 * are none (the page then looks exactly as it did before this existed).
 */
export function renderNotFound(
  host: HTMLElement,
  suggestions: NotFoundSuggestions,
  resolveUrl: ResolveUrl,
): void {
  const { matches, folder } = suggestions
  if (matches.length > 0) {
    const list = document.createElement('ul')
    list.className = 'mbr-not-found-matches'
    list.append(...matches.map((page) => pageItem(page, resolveUrl, true)))
    host.replaceChildren(heading('Maybe you were looking for:'), list)
    return
  }
  if (folder) {
    const intro = heading('')
    intro.firstElementChild!.append('Its folder is still here: ', link(folder.url, folder.title, resolveUrl))
    const children: Node[] = [intro]
    if (folder.notes.length > 0) {
      const list = document.createElement('ul')
      list.className = 'mbr-not-found-folder'
      list.append(...folder.notes.map((page) => pageItem(page, resolveUrl, false)))
      if (folder.more > 0) {
        const li = document.createElement('li')
        li.append(link(folder.url, `…and ${folder.more} more`, resolveUrl))
        list.append(li)
      }
      children.push(list)
    }
    host.replaceChildren(...children)
    return
  }
  host.replaceChildren()
}
