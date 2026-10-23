---
title: QuickLook Preview
description: Preview markdown and plain text in macOS Finder, and show rendered markdown thumbnails
order: 4
---

# QuickLook Preview (macOS)

mbr includes two QuickLook extensions for macOS: a preview extension, which renders markdown (and plain text files) when you press Space in Finder, and a thumbnail extension, which draws markdown files' icons in Finder as miniatures of the rendered page.

## What is QuickLook?

QuickLook is a macOS feature that provides instant file previews. Press **Space** on any file in Finder to see its contents without opening an application.

With mbr's QuickLook extension, markdown files render as formatted HTML instead of plain text.

## Features

- **Full rendering**: Headers, lists, tables, code blocks
- **Syntax highlighting**: Colored code with language detection
- **Mermaid diagrams**: Flowcharts and diagrams render inline
- **Math equations**: LaTeX rendering (if configured)
- **Custom styling**: Respects your `.mbr/theme.css`
- **Table of contents**: In the QuickLook info panel
- **Frontmatter display**: Shows metadata in a clean format
- **Plain text and source files**: Shown verbatim, with syntax highlighting where available
- **Contact cards**: `type: person` and `type: organization` notes get the same [contact card](../markdown/contacts.md) as in mbr (see [Contacts](#contacts) below)

## Thumbnails

Finder's icon and gallery views, the column-view preview pane, and Get Info show a *thumbnail* of a file rather than a preview. For markdown files, mbr's thumbnail extension draws one of two things:

- **The cover image**, when the note's frontmatter names one as `cover:` or `image:` (checked in that order). The value may be relative to the note (`cover: images/cover.png`) or root-relative (`cover: /images/cover.png`, which also resolves through the `static_folder` overlay). It must be a local image (PNG, JPEG, GIF, WebP, HEIC/HEIF, AVIF, TIFF or BMP; not SVG) inside the repository and no larger than 16 MB. A URL, a file outside the repository, or anything else that does not qualify is skipped, and the next key or the rendered page is used instead.
- **The rendered page** otherwise: the top of the same page the preview shows, with your `.mbr/theme.css` and `user.css`, laid out at 800 pixels wide on a letter-shaped sheet and scaled down.

A few things differ from the preview, on purpose:

| | Preview | Thumbnail |
|-|---------|-----------|
| File types | Markdown and plain text | Markdown only. Text and source files keep the system's own thumbnails |
| Appearance | Follows light/dark mode | Always light. macOS caches a thumbnail once, whatever the mode was when it was drawn, and document thumbnails are paper-white by convention |
| Remote images and embeds | Loaded | Never loaded (they show as empty boxes): the thumbnail's web view blocks every remote load |
| Mermaid diagrams | Rendered | Shown as their source |
| Source read | Whole file | The first 32 KB, which is far more than fits on the visible page |
| File-type badge | n/a | "MD", drawn by Finder in its standard badge style (`QLThumbnailReply.extensionBadge`) wherever it badges icons; larger renderings may show none |
| Smallest size | n/a | Icons under 32 points (list view, sidebar) use the generic markdown icon |

Thumbnails are drawn by the same WebKit engine as previews. The extension keeps two web views alive between requests, so after the first thumbnail (around 200 ms while the web view starts) each one takes about 10 ms. If a page cannot be drawn within a few seconds, or anything else fails, Finder shows the generic document icon instead; mbr never caches an error message as a thumbnail.

### Turning thumbnails on

After installing or updating `MBR.app` and launching it once:

1. **Log out and back in, or restart.** macOS's thumbnail service only looks for new thumbnail extensions when it starts, so a freshly installed or updated one is not used until then. Previews do not have this problem.
2. **Make sure the extension is enabled.** The Quick Look checkbox in **System Settings → General → Login Items & Extensions → Quick Look** reflects mbr's preview extension and may not turn thumbnails on. If thumbnails still do not appear, enable the thumbnail extension directly:
   ```bash
   pluginkit -e use -i com.zmre.mbr.MBRThumbnail
   ```
3. **Clear old thumbnails** with `qlmanage -r cache` (see below).

### Refreshing old thumbnails

Finder caches thumbnails, so markdown files it has already drawn keep their old plain-text icons until the cache is cleared:

```bash
qlmanage -r cache
```

The same applies after you change a note's theme or cover image: Finder redraws a thumbnail when the file itself changes, not when a file it references does.

## Plain Text Previews

The extension previews plain text as well as markdown. Which path a file takes is decided by its extension:

| File | Rendered as |
|------|-------------|
| `.md`, `.markdown`, `.mkd`, `.mkdn`, `.mdown`, `.mdwn`, `.mdtxt`, `.mdtext`, `.mdoc` | Full markdown |
| Anything listed in your `markdown_extensions` config | Full markdown |
| A source file mbr ships a grammar for (`.rs`, `.py`, `.json`, `.ts`, `.go`, `.rb`, `.sh`, `.nix`, `.sql`, `.yaml`, `.css`, `.xml`, ...) | Verbatim, syntax highlighted |
| Everything else (`.txt`, `.log`, unknown extensions, files with no extension) | Verbatim, monospace |

Verbatim means verbatim: whitespace, tabs and every literal character are preserved exactly, and markdown syntax in a `.txt` file stays as typed rather than being parsed. Long lines scroll horizontally instead of wrapping, so what you see matches what is on disk.

Markdown extensions are the union of the built-in list above and your `markdown_extensions` setting. The built-in list is always honored, so a `.mkdn` file previews as markdown even in a repository whose config only publishes `.md`.

### Limits

Previews are meant to be instant, so two caps apply:

| Cap | Value | Effect |
|-----|-------|--------|
| Bytes read | 1 MB | Longer files are cut off and the preview says "Preview truncated at 1024 KB" |
| Bytes highlighted | 256 KB | Larger files still render, just without syntax highlighting |

Files that are not valid UTF-8 (a latin-1 log, say) are decoded leniently: undecodable bytes become the Unicode replacement character rather than failing the preview. Empty files produce an empty preview.

## File Types mbr Registers For

Installing `MBR.app` registers these claims with macOS Launch Services:

| Claim | Rank | Consequence |
|-------|------|-------------|
| Markdown (`net.daringfireball.markdown`, plus the nine extensions above) | `Default` | **mbr becomes the default app for markdown files**, replacing your previous default |
| Plain text (`public.plain-text`) | `Alternate` | mbr appears under "Open With" for text files but does *not* displace your text editor |

Both are registered as a **Viewer**, never an Editor: mbr renders the file it is given and never claims the ability to edit it in place.

If you would rather keep your existing markdown app as the default, change it back in Finder: select a markdown file, **Get Info** (Cmd+I), pick your app under "Open with", and click **Change All**. That user choice outranks any app's declared handler rank.

macOS itself declares no markdown type, so `MBR.app` also carries a `UTImportedTypeDeclarations` entry for `net.daringfireball.markdown`. Without it, `.md` files would resolve to an anonymous dynamic type and no UTI-based claim - including the QuickLook one - could match.

`public.plain-text` is a supertype, so the plain-text claim is broader than `.txt`: source code conforms to it, which brings in roughly ninety extensions (`.c`, `.py`, `.rb`, `.sh`, `.swift`, `.js`, `.java`, ...) plus `.log`, `.csv` and `.tsv`. It does **not** cover `.json`, `.yaml`, `.css`, `.html` or `.xml` (which conform to `public.text` directly rather than to `public.plain-text`), nor extensions macOS declares no type for at all, such as `.rs`, `.toml` and `.nix`. Those files still preview correctly when opened through mbr; they just do not route to mbr automatically from Finder.

## Installation

The QuickLook extension is bundled with the macOS app.

### Automatic Registration

1. Download and install `MBR.app`
2. Launch the app once
3. The extension registers automatically

### Manual Registration

If the extension doesn't appear:

```bash
# List registered QuickLook preview and thumbnail extensions
pluginkit -mv -p com.apple.quicklook.preview | grep mbr
pluginkit -mv -p com.apple.quicklook.thumbnail | grep mbr

# Reload QuickLook
qlmanage -r
qlmanage -r cache
```

Do **not** use `qlmanage -m plugins` to check for mbr. That command lists only
the legacy `.qlgenerator` bundles under `/System/Library/QuickLook`; it shows no
modern app extension at all, so it prints nothing for mbr — or for Apparency,
Fantastical or any other working `.appex` — whether or not the extension is
installed and working. `pluginkit` is the only tool that sees app extensions.

The extension registers with the system when **the containing app is launched**
from a real install location. Launching `MBR.app` out of `/nix/store`, a build
output directory, or a disk image does not register it: macOS only tracks
extensions for apps under `/Applications` (or `~/Applications`). Move the app
first, then launch it once.

### Verify Installation

1. Open Finder
2. Navigate to a folder with `.md` files
3. Select a markdown file
4. Press **Space**

You should see rendered markdown instead of plain text.

## How It Works

```mermaid
flowchart LR
    FINDER[Finder] --> QL[QuickLook System]
    QL --> EXT[mbr Extension]
    EXT --> PARSE[Parse Markdown]
    PARSE --> RENDER[Render HTML]
    RENDER --> PREVIEW[Preview Window]
```

The extension:

1. Receives the file path from QuickLook
2. Searches upward for `.mbr/` configuration folder
3. Parses markdown with full extension support
4. Renders HTML with inlined CSS/JS (self-contained)
5. Attaches the local files the page references (images, video, PDFs)
6. Returns the preview to Finder

## Configuration

The QuickLook extension respects your repository's `.mbr/` configuration:

### Custom Theme

Your `theme.css` applies to QuickLook previews:

```css
/* .mbr/theme.css */
:root {
  --pico-primary: #8B5CF6;
}
```

### Custom User Styles

Additional styles from `user.css` are included:

```css
/* .mbr/user.css */
h1 { border-bottom: 2px solid var(--pico-primary); }
```

## Differences from Full App

The QuickLook preview is simplified compared to the full mbr experience:

| Feature | QuickLook | Full App |
|---------|-----------|----------|
| Markdown rendering | Yes | Yes |
| Syntax highlighting | Yes | Yes |
| Mermaid diagrams | Yes | Yes |
| Navigation | No | Yes |
| Search | No | Yes |
| Live reload | No | Yes |
| Link following | Limited | Yes |

### Contacts

A `type: person` or `type: organization` note previews with its contact card at
the top, the same card mbr draws (a customized `.mbr/_contact_card.html` is not
used here; the preview has its own fixed template). Its portrait (`image:`)
follows the asset rules below: relative to the note's folder, inside the
repository and within the size limits, or it is not shown.

The preview knows only the one file, so the card differs in two ways:

- **Relationships** are the ones the note declares itself (including the
  `employer` its `company: "[[…]]"` implies), shown from its point of view, as
  plain text. Edges declared on *other* notes (George's `parent: [[Jane]]`
  showing up as Jane's child) and links to related notes appear only in mbr.
  With `relationship_tracking = false` the section is left off, as in mbr.
- **No "Linked from N notes" chip**: counting backlinks needs the repository.

### Images and Other Local Assets

A QuickLook preview is a single self-contained document, so any local file it
shows has to travel with it. Two consequences:

- **Assets must live inside the repository.** Relative URLs, in markdown
  (`![diagram](x.png)`, `![diagram](../images/x.png)`, `![[x.png]]`) or raw
  HTML (`<img src="x.png">`, each URL of a `<source srcset="x.png 1x, x@2x.png 2x">`), are resolved against the folder the note is in,
  as GitHub or an editor would, and root-relative ones (`![diagram](/images/x.png)`) against the
  repository root; both also resolve through the `static_folder` overlay. A
  file outside the repository — by a `../` climbing above the root, or a
  symlink pointing out of it — is not shown.
- **Large assets are skipped.** A single file over 16 MB, or one that would push
  a page's assets past 64 MB in total, is left out rather than read into the
  preview. Assets are attached in document order, so what is at the top of the
  page is what fits. Open the file in mbr to see the rest.

### Link Behavior

In QuickLook:
- Internal links are disabled (no navigation)
- External links do **not** open your browser. The preview extension installs no
  navigation policy of its own, so a click is handled by the preview's web view
  or ignored — press Space again to dismiss the preview and open the file in mbr
  to follow the link. [GUI mode](gui.md#external-links) is where external and
  application-scheme links are handed to the system
- Anchor links scroll within the preview

## Troubleshooting

### Extension Not Working

If markdown files show as plain text:

1. **Verify registration** (`+` means elected for use):
   ```bash
   pluginkit -mv -p com.apple.quicklook.preview | grep -i mbr
   ```

   Nothing listed? Launch `MBR.app` once from `/Applications`. See
   [Manual Registration](#manual-registration) — and note that
   `qlmanage -m plugins` cannot answer this question.

2. **Reset QuickLook**:
   ```bash
   qlmanage -r
   qlmanage -r cache
   ```

3. **Check the extension is not being rejected**:
   ```bash
   log show --last 5m --info --debug | grep "mis-configured plugin"
   ```

   `plug-ins must be sandboxed` means the extension was signed without the
   `com.apple.security.app-sandbox` entitlement. PlugInKit refuses to load it
   and reports nothing to the user. Confirm the shipped bundle kept its
   entitlements:
   ```bash
   codesign -d --entitlements - /Applications/MBR.app/Contents/PlugIns/MBRPreview.appex
   ```

4. **Check for a crash**:
   ```bash
   ls ~/Library/Logs/DiagnosticReports | grep MBRPreview
   ```

   If the extension crashes, QuickLook silently falls back to the system
   plain-text preview — the symptom is identical to not being installed.

5. **Check for conflicts**: other extensions claiming `public.plain-text`
   (which markdown conforms to) may take precedence. List all of them with
   `pluginkit -mv -p com.apple.quicklook.preview`, and disable one with
   `pluginkit -e ignore -i <identifier>` (restore with `-e default`).

### Markdown Icons Still Show Plain Text

1. **Log out and back in** (or restart) after installing or updating `MBR.app`: the thumbnail service only discovers new thumbnail extensions when it starts.
2. **Enable the extension**: `pluginkit -e use -i com.zmre.mbr.MBRThumbnail` (the System Settings Quick Look checkbox may only cover previews). See [Turning thumbnails on](#turning-thumbnails-on).
3. **Clear the thumbnail cache** with `qlmanage -r cache`. Icons Finder drew before the extension was enabled are cached.
4. **Check the timings.** Each thumbnail is logged with how long it took:
   ```bash
   log stream --info --predicate 'subsystem == "com.zmre.mbr.MBRThumbnail"'
   ```
   No lines at all means Finder is not using the extension (steps 1 and 2). `thumbnail failed` lines say why a file fell back to the generic icon.

`qlmanage -t` is not a reliable test: on recent macOS it draws text-like files with a built-in renderer and never consults thumbnail extensions.

### Text Files Preview With the System Renderer

For `.txt` and source files, mbr's extension competes with the previewer built into macOS, and both claim `public.plain-text`. If you get Apple's plain preview instead of mbr's, confirm the extension is enabled in **System Settings -> General -> Login Items & Extensions -> Quick Look**, then reset the cache with `qlmanage -r && qlmanage -r cache`.

### Wrong Default App for Markdown

Installing mbr makes it the default opener for markdown files. To hand that back to another app, select a markdown file in Finder, press **Cmd+I**, choose the app under "Open with", and click **Change All**.

### Slow Previews

Large files or complex diagrams may slow previews:

1. Files over 1MB may take longer
2. Many Mermaid diagrams add processing time
3. External resources (if any) require network

### Wrong Styling

If styles don't match expectations:

1. Verify `.mbr/` folder is in parent directory
2. Check CSS syntax in `theme.css` / `user.css`
3. Reset QuickLook cache: `qlmanage -r cache`

### Extension Disabled by macOS

If macOS disables the extension:

1. Open **System Preferences** → **Privacy & Security**
2. Look for mbr in the security prompts
3. Allow the extension to run

## Removing the Extension

To uninstall the QuickLook extensions:

Both extensions live inside `MBR.app` and are not separately installable, so
there is no standalone plugin file to delete — removing the app removes them.
To keep the app but stop it drawing thumbnails, turn **MBR Thumbnail** off in
**System Settings → General → Login Items & Extensions → Quick Look**. To stop
it previewing, turn off **MBR Preview** there, or:

```bash
# Find where it is registered
pluginkit -mv -p com.apple.quicklook.preview | grep mbr

# Stop macOS using it (reversible with -e default)
pluginkit -e ignore -i com.zmre.mbr.MBRPreview

# Reload QuickLook
qlmanage -r && qlmanage -r cache
```

Or simply delete `MBR.app` from Applications.
