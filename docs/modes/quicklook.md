---
title: QuickLook Preview
description: Preview markdown in macOS Finder
order: 4
---

# QuickLook Preview (macOS)

mbr includes a QuickLook extension for macOS, allowing you to preview markdown files directly in Finder.

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
- External links open in your default browser
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
