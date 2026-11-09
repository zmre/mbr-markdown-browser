---
title: Links and Wikilinks
description: Relative links, links without extensions, wikilinks and how mbr resolves them, and links to tags and viewers
order: 2
---

# Links and Wikilinks

mbr reads links the way GitHub and most editors do: a relative link is relative
to the **file** it is written in. Write them as you always have; mbr rewrites
each one for the URL the page is served at, in the server, the GUI and static
builds alike.

## Links between notes

All of these, written in `docs/guide.md`, reach `docs/other.md`:

```markdown
[Other](other.md)
[Other](other)
[Other](other/)
[Other](./other.md)
```

| You write | Lands on |
|-----------|----------|
| `other.md`, `other`, `other/` | the page for `docs/other.md` |
| `subfolder/index.md`, `subfolder/` | the folder's index page, or its listing |
| `../folder/file.md` | the page for `folder/file.md` |
| `photo.png`, `report.pdf`, `Makefile` | the file itself |
| `/other`, `/other.md`, `/other/` | the page for `other.md` at the root (root-relative) |
| `../projects` (a folder) | the folder's index note, or its listing |
| `https://…`, `mailto:…` | off-site, untouched |

Every spelling is written out at the page's canonical URL (`…/other/`), so the
link works the same in the server, the GUI and a static build. An
extension-less target that is a real file (`[license](../LICENSE)`) is left as
written.

Every page is served at a folder-style URL with a trailing slash
(`docs/guide.md` → `/docs/guide/`), and mbr adjusts each relative link for it,
so you never write the extra `../` yourself. The details, and why the slash
matters, are in [Architecture](../reference/architecture/#links-and-url-conventions).

**Links without an extension.** `[x](../folder/file)` could name a note
(`file.md`) or a file literally called `file` (`LICENSE`, `Makefile`). mbr
asks the repository which one exists, so both work. Obsidian- and zk-style
extension-less links need no changes.

**Links to headings.** Add the heading's anchor: `[Install](guide.md#install)`,
or `#install` on the same page. Anchors are the heading text lowercased, with
spaces as dashes and punctuation dropped (`## Two Words!` → `#two-words`); give
a heading its own with `## Install {#setup}`.

**Paths with spaces.** CommonMark does not allow a bare space in a link
destination. Wrap it in angle brackets, `[Notes](<meeting notes.md>)`, or
encode it, `[Notes](meeting%20notes.md)`.

**Raw HTML.** A relative `href` or `src` in raw HTML (`<a href="other.md">`,
`<img src="images/hero.png">`) is resolved exactly like the markdown form.

## Wikilinks

```markdown
[[Meeting Notes]]
[[Meeting Notes|last week's meeting]]
[[Meeting Notes#Action items]]
```

A wikilink names a note rather than a path. mbr finds it the way Obsidian does:

1. **A note with that file name in the current folder.** Case does not matter:
   `[[japan]]` finds `Japan.md`.
2. **Otherwise, anywhere in the repository**, matching in order: a note's
   `title`, then its [`aliases`](frontmatter/#aliases), then its file
   name.
3. **Otherwise** the link points at a page that does not exist, and the page's
   problem panel reports it.

Text after `|` is what the reader sees; text after `#` is a heading on the
target page. A wikilink with a `/` in it (`[[projects/plan]]`) is a path,
resolved relative to the current file like an ordinary link.

When several notes answer to one name, mbr picks the one with the
alphabetically first URL and reports the ambiguity in the page-problems panel
of the page containing the link — the only place that can say which note
*that* link reached, since resolution is current-folder first. Rename one of
the notes or give the link a path to settle it. See
[Data problems mbr reports](../features/relationships/#data-problems-mbr-reports).

Wikilinks count as links everywhere: backlinks, the link explorer (`f` / `F`)
and the link graph all see them.

## Links to tag pages

```markdown
[[Tags:rust]]
[Rust posts](Tags:rust)
```

Both link to the tag page `/tags/rust/`. Any configured tag source works
(`[[Performers:Joshua Jay]]`). See [Tags](../features/tags/#wikilink-syntax).

## Bare URLs

URLs in angle brackets become links:

```markdown
<https://example.com>
<user@example.com>
```

A bare URL alone on its own line becomes an **embed** instead: a YouTube player,
a Giphy image, or a link card with the page's title and image. See
[Media](media/#youtube-embedding).

## Links to media viewers

To link to mbr's full-page viewer for a video, PDF, audio file or image instead
of embedding it:

```markdown
[Watch the demo](/.mbr/videos/?path=%2Fvideos%2Fdemo.mp4)
```

See [Linking to a Viewer Page](media/#linking-to-a-viewer-page).

## Broken links

In server and GUI mode, the page-problems panel lists every link on the page
whose target does not exist. `mbr -b` checks every link in the site and reports
them; `--fail-on-broken-links` makes that an error for CI. See
[Build Mode](../modes/build/#6--validate-links).
