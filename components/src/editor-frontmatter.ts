/**
 * Pure helpers for separating YAML frontmatter from the markdown body and
 * recombining them. Milkdown/Crepe edits only the body; the frontmatter is
 * edited as raw YAML. Keeping these pure and dependency-free lets us unit-test
 * the round-trip without loading the editor.
 */

export interface SplitResult {
  /** Inner YAML (without the `---` fences), or `null` if there is no frontmatter. */
  frontmatter: string | null;
  /** The markdown body following the frontmatter (or the whole document). */
  body: string;
}

/**
 * Splits a raw markdown document into its leading YAML frontmatter block and
 * the remaining body.
 *
 * A frontmatter block must start on the very first line with a `---` fence and
 * be terminated by a later `---` (or `...`) fence line. If there is no valid
 * leading block, the whole document is returned as the body (so a `---`
 * horizontal rule in the body is never mistaken for frontmatter).
 */
export function splitFrontmatter(raw: string): SplitResult {
  // Tolerate a leading BOM for detection.
  const text = raw.charCodeAt(0) === 0xfeff ? raw.slice(1) : raw;
  const lines = text.split('\n');

  if (lines.length === 0 || lines[0].trimEnd() !== '---') {
    return { frontmatter: null, body: raw };
  }

  let closeIdx = -1;
  for (let i = 1; i < lines.length; i++) {
    const trimmed = lines[i].trimEnd();
    if (trimmed === '---' || trimmed === '...') {
      closeIdx = i;
      break;
    }
  }
  if (closeIdx === -1) {
    return { frontmatter: null, body: raw };
  }

  const frontmatter = lines.slice(1, closeIdx).join('\n');
  const body = lines.slice(closeIdx + 1).join('\n');
  return { frontmatter, body };
}

/**
 * Recombines a (possibly empty) frontmatter block with a body into a single
 * document. Omits the fence block entirely when the frontmatter is empty.
 *
 * `recombine(splitFrontmatter(x))` round-trips for LF-newline documents.
 */
export function recombine(frontmatter: string | null, body: string): string {
  const fm = frontmatter && frontmatter.trim() ? frontmatter.replace(/\n+$/, '') : '';
  if (!fm) {
    return body;
  }
  return `---\n${fm}\n---\n${body}`;
}

/**
 * Restore `[[wikilink]]` brackets that Milkdown's markdown serializer escapes.
 *
 * `crepe.getMarkdown()` runs remark-stringify, which escapes the `[` and `]` of
 * `[[...]]` (and can escape interior `|`, `#`, or `\`) because `[[...]]` is not a
 * node in the editor schema — turning `[[John Doe]]` into `\[\[John Doe\]\]` and
 * breaking the link on save. This reverses that escaping so wikilinks survive an
 * edit round-trip.
 *
 * Matches `[[...]]` with an optional backslash before ANY of the four brackets
 * (so fully- or partially-escaped forms both restore) and un-escapes interior
 * `\\`, `\|`, and `\#`. Ordinary `[text](url)` links and lone escaped brackets
 * outside a `[[…]]` pair are left untouched.
 */
export function unescapeWikilinks(md: string): string {
  return md.replace(
    /\\?\[\\?\[([^\n\]]*?)\\?\]\\?\]/g,
    (_m, inner: string) => `[[${inner.replace(/\\([\\|#])/g, '$1')}]]`,
  );
}
