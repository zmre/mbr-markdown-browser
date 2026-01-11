---
title: Quick Start
description: Get started with mbr in 5 minutes
---

# Quick Start

This guide gets you productive with mbr in 5 minutes.

## 1. Preview a Single File

The simplest use case - render a markdown file to HTML:

```bash
mbr README.md
```

This outputs HTML to stdout, which you can pipe to other tools or redirect to a file:

```bash
mbr README.md > output.html
```

## 2. Browse Your Notes

Start a local web server to browse and search your markdown files:

```bash
mbr -s ~/notes
```

Open [http://127.0.0.1:5200/](http://127.0.0.1:5200/) in your browser.

**What you get:**

- Live-updating preview as you edit files
- Directory navigation with file lists
- Full-text search across all files
- Tag browsing (from YAML frontmatter)
- Recent files tracking

**Keyboard shortcuts to try:**

| Key | Action |
|-----|--------|
| `-` | Open file browser sidebar |
| `/` | Open search dialog |
| `Escape` | Close sidebar or search |

## 3. Launch the GUI

For a native desktop experience:

```bash
mbr -g ~/notes  # open index.md by default or browse the dir
# OR
mbr -g ~/notes/my-note.md  # preview a specific file
```

This opens a native window with:

- Native menu bar (File, Edit, View, History)
- Keyboard shortcuts (Cmd+O to open folder, Cmd+R to reload)
- History navigation (Cmd+[ and Cmd+])
- Developer tools (Cmd+Option+I)

## 4. Build a Static Site

Generate a deployable website:

```bash
mbr -b ~/notes
```

This creates a `build/` directory with:

```
build/
├── index.html              # Home page
├── docs/guide/index.html   # Rendered markdown
├── .mbr/
│   ├── site.json           # Site metadata
│   ├── pagefind/           # Search index
│   └── *.css, *.js         # Assets
└── images/ → ../images     # Symlinked assets
```

Deploy the `build/` folder to any static host (GitHub Pages, Netlify, etc.).

### Custom Output Directory

```bash
mbr -b --output ./public ~/notes
```

## 5. Add Custom Styling

Create a `.mbr/` folder in your notes directory:

```bash
mkdir ~/notes/.mbr
```

Add a `user.css` file to customize colors:

```css
/* ~/notes/.mbr/user.css */
:root {
  --pico-primary: #8B5CF6;
  --pico-primary-hover: #7C3AED;
}
```

Reload the page to see your changes.

## 6. Use YAML Frontmatter

Add metadata to your markdown files:

```yaml
---
title: My Guide
description: A helpful guide to getting started
tags: guide, documentation, tutorial
date: 2025-01-09
---

# Content starts here
```

This metadata powers:

- Page titles in the browser tab
- Search results with descriptions
- Tag-based navigation

## Common Workflows

### Writing with Live Preview

```bash
# Terminal 1: Start server
mbr -s ~/notes

# Terminal 2: Edit files
vim ~/notes/draft.md
```

Pages reload when specific changes (to the current markdown file or any css, for example) are detected.

### Building Documentation

```bash
# Serve locally during development
mbr -s ./docs

# Build for deployment
mbr -b --output ./public ./docs
```

## Next Steps

- [Modes of Operation](../modes/) - Deep dive into each mode
- [Customization](../customization/) - Themes, templates, and components
- [Markdown Extensions](../markdown/) - Extended syntax reference
- [Integrations](../integration) - See how to use mbr with Obsidian or other programs
