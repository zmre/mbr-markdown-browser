---
title: QuickLook Preview
description: Preview markdown and plain text in macOS Finder
order: 4
---

# QuickLook Preview (macOS)

mbr includes a QuickLook extension for macOS, allowing you to preview markdown - and plain text files - directly in Finder.

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
# List installed QuickLook generators
qlmanage -m plugins | grep mbr

# Reload QuickLook
qlmanage -r
```

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
5. Returns the preview to Finder

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

1. **Verify installation**:
   ```bash
   qlmanage -m plugins | grep -i mbr
   ```

2. **Reset QuickLook**:
   ```bash
   qlmanage -r
   qlmanage -r cache
   ```

3. **Check for conflicts**:
   ```bash
   qlmanage -m plugins | grep -i markdown
   ```

   Other markdown QuickLook extensions may take precedence.

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

To uninstall the QuickLook extension:

```bash
# Find the extension location
qlmanage -m plugins | grep mbr

# Remove the extension (path from above command)
sudo rm -rf /path/to/mbr.qlgenerator

# Reload QuickLook
qlmanage -r
```

Or simply delete `MBR.app` from Applications.
